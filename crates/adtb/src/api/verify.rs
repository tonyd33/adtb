//! The `verify` command: a run of the checks, the rows of `sys::verify_findings` it replaces, and
//! the row of `sys::jobs` while it runs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::ids::{
    CatConstraintId, CatCtorId, CatRelId, Stored, engine_ctor, engine_rel, held_id,
};
use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, Ty};
use adtb_sys::lock::Lock;

use crate::catalog::codec::{
    Codec, Ctor, Decode, Field, Fields, IdField, Relation, Row, field, listed, relation,
};
use crate::catalog::{self, Catalog, Denial, KeptField, KeptRow, Rot, engine_relation};
use crate::exec::{F64, Val};
use crate::key::Value;
use crate::mvcc::Transactions;
use crate::runtime::Runtime;
use crate::storage::{Damage, PageError, Pager, Pid};
use crate::verify::{self, Depth};
use crate::write::{self, Found, HoldWait, VerifyScope};

use super::cause::Corruption;
use super::text::{DynRows, rows};
use super::{Engine, OpenError, OpenOptions, corrupt, micros};

/// The group of the kept row holding the next run number and finding id.
const COUNTERS: &str = "counters";

/// The group of the kept rows of the scrub's findings.
const SCRUB: &str = "scrub";

/// The group of the kept row holding where the background scrub is.
const CURSOR: &str = "scrub cursor";

/// Where the background scrub is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Cursor {
    /// The next page of the pass running, or `None` between passes.
    pub(super) next: Option<Pid>,
    /// The wall instant the last pass started, in microseconds.
    pub(super) started: i64,
}

/// Returns the position of `JobKind` constructor `kind`.
fn job_kind(schema: &Schema, kind: CatCtorId) -> u32 {
    let ctor = schema.ids().ctor_of(kind);
    ctor.expect("the engine declares each `JobKind` constructor `engine_ctor` names")
        .index()
}

/// What the engine holds of `verify` between and during runs.
pub(super) struct Findings {
    /// The rows of `sys::verify_findings`, by id, each with its value.
    rows: BTreeMap<i64, (KeptRow, Val)>,
    codec: Arc<Codec<FindingRow>>,
    next_run: i64,
    next_id: i64,
    /// The row of `sys::jobs` of each running run, by run number.
    jobs: BTreeMap<i64, Val>,
    cursor: Option<Cursor>,
}

impl std::fmt::Debug for Findings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Findings")
            .field("rows", &self.rows)
            .field("next_run", &self.next_run)
            .field("next_id", &self.next_id)
            .field("jobs", &self.jobs)
            .field("cursor", &self.cursor)
            .finish_non_exhaustive()
    }
}

/// A row of `sys::verify_findings`, read for what the engine keeps track of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FindingRow {
    id: i64,
    run: i64,
    /// The catalog id of the constraint a `Violation` names.
    violated: Option<CatConstraintId>,
}

impl FindingRow {
    pub(super) fn violated(self) -> Option<CatConstraintId> {
        self.violated
    }
}

const WHERE: Fields = &["id", "run", "at", "where"];
const ORDER: Fields = &["id", "run", "at", "index", "where"];
const ORPHAN: Fields = &["id", "run", "at", "index", "ctor", "where"];
const MISSING: Fields = &["id", "run", "at", "relation", "ctor", "index", "where"];
const VIOLATION: Fields = &["id", "run", "at", "constraint", "witness"];

/// The `id`, `run` and `at` fields of one constructor's list of fields.
#[derive(Clone, Copy)]
struct Head {
    id: Field,
    run: Field,
    at: Field,
}

/// Returns the header fields of `fields`. Fails to compile in a constant when it lists one not.
const fn head(fields: Fields) -> Head {
    Head {
        id: field(fields, "id"),
        run: field(fields, "run"),
        at: field(fields, "at"),
    }
}

const WHERE_HEAD: Head = head(WHERE);
const ORDER_HEAD: Head = head(ORDER);
const ORPHAN_HEAD: Head = head(ORPHAN);
const MISSING_HEAD: Head = head(MISSING);
const VIOLATION_HEAD: Head = head(VIOLATION);

