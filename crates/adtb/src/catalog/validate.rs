use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::kernel::{self, Build, Kind, Terms, Tm, Valid, Var};
use adtb_lang::norm::{self, NormalProgram, Root};
use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::types::{RelId, RelRef, Ty, TypeId};

use crate::exec::types::ctors;
use crate::exec::{self, Budget, Call, Store, Val};
use crate::plan;
use crate::runtime::Runtime;
use crate::storage::PageError;

use super::hosts::{self, classify};
use super::ids::{CatConstraintId, CatRelId};
use super::read::Judged;
use super::{Broken, Catalog, Error, Migrating, Rot, Snapshot, not_normal};

impl Catalog {
    /// Returns, for each constraint and check of this catalog that `old` lacks, whose root is in
    /// `roots`, the roots [`adtb_lang::depends::changed`] finds changed from `old` to this
    /// catalog, or that reads a relation whose catalog id is in `rewritten`, why the rows
    /// transaction `m.x` sees at the snapshot of `m.hold` violate it, with its catalog id; then
    /// why they violate each domain.
    ///
    /// A constraint that reads a transition is validated over the empty change.
    ///
    /// Fail with [`Broken::ConstraintId`] when a constraint or check has no catalog id, and with
    /// [`Broken::ConstraintTerm`] when it has no normal form.
    pub(super) fn violations<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        old: &Schema,
        rewritten: &BTreeSet<CatRelId>,
        roots: &BTreeSet<Root>,
    ) -> Result<Vec<(Option<CatConstraintId>, String)>, Error> {
        let compiled = self.checking();
        let schema = compiled.committed();
        let changed: BTreeSet<CatConstraintId> = roots
            .iter()
            .filter_map(|root| match *root {
                Root::Constraint(c) => schema.ids().constraint_id(c),
                Root::Check(c) => schema.ids().check_id(c),
                _ => None,
            })
            .collect();
        let domains: BTreeSet<TypeId> = roots
            .iter()
            .filter_map(|root| match *root {
                Root::Domain(t) => Some(t),
                _ => None,
            })
            .collect();
        let normal = norm::program(compiled)
            .map_err(|e| not_normal(schema, e, |e| Error::from(Broken::NotNormal(e))))?;
        let existed: BTreeSet<CatConstraintId> = old
            .ids()
            .constraint_ids()
            .map(|(_, id)| id)
            .chain(old.ids().check_ids().map(|(_, id)| id))
            .collect();
        let catalog = self.plan();
        let store = self.store(&Snapshot::writing(m));
        let budget = Budget::new(self.query_memory);
        let (terms, held) = unchanged(schema, &normal)?;
        let q = Query {
            schema,
            catalog: &catalog,
            normal: &normal,
            terms: terms.valid(),
            store: &store,
            budget: &budget,
            hosts: &self.hosts,
        };
        let mut why = Vec::new();
        for Held { name, id, t } in held {
            let mut reads = BTreeSet::new();
            relations(terms.valid().terms(), t, &mut reads);
            let touched = reads.iter().any(|r| {
                schema
                    .ids()
                    .relation_id(*r)
                    .is_some_and(|id| rewritten.contains(&id))
            });
            if existed.contains(&id) && !touched && !changed.contains(&id) {
                continue;
            }
            let found = violating(&q, &(t, format!("constraint {name}")))?;
            if !found.is_empty() {
                why.push((
                    Some(id),
                    format!("constraint {name} is violated by {}", found.join(", ")),
                ));
            }
        }
        let mut terms = normal.terms().clone();
        let rels: Vec<Tm> = schema
            .ids()
            .relation_ids()
            .map(|(r, _)| Build::new(schema, &mut terms).rel(r, None))
            .collect();
        let roots: Vec<(Tm, Vec<Var>)> = rels.iter().map(|t| (*t, Vec::new())).collect();
        let terms = kernel::ValidTerms::new(schema, terms, &roots)
            .map_err(|e| Error::from(Broken::IllTyped(e)))?;
        let q = Query {
            schema,
            catalog: &catalog,
            normal: &normal,
            terms: terms.valid(),
            store: &store,
            budget: &budget,
            hosts: &self.hosts,
        };
        why.extend(q.domains(&rels, &domains)?.into_iter().map(|w| (None, w)));
        Ok(why)
    }
}

