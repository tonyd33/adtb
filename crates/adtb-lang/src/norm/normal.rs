//! The normal forms `norm` returns, each built only by validating its terms.

use std::collections::BTreeMap;

use super::{Error, FNS, MAIN, Normaliser, Root};
use crate::check::{AdhocCommand, Program};
use crate::kernel::{self, Body, IndexField, Invalid, Terms, Tm, Valid, Var};
use crate::schema::Schema;
use crate::types::*;

/// The normal forms of a schema's bodies, in one arena.
#[derive(Debug)]
pub struct NormalProgram {
    terms: Terms,
    functions: Vec<Option<(Vec<Var>, Tm)>>,
    constraints: BTreeMap<ConstraintIdx, Tm>,
    checks: BTreeMap<CheckIdx, Tm>,
    domains: BTreeMap<TypeId, (Var, Tm)>,
    retags: BTreeMap<RelId, (Var, Tm)>,
    defaults: BTreeMap<(RelId, usize), Tm>,
    computed: BTreeMap<(RelId, usize), (Var, Tm)>,
    index_fields: BTreeMap<(RelId, usize, usize), (Var, IndexField)>,
    aggregates: Vec<Option<NormalAggregate>>,
}

impl NormalProgram {
    /// Returns the normal form of every body of `p`, refused with the first term that breaks a
    /// typing invariant, is not closed over its body's variables, is not in normal form or sits
    /// where its kind may not, or with the first broken node of `p`'s arena or of the normal
    /// forms', charged to the first body.
    pub(super) fn normalised(schema: &Schema, p: &Program) -> Result<NormalProgram, Error> {
        let mut n = Normaliser::new(schema, [&p.terms, &p.terms], p);
        let mut functions = Vec::with_capacity(p.functions.len());
        for (i, (f, def)) in p.functions.iter().zip(&schema.functions).enumerate() {
            functions.push(match f {
                Some(t) => n.function(
                    Root::Function(FnId(i as u32)),
                    FNS,
                    *t,
                    def.params.len().max(1),
                )?,
                None => None,
            });
        }
        let mut constraints = BTreeMap::new();
        for (c, t) in p.constraints() {
            constraints.insert(c, n.root(Root::Constraint(c), t)?);
        }
        let mut checks = BTreeMap::new();
        for (c, t) in p.checks() {
            checks.insert(c, n.root(Root::Check(c), t)?);
        }
        let mut domains = BTreeMap::new();
        for (id, (v, t)) in &p.domains {
            let x = n.free_var(MAIN, *v);
            domains.insert(*id, (x, n.root(Root::Domain(*id), *t)?));
        }
        let mut retags = BTreeMap::new();
        for (r, t) in &p.retags {
            retags.insert(*r, n.row_function(Root::Retag(*r), *t)?);
        }
        let mut defaults = BTreeMap::new();
        for (&(r, i), t) in &p.defaults {
            defaults.insert((r, i), n.root(Root::Default(r, i), *t)?);
        }
        let mut computed = BTreeMap::new();
        for (&(r, i), (v, t)) in &p.computed {
            let x = n.free_var(MAIN, *v);
            computed.insert((r, i), (x, n.root(Root::Computed(r, i), *t)?));
        }
        let mut index_fields = BTreeMap::new();
        for (&(r, i, j), f) in &p.index_fields {
            let (x, value) = n.row_function(Root::IndexField(r, i, j), f.value)?;
            let path = f.path.clone();
            index_fields.insert((r, i, j), (x, IndexField { value, path }));
        }
        let mut aggregates = Vec::with_capacity(p.aggregates.len());
        for (i, a) in p.aggregates.iter().enumerate() {
            let Some(a) = a else {
                aggregates.push(None);
                continue;
            };
            let root = Root::Aggregate(i);
            let init = n.root(root, a.init)?;
            let step = n.function(root, MAIN, a.step, 2)?;
            let merge = n.function(root, MAIN, a.merge, 2)?;
            let finish = n.function(root, MAIN, a.finish, 1)?;
            aggregates.push(match (step, merge, finish) {
                (Some(step), Some(merge), Some(finish)) => Some(NormalAggregate {
                    init,
                    step,
                    merge,
                    finish,
                }),
                _ => None,
            });
        }
        let program = NormalProgram {
            terms: n.out,
            functions,
            constraints,
            checks,
            domains,
            retags,
            defaults,
            computed,
            index_fields,
            aggregates,
        };
        let bodies = program.bodies();
        for (root, t, free) in &bodies {
            normal(schema, &program.terms, *root, &[(*t, free.clone())])?;
        }
        if let Some((root, ..)) = bodies.first() {
            unbroken(schema, *root, &[&p.terms, &program.terms])?;
        }
        Ok(program)
    }

