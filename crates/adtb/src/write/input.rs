//! What a statement reads besides the database file: the rows of its transaction's temp
//! relations and the records of the files it scans.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use adtb_file::{Files, Record};
use adtb_lang::check::TEMP_MODULE;
use adtb_lang::kernel::{FileFormat, Kind, Prim, Terms, Tm, Var};
use adtb_lang::schema::Schema;
use adtb_lang::types::{CtorSet, RelId, Ty};

use crate::exec::Val;
use crate::exec::types::key_fields;

use super::eval::Eff;
use super::{Error, Rejection};

/// What one transaction holds outside the database file: the rows of its temp relations and
/// the files its scans read.
#[derive(Clone, Debug, Default)]
pub struct Local {
    /// The rows of each temp relation, by key.
    temps: BTreeMap<RelId, BTreeMap<Val, Val>>,
    /// The files each scanned path named, by name, with their BLAKE3 checksums.
    pins: BTreeMap<String, Vec<(String, [u8; 32])>>,
}

/// What a variable [`rewrite`] introduced stands for.
#[derive(Clone, Debug)]
pub enum Input {
    /// The rows of temp relation `rel` of the constructors `ctors`, or of all.
    Temp { rel: RelId, ctors: Option<CtorSet> },
    /// The rows of the files `path` names, read in `format` at `row`.
    File {
        format: FileFormat,
        path: Tm,
        row: Ty,
    },
}

/// Returns whether `rel` is a temp relation of `schema`.
pub fn is_temp(schema: &Schema, rel: RelId) -> bool {
    schema.module(schema.relation(rel).module).name == TEMP_MODULE
}

/// Replaces each read of a temp relation and each file scan reachable from `root` by a variable
/// of its own. Returns what each variable stands for.
pub fn rewrite(schema: &Schema, terms: &mut Terms, root: Tm) -> BTreeMap<Var, Input> {
    let mut out = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut todo = vec![root];
    while let Some(t) = todo.pop() {
        if !seen.insert(t) {
            continue;
        }
        let input = match terms.kind(t) {
            Kind::Rel(r, s) if is_temp(schema, *r) => Some(Input::Temp {
                rel: *r,
                ctors: s.clone(),
            }),
            Kind::Prim(Prim::ReadFile { format, .. }, args) => {
                let Ty::Set(row) = terms.ty(t) else {
                    panic!("a file scan is a set, by the typing of `ReadFile`")
                };
                Some(Input::File {
                    format: *format,
                    path: args[0],
                    row: row.as_ref().clone(),
                })
            }
            _ => None,
        };
        todo.extend(terms.children(t));
        if let Some(input) = input {
            let v = terms.var("input·", terms.ty(t).clone());
            terms.nodes[t.0 as usize].kind = Kind::Var(v);
            out.insert(v, input);
        }
    }
    out
}

/// Returns the key of `row`, a row of temp relation `rel`: its one key field's value, or the
/// record of its key fields.
pub fn temp_key(schema: &Schema, rel: RelId, row: &Val) -> Result<Val, Error> {
    let (at, single) = key_fields(schema, rel, &schema.relation(rel).elem)?;
    let c = row.ctor_of()?;
    let fs = row.fields()?;
    let mut parts = Vec::with_capacity(at.len());
    for a in &at {
        parts.push(fs[a.of(c)?].clone());
    }
    Ok(if single {
        parts.swap_remove(0)
    } else {
        Val::record(parts)
    })
}

impl Local {
    /// Put back the temp relation rows of `before`, keeping every file pinned since.
    pub fn restore_temps(&mut self, before: Local) {
        self.temps = before.temps;
    }