/// Returns the row whose header fields are `head`.
fn finding(row: &Row<'_>, head: Head) -> Result<FindingRow, Rot> {
    Ok(FindingRow {
        id: row.int(head.id)?,
        run: row.int(head.run)?,
        violated: None,
    })
}

impl Relation for relation::VerifyFindings {
    type Row = FindingRow;
}

impl Decode for FindingRow {
    type Relation = relation::VerifyFindings;
    const CTORS: &'static [Ctor<FindingRow>] = {
        use engine_ctor::{
            FINDING_CHECKSUM, FINDING_MISSING, FINDING_ORDER, FINDING_ORPHAN, FINDING_STALE_PAGE,
            FINDING_STRUCTURE, FINDING_VIOLATION,
        };
        &[
            Ctor::new(FINDING_CHECKSUM, WHERE, |row| finding(row, WHERE_HEAD)),
            Ctor::new(FINDING_STALE_PAGE, WHERE, |row| finding(row, WHERE_HEAD)),
            Ctor::new(FINDING_ORDER, ORDER, |row| finding(row, ORDER_HEAD)),
            Ctor::new(FINDING_ORPHAN, ORPHAN, |row| finding(row, ORPHAN_HEAD)),
            Ctor::new(FINDING_MISSING, MISSING, |row| finding(row, MISSING_HEAD)),
            Ctor::new(FINDING_VIOLATION, VIOLATION, |row| {
                const CONSTRAINT: IdField<CatConstraintId> =
                    IdField::new(field(VIOLATION, "constraint"));
                Ok(FindingRow {
                    violated: Some(row.id(CONSTRAINT)?),
                    ..finding(row, VIOLATION_HEAD)?
                })
            }),
            Ctor::new(FINDING_STRUCTURE, WHERE, |row| finding(row, WHERE_HEAD)),
        ]
    };
}

/// Returns the refusal of a kept row of `group` that is not two integers.
fn not_two_ints(group: &str) -> Rot {
    Rot::CatalogRow {
        row: format!("the {group} of sys::verify_findings"),
        detail: "is not two integers of no constructor".into(),
    }
}

/// Returns the two integers of `row`, a kept row of no constructor.
fn two_ints(row: &KeptRow) -> Result<(i64, i64), Rot> {
    match (row.ctor, row.fields.as_slice()) {
        (None, [KeptField::Int(a), KeptField::Int(b)]) => Ok((*a, *b)),
        _ => Err(not_two_ints(&row.group)),
    }
}

/// A finding as a kept row holds it: its constructor of `Finding`, and each field after the
/// header by its handle in the constructor's list.
struct Kept {
    ctor: CatCtorId,
    head: Head,
    rest: Vec<(Field, KeptField)>,
}

impl Kept {
    /// Returns the fields of a kept row of this with header `id`, `run` and `at`, in the order
    /// its constructor lists them.
    fn fields(&self, (id, run, at): (i64, i64, i64)) -> Vec<KeptField> {
        let header = [
            (self.head.id, KeptField::Int(id)),
            (self.head.run, KeptField::Int(run)),
            (self.head.at, KeptField::Instant(at)),
        ];
        listed(
            self.head.id.fields(),
            header.into_iter().chain(self.rest.iter().cloned()),
        )
    }

    /// Returns whether `row` records this finding, whatever its header.
    fn states(&self, row: &KeptRow) -> bool {
        row.ctor == Some(self.ctor)
            && row.fields.len() == self.head.id.fields().len()
            && self
                .rest
                .iter()
                .all(|(f, v)| row.fields.get(f.at()) == Some(v))
    }
}

