//! The `verify` command: a run of the checks, the rows of `sys::verify_findings` it replaces, and
//! the row of `sys::jobs` while it runs.

use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, Ty};

use crate::catalog::{Denial, KeptField, KeptRow};
use crate::exec::{F64, Val};
use crate::runtime::Runtime;
use crate::verify::{self, Depth};
use crate::write::{self, Found, VerifyScope};

use super::command::engine_relation;
use super::text::{DynRows, rows};
use super::{Engine, OpenError, corrupt};

/// The constructor of `Verify` in `JobKind`.
const VERIFY_JOB: u32 = 3;

/// The group of the kept row holding the next run number and finding id.
const COUNTERS: &str = "counters";

/// What the engine holds of `verify` between and during runs.
#[derive(Debug)]
pub(super) struct Findings {
    /// The rows of `sys::verify_findings`, by id.
    rows: BTreeMap<i64, KeptRow>,
    next_run: i64,
    next_id: i64,
    /// The row of `sys::jobs` of each running run, by run number.
    jobs: BTreeMap<i64, Val>,
}

fn int(f: &KeptField) -> Option<i64> {
    match f {
        KeptField::Int(n) => Some(*n),
        KeptField::Instant(_) | KeptField::Text(_) => None,
    }
}

fn val(row: &KeptRow) -> Val {
    Val::ctor(
        row.ctor,
        row.fields
            .iter()
            .map(|f| match f {
                KeptField::Int(n) => Val::Int(*n),
                KeptField::Instant(n) => Val::Instant(*n),
                KeptField::Text(t) => Val::text(t),
            })
            .collect(),
    )
}

/// Returns the constructor of `f` in `Finding` and the fields after its header.
fn fields(f: Found) -> (u32, Vec<KeptField>) {
    use KeptField::{Int, Text};
    let id = |n: u64| Int(i64::try_from(n).unwrap_or(i64::MAX));
    match f {
        Found::Checksum { at } => (0, vec![Text(at)]),
        Found::StalePage { at } => (1, vec![Text(at)]),
        Found::Order { index, at } => (2, vec![id(index), Text(at)]),
        Found::Orphan { index, ctor, at } => (3, vec![id(index), id(ctor), Text(at)]),
        Found::Missing {
            relation,
            ctor,
            index,
            at,
        } => (4, vec![id(relation), id(ctor), id(index), Text(at)]),
        Found::Violation {
            constraint,
            witness,
        } => (5, vec![id(constraint), Text(witness)]),
        Found::Structure { at } => (6, vec![Text(at)]),
    }
}

fn group(scope: VerifyScope, schema: &Schema) -> String {
    match scope {
        VerifyScope::Database => "database".into(),
        VerifyScope::Scrub => "scrub".into(),
        VerifyScope::Relation(r) => format!("relation {}", schema.ids.relations[&r]),
    }
}

/// Returns whether `group` is the scope of a relation `schema` no longer has.
fn removed(group: &str, schema: &Schema) -> bool {
    group
        .strip_prefix("relation ")
        .and_then(|id| id.parse::<u64>().ok())
        .is_some_and(|id| !schema.ids.relations.values().any(|&r| r == id))
}

impl Findings {
    /// Returns what `kept` holds of relation `id`, the catalog id of `sys::verify_findings`.
    pub(super) fn of(kept: Vec<KeptRow>, id: u64) -> Result<Findings, OpenError> {
        let mut out = Findings {
            rows: BTreeMap::new(),
            next_run: 1,
            next_id: 1,
            jobs: BTreeMap::new(),
        };
        for row in kept.into_iter().filter(|k| k.relation == id) {
            let bad = || corrupt(format!("a finding holds {row:?}"));
            if row.group == COUNTERS {
                let [run, next] = row.fields.as_slice() else {
                    return Err(bad());
                };
                out.next_run = out.next_run.max(int(run).ok_or_else(bad)?);
                out.next_id = out.next_id.max(int(next).ok_or_else(bad)?);
                continue;
            }
            let key = row.fields.first().and_then(int).ok_or_else(bad)?;
            let run = row.fields.get(1).and_then(int).ok_or_else(bad)?;
            out.next_run = out.next_run.max(run + 1);
            out.next_id = out.next_id.max(key + 1);
            out.rows.insert(key, row);
        }
        Ok(out)
    }