    /// Returns the rows of temp relation `rel` of the constructors `ctors`, or of all.
    pub fn rows(&self, rel: RelId, ctors: Option<&CtorSet>) -> Result<BTreeSet<Val>, Error> {
        let mut out = BTreeSet::new();
        for v in self.temps.get(&rel).into_iter().flat_map(BTreeMap::values) {
            if let Some(s) = ctors
                && !s.contains(&v.ctor_of()?)
            {
                continue;
            }
            out.insert(v.clone());
        }
        Ok(out)
    }

    /// Returns the row of temp relation `rel` with key `key`.
    pub fn by_key(&self, rel: RelId, key: &Val) -> Option<&Val> {
        self.temps.get(&rel).and_then(|rows| rows.get(key))
    }

    /// Applies the effects of `effects` on temp relations as one change set.
    ///
    /// Fails with `KeyViolation`, changing nothing, when an inserted row's key is held by a row
    /// the change set keeps, or two rows of the result share a key.
    pub fn apply(&mut self, schema: &Schema, effects: &[&Eff]) -> Result<(), Error> {
        let mut plus: BTreeSet<(RelId, Val)> = BTreeSet::new();
        let mut minus: BTreeSet<(RelId, Val)> = BTreeSet::new();
        let mut inserted: Vec<(RelId, Val)> = Vec::new();
        for e in effects {
            match e {
                Eff::Ins(r, v) => {
                    plus.insert((*r, v.clone()));
                    inserted.push((*r, v.clone()));
                }
                Eff::Del(r, v) => {
                    minus.insert((*r, v.clone()));
                }
                Eff::Rep(r, v, n) => {
                    minus.insert((*r, v.clone()));
                    plus.insert((*r, n.clone()));
                }
            }
        }
        let violation = |relation: RelId, key: Val| {
            Err(Error::Rejected(Rejection::KeyViolation { relation, key }))
        };
        for (r, v) in &inserted {
            let k = temp_key(schema, *r, v)?;
            if self
                .by_key(*r, &k)
                .is_some_and(|held| !minus.contains(&(*r, held.clone())))
            {
                return violation(*r, k);
            }
        }
        let mut next = self.temps.clone();
        for (r, v) in &minus {
            let k = temp_key(schema, *r, v)?;
            let rows = next.entry(*r).or_default();
            if rows.get(&k) == Some(v) {
                rows.remove(&k);
            }
        }
        let mut placed: BTreeSet<(RelId, Val)> = BTreeSet::new();
        for (r, v) in plus {
            let k = temp_key(schema, r, &v)?;
            if !placed.insert((r, k.clone())) {
                return violation(r, k);
            }
            next.entry(r).or_default().insert(k, v);
        }
        self.temps = next;
        Ok(())
    }

    /// Returns the records of the files `path` names, read through `files` in `format` at
    /// `row`, pinning them to the transaction.
    ///
    /// Fails with `FileChanged` when the transaction read `path` before and the files it names,
    /// or their contents, differ.
    pub fn scan(
        &mut self,
        schema: &Schema,
        files: &dyn Files,
        format: FileFormat,
        path: &str,
        row: &Ty,
    ) -> Result<Vec<Record<Val>>, Error> {
        let unreadable = |source: String, detail: String| {
            Error::Rejected(Rejection::BadInput {
                source,
                line: 0,
                detail,
            })
        };
        let found = files
            .read(path)
            .map_err(|e| unreadable(path.to_string(), e))?;
        let sums: Vec<(String, [u8; 32])> = found
            .iter()
            .map(|(name, bytes)| (name.clone(), *blake3::hash(bytes).as_bytes()))
            .collect();
        match self.pins.get(path) {
            Some(pinned) if *pinned != sums => return Err(Error::FileChanged(path.to_string())),
            Some(_) => {}
            None => {
                self.pins.insert(path.to_string(), sums);
            }
        }
        adtb_file::records_in(schema, format, found, row).map_err(|e| match e {
            adtb_file::Error::Unreadable { source, detail } => unreadable(source, detail),
            adtb_file::Error::NotYet(what) => Error::Unsupported(what.to_string()),
        })
    }
}