/// Returns `f` as a kept row holds it.
fn fields(f: Found) -> Kept {
    use KeptField::{Int, Text};
    use engine_ctor::{
        FINDING_CHECKSUM, FINDING_MISSING, FINDING_ORDER, FINDING_ORPHAN, FINDING_STALE_PAGE,
        FINDING_STRUCTURE, FINDING_VIOLATION,
    };
    let id = |n: u64| Int(i64::try_from(n).unwrap_or(i64::MAX));
    let kept = |ctor, head, rest| Kept { ctor, head, rest };
    match f {
        Found::Checksum { at, .. } => kept(
            FINDING_CHECKSUM,
            WHERE_HEAD,
            vec![(const { field(WHERE, "where") }, Text(at))],
        ),
        Found::StalePage { at, .. } | Found::Settled { at, .. } => kept(
            FINDING_STALE_PAGE,
            WHERE_HEAD,
            vec![(const { field(WHERE, "where") }, Text(at))],
        ),
        Found::Order { index, at } => kept(
            FINDING_ORDER,
            ORDER_HEAD,
            vec![
                (const { field(ORDER, "index") }, id(index)),
                (const { field(ORDER, "where") }, Text(at)),
            ],
        ),
        Found::Orphan { index, ctor, at } => kept(
            FINDING_ORPHAN,
            ORPHAN_HEAD,
            vec![
                (const { field(ORPHAN, "index") }, id(index)),
                (const { field(ORPHAN, "ctor") }, id(ctor)),
                (const { field(ORPHAN, "where") }, Text(at)),
            ],
        ),
        Found::Missing {
            relation,
            ctor,
            index,
            at,
        } => kept(
            FINDING_MISSING,
            MISSING_HEAD,
            vec![
                (const { field(MISSING, "relation") }, id(relation)),
                (const { field(MISSING, "ctor") }, id(ctor)),
                (const { field(MISSING, "index") }, id(index)),
                (const { field(MISSING, "where") }, Text(at)),
            ],
        ),
        Found::Violation {
            constraint,
            witness,
        } => kept(
            FINDING_VIOLATION,
            VIOLATION_HEAD,
            vec![
                (const { field(VIOLATION, "constraint") }, id(constraint)),
                (const { field(VIOLATION, "witness") }, Text(witness)),
            ],
        ),
        Found::Structure { at } => kept(
            FINDING_STRUCTURE,
            WHERE_HEAD,
            vec![(const { field(WHERE, "where") }, Text(at))],
        ),
    }
}

fn group(scope: VerifyScope, schema: &Schema) -> String {
    match scope {
        VerifyScope::Database => "database".into(),
        VerifyScope::Relation(r) => format!("relation {}", held_id(schema.ids().relation_id(r))),
    }
}

/// Returns whether `group` is the scope of a relation `schema` no longer has.
fn removed(group: &str, schema: &Schema) -> bool {
    group
        .strip_prefix("relation ")
        .and_then(|id| id.parse::<u64>().ok())
        .is_some_and(|id| {
            let id: CatRelId = Stored::new(id).id();
            schema.ids().relation_of(id).is_none()
        })
}

impl Findings {
    /// Returns what `kept` holds of `table`, `sys::verify_findings` of `schema`.
    pub(super) fn of(
        kept: Vec<KeptRow>,
        schema: &Schema,
        table: RelId,
    ) -> Result<Findings, OpenError> {
        let id = held_id(schema.ids().relation_id(table)).raw();
        let mut out = Findings {
            rows: BTreeMap::new(),
            codec: Arc::new(Codec::new(schema)),
            next_run: 1,
            next_id: 1,
            jobs: BTreeMap::new(),
            cursor: None,
        };
        for row in kept.into_iter().filter(|k| k.relation == id) {
            let bad = |rot| corrupt(Corruption::Finding(row.clone(), rot));
            if row.group == COUNTERS {
                let (run, next) = two_ints(&row).map_err(bad)?;
                out.next_run = out.next_run.max(run);
                out.next_id = out.next_id.max(next);
                continue;
            }
            if row.group == CURSOR {
                let (next, started) = two_ints(&row).map_err(bad)?;
                let next = u64::try_from(next).map_err(|_| {
                    bad(Rot::CatalogRow {
                        row: format!("the {CURSOR} of sys::verify_findings"),
                        detail: format!("names page {next}"),
                    })
                })?;
                out.cursor = Some(Cursor {
                    next: (next != 0).then_some(Pid(next)),
                    started,
                });
                continue;
            }
            let (read, val) = out.read(&row).map_err(bad)?;
            out.next_run = out.next_run.max(read.run + 1);
            out.next_id = out.next_id.max(read.id + 1);
            out.rows.insert(read.id, (row, val));
        }
        Ok(out)
    }

