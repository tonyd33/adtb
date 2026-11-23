//! What each body of a checked program reaches, and which bodies a schema change changes.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::FnBody;
use crate::check::Program;
use crate::kernel::{Agg, Kind, Terms, Tm, alpha_equal_under};
use crate::norm::Root;
use crate::schema::Schema;
use crate::types::{DenialIdx, FnId, RelId, RelRef, Ty, TypeId};

#[cfg(test)]
#[path = "depends_tests.rs"]
mod tests;

/// Returns, for each body of `p`, every function and aggregate it calls, directly or through
/// the bodies of what it calls, each as its own root.
pub fn reached(p: &Program) -> BTreeMap<Root, BTreeSet<Root>> {
    let mut direct: BTreeMap<Root, BTreeSet<Root>> = BTreeMap::new();
    for (root, t, _) in p.bodies() {
        direct.entry(root).or_default().extend(calls(&p.terms, t));
    }
    direct
        .keys()
        .map(|&root| {
            let mut seen = BTreeSet::new();
            let mut todo: Vec<Root> = direct[&root].iter().copied().collect();
            while let Some(r) = todo.pop() {
                if seen.insert(r) {
                    todo.extend(direct.get(&r).into_iter().flatten().copied());
                }
            }
            (root, seen)
        })
        .collect()
}

/// Returns the function or aggregate each call under `t` names.
fn calls(terms: &Terms, t: Tm) -> BTreeSet<Root> {
    let mut out = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut todo = vec![t];
    while let Some(t) = todo.pop() {
        if !seen.insert(t) {
            continue;
        }
        match terms.kind(t) {
            Kind::Call { func, .. } => {
                out.insert(Root::Function(*func));
            }
            Kind::Agg {
                op: Agg::User(i), ..
            } => {
                out.insert(Root::Aggregate(*i as usize));
            }
            _ => {}
        }
        todo.extend(terms.children(t));
    }
    out
}

/// Returns each body of `new`, the program of schema `b`, whose meaning may differ from that of
/// the same catalog object in `old`, the program of schema `a`: one with no counterpart in `old`,
/// one whose own body differs from its counterpart's, and one that reaches such a function or
/// aggregate. A host or Wasm function differs when its code or signature does.
///
/// Bodies are compared up to the names of their variables, each object an old body names by
/// position taken to the position of the same object in `b`.
pub fn changed(a: &Schema, old: &Program, b: &Schema, new: &Program) -> BTreeSet<Root> {
    let at = Positions::new(a, b);
    let kind = |k: &Kind| at.kind(k);
    let ty = |t: &Ty| at.ty(t);
    let mut before: BTreeMap<Root, Vec<Tm>> = BTreeMap::new();
    for (root, t, _) in old.bodies() {
        before.entry(root).or_default().push(t);
    }
    let mut after: BTreeMap<Root, Vec<Tm>> = BTreeMap::new();
    for (root, t, _) in new.bodies() {
        after.entry(root).or_default().push(t);
    }
    let mut differs: BTreeSet<Root> = after
        .iter()
        .filter(|&(&root, ts)| {
            let same = counterpart(a, b, root)
                .and_then(|r| before.get(&r))
                .is_some_and(|us| {
                    us.len() == ts.len()
                        && us.iter().zip(ts).all(|(&u, &t)| {
                            alpha_equal_under(&old.terms, u, &new.terms, t, &kind, &ty)
                        })
                });
            !same
        })
        .map(|(&root, _)| root)
        .collect();
    for (f, def) in b.functions.iter().enumerate() {
        let f = FnId(f as u32);
        if matches!(def.body, FnBody::Expr(_)) {
            continue;
        }
        let alike = |x: &Ty, y: &Ty| at.ty(x).is_some_and(|x| x.alike(y));
        let same = b
            .ids()
            .function_id(f)
            .and_then(|id| a.ids().function_of(id))
            .is_some_and(|g| {
                let was = a.function(g);
                same_code(&was.body, &def.body)
                    && alike(&was.result, &def.result)
                    && was.params.len() == def.params.len()
                    && was
                        .params
                        .iter()
                        .zip(&def.params)
                        .all(|((_, x), (_, y))| alike(x, y))
            });
        if !same {
            differs.insert(Root::Function(f));
        }
    }
    let reached = reached(new);
    after
        .keys()
        .copied()
        .filter(|root| {
            differs.contains(root)
                || reached
                    .get(root)
                    .is_some_and(|rs| !rs.is_disjoint(&differs))
        })
        .collect()
}