impl Catalog {
    /// Returns what the denial query of each constraint and check of this catalog found over
    /// `store`, over the empty change, for those `keep` accepts by the relations they read.
    pub fn denied<R: Runtime>(
        &self,
        store: &Arc<Store<R>>,
        keep: impl Fn(&BTreeSet<RelId>) -> bool,
    ) -> Result<Vec<Denial>, Error> {
        let compiled = self.checking();
        let schema = compiled.committed();
        let normal = norm::program(compiled)
            .map_err(|e| not_normal(schema, e, |e| Error::from(Broken::NotNormal(e))))?;
        let catalog = self.plan();
        let budget = Budget::new(self.query_memory);
        let (terms, held) = unchanged(schema, &normal)?;
        let store = Judged::verifying(store.clone());
        let q = Query {
            schema,
            catalog: &catalog,
            normal: &normal,
            terms: terms.valid(),
            store: &store,
            budget: &budget,
            hosts: &self.hosts,
        };
        let mut out = Vec::new();
        for Held { id, t, name } in held {
            let id = id.raw();
            let mut reads = BTreeSet::new();
            relations(terms.valid().terms(), t, &mut reads);
            if !keep(&reads) {
                continue;
            }
            let found = run_denial(&q, &(t, format!("constraint {name}")));
            let denial = match found {
                Err(Error::Page(e)) => Some(Denial::Damaged(e)),
                Err(e) => return Err(e),
                Ok(Ok(rows)) => rows
                    .into_iter()
                    .next()
                    .map(|witness| Denial::Violated { id, witness }),
                Ok(Err(Denied::Fault(witness))) => Some(Denial::Violated { id, witness }),
                Ok(Err(Denied::Memory(why))) => Some(Denial::Unrun { id, why }),
                Ok(Err(Denied::Rotten(rot))) => Some(Denial::Rotten { id, rot }),
                Ok(Err(Denied::Orphaned(index))) => Some(Denial::Orphaned { id, index }),
            };
            out.extend(denial);
        }
        Ok(out)
    }
}

/// What a whole run of one denial query found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denial {
    /// The constraint of catalog id `id` is violated; `witness` is its first row, printed.
    Violated { id: u64, witness: String },
    /// The query read a damaged page.
    Damaged(PageError),
    /// The query of the constraint of catalog id `id` read a stored structure that does not
    /// decode.
    Rotten { id: u64, rot: exec::Rot },
    /// The query of the constraint of catalog id `id` met an entry of the index `index` owns
    /// that names no version.
    Orphaned { id: u64, index: u32 },
    /// The query needs more than the query memory.
    Unrun { id: u64, why: String },
}

/// Why a denial query returned no rows to read.
enum Denied {
    /// The query itself faults or rejects, a violation of no row.
    Fault(String),
    /// The query needs more than the query memory.
    Memory(String),
    /// The query read a stored structure that does not decode.
    Rotten(exec::Rot),
    /// The query met an entry of the index this owner number owns that names no version.
    Orphaned(u32),
}

/// A constraint or check of a schema: its qualified name, its catalog id and its normal form.
struct Held {
    name: String,
    id: CatConstraintId,
    t: Tm,
}