    pub(super) fn vals(&self) -> BTreeSet<Val> {
        self.rows.values().map(|(_, v)| v.clone()).collect()
    }

    /// Returns what `row` holds and its value.
    ///
    /// Fail with [`Rot::CatalogRow`] when `row` names no constructor of `sys::Finding`, or its
    /// fields are not those the constructor declares, in the order [`fields`] writes them.
    fn read(&self, row: &KeptRow) -> Result<(FindingRow, Val), Rot> {
        let Some(id) = row.ctor else {
            return Err(Rot::CatalogRow {
                row: "sys::verify_findings".into(),
                detail: "names no constructor".into(),
            });
        };
        let (at, fields) = self.codec.declared(id, row.fields.clone())?;
        let (stored, vals): (Vec<Value>, Vec<Val>) = fields
            .into_iter()
            .map(|f| match f {
                KeptField::Int(n) => (Value::I64(n), Val::Int(n)),
                KeptField::Instant(n) => (Value::Instant(n), Val::Instant(n)),
                KeptField::Text(t) => (Value::Text(t.clone()), Val::text(&t)),
            })
            .unzip();
        let read = self.codec.decode(at, &stored)?;
        Ok((read, Val::ctor(at, vals)))
    }
}

/// What the verify runs and the background scrub share of the engine, so that holding it keeps no
/// engine open.
pub(super) struct Verifier<R: Runtime> {
    pub(super) rt: R,
    pub(super) pager: Pager<R>,
    pub(super) db: Arc<write::Database<R>>,
    pub(super) catalog: Arc<Catalog>,
    /// The findings of `verify` and the scrub.
    pub(super) findings: Lock<Findings>,
    /// The text to panic with in the next kept write, after its first replacement.
    pub(super) fail_kept: Lock<Option<String>>,
}

/// The rows a replacement of a group's findings leaves it with.
struct Replacement {
    fresh: BTreeSet<Found>,
    /// Rows of the group that stay as they are.
    retained: Vec<KeptRow>,
}

impl From<BTreeSet<Found>> for Replacement {
    fn from(fresh: BTreeSet<Found>) -> Replacement {
        Replacement {
            fresh,
            retained: Vec::new(),
        }
    }
}

/// The row of `sys::jobs` of one run. Removes it when dropped, a panic included.
pub(super) struct Job<'e, R: Runtime> {
    verifier: &'e Verifier<R>,
    table: RelId,
    run: i64,
    /// The position of the job's constructor of `JobKind`.
    kind: u32,
    target: String,
    started: i64,
}

/// A rebuild job's row of `sys::jobs`, shown, on its way to the job's thread.
pub(super) struct Started {
    pub(super) run: i64,
    target: String,
    at: i64,
}

impl<R: Runtime> Job<'_, R> {
    /// Returns the row, left shown, for the job's thread to take over.
    pub(super) fn hand_over(self) -> Started {
        let started = Started {
            run: self.run,
            target: self.target.clone(),
            at: self.started,
        };
        std::mem::forget(self);
        started
    }

    fn show(&self, held: &Findings) {
        self.verifier
            .db
            .set_volatile(self.table, held.jobs.values().cloned().collect());
    }

    pub(super) fn phase(&self, phase: &str, done: f64) {
        let row = Val::ctor(
            0,
            vec![
                Val::Int(self.run),
                Val::ctor(self.kind, Vec::new()),
                Val::text(&self.target),
                Val::text(phase),
                Val::F64(F64::new(done)),
                Val::Instant(self.started),
            ],
        );
        let mut held = self.verifier.findings.lock(&self.verifier.rt);
        held.jobs.insert(self.run, row);
        self.show(&held);
    }
}