    pub(super) fn vals(&self) -> BTreeSet<Val> {
        self.rows.values().map(val).collect()
    }
}

/// The row of `sys::jobs` of one run. Removes it when dropped, a panic included.
struct Job<'e, R: Runtime> {
    engine: &'e Engine<R>,
    table: RelId,
    run: i64,
    target: String,
    started: i64,
}

impl<R: Runtime> Job<'_, R> {
    fn show(&self, held: &Findings) {
        self.engine
            .db
            .set_volatile(self.table, held.jobs.values().cloned().collect());
    }

    fn phase(&self, phase: &str, done: f64) {
        let row = Val::ctor(
            0,
            vec![
                Val::Int(self.run),
                Val::ctor(VERIFY_JOB, Vec::new()),
                Val::text(&self.target),
                Val::text(phase),
                Val::F64(F64::new(done)),
                Val::Instant(self.started),
            ],
        );
        let mut held = self.engine.findings.lock(&self.engine.rt);
        held.jobs.insert(self.run, row);
        self.show(&held);
    }
}

impl<R: Runtime> Drop for Job<'_, R> {
    fn drop(&mut self) {
        let mut held = self.engine.findings.lock(&self.engine.rt);
        held.jobs.remove(&self.run);
        self.show(&held);
    }
}

impl<R: Runtime> Engine<R> {
    /// Run `verify` over `rel`, or the whole database. Returns the rows it wrote.
    ///
    /// Precondition: the calling thread holds no transaction with an id.
    pub(super) fn verified(
        &self,
        schema: &Schema,
        rel: Option<RelId>,
    ) -> Result<DynRows, write::Error> {
        let scope = rel.map_or(VerifyScope::Database, VerifyScope::Relation);
        self.verify_run(schema, scope, false)
    }

    /// Run `verify` over the whole database with the checks that hold only with no writer as well.
    ///
    /// Precondition: no other call runs on this database while it runs.
    pub(super) fn verified_holding(&self, schema: &Schema) -> Result<DynRows, write::Error> {
        self.verify_run(schema, VerifyScope::Database, true)
    }

    fn verify_run(
        &self,
        schema: &Schema,
        scope: VerifyScope,
        holding: bool,
    ) -> Result<DynRows, write::Error> {
        let table = engine_relation(schema, "sys", "verify_findings")?;
        let run = {
            let mut held = self.findings.lock(&self.rt);
            held.next_run += 1;
            held.next_run - 1
        };
        let started = self.clock();
        let job = Job {
            engine: self,
            table: engine_relation(schema, "sys", "jobs")?,
            run,
            target: match scope {
                VerifyScope::Relation(r) => schema.relation(r).name.clone(),
                VerifyScope::Database | VerifyScope::Scrub => String::new(),
            },
            started,
        };
        let found = self.verify_checks(scope, holding, &job)?;
        let vals = self.replace_findings(schema, table, scope, (run, started), found);
        let elem = Ty::Data {
            def: schema.relation(table).elem_def,
            args: Vec::new(),
            ctors: None,
        };
        Ok(rows(schema, elem, vals))
    }

    /// Make `found` the findings of `scope`, of run `run` at instant `at`, and drop those of
    /// relations `schema` no longer has. Returns the rows, with one saying so when the file could
    /// not keep them.
    fn replace_findings(
        &self,
        schema: &Schema,
        table: RelId,
        scope: VerifyScope,
        (run, at): (i64, i64),
        found: BTreeSet<Found>,
    ) -> Vec<Val> {
        let id = schema.ids.relations[&table];
        let group = group(scope, schema);
        let mut held = self.findings.lock(&self.rt);
        let row = |held: &mut Findings, f: Found| {
            let (ctor, rest) = fields(f);
            let key = held.next_id;
            held.next_id += 1;
            let mut fields = vec![
                KeptField::Int(key),
                KeptField::Int(run),
                KeptField::Instant(at),
            ];
            fields.extend(rest);
            KeptRow {
                relation: id,
                group: group.clone(),
                ctor,
                fields,
            }
        };
        let mut written: Vec<KeptRow> = found.into_iter().map(|f| row(&mut held, f)).collect();
        let gone: BTreeSet<String> = held
            .rows
            .values()
            .map(|r| r.group.clone())
            .filter(|g| removed(g, schema))
            .collect();
        let counters = KeptRow {
            relation: id,
            group: COUNTERS.into(),
            ctor: 0,
            fields: vec![KeptField::Int(held.next_run), KeptField::Int(held.next_id)],
        };
        if let Err(why) = self.keep_findings(id, &group, &written, &gone, &counters) {
            let at = format!("the findings of run {run} were not kept: {why}");
            written.push(row(&mut held, Found::Structure { at }));
        }
        held.rows
            .retain(|_, r| r.group != group && !gone.contains(&r.group));
        for r in &written {
            if let Some(key) = r.fields.first().and_then(int) {
                held.rows.insert(key, r.clone());
            }
        }
        self.db.set_volatile(table, held.vals());
        written.iter().map(val).collect()
    }