/// Returns every constraint, then every check, of `schema`, in declaration order.
///
/// Fail with [`Broken::ConstraintId`] when one has no catalog id, and with
/// [`Broken::ConstraintTerm`] when `normal` holds no normal form of it.
fn held(schema: &Schema, normal: &NormalProgram) -> Result<Vec<Held>, Error> {
    let ids = schema.ids();
    let name = |m, n: &str| format!("{}::{n}", schema.module(m).name);
    let constraints = schema.constraint_defs().map(|(c, def)| {
        let name = name(def.module, &def.name);
        (ids.constraint_id(c), normal.constraint(c), name)
    });
    let checks = schema.check_defs().map(|(c, def)| {
        let name = name(def.module, &def.name);
        (ids.check_id(c), normal.check(c), name)
    });
    constraints
        .chain(checks)
        .map(|(id, t, name)| match (id, t) {
            (None, _) => Err(Error::from(Broken::ConstraintId(name))),
            (_, None) => Err(Error::from(Broken::ConstraintTerm(name))),
            (Some(id), Some(t)) => Ok(Held { name, id, t }),
        })
        .collect()
}

/// Returns every constraint, then every check, of `schema`, each over the empty change, in an
/// arena of `normal`'s terms.
fn unchanged(
    schema: &Schema,
    normal: &NormalProgram,
) -> Result<(kernel::ValidTerms, Vec<Held>), Error> {
    let mut terms = normal.terms().clone();
    let mut held = held(schema, normal)?;
    for h in &mut held {
        h.t = crate::write::unchanged(schema, &mut terms, h.t);
    }
    let roots: Vec<(Tm, Vec<Var>)> = held.iter().map(|h| (h.t, Vec::new())).collect();
    let terms = kernel::ValidTerms::new(schema, terms, &roots)
        .map_err(|e| Error::from(Broken::IllTyped(e)))?;
    Ok((terms, held))
}

/// Add the relations `t` reads to `reads`.
fn relations(terms: &Terms, t: Tm, reads: &mut BTreeSet<RelId>) {
    let mut stack = vec![t];
    let mut seen = BTreeSet::new();
    while let Some(t) = stack.pop() {
        if !seen.insert(t) {
            continue;
        }
        match terms.kind(t) {
            Kind::Rel(r, _) => {
                reads.insert(*r);
            }
            Kind::Deref { key, .. } => {
                if let Ty::Key {
                    rel: RelRef::Declared(rel),
                    ..
                } = terms.ty(*key)
                {
                    reads.insert(*rel);
                }
            }
            _ => {}
        }
        stack.extend(terms.children(t));
    }
}

/// Returns each row the query of constraint `query` returns over the store of `q`, printed, or
/// why it could not run, that printed as its one row. `query` is the query and the constraint's
/// name.
fn violating<R: Runtime>(q: &Query<'_, R>, query: &(Tm, String)) -> Result<Vec<String>, Error> {
    Ok(match run_denial(q, query)? {
        Ok(rows) => rows,
        Err(Denied::Fault(why) | Denied::Memory(why)) => vec![why],
        Err(Denied::Rotten(rot)) => return Err(Rot::Exec(rot).into()),
        Err(Denied::Orphaned(index)) => return Err(Rot::Orphaned(index).into()),
    })
}

/// Returns each row the query of constraint `query` returns over the store of `q`, printed, or
/// why it could not run. `query` is the query and the constraint's name.
fn run_denial<R: Runtime>(
    q: &Query<'_, R>,
    (t, object): &(Tm, String),
) -> Result<Result<Vec<String>, Denied>, Error> {
    let (rows, elem) = match q.run(*t, object)? {
        Ok(found) => found,
        Err(d) => return Ok(Err(d)),
    };
    let mut printed = Vec::new();
    for r in &rows {
        match exec::print(q.schema, &elem, r) {
            Ok(row) => printed.push(row),
            Err(e) => return damaged(object, e).map(Err),
        }
    }
    Ok(Ok(printed))
}

/// What a set query runs over during a migration's validation.
struct Query<'a, R: Runtime> {
    schema: &'a Schema,
    catalog: &'a plan::Catalog,
    normal: &'a NormalProgram,
    terms: Valid<'a>,
    store: &'a Judged<R>,
    budget: &'a Arc<Budget>,
    hosts: &'a Arc<exec::Hosts>,
}