impl<R: Runtime> Drop for Job<'_, R> {
    fn drop(&mut self) {
        let mut held = self.verifier.findings.lock(&self.verifier.rt);
        held.jobs.remove(&self.run);
        self.show(&held);
    }
}

impl<R: Runtime> Verifier<R> {
    fn clock(&self) -> i64 {
        micros(self.pager.wall())
    }

    fn next_run(&self) -> i64 {
        let mut held = self.findings.lock(&self.rt);
        held.next_run += 1;
        held.next_run - 1
    }

    /// Returns whether a run may write to the file: a database quarantined whole keeps nothing.
    pub(super) fn keeps(&self) -> bool {
        !self.pager.refused_whole()
    }

    /// Returns the row of `sys::jobs` of a new pass of the scrub, and its run number and start.
    pub(super) fn scrub_row(&self, schema: &Schema, table: RelId) -> (Job<'_, R>, i64, i64) {
        let run = self.next_run();
        let started = self.clock();
        let job = Job {
            verifier: self,
            table,
            run,
            kind: job_kind(schema, engine_ctor::JOB_VERIFY),
            target: String::new(),
            started,
        };
        (job, run, started)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(super) fn held_findings(&self) -> usize {
        self.findings.lock(&self.rt).rows.len()
    }

    /// Returns the codec of `sys::verify_findings` the findings are read with.
    pub(super) fn codec(&self) -> Arc<Codec<FindingRow>> {
        self.findings.lock(&self.rt).codec.clone()
    }

    /// Returns where the background scrub is, as last saved.
    pub(super) fn cursor(&self) -> Option<Cursor> {
        self.findings.lock(&self.rt).cursor
    }

    /// Save `cursor` in the file. Returns whether it is there.
    pub(super) fn save_cursor(&self, schema: &Schema, table: RelId, cursor: Cursor) -> bool {
        let next = cursor.next.map_or(0, |p| i64::try_from(p.0).unwrap_or(0));
        let row = KeptRow {
            relation: held_id(schema.ids().relation_id(table)).raw(),
            group: CURSOR.into(),
            ctor: None,
            fields: vec![KeptField::Int(next), KeptField::Int(cursor.started)],
        };
        if !self.keeps() {
            return false;
        }
        let kept = self.in_kept_transaction(|w| {
            let txns = self.db.txns();
            let group = (row.relation, CURSOR);
            self.catalog
                .replace_kept(&self.pager, txns, w, group, std::slice::from_ref(&row))
        });
        if kept.is_ok() {
            self.findings.lock(&self.rt).cursor = Some(cursor);
        }
        kept.is_ok()
    }

    /// Make the findings of the scrub the rows `old_stays` keeps and those of `found` no row
    /// states yet, and mark what the latter implicate. Writes nothing when that changes no row,
    /// and only in memory on a database quarantined whole.
    pub(super) fn keep_scrub(
        &self,
        schema: &Schema,
        table: RelId,
        (run, at): (i64, i64),
        found: &BTreeSet<Found>,
        old_stays: bool,
    ) {
        let old: Vec<KeptRow> = {
            let held = self.findings.lock(&self.rt);
            let of_scrub = held.rows.values().filter(|(r, _)| r.group == SCRUB);
            of_scrub.map(|(r, _)| r.clone()).collect()
        };
        let states = |f: &Found, r: &KeptRow| fields(f.clone()).states(r);
        let fresh: BTreeSet<Found> = found
            .iter()
            .filter(|f| !old.iter().any(|r| states(f, r)))
            .cloned()
            .collect();
        let retained: Vec<KeptRow> = old
            .iter()
            .filter(|r| old_stays || found.iter().any(|f| states(f, r)))
            .cloned()
            .collect();
        if fresh.is_empty() && retained.len() == old.len() {
            return;
        }
        let keep = self.keeps();
        let rows = Replacement { fresh, retained };
        self.replace_findings(schema, table, SCRUB, (run, at), rows, keep);
    }

    /// Make `found` the findings of `group`, of run `run` at instant `at`, and drop those of
    /// relations `schema` no longer has, in the file too when `keep`. Returns the rows, with one
    /// saying so when the file could not keep them.
    fn replace_findings(
        &self,
        schema: &Schema,
        table: RelId,
        group: &str,
        (run, at): (i64, i64),
        found: Replacement,
        keep: bool,
    ) -> Vec<Val> {
        let id = held_id(schema.ids().relation_id(table)).raw();
        let mut held = self.findings.lock(&self.rt);
        let row = |held: &mut Findings, f: Found| {
            let kept = fields(f);
            let key = held.next_id;
            held.next_id += 1;
            KeptRow {
                relation: id,
                group: group.to_owned(),
                ctor: Some(kept.ctor),
                fields: kept.fields((key, run, at)),
            }
        };
        let mut written = found.retained;
        for f in found.fresh {
            let r = row(&mut held, f);
            written.push(r);
        }
        let gone: BTreeSet<String> = held
            .rows
            .values()
            .map(|(r, _)| r.group.clone())
            .filter(|g| removed(g, schema))
            .collect();
        let counters = KeptRow {
            relation: id,
            group: COUNTERS.into(),
            ctor: None,
            fields: vec![KeptField::Int(held.next_run), KeptField::Int(held.next_id)],
        };
        let kept = if keep {
            self.keep_findings(id, group, &written, &gone, &counters)
        } else {
            Ok(())
        };
        if let Err(why) = kept {
            let at = format!("the findings of run {run} were not kept: {why}");
            written.push(row(&mut held, Found::Structure { at }));
        }
        held.rows
            .retain(|_, (r, _)| r.group != group && !gone.contains(&r.group));
        let mut vals = Vec::new();
        for r in written {
            let (read, val) = held
                .read(&r)
                .unwrap_or_else(|rot| panic!("a finding row the engine writes reads back: {rot}"));
            vals.push(val.clone());
            held.rows.insert(read.id, (r, val));
        }
        self.db.set_volatile(table, held.vals());
        vals
    }

    fn keep_findings(
        &self,
        id: u64,
        group: &str,
        written: &[KeptRow],
        gone: &BTreeSet<String>,
        counters: &KeptRow,
    ) -> Result<(), String> {
        self.in_kept_transaction(|w| {
            let txns = self.db.txns();
            self.catalog
                .replace_kept(&self.pager, txns, w, (id, group), written)?;
            let fail = self.fail_kept.lock(&self.rt).take();
            if let Some(text) = fail {
                panic!("{text}");
            }
            for g in gone {
                self.catalog
                    .replace_kept(&self.pager, txns, w, (id, g), &[])?;
            }
            self.catalog.replace_kept(
                &self.pager,
                txns,
                w,
                (id, COUNTERS),
                std::slice::from_ref(counters),
            )
        })
    }

    /// Run `write`, a change of kept rows, as one transaction of the write path.
    fn in_kept_transaction(
        &self,
        write: impl FnOnce(catalog::Writer<'_>) -> Result<(), catalog::Error>,
    ) -> Result<(), String> {
        let tx = self.db.begin().map_err(|e| self.db.detail(&e))?;
        let w = tx
            .catalog_writer()
            .ok_or_else(|| "a read-write transaction has an id".to_owned())?;
        if let Err(e) = write(w) {
            let reply = self
                .db
                .abandon(tx, || catalog::detail(&e), |r| self.db.detail(&r));
            return Err(write::either(reply));
        }
        self.db.commit(tx).map(drop).map_err(|e| self.db.detail(&e))
    }
}

impl<R: Runtime> Engine<R> {
    /// Returns the row of `sys::jobs` of a new rebuild job, which its [`Job`] will remove.
    pub(super) fn reindex_row(&self, schema: &Schema, table: RelId, target: String) -> Job<'_, R> {
        let run = self.verifier.next_run();
        let job = Job {
            verifier: &self.verifier,
            table,
            run,
            kind: job_kind(schema, engine_ctor::JOB_REINDEX),
            target,
            started: self.clock(),
        };
        job.phase("waiting", 0.0);
        job
    }

    /// Returns what a job's thread holds of its row.
    pub(super) fn reindex_row_of(
        &self,
        schema: &Schema,
        table: RelId,
        started: Started,
    ) -> Job<'_, R> {
        Job {
            verifier: &self.verifier,
            table,
            run: started.run,
            kind: job_kind(schema, engine_ctor::JOB_REINDEX),
            target: started.target,
            started: started.at,
        }
    }

