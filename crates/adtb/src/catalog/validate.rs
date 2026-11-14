use std::collections::BTreeSet;
use std::sync::Arc;

use adtb_lang::kernel::{Kind, Terms, Tm};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, RelRef, Ty};

use crate::exec::{self, Budget, Call, Store, Val};
use crate::plan;
use crate::runtime::Runtime;
use crate::storage::PageError;

use super::hosts::unbound;
use super::ids::{CatConstraintId, CatRelId};
use super::{Broken, Catalog, Error, Migrating};

impl Catalog {
    /// Returns, for each constraint and check of this catalog that `old` lacks or that reads a
    /// relation whose catalog id is in `rewritten`, why the rows transaction `m.x` sees at
    /// the snapshot of `m.hold` violate it.
    ///
    /// A constraint that reads a transition is not validated.
    ///
    /// Fail with [`Broken::ConstraintId`] when a constraint or check has no catalog id, and with
    /// [`Broken::ConstraintTerm`] when it has no normal form.
    pub(super) fn violations<R: Runtime>(
        &self,
        m: &Migrating<'_, R>,
        old: &Schema,
        rewritten: &BTreeSet<CatRelId>,
    ) -> Result<Vec<String>, Error> {
        let (pager, x) = (m.pager, m.x);
        let (schema, program) = self.checking();
        let normal =
            norm::program(schema, program).map_err(|e| Error::from(Broken::NotNormal(e)))?;
        let existed: BTreeSet<CatConstraintId> = old
            .ids()
            .constraint_ids()
            .map(|(_, id)| id)
            .chain(old.ids().check_ids().map(|(_, id)| id))
            .collect();
        let catalog = self.plan();
        let bindings = self.bindings();
        let store = Arc::new(Store::migrating(
            m.txns.reader(pager, m.hold, Some(x)),
            bindings,
        ));
        let budget = Budget::new(self.query_memory);
        let terms = &normal.terms;
        let mut why = Vec::new();
        for Held { name, id, t } in held(schema, &normal)? {
            let mut reads = BTreeSet::new();
            if transitions(terms, t, &mut reads) {
                continue;
            }
            let touched = reads.iter().any(|r| {
                schema
                    .ids()
                    .relation_id(*r)
                    .is_some_and(|id| rewritten.contains(&id))
            });
            if existed.contains(&id) && !touched {
                continue;
            }
            let found = violating(schema, &catalog, &normal, t, &store, &budget, &self.hosts)?;
            if !found.is_empty() {
                why.push(format!(
                    "constraint {name} is violated by {}",
                    found.join(", ")
                ));
            }
        }
        Ok(why)
    }
}

impl Catalog {
    /// Returns what the denial query of each constraint and check of this catalog found over
    /// `store`, for those no transition reads and that `keep` accepts by the relations they read.
    pub fn denied<R: Runtime>(
        &self,
        store: &Arc<Store<R>>,
        keep: impl Fn(&BTreeSet<RelId>) -> bool,
    ) -> Result<Vec<Denial>, Error> {
        let (schema, program) = self.checking();
        let normal =
            norm::program(schema, program).map_err(|e| Error::from(Broken::NotNormal(e)))?;
        let catalog = self.plan();
        let budget = Budget::new(self.query_memory);
        let mut out = Vec::new();
        for Held { id, t, .. } in held(schema, &normal)? {
            let id = id.raw();
            let mut reads = BTreeSet::new();
            if transitions(&normal.terms, t, &mut reads) || !keep(&reads) {
                continue;
            }
            let found = run_denial(schema, &catalog, &normal, t, store, &budget, &self.hosts);
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

/// Adds the relations `t` reads to `reads`; returns whether it reads a transition.
fn transitions(terms: &Terms, t: Tm, reads: &mut BTreeSet<RelId>) -> bool {
    let mut stack = vec![t];
    let mut seen = BTreeSet::new();
    let mut found = false;
    while let Some(t) = stack.pop() {
        if !seen.insert(t) {
            continue;
        }
        match terms.kind(t) {
            Kind::Transition(..) => found = true,
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
    found
}

/// Returns each row constraint `t` returns over `store`, printed, or why it could not run,
/// that printed as its one row.
fn violating<R: Runtime>(
    schema: &Schema,
    catalog: &plan::Catalog,
    normal: &NormalProgram,
    t: Tm,
    store: &Arc<Store<R>>,
    budget: &Arc<Budget>,
    hosts: &Arc<exec::Hosts>,
) -> Result<Vec<String>, Error> {
    Ok(
        match run_denial(schema, catalog, normal, t, store, budget, hosts)? {
            Ok(rows) => rows,
            Err(Denied::Fault(why) | Denied::Memory(why)) => vec![why],
            Err(Denied::Rotten(rot)) => return Err(failed(exec::Error::Corrupt(rot))),
            Err(Denied::Orphaned(index)) => return Err(failed(exec::Error::OrphanEntry(index))),
        },
    )
}

/// Returns each row constraint `t` returns over `store`, printed, or why it could not run.
fn run_denial<R: Runtime>(
    schema: &Schema,
    catalog: &plan::Catalog,
    normal: &NormalProgram,
    t: Tm,
    store: &Arc<Store<R>>,
    budget: &Arc<Budget>,
    hosts: &Arc<exec::Hosts>,
) -> Result<Result<Vec<String>, Denied>, Error> {
    let terms = &normal.terms;
    let p = plan::plan(schema, normal, catalog, terms, t, plan::Options::default())
        .map_err(|e| Error::from(Broken::Plan(e)))?;
    let mut program = match exec::compile(schema, catalog, &p, None) {
        Ok(program) => program,
        Err(e) => return damaged(e).map(Err),
    };
    program.shared.hosts = hosts.clone();
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
    let v = match exec::execute(store.pager().rt().clone(), program, call, &[]) {
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
        Err(e) => return damaged(e).map(Err),
    };
    let Val::Set(rows) = &v else {
        return Err(Error::from(Broken::NotASet(v)));
    };
    let elem = match terms.ty(t) {
        Ty::Set(e) => e.as_ref().clone(),
        other => {
            return Err(Error::from(Broken::NotSetTyped(other.clone())));
        }
    };
    let mut printed = Vec::new();
    for r in rows.iter() {
        match exec::print(schema, &elem, r) {
            Ok(row) => printed.push(row),
            Err(e) => return damaged(e).map(Err),
        }
    }
    Ok(Ok(printed))
}

/// Returns the damage a denial run met as why it could not run, any other failure as the
/// catalog's error.
fn damaged(e: exec::Error) -> Result<Denied, Error> {
    match e {
        exec::Error::Corrupt(rot) => Ok(Denied::Rotten(rot)),
        exec::Error::OrphanEntry(index) => Ok(Denied::Orphaned(index)),
        other => Err(failed(other)),
    }
}

fn failed(e: exec::Error) -> Error {
    if let Some(e) = unbound(&e) {
        return e;
    }
    match e {
        exec::Error::Page(p) => Error::Page(p),
        other => Error::from(Broken::Validation(other)),
    }
}
