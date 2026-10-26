use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use adtb_lang::kernel::{Kind, Terms, Tm};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, Ty};

use crate::exec::{self, AtSnapshot, Budget, Call, Store, Val};
use crate::mvcc::{Snapshot, Transactions, Xid};
use crate::plan;
use crate::runtime::Runtime;
use crate::storage::Pager;

use super::{Catalog, Error};

/// The query memory one constraint's validation may hold.
const VALIDATION_MEMORY: usize = 1 << 30;

impl Catalog {
    /// Returns, for each constraint and check of this catalog that `old` lacks or that reads a
    /// relation whose catalog id is in `rewritten`, why the rows transaction `x` sees at
    /// `snapshot` violate it.
    ///
    /// A constraint that reads a transition is not validated.
    pub(super) fn violations<R: Runtime>(
        &self,
        pager: &Pager<R>,
        txns: &Transactions<R>,
        x: Xid,
        snapshot: &Snapshot,
        old: &Schema,
        rewritten: &BTreeSet<u64>,
    ) -> Result<Vec<String>, Error> {
        let schema = &self.schema;
        let (_, program) = self
            .source
            .check()
            .map_err(|ds| Error::Internal(format!("an applied migration checks: {ds:?}")))?;
        let normal = norm::program(schema, &program)
            .map_err(|e| Error::Internal(format!("a checked schema normalises: {e:?}")))?;
        let existed: BTreeSet<String> = qualified(old).into_iter().collect();
        let catalog = self.plan();
        let (heaps, indexes) = self.bindings();
        let store = Arc::new(Store {
            pager: Arc::new(pager.clone()),
            reads: Arc::new(AtSnapshot {
                rt: pager.rt.clone(),
                pager: Arc::new(pager.clone()),
                txns: Arc::new(txns.clone()),
                snapshot: snapshot.clone(),
                reader: Some(x),
                serial: None,
            }),
            heaps,
            indexes,
        });
        let budget = Budget::new(VALIDATION_MEMORY);
        let terms = &normal.terms;
        let named = qualified(schema)
            .into_iter()
            .zip(normal.constraints.iter().chain(&normal.checks));
        let mut why = Vec::new();
        for (name, &t) in named {
            let mut reads = BTreeSet::new();
            if transitions(terms, t, &mut reads) {
                continue;
            }
            let touched = reads.iter().any(|r| {
                schema
                    .ids
                    .relations
                    .get(r)
                    .is_some_and(|id| rewritten.contains(id))
            });
            if existed.contains(&name) && !touched {
                continue;
            }
            let found = violating(schema, &catalog, &normal, t, &store, &budget, pager)?;
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

/// The qualified name of every constraint, then every check, of `schema`, in declaration order.
fn qualified(schema: &Schema) -> Vec<String> {
    let name = |m, n: &str| format!("{}::{n}", schema.module(m).name);
    schema
        .constraints
        .iter()
        .map(|c| name(c.module, &c.name))
        .chain(
            schema
                .checks
                .iter()
                .filter(|c| c.relation.0 != u32::MAX)
                .map(|c| name(c.module, &c.name)),
        )
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
                if let Ty::Key { rel, .. } = terms.ty(*key) {
                    reads.insert(*rel);
                }
            }
            _ => {}
        }
        stack.extend(terms.children(t));
    }
    found
}

/// Returns each row constraint `t` returns over `store`, printed.
fn violating<R: Runtime>(
    schema: &Schema,
    catalog: &plan::Catalog,
    normal: &NormalProgram,
    t: Tm,
    store: &Arc<Store<R>>,
    budget: &Arc<Budget>,
    pager: &Pager<R>,
) -> Result<Vec<String>, Error> {
    let terms = &normal.terms;
    let internal = Error::Internal;
    let p = plan::plan(
        schema,
        normal,
        catalog,
        terms,
        t,
        &[],
        plan::Options::default(),
    )
    .map_err(|e| internal(e.to_string()))?;
    let program = Arc::new(exec::compile(schema, catalog, &p, None).map_err(failed)?);
    let grant = budget
        .admit(VALIDATION_MEMORY)
        .expect("one validation runs at a time");
    let call = Call {
        store: store.clone(),
        grant,
        pool: None,
        options: exec::Options::default(),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let v = match exec::execute(pager.rt.clone(), program, call, &[]) {
        Ok(v) => v,
        Err(exec::Error::Fault(f)) => {
            return Ok(vec![format!("its query faults with {}", f.name())]);
        }
        Err(exec::Error::Rejected(why)) => return Ok(vec![format!("its query rejects: {why}")]),
        Err(e) => return Err(failed(e)),
    };
    let Val::Set(rows) = &v else {
        return Err(internal(format!("a constraint returns a set, found {v:?}")));
    };
    let elem = match terms.ty(t) {
        Ty::Set(e) => e.as_ref().clone(),
        other => {
            return Err(internal(format!(
                "a constraint is typed as a set, found {other:?}"
            )));
        }
    };
    rows.iter()
        .map(|r| exec::print(schema, &elem, r).map_err(failed))
        .collect()
}

fn failed(e: exec::Error) -> Error {
    match e {
        exec::Error::Page(p) => Error::Page(p),
        other => Error::Internal(format!("{other:?}")),
    }
}