    /// Make `found` the findings of the last rebuild of the index with catalog id `index`.
    pub(super) fn keep_reindex_findings(
        &self,
        schema: &Schema,
        index: u64,
        run: i64,
        found: BTreeSet<Found>,
    ) {
        let table = engine_relation(schema, engine_rel::VERIFY_FINDINGS);
        let group = format!("reindex {index}");
        let at = (run, self.clock());
        self.verifier
            .replace_findings(schema, table, &group, at, found.into(), true);
    }

    /// Run `verify` over `rel`, or the whole database. Returns the rows it wrote.
    ///
    /// Precondition: the calling thread holds no transaction with an id.
    pub(super) fn verified(
        &self,
        schema: &Schema,
        rel: Option<RelId>,
        waiting: HoldWait<'_>,
    ) -> Result<DynRows, write::Error> {
        write::outside_writer();
        let scope = rel.map_or(VerifyScope::Database, VerifyScope::Relation);
        self.verify_run(schema, scope, false, waiting)
    }

    /// Run `verify` over the whole database with the checks that hold only with no writer as well,
    /// holding off vacuum, the background worker's included, for the whole run.
    ///
    /// Precondition: no other call runs on this database while it runs.
    pub(super) fn verified_holding(&self, schema: &Schema) -> Result<DynRows, write::Error> {
        self.verify_run(schema, VerifyScope::Database, true, HoldWait::unbounded())
    }