    fn keep_findings(
        &self,
        id: u64,
        group: &str,
        written: &[KeptRow],
        gone: &BTreeSet<String>,
        counters: &KeptRow,
    ) -> Result<(), String> {
        let shown = |e: &dyn std::fmt::Debug| format!("{e:?}");
        let tx = self.db.begin().map_err(|e| shown(&e))?;
        let x = tx
            .xid()
            .ok_or_else(|| "a read-write transaction has an id".to_owned())?;
        let txns = self.db.txns();
        let mut replaced = self
            .catalog
            .replace_kept(&self.pager, txns, x, (id, group), written);
        for g in gone {
            replaced = replaced.and_then(|()| {
                self.catalog
                    .replace_kept(&self.pager, txns, x, (id, g), &[])
            });
        }
        replaced = replaced.and_then(|()| {
            self.catalog.replace_kept(
                &self.pager,
                txns,
                x,
                (id, COUNTERS),
                std::slice::from_ref(counters),
            )
        });
        if let Err(e) = replaced {
            self.db.abort(tx).map_err(|e| shown(&e))?;
            return Err(shown(&e));
        }
        self.db.commit(tx).map(drop).map_err(|e| shown(&e))
    }

    fn verify_checks(
        &self,
        scope: VerifyScope,
        holding: bool,
        job: &Job<'_, R>,
    ) -> Result<BTreeSet<Found>, write::Error> {
        job.phase("waiting for a safe snapshot", 0.0);
        let snapshot = self.db.begin_verify()?;
        let tx = snapshot.txn();
        let mut found = BTreeSet::new();
        job.phase("pages", 0.1);
        found.extend(self.db.check_pages(scope)?);
        job.phase("indexes", 0.4);
        found.extend(self.db.check_indexes(tx, scope)?);
        if !holding {
            found.extend(self.db.check_links(scope));
        }
        job.phase("constraints", 0.7);
        let reads = |r: &BTreeSet<RelId>| match scope {
            VerifyScope::Database => true,
            VerifyScope::Relation(rel) => r.contains(&rel),
            VerifyScope::Scrub => false,
        };
        let denied = self
            .catalog
            .denied(&self.db.store_at(tx), reads)
            .map_err(|e| write::Error::Internal(format!("{e:?}")))?;
        found.extend(denied.into_iter().map(|d| match d {
            Denial::Violated { id, witness } => Found::Violation {
                constraint: id,
                witness,
            },
            Denial::Damaged(e) => Found::of_page(e),
            Denial::Unrun { id, why } => Found::Structure {
                at: format!("constraint {id}: {why}"),
            },
        }));
        if holding {
            job.phase("the file", 0.9);
            let clog = self.db.txns().commit_log();
            let report =
                verify::verify_with(&self.pager, &self.catalog.inventory(&clog), Depth::FILE);
            found.extend(report.findings.iter().filter_map(Found::of_problem));
        }
        Ok(found)
    }

    /// Refuse every page of the heap kept rows lie in, as if each failed its checksum.
    pub(super) fn damage_kept_store(&self) {
        if let Ok(pages) = self.catalog.kept_pages(&self.pager) {
            for pid in pages {
                self.pager.damage(pid, crate::storage::Damage::Checksum);
            }
        }
    }
}
