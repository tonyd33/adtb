//! Host functions: the bodies an embedding program supplies at open, checked against the
//! schema's declarations and bound before migrations run.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use adtb_lang::ast::{FnBody, OperatorKind};
use adtb_lang::kernel::{Kind, Terms, Tm};
use adtb_lang::norm::NormalProgram;
use adtb_lang::parse::{Options as ParseOptions, parse_migration};
use adtb_lang::schema::{FunctionDef, Schema};
use adtb_lang::types::FnId;

use crate::batch::Batch;
use crate::catalog::layout::batch_type;
use crate::exec::{HostBody, HostError, Hosts};

use super::{Log, OpenError};

/// Whether a host function returns the same result whenever it is given the same arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Determinism {
    Deterministic,
    Volatile,
}

#[derive(Clone)]
struct Supplied {
    version: String,
    det: Determinism,
    body: Arc<HostBody>,
}

/// The host functions an embedding program supplies at open, by the name each declaration
/// registers its body under.
#[derive(Clone, Default)]
pub struct HostFns(BTreeMap<String, Supplied>);

impl fmt::Debug for HostFns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.iter().map(|(n, s)| (n, (&s.version, s.det))))
            .finish()
    }
}

impl HostFns {
    pub fn new() -> HostFns {
        HostFns::default()
    }

    /// Supply the body of the host function the schema declares as `host "name" version
    /// "version"`, replacing any body supplied for `name` before.
    ///
    /// `f` takes one column per parameter, all of one length, and returns one column of the
    /// result type of that length. It may be called from several threads at once. An error or
    /// a panic is a fault of the statement evaluating the call.
    pub fn register(
        &mut self,
        name: &str,
        version: &str,
        det: Determinism,
        f: impl Fn(&Batch) -> Result<Batch, HostError> + Send + Sync + 'static,
    ) {
        self.0.insert(
            name.to_owned(),
            Supplied {
                version: version.to_owned(),
                det,
                body: Arc::new(f),
            },
        );
    }

    /// Returns the name and version of each function supplied, as `name version`.
    pub(super) fn names(&self) -> Vec<String> {
        self.0
            .iter()
            .map(|(n, s)| format!("{n} {}", s.version))
            .collect()
    }

    /// Returns the bodies to bind, by name.
    pub(super) fn hosts(&self) -> Hosts {
        let mut out = Hosts::default();
        for (name, s) in &self.0 {
            out.insert(name.clone(), s.version.clone(), s.body.clone());
        }
        out
    }

    /// Refuse a supplied function whose name `schema` declares with a host body at another
    /// determinism, at a type no batch column holds, or at a version other than the declared
    /// one, unless a migration of `pending` redefines it at the supplied version.
    pub(super) fn check(&self, schema: &Schema, pending: &[&str]) -> Result<(), OpenError> {
        for (name, version, s, ok) in self.declared(schema) {
            if !ok || (s.version != version && !redefined(pending, name, &s.version)) {
                return Err(OpenError::HostFnMismatch {
                    name: name.to_owned(),
                });
            }
        }
        Ok(())
    }

    /// Refuse, once migrations have applied, what [`check`](HostFns::check) refuses with no
    /// migration pending, and a declared host function with no body when `normal` reaches it
    /// from an index, a constraint, a check, a stored view, a default or a computed field.
    pub(super) fn bound(&self, schema: &Schema, normal: &NormalProgram) -> Result<(), OpenError> {
        self.check(schema, &[])?;
        let reached = reached(schema, normal);
        for (i, def) in schema.functions.iter().enumerate() {
            if let FnBody::Host { name, .. } = &def.body
                && !self.0.contains_key(name)
                && reached.contains(&FnId(i as u32))
            {
                return Err(OpenError::HostFnMissing { name: name.clone() });
            }
        }
        Ok(())
    }

    /// Returns each supplied function `schema` declares with a host body: its name, its declared
    /// version, what was supplied, and whether the supplied determinism and the declared types
    /// agree with the declaration.
    fn declared<'s>(
        &'s self,
        schema: &'s Schema,
    ) -> impl Iterator<Item = (&'s str, &'s str, &'s Supplied, bool)> {
        schema.functions.iter().filter_map(move |def| {
            let FnBody::Host {
                name,
                version,
                deterministic,
            } = &def.body
            else {
                return None;
            };
            let s = self.0.get(name)?;
            let det = (s.det == Determinism::Deterministic) == *deterministic;
            Some((
                name.as_str(),
                version.as_str(),
                s,
                det && mapped(schema, def),
            ))
        })
    }
}

/// Returns whether every parameter and the result of `def` have a batch column type.
fn mapped(schema: &Schema, def: &FunctionDef) -> bool {
    def.params
        .iter()
        .map(|(_, t)| t)
        .chain([&def.result])
        .all(|t| batch_type(schema, t).is_some())
}

/// Returns whether a migration of `pending` redefines a function with host body `name` at
/// `version`.
fn redefined(pending: &[&str], name: &str, version: &str) -> bool {
    pending.iter().any(|text| {
        let Some(file) = parse_migration(text, ParseOptions::default()).ok().ok() else {
            return false;
        };
        file.operators.iter().any(|op| match &op.kind {
            OperatorKind::RedefineFn(f) => matches!(
                &f.body,
                FnBody::Host { name: n, version: v, .. } if n == name && v == version
            ),
            _ => false,
        })
    })
}

/// Returns the functions called from the stored parts of `normal`: its constraints, checks,
/// domains, retags, defaults, computed fields, index fields and stored views.
fn reached(schema: &Schema, normal: &NormalProgram) -> BTreeSet<FnId> {
    let mut roots: Vec<Tm> = Vec::new();
    roots.extend(&normal.constraints);
    roots.extend(&normal.checks);
    roots.extend(normal.domains.values().map(|(_, t)| *t));
    roots.extend(normal.retags.values().map(|(_, t)| *t));
    roots.extend(normal.defaults.values());
    roots.extend(normal.computed.values().map(|(_, t)| *t));
    roots.extend(normal.index_fields.values().map(|(_, f)| f.value));
    for (f, body) in normal.functions.iter().enumerate() {
        if let Some((_, t)) = body
            && schema.functions[f].stored.is_some()
        {
            roots.push(*t);
        }
    }
    calls(&normal.terms, roots)
}

/// Returns every function a `Call` reachable from `roots` names.
fn calls(terms: &Terms, roots: Vec<Tm>) -> BTreeSet<FnId> {
    let mut seen = BTreeSet::new();
    let mut out = BTreeSet::new();
    let mut todo = roots;
    while let Some(t) = todo.pop() {
        if !seen.insert(t) {
            continue;
        }
        if let Kind::Call { func, .. } = terms.kind(t) {
            out.insert(*func);
        }
        todo.extend(terms.children(t));
    }
    out
}

/// Returns the migrations of `log` past the first `applied`.
pub(super) fn pending(log: &Log, applied: usize) -> Vec<&str> {
    log.migrations
        .iter()
        .skip(applied)
        .map(String::as_str)
        .collect()
}