    fn verify_run(
        &self,
        schema: &Schema,
        scope: VerifyScope,
        holding: bool,
        waiting: HoldWait<'_>,
    ) -> Result<DynRows, write::Error> {
        let table = engine_relation(schema, engine_rel::VERIFY_FINDINGS);
        let run = self.verifier.next_run();
        let started = self.clock();
        let job = Job {
            verifier: &self.verifier,
            table: engine_relation(schema, engine_rel::JOBS),
            run,
            kind: job_kind(schema, engine_ctor::JOB_VERIFY),
            target: match scope {
                VerifyScope::Relation(r) => schema.relation(r).name.clone(),
                VerifyScope::Database => String::new(),
            },
            started,
        };
        let mut found = self.verify_checks(scope, holding, waiting, &job)?;
        if self.db.admit_database().is_ok() {
            found.extend(self.db.quarantine_findings(&found));
        }
        let keep = self.verifier.keeps();
        if !keep {
            found.insert(Found::Structure {
                at: "the database: quarantined whole, so the findings of this run are not kept"
                    .to_owned(),
            });
        }
        let group = group(scope, schema);
        let stamp = (run, started);
        let vals = self
            .verifier
            .replace_findings(schema, table, &group, stamp, found.into(), keep);
        let elem = Ty::Data {
            def: schema.relation(table).elem_def,
            args: Vec::new(),
            ctors: None,
        };
        Ok(rows(schema, elem, vals))
    }

    fn verify_checks(
        &self,
        scope: VerifyScope,
        holding: bool,
        waiting: HoldWait<'_>,
        job: &Job<'_, R>,
    ) -> Result<BTreeSet<Found>, write::Error> {
        let held = if holding {
            Some(self.db.hold_vacuum(waiting)?)
        } else {
            None
        };
        let mut found = BTreeSet::new();
        let damage = match self.snapshot_checks(scope, held.as_ref(), waiting, job, &mut found) {
            Ok(()) => return Ok(found),
            Err(e) => self.commit_log_damage(&e).ok_or(e)?,
        };
        job.phase("pages", 0.1);
        found.extend(self.db.check_pages(scope)?);
        found.insert(Found::of_page(damage));
        let at = if matches!(scope, VerifyScope::Relation(_)) {
            format!("relation {}: only its pages were checked", job.target)
        } else {
            "the database: only the pages and the log were checked".to_owned()
        };
        found.insert(Found::Structure { at });
        Ok(found)
    }