/// The position in one schema of each function, aggregate, relation and type of another that it
/// also has: the same catalog id, or, for one with none, the same name in the same module. A type
/// has an image only when its constructors are the same in both, in order, and a relation only
/// when its indexes are and its element type's image is its element type.
struct Positions {
    functions: BTreeMap<FnId, FnId>,
    aggregates: BTreeMap<usize, usize>,
    relations: BTreeMap<RelId, RelId>,
    types: BTreeMap<TypeId, TypeId>,
}

impl Positions {
    fn new(a: &Schema, b: &Schema) -> Positions {
        let (x, y) = (a.ids(), b.ids());
        let module = |s: &Schema, m| s.module(m).name.clone();
        let functions = images(
            y.function_ids()
                .filter_map(|(f, id)| Some((x.function_of(id)?, f))),
            a.functions
                .iter()
                .map(|d| (module(a, d.module), d.name.clone())),
            b.functions
                .iter()
                .map(|d| (module(b, d.module), d.name.clone())),
            |i| FnId(i as u32),
            |f| x.function_id(f).is_none(),
            |f| y.function_id(f).is_none(),
        );
        let aggregates = y
            .aggregate_ids()
            .filter_map(|(i, id)| Some((x.aggregate_of(id)?, i)))
            .collect();
        let ctors = |s: &Schema, t: TypeId| {
            let names: Vec<String> = s.ty(t).ctors().iter().map(|c| c.name.clone()).collect();
            let ids: Vec<_> = s.ids().type_ctor_ids(t).map(|(_, id)| id).collect();
            (names, ids)
        };
        let types: BTreeMap<TypeId, TypeId> = images(
            y.type_ids().filter_map(|(t, id)| Some((x.type_of(id)?, t))),
            a.types
                .iter()
                .map(|d| (module(a, d.module), d.name.clone())),
            b.types
                .iter()
                .map(|d| (module(b, d.module), d.name.clone())),
            |i| TypeId(i as u32),
            |t| x.type_id(t).is_none(),
            |t| y.type_id(t).is_none(),
        )
        .into_iter()
        .filter(|&(s, t)| ctors(a, s) == ctors(b, t))
        .collect();
        let indexes = |s: &Schema, r| s.ids().relation_index_ids(r).collect::<Vec<_>>();
        let relations = images(
            y.relation_ids()
                .filter_map(|(r, id)| Some((x.relation_of(id)?, r))),
            a.relations
                .iter()
                .map(|d| (module(a, d.module), d.name.clone())),
            b.relations
                .iter()
                .map(|d| (module(b, d.module), d.name.clone())),
            |i| RelId(i as u32),
            |r| x.relation_id(r).is_none(),
            |r| y.relation_id(r).is_none(),
        )
        .into_iter()
        .filter(|&(r, s)| {
            let (d, e) = (a.relation(r).elem_def, b.relation(s).elem_def);
            types.get(&d) == Some(&e) && indexes(a, r) == indexes(b, s)
        })
        .collect();
        Positions {
            functions,
            aggregates,
            relations,
            types,
        }
    }