    pub fn terms(&self) -> &Terms {
        &self.terms
    }

    pub fn valid(&self) -> Valid<'_> {
        Valid::trusted(&self.terms)
    }

    /// Returns each function's parameters and normalised body; `None` for Wasm and host
    /// functions and for a function that takes or returns a function.
    pub fn functions(&self) -> &[Option<(Vec<Var>, Tm)>] {
        &self.functions
    }

    pub fn domains(&self) -> &BTreeMap<TypeId, (Var, Tm)> {
        &self.domains
    }

    /// Returns each `on delete retag` new value over the referrer's row.
    pub fn retags(&self) -> &BTreeMap<RelId, (Var, Tm)> {
        &self.retags
    }

    pub fn defaults(&self) -> &BTreeMap<(RelId, usize), Tm> {
        &self.defaults
    }

    pub fn computed(&self) -> &BTreeMap<(RelId, usize), (Var, Tm)> {
        &self.computed
    }

    /// Returns each index field, its value a body over the row's variable.
    pub fn index_fields(&self) -> &BTreeMap<(RelId, usize, usize), (Var, IndexField)> {
        &self.index_fields
    }

    /// Returns each user-defined aggregate's parts; `None` for one that did not check or whose
    /// parts are not first-order.
    pub fn aggregates(&self) -> &[Option<NormalAggregate>] {
        &self.aggregates
    }

    /// Returns the normal form of constraint `c`; `None` when it has none.
    pub fn constraint(&self, c: ConstraintIdx) -> Option<Tm> {
        self.constraints.get(&c).copied()
    }

    /// Returns the normal form of row check `c`; `None` when it has none.
    pub fn check(&self, c: CheckIdx) -> Option<Tm> {
        self.checks.get(&c).copied()
    }

    pub fn constraints(&self) -> impl Iterator<Item = (ConstraintIdx, Tm)> + '_ {
        self.constraints.iter().map(|(&c, &t)| (c, t))
    }

    pub fn checks(&self) -> impl Iterator<Item = (CheckIdx, Tm)> + '_ {
        self.checks.iter().map(|(&c, &t)| (c, t))
    }

    /// Returns each user-defined aggregate's init, step, merge and finish bodies; `None` for one
    /// with none.
    pub fn aggregate_parts(&self) -> impl Iterator<Item = Option<[Tm; 4]>> + '_ {
        self.aggregates.iter().map(|a| {
            a.as_ref()
                .map(|a| [a.init, a.step.1, a.merge.1, a.finish.1])
        })
    }

    /// Returns every body of the program with the variables free in it.
    pub fn roots(&self) -> Vec<(Tm, Vec<Var>)> {
        self.bodies()
            .into_iter()
            .map(|(_, t, free)| (t, free))
            .collect()
    }

    /// Returns every body of the program with the object it belongs to and the variables free in it.
    pub fn bodies(&self) -> Vec<(Root, Tm, Vec<Var>)> {
        let NormalProgram {
            terms: _,
            functions,
            constraints,
            checks,
            domains,
            retags,
            defaults,
            computed,
            index_fields,
            aggregates,
        } = self;
        let mut out: Vec<(Root, Tm, Vec<Var>)> = Vec::new();
        for (i, f) in functions.iter().enumerate() {
            if let Some((ps, t)) = f {
                out.push((Root::Function(FnId(i as u32)), *t, ps.clone()));
            }
        }
        out.extend(
            constraints
                .iter()
                .map(|(&c, &t)| (Root::Constraint(c), t, vec![])),
        );
        out.extend(checks.iter().map(|(&c, &t)| (Root::Check(c), t, vec![])));
        out.extend(
            domains
                .iter()
                .map(|(&d, (v, t))| (Root::Domain(d), *t, vec![*v])),
        );
        out.extend(
            retags
                .iter()
                .map(|(&r, (v, t))| (Root::Retag(r), *t, vec![*v])),
        );
        out.extend(
            defaults
                .iter()
                .map(|(&(r, i), &t)| (Root::Default(r, i), t, vec![])),
        );
        out.extend(
            computed
                .iter()
                .map(|(&(r, i), (v, t))| (Root::Computed(r, i), *t, vec![*v])),
        );
        out.extend(
            index_fields
                .iter()
                .map(|(&(r, i, j), (v, f))| (Root::IndexField(r, i, j), f.value, vec![*v])),
        );
        for (i, a) in aggregates.iter().enumerate() {
            if let Some(a) = a {
                out.push((Root::Aggregate(i), a.init, vec![]));
                for (vs, t) in [&a.step, &a.merge, &a.finish] {
                    out.push((Root::Aggregate(i), *t, vs.clone()));
                }
            }
        }
        out
    }
}