    fn snapshot_checks(
        &self,
        scope: VerifyScope,
        held: Option<&write::VacuumHold<'_, R>>,
        waiting: HoldWait<'_>,
        job: &Job<'_, R>,
        found: &mut BTreeSet<Found>,
    ) -> Result<(), write::Error> {
        let holding = held.is_some();
        job.phase("waiting for a safe snapshot", 0.0);
        let snapshot = self.db.begin_verify()?;
        let tx = snapshot.txn();
        job.phase("pages", 0.1);
        found.extend(self.db.check_pages(scope)?);
        job.phase("indexes", 0.4);
        let mut indexed = Vec::new();
        let checked = match held {
            Some(held) => held.check_indexes(tx, scope, &mut indexed),
            None => self.db.check_indexes(tx, scope, waiting, &mut indexed),
        };
        found.extend(indexed);
        checked?;
        if !holding {
            found.extend(self.db.check_links(scope));
        }
        job.phase("constraints", 0.7);
        let reads = |r: &BTreeSet<RelId>| match scope {
            VerifyScope::Database => true,
            VerifyScope::Relation(rel) => r.contains(&rel),
        };
        let denied = self.catalog.denied(&snapshot.store(), reads)?;
        found.extend(denied.into_iter().map(|d| match d {
            Denial::Violated { id, witness } => Found::Violation {
                constraint: id,
                witness,
            },
            Denial::Damaged(e) => Found::of_page(e),
            Denial::Rotten { id, rot } => Found::Structure {
                at: format!("constraint {id}: {rot}"),
            },
            Denial::Orphaned { id, index } => Found::Structure {
                at: format!("constraint {id}: an entry of index {index} names no version"),
            },
            Denial::Unrun { id, why } => Found::Structure {
                at: format!("constraint {id}: {why}"),
            },
        }));
        if holding {
            job.phase("the file", 0.9);
            let clog = self.db.txns().commit_log();
            let inventory = self.db.structures().inventory(Some(&clog));
            let report = verify::verify_with(&self.pager, &inventory, Depth::FILE);
            found.extend(report.findings.iter().filter_map(Found::of_problem));
        }
        Ok(())
    }

    /// Returns the damage that quarantined the commit log when `e` is that damage met by a read,
    /// or the refusal of a transaction it causes. Returns `None` for an unreadable page.
    fn commit_log_damage(&self, e: &write::Error) -> Option<PageError> {
        let damage = self.pager.whole_cause()?;
        if matches!(damage.damage, Damage::Unreadable) {
            return None;
        }
        match e {
            write::Error::Transactions(_) => Some(damage),
            e => (e.page() == Some(damage)).then_some(damage),
        }
    }

    /// Refuse every page of the heap kept rows lie in, as if each failed its checksum.
    #[cfg(any(test, feature = "testing"))]
    pub(super) fn damage_kept_store(&self) {
        if let Ok(pages) = self.catalog.kept_pages(&self.pager) {
            for pid in pages {
                self.pager.plant_damage(pid, Damage::Checksum);
            }
        }
    }
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;

/// Returns what `verify` holds of the file and the rows of the volatile relations, with no kept
/// rows when the commit log's damage leaves them unreadable.
pub(super) fn held_rows<R: Runtime>(
    catalog: &Catalog,
    pager: &Pager<R>,
    txns: &Transactions<R>,
    schema: &Schema,
    options: &OpenOptions,
    clog_damaged: bool,
) -> Result<(Findings, BTreeMap<RelId, BTreeSet<Val>>), OpenError> {
    let kept = if clog_damaged {
        Vec::new()
    } else {
        catalog.kept(pager, txns).map_err(corrupt)?
    };
    let table = engine_relation(schema, engine_rel::VERIFY_FINDINGS);
    let findings = Findings::of(kept, schema, table)?;
    let mut volatile = super::settings::process_options(schema, &options.shown());
    volatile.insert(table, findings.vals());
    Ok((findings, volatile))
}