    /// Returns `k` with each object it names by position taken to its image; `None` when one
    /// has none.
    fn kind(&self, k: &Kind) -> Option<Kind> {
        let rel = |r: &RelId| self.relations.get(r).copied();
        Some(match k {
            Kind::Call {
                func,
                type_args,
                args,
            } => Kind::Call {
                func: *self.functions.get(func)?,
                type_args: type_args
                    .iter()
                    .map(|t| self.ty(t))
                    .collect::<Option<_>>()?,
                args: args.clone(),
            },
            Kind::Agg {
                op: Agg::User(i),
                var,
                source,
                body,
            } => Kind::Agg {
                op: Agg::User(*self.aggregates.get(&(*i as usize))? as u32),
                var: *var,
                source: *source,
                body: *body,
            },
            Kind::KeyOf(r, t) => Kind::KeyOf(rel(r)?, *t),
            Kind::Rel(r, s) => Kind::Rel(rel(r)?, s.clone()),
            Kind::Transition(x, r, s) => Kind::Transition(*x, rel(r)?, s.clone()),
            Kind::Fresh(g, r) => Kind::Fresh(*g, rel(r)?),
            Kind::Placeholder {
                occurrence,
                generator,
                rel: r,
                scope,
            } => Kind::Placeholder {
                occurrence: *occurrence,
                generator: *generator,
                rel: rel(r)?,
                scope: scope.clone(),
            },
            Kind::Insert(r, t) => Kind::Insert(rel(r)?, *t),
            Kind::Delete(r, t) => Kind::Delete(rel(r)?, *t),
            Kind::Replace(r, t, u) => Kind::Replace(rel(r)?, *t, *u),
            Kind::Upsert {
                rel: r,
                rows,
                on,
                old,
                new,
                otherwise,
                single,
            } => Kind::Upsert {
                rel: rel(r)?,
                rows: *rows,
                on: *on,
                old: *old,
                new: *new,
                otherwise: *otherwise,
                single: *single,
            },
            Kind::Rejects { into, insert } => Kind::Rejects {
                into: rel(into)?,
                insert: *insert,
            },
            Kind::Agg { .. }
            | Kind::Var(_)
            | Kind::Const(_)
            | Kind::Prim(..)
            | Kind::Record(_)
            | Kind::Field(..)
            | Kind::Ctor { .. }
            | Kind::Case { .. }
            | Kind::If(..)
            | Kind::Let(..)
            | Kind::Lam(..)
            | Kind::App(..)
            | Kind::Caller
            | Kind::Key(_)
            | Kind::Deref { .. }
            | Kind::Tag(_)
            | Kind::Try(_)
            | Kind::Restrict(..)
            | Kind::EmptySet
            | Kind::Singleton(_)
            | Kind::Union(..)
            | Kind::For(..)
            | Kind::IsEmpty(_)
            | Kind::Exists(_)
            | Kind::Group { .. }
            | Kind::Top { .. }
            | Kind::Fix(..)
            | Kind::Only(_)
            | Kind::At { .. }
            | Kind::Present { .. }
            | Kind::Reject(_)
            | Kind::Unreachable
            | Kind::Skip
            | Kind::Par(_)
            | Kind::Fail(_)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => k.clone(),
        })
    }

    /// Returns `t` with each type and relation it names taken to its image; `None` when one has
    /// none.
    fn ty(&self, t: &Ty) -> Option<Ty> {
        let boxed = |x: &Ty| self.ty(x).map(Box::new);
        let fields = |fs: &crate::types::Fields| fs.try_map(|t| self.ty(t).ok_or(())).ok();
        Some(match t {
            Ty::Data { def, args, ctors } => Ty::Data {
                def: *self.types.get(def)?,
                args: args.iter().map(|a| self.ty(a)).collect::<Option<_>>()?,
                ctors: ctors.clone(),
            },
            Ty::Key {
                rel: RelRef::Declared(r),
                ctors,
            } => Ty::Key {
                rel: RelRef::Declared(*self.relations.get(r)?),
                ctors: ctors.clone(),
            },
            Ty::Key {
                rel: RelRef::Ahead(_),
                ..
            } => return None,
            Ty::Set(x) => Ty::Set(boxed(x)?),
            Ty::NonEmpty(x) => Ty::NonEmpty(boxed(x)?),
            Ty::Tag(x) => Ty::Tag(boxed(x)?),
            Ty::Group(x, y) => Ty::Group(boxed(x)?, boxed(y)?),
            Ty::Fn(x, y) => Ty::Fn(boxed(x)?, boxed(y)?),
            Ty::Record(fs) => Ty::Record(fields(fs)?),
            Ty::Sum { ctors, sel } => Ty::Sum {
                ctors: ctors.try_map(|t| self.ty(t).ok_or(())).ok()?,
                sel: sel.clone(),
            },
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => t.clone(),
        })
    }
}