/// The normal forms of a user-defined aggregate's parts, each over its variables.
#[derive(Debug)]
pub struct NormalAggregate {
    init: Tm,
    step: (Vec<Var>, Tm),
    merge: (Vec<Var>, Tm),
    finish: (Vec<Var>, Tm),
}

impl NormalAggregate {
    pub fn init(&self) -> Tm {
        self.init
    }

    pub fn step(&self) -> &(Vec<Var>, Tm) {
        &self.step
    }

    pub fn merge(&self) -> &(Vec<Var>, Tm) {
        &self.merge
    }

    pub fn finish(&self) -> &(Vec<Var>, Tm) {
        &self.finish
    }
}

/// The normal forms of the items of ad hoc text.
#[derive(Debug)]
pub struct NormalAdhoc {
    terms: Terms,
    items: Vec<Tm>,
    commands: Vec<AdhocCommand>,
    params: BTreeMap<String, Var>,
}

/// The parts of a [`NormalAdhoc`]: its arena, items, commands and parameters.
pub type AdhocParts = (Terms, Vec<Tm>, Vec<AdhocCommand>, BTreeMap<String, Var>);

impl NormalAdhoc {
    /// Returns the text of `parts`, refused with the first term that breaks a typing invariant,
    /// is not closed over the text's parameters, is not in normal form or sits where its kind
    /// may not, charged to its item or command, or, for a text with an item or a command, with
    /// the first broken node of `inputs` or of the normal forms' arena.
    pub(super) fn validated(
        schema: &Schema,
        inputs: &[&Terms],
        parts: AdhocParts,
    ) -> Result<NormalAdhoc, Error> {
        let (terms, items, commands, params) = parts;
        let adhoc = NormalAdhoc {
            terms,
            items,
            commands,
            params,
        };
        let first = match (adhoc.items.is_empty(), adhoc.commands.is_empty()) {
            (false, _) => Some(Root::Item(0)),
            (true, false) => Some(Root::Command(0)),
            (true, true) => None,
        };
        if let Some(root) = first {
            let mut arenas = inputs.to_vec();
            arenas.push(&adhoc.terms);
            unbroken(schema, root, &arenas)?;
        }
        let free: Vec<Var> = adhoc.params.values().copied().collect();
        for (i, t) in adhoc.items.iter().enumerate() {
            normal(schema, &adhoc.terms, Root::Item(i), &[(*t, free.clone())])?;
        }
        for (i, c) in adhoc.commands.iter().enumerate() {
            let roots: Vec<(Tm, Vec<Var>)> = c
                .command
                .terms()
                .into_iter()
                .map(|t| (t, free.clone()))
                .collect();
            normal(schema, &adhoc.terms, Root::Command(i), &roots)?;
        }
        Ok(adhoc)
    }

    pub fn terms(&self) -> &Terms {
        &self.terms
    }