impl<R: Runtime> Query<'_, R> {
    /// Returns each row set query `t` of `self.terms` returns over the store, with the type of
    /// a row, or why it could not run; a failure names `object`, what the query checks.
    fn run(&self, t: Tm, object: &str) -> Result<Result<(Vec<Val>, Ty), Denied>, Error> {
        let (schema, catalog, normal, terms) = (self.schema, self.catalog, self.normal, self.terms);
        let (store, budget, hosts) = (self.store, self.budget, self.hosts);
        let p = plan::plan(schema, normal, catalog, terms, t, plan::Options::default())
            .map_err(|e| Error::from(Broken::Plan(e)))?;
        let program = hosts::program(object, (schema, catalog), &p, hosts, Broken::Validation)?;
        store.admit(program.reads())?;
        let store = store.store();
        let program = Arc::new(program);
        let grant = budget
            .admit(budget.limit())
            .expect("one validation runs at a time");
        let call = Call::new(
            store.clone(),
            grant,
            exec::Options::default(),
            Arc::default(),
            None,
        );
        let run = program
            .bind(|_| None)
            .and_then(|bound| exec::execute(store.pager().rt().clone(), bound, call));
        let v = match run {
            Ok(v) => v,
            Err(exec::Error::Fault(f)) => {
                return Ok(Err(Denied::Fault(format!(
                    "its query faults with {}",
                    f.name()
                ))));
            }
            Err(exec::Error::Rejected(why)) => {
                return Ok(Err(Denied::Fault(format!("its query rejects: {why}"))));
            }
            Err(exec::Error::StatementLimit(
                exec::Exceeded::Memory { limit } | exec::Exceeded::Spill { grant: limit, .. },
            )) => {
                return Ok(Err(Denied::Memory(format!(
                    "its validation needs more than the query memory of {limit} bytes"
                ))));
            }
            Err(e) => return damaged(object, e).map(Err),
        };
        let Val::Set(rows) = &v else {
            return Err(Error::from(Broken::NotASet(v)));
        };
        let elem = match terms.terms().ty(t) {
            Ty::Set(e) => e.as_ref().clone(),
            other => {
                return Err(Error::from(Broken::NotSetTyped(other.clone())));
            }
        };
        Ok(Ok((rows.iter().cloned().collect(), elem)))
    }

    /// Returns why the rows of each of `rels`, each a whole relation, that hold a value of a
    /// newtype of `domains` violate the newtype's domain, one reason per newtype.
    fn domains(&self, rels: &[Tm], domains: &BTreeSet<TypeId>) -> Result<Vec<String>, Error> {
        let schema = self.schema;
        let name = |d: TypeId| {
            let def = schema.ty(d);
            format!("{}::{}.domain", schema.module(def.module).name, def.name)
        };
        let mut checks = BTreeMap::new();
        for t in domains {
            if let Some((v, body)) = self.normal.domains().get(t) {
                let object = format!("constraint {}", name(*t));
                let terms = (schema, self.normal.valid());
                let scalar =
                    hosts::scalar(&object, terms, (*v, *body), self.hosts, Broken::Validation)?;
                checks.insert(*t, scalar);
            }
        }
        let mut found: BTreeMap<TypeId, Vec<String>> = BTreeMap::new();
        let mut why = Vec::new();
        for &t in rels {
            let Ty::Set(e) = self.terms.terms().ty(t) else {
                continue;
            };
            let held: Vec<TypeId> = checks
                .keys()
                .copied()
                .filter(|&d| holds(schema, e, d, &mut BTreeSet::new()))
                .collect();
            if held.is_empty() {
                continue;
            }
            let names: Vec<String> = held.iter().map(|&d| name(d)).collect();
            let object = format!("constraint {}", names.join(", "));
            let (rows, elem) = match self.run(t, &object)? {
                Ok(found) => found,
                Err(Denied::Fault(w) | Denied::Memory(w)) => {
                    why.extend(
                        held.iter()
                            .map(|&d| format!("constraint {} is violated by {w}", name(d))),
                    );
                    continue;
                }
                Err(Denied::Rotten(rot)) => return Err(Rot::Exec(rot).into()),
                Err(Denied::Orphaned(index)) => return Err(Rot::Orphaned(index).into()),
            };
            for row in &rows {
                let mut broken = BTreeSet::new();
                let fail = |e| failed(&object, e);
                breaks(schema, &checks, row, &elem, &mut broken).map_err(fail)?;
                for d in broken {
                    let printed = exec::print(schema, &elem, row).map_err(fail)?;
                    found.entry(d).or_default().push(printed);
                }
            }
        }
        why.extend(
            found.into_iter().map(|(d, rows)| {
                format!("constraint {} is violated by {}", name(d), rows.join(", "))
            }),
        );
        Ok(why)
    }
}