/// Returns the image in `b` of each position of `a`: those `by_id` gives, and each position with
/// no catalog id in `a` (`a_bare`) at the position with none in `b` (`b_bare`) of the same name.
fn images<K: Ord + Copy>(
    by_id: impl Iterator<Item = (K, K)>,
    a: impl Iterator<Item = (String, String)>,
    b: impl Iterator<Item = (String, String)>,
    at: impl Fn(usize) -> K,
    a_bare: impl Fn(K) -> bool,
    b_bare: impl Fn(K) -> bool,
) -> BTreeMap<K, K> {
    let mut out: BTreeMap<K, K> = by_id.collect();
    let named: BTreeMap<(String, String), K> = b
        .enumerate()
        .map(|(i, n)| (n, at(i)))
        .filter(|&(_, k)| b_bare(k))
        .collect();
    for (i, n) in a.enumerate() {
        let k = at(i);
        if a_bare(k)
            && let Some(&j) = named.get(&n)
        {
            out.insert(k, j);
        }
    }
    out
}

/// Returns whether host or Wasm body `x` names the same code as `y`.
fn same_code(x: &FnBody, y: &FnBody) -> bool {
    match (x, y) {
        (
            FnBody::Wasm { module, export },
            FnBody::Wasm {
                module: m,
                export: e,
            },
        ) => module == m && export == e,
        (
            FnBody::Host {
                name,
                version,
                deterministic,
            },
            FnBody::Host {
                name: n,
                version: v,
                deterministic: d,
            },
        ) => name == n && version == v && deterministic == d,
        _ => false,
    }
}

/// Returns the root of `a` that belongs to the same catalog object as root `root` of `b`.
fn counterpart(a: &Schema, b: &Schema, root: Root) -> Option<Root> {
    let (x, y) = (a.ids(), b.ids());
    match root {
        Root::Function(f) => Some(Root::Function(x.function_of(y.function_id(f)?)?)),
        Root::Aggregate(i) => Some(Root::Aggregate(x.aggregate_of(y.aggregate_id(i)?)?)),
        Root::Constraint(c) => match x.denial_of(y.constraint_id(c)?)? {
            DenialIdx::Constraint(c) => Some(Root::Constraint(c)),
            DenialIdx::Check(_) => None,
        },
        Root::Check(c) => match x.denial_of(y.check_id(c)?)? {
            DenialIdx::Check(c) => Some(Root::Check(c)),
            DenialIdx::Constraint(_) => None,
        },
        Root::IndexField(r, i, j) => {
            let (r, i) = x.index_of(y.index_id(r, i)?)?;
            Some(Root::IndexField(r, i, j))
        }
        Root::Domain(t) => Some(Root::Domain(x.type_of(y.type_id(t)?)?)),
        Root::Retag(r) => Some(Root::Retag(x.relation_of(y.relation_id(r)?)?)),
        Root::Computed(r, i) => {
            let id = y.computed_id(r, i)?;
            let (at, _) = x.computed_ids().find(|&(_, c)| c == id)?;
            Some(Root::Computed(at.0, at.1))
        }
        Root::Default(r, i) => {
            let (_, id) = y.clause_default_ids(r, i).next()?;
            let ((r, i, _), _) = x.default_ids().find(|&(_, d)| d == id)?;
            Some(Root::Default(r, i))
        }
        Root::Item(_) | Root::Command(_) | Root::Query => None,
    }
}