    pub fn valid(&self) -> Valid<'_> {
        Valid::trusted(&self.terms)
    }

    pub fn items(&self) -> &[Tm] {
        &self.items
    }

    /// Returns each command, its terms normalised.
    pub fn commands(&self) -> &[AdhocCommand] {
        &self.commands
    }

    pub fn params(&self) -> &BTreeMap<String, Var> {
        &self.params
    }

    pub fn into_parts(self) -> AdhocParts {
        (self.terms, self.items, self.commands, self.params)
    }

    /// Returns each item and each term of a command, its parameters free in it.
    pub fn roots(&self) -> Vec<(Tm, Vec<Var>)> {
        let free: Vec<Var> = self.params.values().copied().collect();
        self.items
            .iter()
            .copied()
            .chain(self.commands.iter().flat_map(|c| c.command.terms()))
            .map(|t| (t, free.clone()))
            .collect()
    }
}

/// The normal form of a registered query's or transaction's body over its parameters.
#[derive(Debug)]
pub struct NormalQuery {
    terms: Terms,
    params: Vec<Var>,
    body: Tm,
}

impl NormalQuery {
    /// Returns `body` of `terms` over `params` as a normal form, refused with the first term
    /// that breaks a typing invariant, is not closed over `params`, is not in normal form or
    /// sits where its kind may not, or with the first broken node of `terms`.
    pub fn validated(
        schema: &Schema,
        terms: Terms,
        params: Vec<Var>,
        body: Tm,
    ) -> Result<NormalQuery, Error> {
        unbroken(schema, Root::Query, &[&terms])?;
        normal(schema, &terms, Root::Query, &[(body, params.clone())])?;
        Ok(NormalQuery {
            terms,
            params,
            body,
        })
    }

    pub fn terms(&self) -> &Terms {
        &self.terms
    }

    pub fn valid(&self) -> Valid<'_> {
        Valid::trusted(&self.terms)
    }

    pub fn params(&self) -> &[Var] {
        &self.params
    }

    pub fn body(&self) -> Tm {
        self.body
    }

    pub fn into_parts(self) -> (Terms, Vec<Var>, Tm) {
        (self.terms, self.params, self.body)
    }
}

/// Fail with the first broken node any of `arenas` records, charged to `root`.
fn unbroken(schema: &Schema, root: Root, arenas: &[&Terms]) -> Result<(), Error> {
    for terms in arenas {
        if let Some(e) = terms.broken() {
            return Err(Error::invalid(schema, root, terms, e.clone()));
        }
    }
    Ok(())
}

/// Fail, charged to `root`, on the first term reachable from `roots` that breaks a typing
/// invariant, reads a variable nothing binds, is not in normal form, or sits where its kind may
/// not in a body of `root`.
fn normal(
    schema: &Schema,
    terms: &Terms,
    root: Root,
    roots: &[(Tm, Vec<Var>)],
) -> Result<(), Error> {
    let at = match root {
        Root::Constraint(_) => Body::Constraint,
        Root::Function(f) => Body::Generic(schema.functions[f.0 as usize].type_params.len() as u32),
        Root::Domain(d) => Body::Generic(schema.ty(d).params.len() as u32),
        Root::Aggregate(i) => {
            let a = &schema.aggregates[i];
            let tys = a.params.iter().map(|(_, t)| t).chain([&a.result, &a.state]);
            Body::Generic(tys.map(Ty::params_named).max().unwrap_or(0))
        }
        Root::Query => Body::Generic(
            roots
                .iter()
                .flat_map(|(_, free)| free.iter().map(|v| terms.var_info(*v).ty.params_named()))
                .max()
                .unwrap_or(0),
        ),
        Root::Check(_)
        | Root::Retag(_)
        | Root::Default(..)
        | Root::Computed(..)
        | Root::IndexField(..)
        | Root::Item(_)
        | Root::Command(_) => Body::Plain,
    };
    let tops: Vec<Tm> = roots.iter().map(|(t, _)| *t).collect();
    kernel::validate(schema, terms, roots)
        .and_then(|_| kernel::validate_normal(schema, terms, roots))
        .and_then(|()| kernel::validate_placed(terms, &tops, at))
        .map_err(|e: Invalid| Error::invalid(schema, root, terms, e))
}