/// Returns whether a value of type `ty` can hold a value of newtype `domain`.
fn holds(schema: &Schema, ty: &Ty, domain: TypeId, seen: &mut BTreeSet<TypeId>) -> bool {
    match ty {
        Ty::Data { def, args, .. } => {
            if *def == domain {
                return true;
            }
            if args.iter().any(|a| holds(schema, a, domain, seen)) {
                return true;
            }
            if !seen.insert(*def) {
                return false;
            }
            match &schema.ty(*def).kind {
                TypeDefKind::Newtype { base, .. } => holds(schema, &base.subst(args), domain, seen),
                TypeDefKind::Sum { .. } => ctors(schema, ty)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|(_, t)| holds(schema, &t, domain, seen)),
            }
        }
        Ty::Record(fs) => fs.iter().any(|(_, t)| holds(schema, t, domain, seen)),
        Ty::Sum { ctors: cs, .. } => cs
            .iter()
            .flat_map(|(_, fs)| fs)
            .any(|(_, t)| holds(schema, t, domain, seen)),
        _ => false,
    }
}

/// Add to `broken` each newtype of `domains` whose domain a value inside `v`, of type `ty`,
/// violates.
fn breaks(
    schema: &Schema,
    domains: &BTreeMap<TypeId, exec::Scalar>,
    v: &Val,
    ty: &Ty,
    broken: &mut BTreeSet<TypeId>,
) -> Result<(), exec::Error> {
    let fields = |fs: &[(String, Ty)], vs: &[Val], broken: &mut BTreeSet<TypeId>| {
        fs.iter()
            .zip(vs)
            .try_for_each(|((_, t), x)| breaks(schema, domains, x, t, broken))
    };
    match ty {
        Ty::Data { def, args, .. } => match &schema.ty(*def).kind {
            TypeDefKind::Newtype { base, .. } => {
                if let Some(check) = domains.get(def)
                    && !check.eval(v.clone())?.truth()?
                {
                    broken.insert(*def);
                }
                breaks(schema, domains, v, &base.subst(args), broken)
            }
            TypeDefKind::Sum { .. } => {
                let cs = ctors(schema, ty).expect("a declared sum's type has constructors");
                let c = v.ctor_of()? as usize;
                fields(&cs[c], v.fields()?, broken)
            }
        },
        Ty::Record(fs) => fields(fs, v.fields()?, broken),
        Ty::Sum { ctors: cs, .. } => fields(&cs[v.ctor_of()? as usize].1, v.fields()?, broken),
        _ => Ok(()),
    }
}

/// Returns the damage a denial run met as why it could not run, any other failure as the
/// catalog's error naming `object`.
fn damaged(object: &str, e: exec::Error) -> Result<Denied, Error> {
    match e {
        exec::Error::Corrupt(rot) => Ok(Denied::Rotten(rot)),
        exec::Error::OrphanEntry(index) => Ok(Denied::Orphaned(index)),
        other => Err(failed(object, other)),
    }
}

fn failed(object: &str, e: exec::Error) -> Error {
    classify(object, e, Broken::Validation)
}
