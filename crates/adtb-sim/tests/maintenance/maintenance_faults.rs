//! Search crashes and failed writes and forces through `Database` while a vacuum, a reindex and a
//! migration run after acknowledged commits, and rot each page of the closed file. Every expected
//! row comes from a table written here.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use adtb::key::Value;
use adtb::runtime::{Clock, FileId, Instant, IoError, Op, Runtime, Threads};
use adtb::storage::{self, PageKind, Pid, Quarantine};
use adtb::testing::Knobs;
use adtb::{
    CallOptions, Database, DynRows, Error, FatalReason, Log, OpenError, OpenOptions,
    TxnCallOptions, TxnOptions,
};
use adtb_lang::textual::print;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::crash::silence_fatal_panics;
use adtb_simkit::search::{self, Files, Hooked, WriteOrSync};
use adtb_simkit::{api, common};
use adtb_testkit::profile::{self, assert_reached};

const FILE: &str = "maintenance.adt";
const SEED: u64 = 97;
const FAILED_OPS: u64 = 1;
const CRASH_POINTS: u64 = 2;
const CRASH_IMAGES: usize = 2;
const JOB_ROUNDS: usize = 4096;
const FILL_RANK: i64 = 7;
const FILLER_ROWS: usize = 150;
const FILLER_BYTES: usize = 1500;
const SEARCH_POOL_BYTES: u64 = 256 * 1024;

const SCHEMA: &str = r"module app
add module app
add type Row = { id: Key rows, name: Text }
add relation rows : Row key id generate sequence
add index (name) in rows
add type Filler = { id: Key filler, body: Text }
add relation filler : Filler key id generate sequence
add index (body) in filler
";

const MIGRATION: &str = r"module app
add field Row.rank: I64 fill 7
add type Label = { code: Text }
add relation labels : Label key code
add index (rank) in rows
";

#[derive(Clone, Copy)]
struct Row {
    id: u32,
    name: &'static str,
    rank: i64,
}

const fn row(id: u32, name: &'static str) -> Row {
    Row {
        id,
        name,
        rank: FILL_RANK,
    }
}

const SEEDED: &[Row] = &[row(10, "seed-a"), row(11, "seed-b")];
const CHANGED: &[Row] = &[row(11, "seed-b2"), row(20, "first-a"), row(21, "first-b")];
const FINISHED: &[Row] = &[
    row(11, "seed-b2"),
    row(20, "first-a"),
    row(21, "first-b"),
    row(30, "last-a"),
    row(31, "last-b"),
];
const MIGRATED_FINISHED: &[Row] = &[
    row(11, "seed-b2"),
    row(20, "first-a"),
    row(21, "first-b"),
    Row {
        id: 30,
        name: "last-a",
        rank: 9,
    },
    Row {
        id: 31,
        name: "last-b",
        rank: 9,
    },
];

const SEED_TEXT: &str = r#"insert rows { id: rows.key(10), name: "seed-a" }
    insert rows { id: rows.key(11), name: "seed-b" }"#;
const CHANGE_TEXT: &str = r#"insert rows { id: rows.key(20), name: "first-a" }
    insert rows { id: rows.key(21), name: "first-b" }
    update r in rows where key r == rows.key(11) set Row { r with name: "seed-b2" }
    delete r in rows where key r == rows.key(10)"#;
const VACUUM_CHANGE: &str = r#"insert rows { id: rows.key(20), name: "first-a" }
    insert rows { id: rows.key(21), name: "first-b" }
    update r in rows where key r == rows.key(11) set Row { r with name: "seed-b2" }
    delete r in rows where key r == rows.key(10)
    delete f in filler"#;
const LAST_TEXT: &str = r#"insert rows { id: rows.key(30), name: "last-a" }
    insert rows { id: rows.key(31), name: "last-b" }"#;
const MIGRATED_LAST_TEXT: &str = r#"insert rows { id: rows.key(30), name: "last-a", rank: 9 }
    insert rows { id: rows.key(31), name: "last-b", rank: 9 }"#;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Maintenance {
    Vacuum,
    Reindex,
    Migrate,
}

/// A workload: acknowledge `CHANGE_TEXT`, run the maintenance searched, acknowledge `last`, close.
#[derive(Clone, Copy)]
struct Workload {
    name: &'static str,
    maintenance: Maintenance,
    change: &'static str,
    filler_kept: bool,
    pool_bytes: u64,
    last: &'static str,
    finished: &'static [Row],
}

const VACUUM: Workload = Workload {
    name: "vacuum",
    maintenance: Maintenance::Vacuum,
    change: VACUUM_CHANGE,
    filler_kept: false,
    pool_bytes: SEARCH_POOL_BYTES,
    last: LAST_TEXT,
    finished: FINISHED,
};
const REINDEX: Workload = Workload {
    name: "reindex",
    maintenance: Maintenance::Reindex,
    change: CHANGE_TEXT,
    filler_kept: true,
    pool_bytes: SEARCH_POOL_BYTES,
    last: LAST_TEXT,
    finished: FINISHED,
};
const MIGRATE: Workload = Workload {
    name: "migrate",
    maintenance: Maintenance::Migrate,
    change: CHANGE_TEXT,
    filler_kept: false,
    pool_bytes: SEARCH_POOL_BYTES,
    last: MIGRATED_LAST_TEXT,
    finished: MIGRATED_FINISHED,
};

/// A pool the migration does not evict from, so only a force makes it durable.
const ROOMY_MIGRATE: Workload = Workload {
    name: "migrate in a roomy pool",
    pool_bytes: 64 * 1024 * 1024,
    ..MIGRATE
};

/// Step numbers of a workload. The state after `n` steps is `Workload::state(n)`.
const MAINTAIN: usize = 1;
const STEPS: usize = 3;

struct State {
    rows: &'static [Row],
    migrated: bool,
}

impl Workload {
    fn migrates(self) -> bool {
        self.maintenance == Maintenance::Migrate
    }

    fn state(self, steps: usize) -> State {
        State {
            rows: match steps {
                0 => SEEDED,
                1 | 2 => CHANGED,
                _ => self.finished,
            },
            migrated: self.migrates() && steps > MAINTAIN,
        }
    }
}

/// An eligible op of a run.
#[derive(Clone, Copy)]
struct Mark {
    attempted: usize,
    acked: usize,
    in_step: bool,
    data_write: bool,
}

/// What the public API has answered, with the device event count at each answer.
#[derive(Default)]
struct Ledger {
    attempts: Vec<usize>,
    acks: Vec<usize>,
    marks: Vec<Mark>,
    /// Device event count where the maintenance action starts.
    step_start: usize,
    in_step: bool,
    phases: BTreeSet<String>,
    removed: i64,
}

impl Ledger {
    fn counts_at(&self, event: usize) -> (usize, usize) {
        (
            self.acks.iter().filter(|&&e| e <= event).count(),
            self.attempts.iter().filter(|&&e| e <= event).count(),
        )
    }
}

type Shared = Arc<Mutex<Ledger>>;

fn no_create(workload: Workload) -> OpenOptions {
    let mut opts = OpenOptions {
        create: None,
        ..api::options(2)
    };
    opts.pool_bytes = Some(workload.pool_bytes);
    opts
}

fn old_log() -> Log {
    Log::new([SCHEMA])
}

fn new_log() -> Log {
    Log::new([SCHEMA, MIGRATION])
}

fn failed(what: &str, e: impl std::fmt::Debug) -> String {
    format!("{what}: {e:?}")
}

fn seeded_files(workload: Workload) -> Files {
    let mut sim = Sim::new(SEED, Config::default());
    api::run(&mut sim, |rt| {
        let opts = OpenOptions {
            create: api::options(2).create,
            ..no_create(workload)
        };
        let db = Database::open_in(rt, FILE, &old_log(), &opts).expect("the fixture opens");
        query(&db, SEED_TEXT).expect("the seed commits");
        if workload.maintenance != Maintenance::Migrate {
            let body = "f".repeat(FILLER_BYTES);
            let text: String = (0..FILLER_ROWS)
                .map(|i| format!("insert filler {{ id: filler.key({i}), body: \"{body}\" }}\n"))
                .collect();
            query(&db, &text).expect("the filler commits");
        }
        db.close().expect("the fixture closes");
    });
    sim.files()
}

fn query<R: Runtime>(db: &Database<R>, text: &str) -> Result<Vec<DynRows>, Error> {
    db.query_text(text, &[], &CallOptions::default())
}

/// Wait for every job to finish, failing the search when one never does.
fn settle<R: Runtime>(db: &Database<R>, rt: &impl Threads, label: &str) -> Result<(), Error> {
    for _ in 0..JOB_ROUNDS {
        let mut jobs = query(db, "from j in sys::jobs select j.id")?;
        if jobs.remove(0).next().is_none() {
            return Ok(());
        }
        for _ in 0..256 {
            rt.yield_now();
        }
    }
    panic!("{label}: a job did not finish in {JOB_ROUNDS} rounds");
}

fn commit<R: Runtime>(
    db: &Database<R>,
    text: &str,
    attempting: impl FnOnce(),
) -> Result<(), Error> {
    let mut tx = db.begin(&TxnOptions::default())?;
    tx.query_text(text, &[], &TxnCallOptions::default())?;
    attempting();
    tx.commit(None).map(|_| ())
}

fn enter_step(hook: &Hooked, ledger: &Shared) {
    let mut ledger = ledger.lock().unwrap();
    ledger.step_start = hook.rt.recorded();
    ledger.in_step = true;
}

/// Run `workload.maintenance` on `db`, returning the database it leaves open.
fn maintain(
    hook: &Hooked,
    workload: Workload,
    db: Database<Hooked>,
    ledger: &Shared,
) -> Result<Database<Hooked>, String> {
    match workload.maintenance {
        Maintenance::Vacuum => {
            enter_step(hook, ledger);
            let mut rows = query(&db, "vacuum filler").map_err(|e| failed("vacuum", e))?;
            let removed: i64 = rows
                .remove(0)
                .map(|r| r.unwrap().fields().unwrap()[2].int().unwrap())
                .sum();
            ledger.lock().unwrap().removed += removed;
        }
        Maintenance::Reindex => {
            let seen = ledger.clone();
            db.testing().on_reindex_phase(move |phase| {
                seen.lock().unwrap().phases.insert(phase.to_owned());
            });
            enter_step(hook, ledger);
            query(&db, "reindex filler").map_err(|e| failed("reindex", e))?;
            settle(&db, hook, workload.name).map_err(|e| failed("jobs", e))?;
        }
        Maintenance::Migrate => {
            db.close().map_err(|e| failed("close", e))?;
            enter_step(hook, ledger);
            return Database::open_in(hook.clone(), FILE, &new_log(), &no_create(workload))
                .map_err(|e| failed("migrate", e));
        }
    }
    Ok(db)
}

/// Run `workload` on the hooked runtime, numbering what `eligible` accepts after `CHANGE_TEXT` is
/// acknowledged. Returns the first refusal the public API answers.
fn run_workload(
    hook: &Hooked,
    workload: Workload,
    ledger: &Shared,
    eligible: impl Fn(&Op) -> bool + Send + Sync + 'static,
) -> Result<(), String> {
    *ledger.lock().unwrap() = Ledger::default();
    let mark = |attempting: bool| {
        let mut ledger = ledger.lock().unwrap();
        if attempting {
            ledger.attempts.push(hook.rt.recorded());
        } else {
            ledger.acks.push(hook.rt.recorded());
        }
    };
    let db = Database::open_in(hook.clone(), FILE, &old_log(), &no_create(workload))
        .expect("the workload opens before the search arms");
    commit(&db, workload.change, || mark(true)).expect("the change commits before the search arms");
    mark(false);
    let observing = ledger.clone();
    hook.arm(move |op, path| {
        if !eligible(op) {
            return false;
        }
        let mut ledger = observing.lock().unwrap();
        let mark = Mark {
            attempted: ledger.attempts.len(),
            acked: ledger.acks.len(),
            in_step: ledger.in_step,
            data_write: matches!(op, Op::Write { .. }) && path == Some(FILE),
        };
        ledger.marks.push(mark);
        true
    });
    mark(true);
    let db = maintain(hook, workload, db, ledger)?;
    ledger.lock().unwrap().in_step = false;
    mark(false);
    commit(&db, workload.last, || mark(true)).map_err(|e| failed("last commit", e))?;
    mark(false);
    let closed = db.close().map_err(|e| failed("close", e));
    hook.disarm();
    closed
}

/// Returns the eligible ops and device events of an open with no migration to apply, at the point
/// `run_workload` opens the migrating log. Those lead the migrating open, before its apply.
fn plain_open_cost(workload: Workload, files: &Files) -> (usize, usize) {
    let eligible = |op: &Op| matches!(op, Op::Write { .. } | Op::Sync { .. });
    let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
    sim.record();
    api::run(&mut sim, |rt| {
        let hook = Hooked::new(&rt);
        let db = Database::open_in(hook.clone(), FILE, &old_log(), &no_create(workload))
            .expect("the control opens");
        commit(&db, workload.change, || {}).expect("the control commits");
        db.close().expect("the control closes");
        hook.arm(move |op, _| eligible(op));
        let before = rt.recorded();
        let db = Database::open_in(hook.clone(), FILE, &old_log(), &no_create(workload))
            .expect("the control reopens");
        let cost = (hook.disarm() as usize, rt.recorded() - before);
        db.close().expect("the control closes");
        cost
    })
}

/// Returns how many leading eligible ops and device events of the step precede the action searched.
fn step_lead(workload: Workload, files: &Files) -> (usize, usize) {
    if workload.migrates() {
        plain_open_cost(workload, files)
    } else {
        (0, 0)
    }
}

fn printed(row: &Row, migrated: bool) -> String {
    let Row { id, name, rank } = row;
    if migrated {
        format!(r#"{{ id: rows.key({id}), name: "{name}", rank: {rank} }}"#)
    } else {
        format!(r#"{{ id: rows.key({id}), name: "{name}" }}"#)
    }
}

fn expected(rows: &[Row], migrated: bool) -> Vec<String> {
    rows.iter().map(|r| printed(r, migrated)).collect()
}

fn rows_of<R: Runtime>(db: &Database<R>, migrated: bool, label: &str) -> Vec<String> {
    let select = if migrated {
        "from r in rows select { id: r.id, name: r.name, rank: r.rank }"
    } else {
        "from r in rows select { id: r.id, name: r.name }"
    };
    let mut results = query(db, select).unwrap_or_else(|e| panic!("{label}: read rows: {e:?}"));
    api::sorted(results.remove(0))
}

fn listed<R: Runtime>(db: &Database<R>, text: &str, label: &str) -> Vec<String> {
    let mut results = query(db, text).unwrap_or_else(|e| panic!("{label}: {text}: {e:?}"));
    api::sorted(results.remove(0))
}

/// Check every read through the public catalog and queries agrees that the migration is whole or
/// absent. Returns whether it is applied.
fn assert_migration_whole<R: Runtime>(db: &Database<R>, label: &str) -> bool {
    let versions = listed(db, "from m in catalog::migrations select m.version", label);
    let clauses = listed(
        db,
        r#"from i in catalog::indexes where i.relation.module == "app" and i.relation.name == "rows" select i.clause"#,
        label,
    );
    match versions.len() {
        1 => {
            assert_eq!(vec!["migrations.key(1)"], versions, "{label}");
            assert_eq!(vec!["\"index (name)\"", "\"key id\""], clauses, "{label}");
            for absent in [
                "from r in rows select r.rank",
                "from l in labels select l.code",
            ] {
                let refused = query(db, absent).err();
                assert!(
                    refused.as_ref().is_some_and(api::is_invalid),
                    "{label}: {absent} answered {refused:?}"
                );
            }
            false
        }
        2 => {
            assert_eq!(
                vec!["migrations.key(1)", "migrations.key(2)"],
                versions,
                "{label}"
            );
            assert_eq!(
                vec!["\"index (name)\"", "\"index (rank)\"", "\"key id\""],
                clauses,
                "{label}"
            );
            let labels = listed(db, "from l in labels select l.code", label);
            assert_eq!(Vec::<String>::new(), labels, "{label}");
            true
        }
        n => panic!("{label}: {n} migrations"),
    }
}

fn assert_filler<R: Runtime>(db: &Database<R>, workload: Workload, label: &str) {
    let mut want: Vec<String> = if workload.filler_kept {
        (0..FILLER_ROWS)
            .map(|i| format!("filler.key({i})"))
            .collect()
    } else {
        Vec::new()
    };
    want.sort();
    assert_eq!(
        want,
        listed(db, "from f in filler select f.id", label),
        "{label}: filler"
    );
    let body = "f".repeat(FILLER_BYTES);
    let text = format!(r#"from f in filler where f.body == "{body}" select f.id"#);
    assert_eq!(
        want,
        listed(db, &text, label),
        "{label}: filler by its index"
    );
}

fn assert_found_by_index<R: Runtime>(db: &Database<R>, rows: &[Row], label: &str) {
    for Row { id, name, .. } in rows {
        let text = format!(r#"from r in rows where r.name == "{name}" select r.id"#);
        assert_eq!(
            vec![format!("rows.key({id})")],
            listed(db, &text, label),
            "{label}: the index lookup"
        );
    }
}

/// Reopen `files` and require a state `acknowledged..=attempted` steps in, whole. Returns the state's
/// migration flag.
fn assert_recovered(
    workload: Workload,
    files: Files,
    acknowledged: usize,
    attempted: usize,
    label: &str,
) -> bool {
    assert!(
        acknowledged <= attempted && attempted <= STEPS,
        "{label}: the ledger"
    );
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt.clone(), FILE, &old_log(), &no_create(workload))
            .unwrap_or_else(|e| panic!("{label}: reopen: {e:?}"));
        settle(&db, &rt, label).unwrap_or_else(|e| panic!("{label}: jobs: {e:?}"));
        let migrated = assert_migration_whole(&db, label);
        let rows = rows_of(&db, migrated, label);
        assert_filler(&db, workload, label);
        let state = (acknowledged..=attempted)
            .map(|steps| workload.state(steps))
            .find(|s| s.migrated == migrated && expected(s.rows, migrated) == rows)
            .unwrap_or_else(|| {
                panic!("{label}: {acknowledged} acknowledged steps, {attempted} attempted, migrated {migrated}, rows {rows:?}")
            });
        assert_found_by_index(&db, state.rows, label);
        let findings = db.testing().verify_holding().expect("verify runs");
        assert_eq!(Vec::<String>::new(), api::printed(findings), "{label}");
        db.close()
            .unwrap_or_else(|e| panic!("{label}: close: {e:?}"));
        if workload.migrates() && !migrated {
            let db = Database::open_in(rt, FILE, &new_log(), &no_create(workload))
                .unwrap_or_else(|e| panic!("{label}: retry the migration: {e:?}"));
            assert!(
                assert_migration_whole(&db, label),
                "{label}: applied on retry"
            );
            assert_eq!(
                expected(state.rows, true),
                rows_of(&db, true, label),
                "{label}"
            );
            db.close()
                .unwrap_or_else(|e| panic!("{label}: close: {e:?}"));
        }
        migrated
    })
}

/// Check that the unfaulted run of `workload` returned at every step and did its maintenance.
fn assert_ran(workload: Workload, ledger: &Ledger) {
    assert_eq!(
        STEPS,
        ledger.acks.len(),
        "{}: every step returns",
        workload.name
    );
    match workload.maintenance {
        Maintenance::Vacuum => {
            assert_reached("vacuumed versions", ledger.removed as usize, FILLER_ROWS);
        }
        Maintenance::Reindex => {
            assert!(ledger.phases.contains("swap"), "phases {:?}", ledger.phases);
        }
        Maintenance::Migrate => {}
    }
}

/// Returns the op numbers of `marks` inside the action searched, after `lead` ops of its step.
fn inside(marks: &[Mark], lead: usize) -> Vec<u64> {
    let in_step = (0..marks.len()).filter(|&i| marks[i].in_step);
    in_step.skip(lead).map(|i| i as u64).collect()
}

/// Returns the ops inside the action to fail in every profile: its ends, its middle and its first
/// and last data file writes.
fn step_positions(marks: &[Mark], lead: usize) -> Vec<u64> {
    let ops = inside(marks, lead);
    assert!(!ops.is_empty(), "the action searched does I/O");
    let writes: Vec<u64> = ops
        .iter()
        .copied()
        .filter(|&at| marks[at as usize].data_write)
        .collect();
    let mut out = vec![ops[0], ops[ops.len() / 2], ops[ops.len() - 1]];
    out.extend(writes.first());
    out.extend(writes.last());
    out
}

/// Fail each chosen write and force of `workload`, with floors on what the search reached. Returns
/// what it reached.
fn search_failures(workload: Workload) -> String {
    silence_fatal_panics();
    let name = workload.name;
    let files = seeded_files(workload);
    let (lead, _) = step_lead(workload, &files);
    let ledger = Shared::default();
    let eligible = |op: &Op| matches!(op, Op::Write { .. } | Op::Sync { .. });
    let mut baseline = Sim::with_files(SEED, Config::default(), files.clone());
    api::run(&mut baseline, |rt| {
        run_workload(&Hooked::new(&rt), workload, &ledger, eligible).expect("the baseline runs");
    });
    let also = step_positions(&ledger.lock().unwrap().marks, lead);
    let (mut writes, mut syncs, mut in_action, mut data_writes, mut after_action) = (0, 0, 0, 0, 0);
    let (mut absent_in_action, mut absent, mut applied) = (0, 0, 0);
    let mut positions = BTreeSet::new();
    search::each_fatal_failure(
        &files,
        SEED,
        FAILED_OPS,
        &also,
        |hook| run_workload(hook, workload, &ledger, eligible),
        |failure| {
            let at = failure.at;
            assert!(positions.insert(at), "{name}: failure {at} twice");
            match failure.write_or_sync {
                Some(WriteOrSync::Write) => writes += 1,
                Some(WriteOrSync::Sync) => syncs += 1,
                None => panic!("the search only fails writes and file forces"),
            }
            let (acked, attempted, mark, struck_inside) = {
                let ledger = ledger.lock().unwrap();
                let mark = ledger.marks[at as usize];
                let struck_inside = inside(&ledger.marks, lead).contains(&at);
                (
                    ledger.acks.len(),
                    ledger.attempts.len(),
                    mark,
                    struck_inside,
                )
            };
            let label = format!(
                "{name} failure {at} of {}: stage ({}, {}), inside {struck_inside}",
                failure.op, mark.attempted, mark.acked
            );
            assert_eq!(
                None, failure.out,
                "{label}: a failed write or force stops the process"
            );
            assert_reached("an acknowledgement before the fault", mark.acked, 1);
            if struck_inside {
                in_action += 1;
                if mark.data_write {
                    data_writes += 1;
                }
            }
            if mark.acked > MAINTAIN {
                after_action += 1;
            }
            let migrated = assert_recovered(workload, failure.files, acked, attempted, &label);
            if migrated {
                applied += 1;
            } else {
                absent += 1;
                if struck_inside {
                    absent_in_action += 1;
                }
            }
        },
    );
    let reached = format!(
        "{name}: failure positions {}, writes {writes}, forces {syncs}, in the action {in_action}, data writes in it {data_writes}, after it {after_action}, absent {absent}, absent in it {absent_in_action}, applied {applied}",
        positions.len()
    );
    assert_reached(&reached, writes, 1);
    assert_reached(&reached, syncs, 1);
    assert_reached(&reached, in_action, 3);
    assert_reached(&reached, data_writes, 1);
    assert_reached(&reached, after_action, 1);
    if workload.migrates() {
        assert_reached(&reached, absent_in_action, 1);
        assert_reached(&reached, applied, 1);
    }
    reached
}

/// Take the images a crash at each chosen device event of `workload` can leave, always including
/// the action's first event, its middle and its acknowledgement, with floors on what the search
/// reached. Returns what it reached.
fn search_crashes(workload: Workload) -> String {
    let name = workload.name;
    let files = seeded_files(workload);
    let (lead_ops, lead_events) = step_lead(workload, &files);
    let ledger = Shared::default();
    let (mut in_action, mut at_ack, mut after_action) = (0, 0, 0);
    let (mut absent_in_action, mut applied_at_ack) = (0, 0);
    let mut events = BTreeSet::new();
    search::each_crash_image(
        &files,
        SEED,
        CRASH_POINTS,
        CRASH_IMAGES,
        |hook| {
            run_workload(hook, workload, &ledger, |_| true).expect("the crash baseline runs");
            let ledger = ledger.lock().unwrap();
            let (start, ack) = (ledger.step_start + lead_events, ledger.acks[MAINTAIN]);
            assert!(start < ack, "{name}: the action has device events");
            vec![start + 1, start.midpoint(ack), ack]
        },
        |at, image| {
            events.insert(at);
            let (acked, attempted, window) = {
                let ledger = ledger.lock().unwrap();
                let (acked, attempted) = ledger.counts_at(at);
                (
                    acked,
                    attempted,
                    (ledger.step_start + lead_events, ledger.acks[MAINTAIN]),
                )
            };
            let label = format!("{name} crash at event {at}: stage ({attempted}, {acked})");
            assert_reached("an acknowledgement before the crash", acked, 1);
            let migrated = assert_recovered(workload, image, acked, attempted, &label);
            let holds_action_event = at > window.0;
            if holds_action_event && at < window.1 {
                in_action += 1;
                if !migrated {
                    absent_in_action += 1;
                }
            }
            if at == window.1 {
                at_ack += 1;
                if migrated {
                    applied_at_ack += 1;
                }
            }
            if at > window.1 {
                after_action += 1;
            }
        },
    );
    assert_ran(workload, &ledger.lock().unwrap());
    let data_writes = {
        let ledger = ledger.lock().unwrap();
        let ops = inside(&ledger.marks, 0);
        ops.iter()
            .skip(lead_ops)
            .filter(|&&at| ledger.marks[at as usize].data_write)
            .count()
    };
    let reached = format!(
        "{name}: crash events {}, images in it {in_action}, absent in it {absent_in_action}, at its acknowledgement {at_ack}, applied there {applied_at_ack}, after it {after_action}",
        events.len()
    );
    assert_reached(&reached, in_action, 1);
    assert_reached(&reached, at_ack, 1);
    assert_reached(&reached, after_action, 1);
    assert_reached(
        &format!("{name}: data writes in the action"),
        data_writes,
        1,
    );
    if workload.migrates() {
        assert_reached(&reached, absent_in_action, 1);
        assert_reached(&reached, applied_at_ack, 1);
    }
    reached
}

#[test]
fn acknowledged_rows_survive_a_failed_write_or_force_around_a_vacuum() {
    println!("{}", search_failures(VACUUM));
}

#[test]
fn acknowledged_rows_survive_a_crash_around_a_vacuum() {
    println!("{}", search_crashes(VACUUM));
}

#[test]
fn acknowledged_rows_survive_a_failed_write_or_force_around_a_reindex() {
    println!("{}", search_failures(REINDEX));
}

#[test]
fn acknowledged_rows_survive_a_crash_around_a_reindex() {
    println!("{}", search_crashes(REINDEX));
}

#[test]
fn a_migration_is_whole_or_absent_after_a_failed_write_or_force() {
    println!("{}", search_failures(MIGRATE));
}

#[test]
fn a_migration_is_whole_or_absent_after_a_crash() {
    println!("{}", search_crashes(MIGRATE));
}

#[test]
fn an_open_that_migrates_has_made_the_migration_durable_when_it_returns() {
    let workload = ROOMY_MIGRATE;
    let files = seeded_files(workload);
    let ledger = Shared::default();
    let mut images = 0;
    let ack = Mutex::new(0);
    search::each_crash_image(
        &files,
        SEED,
        1,
        CRASH_IMAGES,
        |hook| {
            run_workload(hook, workload, &ledger, |_| true).expect("the baseline runs");
            *ack.lock().unwrap() = ledger.lock().unwrap().acks[MAINTAIN];
            vec![*ack.lock().unwrap()]
        },
        |at, image| {
            if at == *ack.lock().unwrap() {
                images += 1;
                let label = format!("crash at the acknowledgement, event {at}");
                assert!(
                    assert_recovered(workload, image, MAINTAIN + 1, MAINTAIN + 1, &label),
                    "{label}: the migration"
                );
            }
        },
    );
    assert_reached("images at the acknowledgement", images, 1);
}

const PAGE: usize = 8192;
/// Wall time a case may take before it counts as a hang. A hung case leaks its thread.
const ROT_CASE_LIMIT: Duration = Duration::from_secs(20);
const ROT_ROW: i64 = 50;
const SMALL_DOCS: i64 = 40;
const BIG_DOCS: i64 = 3;
const BIG_BYTES: usize = 30_000;

const ROT_SCHEMA: &str = r"module app
add type Doc { id: Key docs, tag: Text }
  = Small { note: Text }
  | Big { body: Text }
add relation docs : Doc key id generate sequence
  index (tag)
  index (note) in Small
";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rot {
    Flip,
    Zeroed,
    /// One half of the page zero, as a write that reached the device for the other half only.
    Torn,
}

const ROTS: [Rot; 3] = [Rot::Flip, Rot::Zeroed, Rot::Torn];

#[rustfmt::skip]
const ROT_CLASSES: [&str; 11] = [
    "file header", "AllocationBitmap", "Quarantine", "CommitLog", "Catalog", "FreeSpaceMap",
    "VisibilityMap", "application heap", "other heap", "application btree", "other btree",
];

fn rot_log() -> Log {
    Log::new([SCHEMA, ROT_SCHEMA])
}

fn big_body(i: i64) -> String {
    format!("{i}").repeat(BIG_BYTES)
}

fn ordered(rows: impl Iterator<Item = String>) -> Vec<String> {
    let mut rows: Vec<String> = rows.collect();
    rows.sort();
    rows
}

/// Returns each read of the database and its rows, from what the fixture wrote.
fn rot_reads(inserted: bool) -> Vec<(String, Vec<String>)> {
    let added = inserted.then(|| format!(r#"{{ id: rows.key({ROT_ROW}), name: "rotted" }}"#));
    let rows = ordered(expected(FINISHED, false).into_iter().chain(added));
    let filler = ordered((0..FILLER_ROWS).map(|i| format!("filler.key({i})")));
    let fill = "f".repeat(FILLER_BYTES);
    let last = SMALL_DOCS + BIG_DOCS;
    let ids = ordered((0..last).map(|i| format!("docs.key({i})")));
    let notes = ordered(
        (0..SMALL_DOCS).map(|i| format!(r#"{{ id: docs.key({i}), note: "n{i}", tag: "t{i}" }}"#)),
    );
    let bigs = ordered(
        (SMALL_DOCS..last).map(|i| format!(r#"{{ body: "{}", id: docs.key({i}) }}"#, big_body(i))),
    );
    let one = |text: &str, row: &str| (text.to_owned(), vec![row.to_owned()]);
    let all = |text: &str, rows: Vec<String>| (text.to_owned(), rows);
    #[rustfmt::skip]
    let reads = vec![
        all("from r in rows select { id: r.id, name: r.name }", rows),
        one(r#"from r in rows where r.name == "last-a" select r.id"#, "rows.key(30)"),
        all("from f in filler select f.id", filler.clone()),
        all(&format!(r#"from f in filler where f.body == "{fill}" select f.id"#), filler),
        all("from d in docs select d.id", ids),
        all("from d in docs.Small select { id: d.id, tag: d.tag, note: d.note }", notes),
        all("from d in docs.Big select { id: d.id, body: d.body }", bigs),
        one(r#"from d in docs where d.tag == "t7" select d.id"#, "docs.key(7)"),
        one(r#"from d in docs.Small where d.note == "n8" select d.id"#, "docs.key(8)"),
    ];
    reads
}

/// The owners of the application's heaps and of its indexes.
type Owners = (BTreeSet<u32>, BTreeSet<u32>);

/// A read that must cross a page of an application object, and the rows it answers.
#[derive(Clone)]
struct Crossing {
    text: String,
    want: Vec<String>,
    /// The line of the read's plan that names the object.
    plan: String,
    /// The encoded index key the read looks up, empty for a full scan.
    key: Vec<u8>,
}

/// A heap, large value store or index of the fixture, with the reads that need its pages.
#[derive(Clone)]
struct RotObject {
    is_heap: bool,
    owner: u32,
    mark: Quarantine,
    name: String,
    of_rows: bool,
    reads: Vec<Crossing>,
}

/// Fail unless the plan of `text` has `line`.
fn check_plan<R: Runtime>(db: &Database<R>, text: &str, line: &str) -> Result<(), String> {
    match rot_read(db, &format!("explain {text}"))? {
        Ok(rows) if rows.join("").contains(&format!("{line}\\n")) => Ok(()),
        plan => Err(format!("{text:.80} does not use {line}: {plan:?}")),
    }
}

/// Returns each heap with its full scan.
fn scan_objects<R: Runtime>(db: &Database<R>) -> Vec<RotObject> {
    let reads = rot_reads(false);
    #[rustfmt::skip]
    let scans = [
        ("rows", "Row", reads[0].clone()),
        ("filler", "Filler", reads[2].clone()),
        ("docs", "Small", reads[5].clone()),
        ("docs", "Big", reads[6].clone()),
    ];
    let t = db.testing();
    let mut objects = Vec::new();
    for (rel, ctor, (text, want)) in scans {
        let name = format!("app::{rel}.{ctor}");
        let heap = t.heap(&format!("app::{rel}"), ctor).unwrap();
        let mut live = 0;
        for pid in t.pages(heap).expect("its pages") {
            let versions = t.versions(heap, Some(&[pid])).unwrap();
            let on_page = versions.iter().filter(|v| v.header.xmax == 0).count();
            assert!(on_page > 0, "{name} {pid:?} holds no row a scan returns");
            live += on_page;
        }
        assert_eq!(want.len(), live, "{name}: its pages and the table");
        let plan = format!("Scan {rel}.{ctor}");
        let key = Vec::new();
        let reads = vec![Crossing {
            text,
            want,
            plan,
            key,
        }];
        let of_rows = rel == "rows";
        let (owner, mark) = (t.owner(heap).unwrap(), t.mark_of(heap).unwrap());
        #[rustfmt::skip]
        objects.push(RotObject { is_heap: true, owner, mark, name, of_rows, reads: reads.clone() });
        if ctor == "Big" {
            let large = t.large(&format!("app::{rel}"), ctor).unwrap();
            let (owner, name) = (t.owner(large).unwrap(), "the database".to_owned());
            #[rustfmt::skip]
            objects.push(RotObject { is_heap: true, owner, mark: Quarantine::Database, name, of_rows, reads });
        }
    }
    objects
}

const BY_NAME: &str = r#"from r in rows where r.name == "{}" select r.id"#;
const BY_ID: &str = "from r in rows where r.id == rows.key({}) select r.id";

fn rows_named(name: &str) -> Vec<String> {
    let rows = FINISHED.iter().filter(|r| r.name == name);
    ordered(rows.map(|r| format!("rows.key({})", r.id)))
}

fn row_with_id(id: i64) -> Vec<String> {
    let there = FINISHED.iter().any(|r| i64::from(r.id) == id);
    there
        .then(|| format!("rows.key({id})"))
        .into_iter()
        .collect()
}

/// Returns every name and id `rows` ever held, each probe's rows, and the scan, with the row
/// `ROT_ROW` in them if `added`.
fn rows_lookups(added: bool) -> Vec<(String, Vec<String>)> {
    let names = SEEDED.iter().chain(CHANGED).chain(FINISHED).map(|r| r.name);
    let names: BTreeSet<&str> = names.collect();
    let by_name = names
        .into_iter()
        .map(|n| (BY_NAME.replace("{}", n), rows_named(n)));
    let ids = SEEDED
        .iter()
        .chain(CHANGED)
        .chain(FINISHED)
        .map(|r| i64::from(r.id));
    let ids: BTreeSet<i64> = ids.collect();
    let by_id = ids
        .into_iter()
        .map(|i| (BY_ID.replace("{}", &i.to_string()), row_with_id(i)));
    let new = if added {
        vec![format!("rows.key({ROT_ROW})")]
    } else {
        Vec::new()
    };
    let probes = [
        (BY_NAME.replace("{}", "rotted"), new.clone()),
        (BY_ID.replace("{}", &ROT_ROW.to_string()), new),
        rot_reads(added).remove(0),
    ];
    by_name.chain(by_id).chain(probes).collect()
}

/// An index's table entry: relation, clause, field in its plan line, lookup form, then each key
/// as the form spells it, as stored, with its rows.
type IndexRows = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Vec<(String, Value, Vec<String>)>,
);

/// Returns each index with its lookup of every key the fixture ever wrote to it.
fn index_objects<R: Runtime>(db: &Database<R>) -> Vec<RotObject> {
    let text_key = |(k, want): (String, Vec<String>)| (k.clone(), Value::Text(k), want);
    let id_key = |(i, want): (i64, Vec<String>)| (i.to_string(), Value::I64(i), want);
    let names = SEEDED.iter().chain(CHANGED).chain(FINISHED).map(|r| r.name);
    let names: BTreeSet<&str> = names.collect();
    let ids = SEEDED
        .iter()
        .chain(CHANGED)
        .chain(FINISHED)
        .map(|r| i64::from(r.id));
    let ids: BTreeSet<i64> = ids.collect();
    let doc = |i: i64| vec![format!("docs.key({i})")];
    let fill = i64::try_from(FILLER_ROWS).expect("few rows");
    let fillers = ordered((0..fill).map(|i| format!("filler.key({i})")));
    let last = SMALL_DOCS + BIG_DOCS;
    #[rustfmt::skip]
    let indexes: [IndexRows; 7] = [
        ("rows", "index (name)", "(name)", BY_NAME, names.iter().map(|n| ((*n).to_owned(), rows_named(n))).map(text_key).collect()),
        ("rows", "key id", "(id)", BY_ID, ids.iter().map(|&i| (i, row_with_id(i))).map(id_key).collect()),
        ("filler", "index (body)", "(body)", r#"from f in filler where f.body == "{}" select f.id"#, vec![text_key(("f".repeat(FILLER_BYTES), fillers))]),
        ("filler", "key id", "(id)", "from f in filler where f.id == filler.key({}) select f.id", (0..fill).map(|i| (i, vec![format!("filler.key({i})")])).map(id_key).collect()),
        ("docs", "index (tag)", "(tag)", r#"from d in docs where d.tag == "{}" select d.id"#, (0..last).map(|i| (format!("t{i}"), doc(i))).map(text_key).collect()),
        ("docs", "index (note) in Small", "(note)", r#"from d in docs.Small where d.note == "{}" select d.id"#, (0..SMALL_DOCS).map(|i| (format!("n{i}"), doc(i))).map(text_key).collect()),
        ("docs", "key id", "(id)", "from d in docs where d.id == docs.key({}) select d.id", (0..last).map(|i| (i, doc(i))).map(id_key).collect()),
    ];
    let t = db.testing();
    let mut objects = Vec::new();
    for (rel, clause, field, form, keys) in indexes {
        let index = t.index(&format!("app::{rel}"), clause).unwrap();
        let schema = t.key_schema(index).unwrap();
        let mut reads = Vec::new();
        for (spelled, value, want) in keys {
            let text = form.replace("{}", &spelled);
            let plan = format!("Probe {rel} {field}");
            let key = schema.encode(&[value]);
            reads.push(Crossing {
                text,
                want,
                plan,
                key,
            });
        }
        let (owner, mark) = (t.owner(index).unwrap(), t.mark_of(index).unwrap());
        #[rustfmt::skip]
        objects.push(RotObject {
            is_heap: false, owner, mark, name: format!("app::{rel} {clause}"),
            of_rows: rel == "rows", reads,
        });
    }
    objects
}

/// Returns the files of the closed fixture, the owners of its pages and its application objects.
#[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
fn rot_fixture() -> (Files, Owners, Vec<RotObject>) {
    let files = seeded_files(REINDEX);
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    let (heaps, indexes, objects) = api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, FILE, &rot_log(), &no_create(REINDEX))
            .expect("the fixture opens");
        commit(&db, CHANGE_TEXT, || {}).expect("the change commits");
        commit(&db, LAST_TEXT, || {}).expect("the last commit commits");
        let small = (0..SMALL_DOCS).map(|i| {
            format!("insert docs.Small {{ id: docs.key({i}), tag: \"t{i}\", note: \"n{i}\" }}\n")
        });
        let big = (SMALL_DOCS..SMALL_DOCS + BIG_DOCS).map(|i| {
            let body = big_body(i);
            format!("insert docs.Big {{ id: docs.key({i}), tag: \"t{i}\", body: \"{body}\" }}\n")
        });
        let docs: String = small.chain(big).collect();
        commit(&db, &docs, || {}).expect("the documents commit");
        let t = db.testing();
        #[rustfmt::skip]
        let heaps = [("app::rows", "Row"), ("app::filler", "Filler"), ("app::docs", "Small"), ("app::docs", "Big")];
        let pages = |(rel, ctor): &(&str, &str)| {
            let heap = t.heap(rel, ctor).expect("the heap");
            t.pages(heap).expect("its pages")
        };
        let heaps: Vec<Vec<Pid>> = heaps.iter().map(pages).collect();
        let mut objects = scan_objects(&db);
        objects.extend(index_objects(&db));
        let indexes = objects
            .iter()
            .filter(|o| !o.is_heap)
            .map(|o| o.owner)
            .collect();
        db.close().expect("the fixture closes");
        (heaps, indexes, objects)
    });
    let files = sim.files();
    let bytes = &files[FILE];
    let heaps = heaps
        .iter()
        .flatten()
        .map(|p| storage::owner(&bytes[p.0 as usize * PAGE..(p.0 as usize + 1) * PAGE]))
        .collect();
    (files, (heaps, indexes), objects)
}

/// Returns the class of `page`, number `at` of a file whose application heaps have `owners`.
#[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
fn page_class(at: usize, page: &[u8], owners: &Owners) -> String {
    if at < Pid::FIRST_DATA.0 as usize {
        return "file header".into();
    }
    if page.iter().all(|&b| b == 0) {
        return "blank".into();
    }
    match PageKind::from_u16(storage::kind(page)) {
        Some(PageKind::HeapFixed | PageKind::HeapSlotted) => {
            if owners.0.contains(&storage::owner(page)) {
                "application heap".into()
            } else {
                "other heap".into()
            }
        }
        Some(PageKind::IndexLeaf | PageKind::IndexInner) => {
            if owners.1.contains(&storage::owner(page)) {
                "application btree".into()
            } else {
                "other btree".into()
            }
        }
        Some(kind) => format!("{kind:?}"),
        None => "unknown".into(),
    }
}

/// Returns a copy of `page` under `rot`, the same bytes when it changes nothing.
fn rotted(page: &[u8], at: usize, rot: Rot) -> Vec<u8> {
    let mut out = page.to_vec();
    match rot {
        Rot::Flip => out[(at * 2_654_435_761) % PAGE] ^= 1 << (at % 8),
        Rot::Zeroed => out.fill(0),
        Rot::Torn if page[PAGE / 2..].iter().any(|&b| b != 0) => out[PAGE / 2..].fill(0),
        Rot::Torn => out[..PAGE / 2].fill(0),
    }
    out
}

#[derive(Default)]
struct Seen {
    opened: bool,
    exact: usize,
    refused: usize,
    acknowledged: bool,
    findings: Vec<String>,
    open_refusal: Option<String>,
}

/// Returns what a refusal naming damage names, and any other failure as a defect.
fn refused_what(e: Error, at: &str, text: &str) -> Result<String, String> {
    match e {
        Error::Fatal {
            reason: FatalReason::Quarantined { what },
        } => Ok(what),
        e => Err(format!("{at} {text:.80}: {e:?}")),
    }
}

fn is_damage_refusal(e: &Error) -> bool {
    matches!(
        e,
        Error::Fatal {
            reason: FatalReason::Quarantined { .. }
        }
    )
}

/// Returns the rows of `rows` printed, or `Err` with what a refusal naming damage names.
fn rot_rows(rows: DynRows, what: &str) -> Result<Result<Vec<String>, String>, String> {
    let form = rows.form().clone();
    let mut out = Vec::new();
    for row in rows {
        match row {
            Ok(row) => out.push(print(&row, &form).map_err(|e| format!("{what}: {e:?}"))?),
            Err(e) => return refused_what(e, "mid-stream", what).map(Err),
        }
    }
    Ok(Ok(out))
}

/// Returns the rows of `text` printed and sorted, or `Err` with what a refusal naming damage names.
fn rot_read<R: Runtime>(
    db: &Database<R>,
    text: &str,
) -> Result<Result<Vec<String>, String>, String> {
    let mut results = match query(db, text) {
        Ok(results) => results,
        Err(e) => return refused_what(e, "query", text).map(Err),
    };
    let rows = rot_rows(results.remove(0), text)?;
    Ok(rows.map(|rows| ordered(rows.into_iter())))
}

fn rot_read_all<R: Runtime>(
    db: &Database<R>,
    inserted: bool,
    seen: &mut Seen,
) -> Result<(), String> {
    for (text, want) in rot_reads(inserted) {
        match rot_read(db, &text)? {
            Err(_) => seen.refused += 1,
            Ok(got) if got == want => seen.exact += 1,
            Ok(got) => {
                let short = |v: &[String]| format!("{} rows {:.300?}", v.len(), v);
                return Err(format!(
                    "wrong rows from {text:.80}: want {}, got {}",
                    short(&want),
                    short(&got)
                ));
            }
        }
    }
    Ok(())
}

fn rot_insert<R: Runtime>(db: &Database<R>, seen: &mut Seen) -> Result<(), String> {
    let text = format!(r#"insert rows {{ id: rows.key({ROT_ROW}), name: "rotted" }}"#);
    match commit(db, &text, || {}) {
        Ok(()) => seen.acknowledged = true,
        Err(e) if is_damage_refusal(&e) => seen.refused += 1,
        Err(e) => return Err(format!("insert: {e:?}")),
    }
    Ok(())
}

fn rot_verify<R: Runtime>(db: &Database<R>, seen: &mut Seen) -> Result<(), String> {
    let found = match db.verify(None) {
        Ok(rows) => rot_rows(rows, "verify")?.ok(),
        Err(e) if is_damage_refusal(&e) => None,
        Err(e) => return Err(format!("verify: {e:?}")),
    };
    seen.refused += usize::from(found.is_none());
    seen.findings = found.unwrap_or_default();
    Ok(())
}

/// Open the file, or return `None` for a refusal naming damage.
fn rot_open(rt: SimRuntime, seen: &mut Seen) -> Result<Option<Database<SimRuntime>>, String> {
    match Database::open_in(rt, FILE, &rot_log(), &no_create(REINDEX)) {
        Ok(db) => Ok(Some(db)),
        Err(OpenError::Corrupt { detail }) => {
            seen.open_refusal = Some(detail);
            Ok(None)
        }
        Err(e) => Err(format!("open: {e:?}")),
    }
}

fn rot_close<R: Runtime>(db: Database<R>) -> Result<(), String> {
    match db.close() {
        Err(e) if !is_damage_refusal(&e) => Err(format!("close: {e:?}")),
        _ => Ok(()),
    }
}

fn finished_run<T>(sim: &mut Sim, body: impl FnOnce(SimRuntime) -> T) -> Result<T, String> {
    let mut got = None;
    match sim.run(|rt| got = Some(body(rt))) {
        Outcome::Finished => got.ok_or_else(|| "the run returned nothing".to_owned()),
        other => Err(format!("the run ended {other:?}")),
    }
}

/// Run the public API over `image`: open, every read, one commit, verify, close, then reopen and
/// read what was acknowledged.
fn rot_case(image: Files) -> Result<Seen, String> {
    let mut seen = Seen::default();
    let mut sim = Sim::with_files(SEED, Config::default(), image);
    finished_run(&mut sim, |rt| -> Result<(), String> {
        let Some(db) = rot_open(rt, &mut seen)? else {
            return Ok(());
        };
        seen.opened = true;
        rot_read_all(&db, false, &mut seen)?;
        rot_insert(&db, &mut seen)?;
        rot_read_all(&db, seen.acknowledged, &mut seen)?;
        rot_verify(&db, &mut seen)?;
        rot_close(db)
    })??;
    let mut sim = Sim::with_files(SEED, Config::default(), sim.files());
    finished_run(&mut sim, |rt| -> Result<(), String> {
        let Some(db) = rot_open(rt, &mut seen)? else {
            return Ok(());
        };
        rot_read_all(&db, seen.acknowledged, &mut seen)?;
        rot_close(db)
    })??;
    Ok(seen)
}

static IN_ROT_CASE: AtomicBool = AtomicBool::new(false);
static ROT_PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn hook_rot_panics() {
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        if IN_ROT_CASE.load(Ordering::SeqCst) {
            ROT_PANICS.lock().unwrap().push(info.to_string());
        } else {
            default(info);
        }
    }));
}

/// Returns what `case` answers in `limit`, or the panic or hang that stopped it.
fn rot_bounded<T: Send + 'static>(
    limit: Duration,
    case: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (send, receive) = mpsc::channel();
    IN_ROT_CASE.store(true, Ordering::SeqCst);
    #[expect(
        clippy::disallowed_methods,
        reason = "gone once the simulator stops a run that spins, so no case needs its own thread"
    )]
    thread::spawn(move || {
        send.send(panic::catch_unwind(AssertUnwindSafe(case)))
            .expect("the search waits");
    });
    let answer = receive.recv_timeout(limit);
    IN_ROT_CASE.store(false, Ordering::SeqCst);
    let panics = std::mem::take(&mut *ROT_PANICS.lock().unwrap());
    match answer {
        Err(_) => Err(format!("no answer in {limit:?}")),
        Ok(_) if !panics.is_empty() => Err(format!("panic: {}", panics.join(" | "))),
        Ok(Err(_)) => Err("panic with no report".to_owned()),
        Ok(Ok(got)) => got,
    }
}

/// Returns why the answer of a run with `rot` on data page `at` fails to name the damage.
fn unnamed_damage(at: usize, rot: Rot, seen: &Seen) -> Option<String> {
    let name = match rot {
        Rot::Flip => format!("where: \"page {at}: Checksum\""),
        Rot::Zeroed | Rot::Torn => format!("where: \"page {at}: "),
    };
    let named = seen.findings.iter().any(|f| f.contains(&name));
    (seen.opened && !named).then(|| format!("verify names no damage: {:?}", seen.findings))
}

#[derive(Default, Debug)]
struct Tally {
    cases: usize,
    refused: usize,
    exact: usize,
    /// Runs of every read of `rows` after a commit through damage, all exact.
    read_back: usize,
    /// Those runs where a read refused.
    read_refused: usize,
}

/// Returns the pages of the file to rot with `ROTS[kind]`: all of them under the deep profile, and
/// otherwise a spread over each class.
fn rot_sample(classes: &BTreeMap<String, Vec<usize>>, kind: usize) -> BTreeSet<usize> {
    classes
        .values()
        .flat_map(|pages| {
            if profile::every_case() {
                pages.clone()
            } else {
                vec![pages[kind * pages.len() / ROTS.len()]]
            }
        })
        .collect()
}

#[test]
fn page_rot_gives_the_written_rows_or_a_refusal_naming_damage() {
    hook_rot_panics();
    let (pristine, owners, _) = rot_fixture();
    let run = |image: Files| rot_bounded(ROT_CASE_LIMIT, move || rot_case(image));
    let clean = run(pristine.clone()).expect("the undamaged file answers");
    assert_eq!(0, clean.refused, "the undamaged file refuses nothing");
    assert_eq!(27, clean.exact, "the undamaged file reads back");
    assert_eq!(Vec::<String>::new(), clean.findings);
    let file = &pristine[FILE];
    assert_eq!(0, file.len() % PAGE, "the file is whole pages");
    let mut classes: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (at, page) in file.chunks(PAGE).enumerate() {
        classes
            .entry(page_class(at, page, &owners))
            .or_default()
            .push(at);
    }
    let (mut defects, mut refusals) = (Vec::new(), BTreeSet::new());
    let mut table: BTreeMap<(String, Rot), Tally> = BTreeMap::new();
    let mut reached = BTreeSet::new();
    for (kind, rot) in ROTS.into_iter().enumerate() {
        for at in rot_sample(&classes, kind) {
            let page = &file[at * PAGE..(at + 1) * PAGE];
            let class = page_class(at, page, &owners);
            let bytes = rotted(page, at, rot);
            assert_ne!(
                page, bytes,
                "page {at} {class} {rot:?}: nothing was damaged"
            );
            let mut image = pristine.clone();
            image.get_mut(FILE).unwrap()[at * PAGE..(at + 1) * PAGE].copy_from_slice(&bytes);
            reached.insert(at);
            let tally = table.entry((class.clone(), rot)).or_default();
            tally.cases += 1;
            let answer = run(image).and_then(|seen| {
                let data = at >= Pid::FIRST_DATA.0 as usize;
                match unnamed_damage(at, rot, &seen).filter(|_| data) {
                    Some(defect) => Err(defect),
                    None => Ok(seen),
                }
            });
            match answer {
                Ok(seen) => {
                    tally.refused += usize::from(seen.refused > 0 || !seen.opened);
                    tally.exact += usize::from(seen.exact > 0);
                    refusals.extend(seen.open_refusal);
                }
                Err(defect) => defects.push(format!("page {at} {class} {rot:?}: {defect:.900}")),
            }
        }
    }
    println!(
        "pages {}, reached {}, open refusals {refusals:?}\n{table:#?}",
        file.len() / PAGE,
        reached.len()
    );
    assert_eq!(Vec::<String>::new(), defects, "{} defects", defects.len());
    if profile::every_case() {
        assert_eq!(file.len() / PAGE, reached.len(), "every page number");
    }
    for class in ROT_CLASSES {
        let tally = |rot| {
            table
                .get(&(class.to_owned(), rot))
                .unwrap_or_else(|| panic!("{class} {rot:?} was never reached"))
        };
        let refused: usize = ROTS.into_iter().map(|rot| tally(rot).refused).sum();
        if class == "file header" {
            assert_eq!(0, refused, "{class}: the other copy serves");
        } else {
            assert_reached(&format!("{class}: a refusal"), refused, 1);
        }
    }
    for class in ["application heap", "application btree"] {
        for rot in ROTS {
            let tally = &table[&(class.to_owned(), rot)];
            let what = format!("{class} {rot:?}");
            assert_reached(&format!("{what}: a refusal"), tally.refused, 1);
            assert_reached(&format!("{what}: exact rows"), tally.exact, 1);
        }
    }
}

/// Run every probe and the scan of `rows`, requiring the row `ROT_ROW` if `added` and none if not.
/// Count in `tally` whether all answered or one refused.
fn read_rows<R: Runtime>(db: &Database<R>, added: bool, tally: &mut Tally) -> Result<(), String> {
    let mut refused = false;
    for (text, want) in rows_lookups(added) {
        match rot_read(db, &text)? {
            Err(_) => refused = true,
            Ok(got) if got == want => {}
            Ok(got) => return Err(format!("{text:.80}: want {want:?}, got {got:?}")),
        }
    }
    tally.read_refused += usize::from(refused);
    tally.read_back += usize::from(!refused);
    Ok(())
}

/// Open the file, an open refusal being a defect.
fn rot_opened(rt: SimRuntime, seen: &mut Seen) -> Result<Database<SimRuntime>, String> {
    let db = rot_open(rt, seen)?;
    db.ok_or_else(|| format!("open refused: {:?}", seen.open_refusal))
}

fn marks<R: Runtime>(db: &Database<R>) -> BTreeSet<Quarantine> {
    db.testing().probe().marks()
}

/// Open `image`, commit to `rows` with no read before it, and read `rows` before a close and after
/// a reopen. Returns one case's tally, `exact` for an acknowledged commit and `refused` otherwise.
fn rot_commit(image: Files) -> Result<Tally, String> {
    let (mut seen, mut tally) = (Seen::default(), Tally::default());
    let mut sim = Sim::with_files(SEED, Config::default(), image);
    finished_run(&mut sim, |rt| -> Result<(), String> {
        let db = rot_opened(rt, &mut seen)?;
        rot_insert(&db, &mut seen)?;
        read_rows(&db, seen.acknowledged, &mut tally)?;
        rot_close(db)
    })??;
    tally.exact = usize::from(seen.acknowledged);
    tally.refused = usize::from(!seen.acknowledged);
    let mut sim = Sim::with_files(SEED, Config::default(), sim.files());
    finished_run(&mut sim, |rt| -> Result<(), String> {
        let db = rot_opened(rt, &mut seen)?;
        read_rows(&db, seen.acknowledged, &mut tally)?;
        rot_close(db)
    })??;
    Ok(tally)
}

/// Open `image` and run first the read `crossing` of `object`: rows only if exact and not
/// `must_refuse`, and a refusal only naming and marking `object`. With `commit_after`, then commit
/// to `rows` and read that commit back, before a close and after a reopen. Returns one case's
/// tally.
#[rustfmt::skip]
fn rot_cross(image: Files, object: &RotObject, crossing: &Crossing, commit_after: bool, must_refuse: bool, damaged: Option<Pid>) -> Result<Tally, String> {
    let (mut seen, mut tally) = (Seen::default(), Tally::default());
    let mut sim = Sim::with_files(SEED, Config::default(), image);
    finished_run(&mut sim, |rt| -> Result<(), String> {
        let db = rot_opened(rt, &mut seen)?;
        check_plan(&db, &crossing.text, &crossing.plan)?;
        if !marks(&db).is_empty() {
            return Err(format!("marked before the read: {:?}", marks(&db)));
        }
        let pages = || db.testing().probe().refusals();
        match rot_read(&db, &crossing.text)? {
            Err(what) if what == object.name && marks(&db) == BTreeSet::from([object.mark]) && damaged.is_none_or(|p| pages().contains_key(&p)) => {
                tally.refused += 1;
            }
            Err(what) => return Err(format!("refusal names {what:?}, marks {:?}, damaged {:?}", marks(&db), pages())),
            Ok(_) if must_refuse => return Err("a damaged page it needs answered".into()),
            Ok(got) if got == crossing.want => tally.exact += 1,
            Ok(got) => return Err(format!("want {:?}, got {got:?}", crossing.want)),
        }
        if commit_after {
            rot_insert(&db, &mut seen)?;
        }
        if seen.acknowledged {
            read_rows(&db, true, &mut tally)?;
        }
        rot_close(db)
    })??;
    if seen.acknowledged {
        let mut sim = Sim::with_files(SEED, Config::default(), sim.files());
        finished_run(&mut sim, |rt| -> Result<(), String> {
            let db = rot_opened(rt, &mut seen)?;
            read_rows(&db, true, &mut tally)?;
            rot_close(db)
        })??;
    }
    Ok(tally)
}

/// Returns the reads of `object` that must cross page `at`: for a leaf, the lookups of the keys of
/// its cells, read from the page's bytes without its fence keys.
fn reads_crossing(object: &RotObject, at: usize, page: &[u8]) -> Vec<Crossing> {
    if object.is_heap {
        return object.reads.clone();
    }
    let u16_at = |at: usize| usize::from(u16::from_le_bytes([page[at], page[at + 1]]));
    let cell = |slot: usize| {
        let at = u16_at(42 + 2 * slot);
        &page[at + 2..at + 2 + (u16_at(at) & 0x7fff)]
    };
    let cells: Vec<&[u8]> = (0..u16_at(32)).map(cell).collect();
    for cell in &cells {
        let known = object.reads.iter().any(|r| cell.starts_with(&r.key));
        assert!(known, "page {at} holds a key {} does not list", object.name);
    }
    let holds = |r: &&Crossing| cells.iter().any(|c| c.starts_with(&r.key));
    let reads: Vec<Crossing> = object.reads.iter().filter(holds).cloned().collect();
    assert!(!reads.is_empty(), "leaf {at} holds no key");
    reads
}

/// Returns the application heap pages and leaves of `file` owned by `object`.
#[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
fn pages_of(file: &[u8], object: &RotObject) -> Vec<usize> {
    let owned = |(_, page): &(usize, &[u8])| {
        let kind = PageKind::from_u16(storage::kind(page));
        let wanted: &[PageKind] = if object.is_heap {
            &[PageKind::HeapFixed, PageKind::HeapSlotted]
        } else {
            &[PageKind::IndexLeaf]
        };
        kind.is_some_and(|k| wanted.contains(&k)) && storage::owner(page) == object.owner
    };
    let pages = file
        .chunks(PAGE)
        .enumerate()
        .skip(Pid::FIRST_DATA.0 as usize);
    pages.filter(owned).map(|(at, _)| at).collect()
}

#[test]
fn a_read_crossing_a_rotted_page_gives_the_written_rows_or_refuses_naming_it() {
    hook_rot_panics();
    let (pristine, _, objects) = rot_fixture();
    let file = &pristine[FILE];
    let (mut defects, mut table) = (Vec::new(), BTreeMap::<(bool, Rot), Tally>::new());
    let deep = profile::every_case();
    for (kind, rot) in ROTS.into_iter().enumerate() {
        for object in &objects {
            let own = pages_of(file, object);
            let sample = if deep {
                own
            } else {
                vec![own[kind * own.len() / ROTS.len()]]
            };
            for at in sample {
                let page = &file[at * PAGE..(at + 1) * PAGE];
                let bytes = rotted(page, at, rot);
                assert_ne!(page, bytes, "page {at} {rot:?}");
                let mut image = pristine.clone();
                image.get_mut(FILE).unwrap()[at * PAGE..(at + 1) * PAGE].copy_from_slice(&bytes);
                let reads = reads_crossing(object, at, page);
                let take = if deep { reads.len() } else { 1 };
                for (n, crossing) in reads.into_iter().take(take).enumerate() {
                    let label = format!("page {at} {} {rot:?} {:.60}", object.name, crossing.text);
                    let (image, owned) = (image.clone(), object.clone());
                    let commit_after = n == 0 && !object.of_rows;
                    let answer = rot_bounded(ROT_CASE_LIMIT, move || {
                        rot_cross(
                            image,
                            &owned,
                            &crossing,
                            commit_after,
                            rot == Rot::Flip,
                            None,
                        )
                    });
                    let tally = table.entry((object.is_heap, rot)).or_default();
                    tally.cases += 1;
                    match answer {
                        Ok(case) => {
                            tally.refused += case.refused;
                            tally.exact += case.exact;
                            tally.read_back += case.read_back;
                            tally.read_refused += case.read_refused;
                        }
                        Err(defect) => defects.push(format!("{label}: {defect:.600}")),
                    }
                }
            }
        }
    }
    println!("{table:#?}");
    assert_eq!(Vec::<String>::new(), defects, "{} defects", defects.len());
    assert_eq!(2 * ROTS.len(), table.len(), "heap and leaf, every kind");
    for (key, tally) in &table {
        let what = format!("{key:?}");
        assert_reached(&format!("{what}: refused"), tally.refused, 1);
        assert_reached(&format!("{what}: read back"), tally.read_back, 1);
        assert_eq!(tally.cases, tally.refused + tally.exact, "{what}");
    }
}

#[test]
fn a_commit_through_a_rotted_page_of_its_target_loses_no_row() {
    hook_rot_panics();
    let (pristine, _, objects) = rot_fixture();
    let file = &pristine[FILE];
    let (mut defects, mut table) = (Vec::new(), BTreeMap::<(String, Rot), Tally>::new());
    for rot in ROTS {
        for object in objects.iter().filter(|o| o.of_rows) {
            for at in pages_of(file, object) {
                let page = &file[at * PAGE..(at + 1) * PAGE];
                let bytes = rotted(page, at, rot);
                assert_ne!(page, bytes, "page {at} {rot:?}");
                let mut image = pristine.clone();
                image.get_mut(FILE).unwrap()[at * PAGE..(at + 1) * PAGE].copy_from_slice(&bytes);
                let tally = table.entry((object.name.clone(), rot)).or_default();
                tally.cases += 1;
                match rot_bounded(ROT_CASE_LIMIT, move || rot_commit(image)) {
                    Ok(case) => {
                        tally.refused += case.refused;
                        tally.exact += case.exact;
                        tally.read_back += case.read_back;
                        tally.read_refused += case.read_refused;
                    }
                    Err(defect) => {
                        defects.push(format!("page {at} {} {rot:?}: {defect:.600}", object.name));
                    }
                }
            }
        }
    }
    println!("{table:#?}");
    assert_eq!(Vec::<String>::new(), defects, "{} defects", defects.len());
    assert_eq!(
        3 * ROTS.len(),
        table.len(),
        "heap, name and key, every kind"
    );
    for (key, tally) in &table {
        assert_reached(&format!("{key:?}: commit"), tally.refused + tally.exact, 1);
        assert_eq!(tally.cases, tally.refused + tally.exact, "{key:?}");
        assert_eq!(
            2 * tally.cases,
            tally.read_back + tally.read_refused,
            "{key:?}"
        );
    }
}

#[test]
fn a_rot_case_that_hangs_or_panics_is_reported() {
    hook_rot_panics();
    let hang = rot_bounded::<()>(Duration::from_millis(20), || {
        loop {
            thread::park();
        }
    });
    assert_eq!(Err("no answer in 20ms".to_owned()), hang);
    let panicked = rot_bounded::<()>(ROT_CASE_LIMIT, || panic!("the case broke"));
    let report = panicked.expect_err("a panic is a defect");
    assert!(
        report.starts_with("panic: ") && report.contains("the case broke"),
        "{report}"
    );
}

const CHECKSUM_AT: usize = 8;
const SLOT_COUNT_AT: usize = 32;

/// Write the checksum of `page` as page `at` into its header, as the pager seals a page.
fn sealed_as(at: usize, page: &mut [u8]) {
    let sum = storage::Crc32c::new()
        .update(&(at as u64).to_le_bytes())
        .update(&page[..CHECKSUM_AT])
        .update(&[0; 4])
        .update(&page[CHECKSUM_AT + 4..])
        .finish();
    page[CHECKSUM_AT..CHECKSUM_AT + 4].copy_from_slice(&sum.to_le_bytes());
}

/// A valid page written in place of a heap page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Foreign {
    /// A heap page of another owner.
    Heap,
    /// An index leaf.
    Leaf,
    /// The page itself with no slot.
    Emptied,
}

const FOREIGN: [Foreign; 3] = [Foreign::Heap, Foreign::Leaf, Foreign::Emptied];

/// Returns `pristine` with page `at` replaced as `foreign` says, `donors` being a heap page of
/// another owner and an index leaf.
fn foreign_image(pristine: &Files, at: usize, foreign: Foreign, donors: (usize, usize)) -> Files {
    let file = &pristine[FILE];
    let page = |p: usize| file[p * PAGE..(p + 1) * PAGE].to_vec();
    let mut bytes = match foreign {
        Foreign::Heap => page(donors.0),
        Foreign::Leaf => page(donors.1),
        Foreign::Emptied => {
            let mut b = page(at);
            let kind = PageKind::from_u16(storage::kind(&b));
            assert_eq!(Some(PageKind::HeapSlotted), kind, "page {at}");
            b[SLOT_COUNT_AT..SLOT_COUNT_AT + 2].fill(0);
            b
        }
    };
    sealed_as(at, &mut bytes);
    let mut image = pristine.clone();
    image.get_mut(FILE).unwrap()[at * PAGE..(at + 1) * PAGE].copy_from_slice(&bytes);
    image
}

/// Returns the reads that cross the first page of heap object `name`, each with its kind: a
/// scan, a lookup through an index, or a large value.
fn first_page_reads(name: &str) -> Vec<(&'static str, Crossing)> {
    let reads = rot_reads(false);
    let doc = |i: i64| vec![format!("docs.key({i})")];
    let lookup = |text: String, want: Vec<String>, plan: &str| Crossing {
        text,
        want,
        plan: plan.to_owned(),
        key: Vec::new(),
    };
    let scan = |n: usize, plan: &str| lookup(reads[n].0.clone(), reads[n].1.clone(), plan);
    let by_doc = |i: i64| format!("from d in docs where d.id == docs.key({i}) select d.id");
    let big = "from d in docs.Big where d.id == docs.key(40) select { id: d.id, body: d.body }";
    let big_want = vec![format!(
        r#"{{ body: "{}", id: docs.key(40) }}"#,
        big_body(40)
    )];
    #[rustfmt::skip]
    let out = match name {
        "app::rows.Row" => vec![
            ("scan", scan(0, "Scan rows.Row")),
            ("lookup", lookup(BY_ID.replace("{}", "11"), row_with_id(11), "Probe rows (id)")),
            ("lookup", lookup(BY_NAME.replace("{}", "seed-b2"), rows_named("seed-b2"), "Probe rows (name)")),
        ],
        "app::filler.Filler" => vec![
            ("scan", scan(2, "Scan filler.Filler")),
            ("lookup", lookup("from f in filler where f.id == filler.key(0) select f.id".into(), vec!["filler.key(0)".into()], "Probe filler (id)")),
        ],
        "app::docs.Small" => vec![
            ("scan", scan(5, "Scan docs.Small")),
            ("lookup", lookup(by_doc(0), doc(0), "Probe docs (id)")),
            ("lookup", lookup(r#"from d in docs where d.tag == "t0" select d.id"#.into(), doc(0), "Probe docs (tag)")),
            ("lookup", lookup(r#"from d in docs.Small where d.note == "n0" select d.id"#.into(), doc(0), "Probe docs (note)")),
        ],
        "app::docs.Big" => vec![
            ("scan", scan(6, "Scan docs.Big")),
            ("lookup", lookup(by_doc(40), doc(40), "Probe docs (id)")),
        ],
        _ => vec![
            ("large", scan(6, "Scan docs.Big")),
            ("large", lookup(big.into(), big_want, "Probe docs (id)")),
        ],
    };
    out
}

#[test]
#[expect(clippy::disallowed_methods, reason = "until heap owns page checks")]
fn a_valid_page_of_another_owner_or_lacking_the_slot_refuses_naming_its_holder() {
    hook_rot_panics();
    let (pristine, _, objects) = rot_fixture();
    let file = &pristine[FILE];
    let first = |o: &RotObject| pages_of(file, o)[0];
    let leaf = objects
        .iter()
        .find(|o| !o.is_heap)
        .map(first)
        .expect("an index");
    let (mut defects, mut table) = (Vec::new(), BTreeMap::<(Foreign, &str), usize>::new());
    for (o, object) in objects.iter().filter(|o| o.is_heap).enumerate() {
        let at = first(object);
        let directory = file.chunks(PAGE).position(|p| {
            storage::kind(p) == PageKind::FreeSpaceMap as u16 && storage::owner(p) == object.owner
        });
        let directory = Pid(directory.expect("the heap's directory root") as u64);
        let blamed = match object.mark {
            Quarantine::Database => Pid(at as u64),
            _ => directory,
        };
        let other = |o: &&RotObject| o.is_heap && o.owner != object.owner && o.of_rows;
        let donor = objects.iter().find(other).map(first).unwrap_or(leaf);
        for (f, foreign) in FOREIGN.into_iter().enumerate() {
            let image = foreign_image(&pristine, at, foreign, (donor, leaf));
            let mut reads = first_page_reads(&object.name);
            reads.retain(|(kind, _)| foreign != Foreign::Emptied || *kind != "scan");
            if !profile::every_case() {
                reads = vec![reads.swap_remove((o + f) % reads.len())];
            }
            for (kind, crossing) in reads {
                let label = format!(
                    "page {at} {} {foreign:?} {:.60}",
                    object.name, crossing.text
                );
                let probed =
                    |o: &&RotObject| !o.is_heap && o.reads.iter().any(|r| r.plan == crossing.plan);
                let holder = match (foreign, kind) {
                    (Foreign::Emptied, "lookup") => objects.iter().find(probed).expect("its index"),
                    _ => object,
                };
                let lsn = |f: &Files| storage::lsn(&f[FILE][at * PAGE..(at + 1) * PAGE]);
                let stale = lsn(&image) < lsn(&pristine);
                let blamed = if stale { Pid(at as u64) } else { blamed };
                let page = (foreign != Foreign::Emptied).then_some(blamed);
                let (image, owned) = (image.clone(), holder.clone());
                let answer = rot_bounded(ROT_CASE_LIMIT, move || {
                    rot_cross(image, &owned, &crossing, true, true, page)
                });
                match answer {
                    Ok(case) => *table.entry((foreign, kind)).or_default() += case.refused,
                    Err(defect) => defects.push(format!("{label}: {defect:.600}")),
                }
            }
        }
    }
    println!("{table:#?}");
    assert_eq!(Vec::<String>::new(), defects, "{} defects", defects.len());
    for foreign in FOREIGN {
        for kind in ["scan", "lookup", "large"] {
            if foreign != Foreign::Emptied || kind != "scan" {
                let refused = table.get(&(foreign, kind)).copied().unwrap_or(0);
                assert_reached(&format!("{foreign:?} {kind}: refused"), refused, 1);
            }
        }
    }
}

/// Open `image`, commit `text`, which must refuse naming and marking `holder`, then, with
/// `read_back`, read `rows` before a close and after a reopen.
fn rows_write_refused(
    image: Files,
    text: &str,
    holder: &RotObject,
    read_back: bool,
) -> Result<(), String> {
    let (mut seen, mut tally) = (Seen::default(), Tally::default());
    let mut sim = Sim::with_files(SEED, Config::default(), image);
    finished_run(&mut sim, |rt| -> Result<(), String> {
        let db = rot_opened(rt, &mut seen)?;
        match commit(&db, text, || {}) {
            Ok(()) => return Err("acknowledged".into()),
            Err(e) => {
                let what = refused_what(e, "commit", text)?;
                if what != holder.name || marks(&db) != BTreeSet::from([holder.mark]) {
                    return Err(format!("refusal names {what:?}, marks {:?}", marks(&db)));
                }
            }
        }
        if read_back {
            read_rows(&db, false, &mut tally)?;
        }
        rot_close(db)
    })??;
    let mut sim = Sim::with_files(SEED, Config::default(), sim.files());
    finished_run(&mut sim, |rt| -> Result<(), String> {
        let db = rot_opened(rt, &mut seen)?;
        if read_back {
            read_rows(&db, false, &mut tally)?;
        }
        rot_close(db)
    })?
}

#[test]
fn a_write_by_key_through_a_valid_page_of_another_owner_or_lacking_the_slot_refuses() {
    hook_rot_panics();
    let (pristine, _, objects) = rot_fixture();
    let file = &pristine[FILE];
    let first = |o: &RotObject| pages_of(file, o)[0];
    let rows = objects
        .iter()
        .find(|o| o.name == "app::rows.Row")
        .expect("rows");
    let filler = objects.iter().find(|o| o.name == "app::filler.Filler");
    let leaf = objects
        .iter()
        .find(|o| !o.is_heap)
        .map(first)
        .expect("an index");
    let donors = (filler.map(first).expect("filler"), leaf);
    let key_index = objects.iter().find(|o| o.name == "app::rows key id");
    let key_index = key_index.expect("the key index of rows");
    let insert = r#"insert rows { id: rows.key(11), name: "again" }"#;
    let texts = [
        r#"update r in rows where key r == rows.key(11) set Row { r with name: "u" }"#,
        "delete r in rows where key r == rows.key(20)",
    ];
    let mut defects = Vec::new();
    for foreign in FOREIGN {
        let image = foreign_image(&pristine, first(rows), foreign, donors);
        let emptied = (foreign == Foreign::Emptied).then_some(insert);
        for text in texts.into_iter().chain(emptied) {
            let foreign_owner = foreign != Foreign::Emptied;
            let holder = if foreign_owner { rows } else { key_index };
            let (image, owned) = (image.clone(), holder.clone());
            let answer = rot_bounded(ROT_CASE_LIMIT, move || {
                rows_write_refused(image, text, &owned, foreign_owner)
            });
            if let Err(defect) = answer {
                defects.push(format!("{foreign:?} {text:.50}: {defect:.600}"));
            }
        }
    }
    assert_eq!(Vec::<String>::new(), defects, "{} defects", defects.len());
}

const SCRUB_SETTINGS: &str = "set setting ScrubInterval = { every: Some { value: 10min } }
set setting VacuumBudget = { io: 1.0, cpu: 1.0 }";
const FINDINGS: &str = "from f in sys::verify_findings select f";
/// Minutes a scrub search waits for the reads of a pass to cover the file before it is a defect.
const SCRUB_NAPS: usize = 120;
/// Milliseconds after the scrub first reads the damaged page that a crash search crashes: before
/// the scrub marks it, then after.
const CRASH_AFTER_MS: [u64; 5] = [100, 1_000, 10_000, 60_000, 600_000];
const SCRUB_ALONE: [&str; 4] = [
    "application heap",
    "application btree",
    "VisibilityMap",
    "FreeSpaceMap",
];

/// Returns every object under the deep profile, and otherwise every fourth from `from`.
fn sampled(objects: &[RotObject], from: usize) -> Vec<RotObject> {
    let step = if profile::every_case() { 1 } else { 4 };
    let kept = objects.iter().skip(from % step).step_by(step);
    kept.cloned().collect()
}

/// Returns the rot fixture with a scrub every ten minutes at the full budget.
fn scrub_fixture() -> (Files, Owners, Vec<RotObject>) {
    let (files, owners, objects) = rot_fixture();
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, FILE, &rot_log(), &no_create(REINDEX))
            .expect("the fixture opens");
        query(&db, SCRUB_SETTINGS).expect("the settings");
        db.close().expect("the fixture closes");
    });
    (sim.files(), owners, objects)
}

/// The data pages read since a search armed, each read of one page with when it ran and whether it
/// failed, whether a read of the last page followed one at or past that page, and the outcomes of
/// its next reads, a failure for each `true`.
#[derive(Default)]
struct Watch {
    data: Option<FileId>,
    read: BTreeSet<u64>,
    page: u64,
    last: u64,
    past: bool,
    ended: bool,
    touched: Vec<(Instant, bool)>,
    plan: VecDeque<bool>,
}

impl Watch {
    fn saw(&mut self, pages: Range<u64>, now: Instant, failed: bool) {
        if pages.contains(&self.page) {
            self.touched.push((now, failed));
        }
        let past = self.past || pages.contains(&self.page) || self.page < Pid::FIRST_DATA.0;
        self.ended |= pages.contains(&self.last) && past;
        self.past |= pages.end > self.page;
        self.read.extend(pages);
    }
}

fn watch(hook: &Hooked, page: u64, last: u64, plan: &[bool]) -> Arc<Mutex<Watch>> {
    let w = Arc::new(Mutex::new(Watch {
        page,
        last,
        plan: plan.iter().copied().collect(),
        ..Watch::default()
    }));
    let pages =
        |offset: u64, len: usize| offset / PAGE as u64..(offset + len as u64).div_ceil(PAGE as u64);
    let (seen, rt) = (w.clone(), hook.rt.clone());
    hook.arm(move |op, path| {
        if let (Op::Read { file, offset, buf }, Some(FILE)) = (op, path) {
            let mut w = seen.lock().unwrap();
            w.data = Some(*file);
            w.saw(pages(*offset, buf.len()), rt.now(), false);
        }
        false
    });
    let (fails, rt) = (w.clone(), hook.rt.clone());
    hook.fail_when(move |op| {
        let mut w = fails.lock().unwrap();
        let at = w.page * PAGE as u64;
        let hit =
            matches!(op, Op::Read { file, offset, .. } if w.data == Some(*file) && *offset == at);
        (hit && w.plan.pop_front() == Some(true)).then(|| {
            w.saw(pages(at, PAGE), rt.now(), true);
            IoError::Failed
        })
    });
    w
}

/// What a scrub search saw of a case: the open's marks, each read of the page, how many planned
/// failures never struck, and what the judge answered.
struct Pass<T> {
    early: BTreeSet<Quarantine>,
    touched: Vec<(Instant, bool)>,
    left: usize,
    judged: T,
}

/// Open `image`, give the next reads of page `at` the outcomes of `plan`, nap until the reads cover
/// every data page and the pass reading page `at` has ended, `judge` the database, close and reopen.
/// Returns `None` for an open refusing damage.
fn scrubbed<T>(
    image: Files,
    at: usize,
    plan: &[bool],
    judge: impl FnOnce(&Database<Hooked>, &BTreeSet<Quarantine>) -> Result<T, String>,
) -> Result<Option<Pass<T>>, String> {
    let pages = (image[FILE].len() / PAGE) as u64;
    let mut sim = Sim::with_files(SEED, Config::default(), image);
    let got = finished_run(&mut sim, |rt| -> Result<Option<Pass<T>>, String> {
        let hook = Hooked::new(&rt);
        let options = OpenOptions {
            testing: Knobs::without_background_vacuum(),
            ..no_create(REINDEX)
        };
        let db = match Database::open_in(hook.clone(), FILE, &rot_log(), &options) {
            Ok(db) => db,
            Err(OpenError::Corrupt { .. }) => return Ok(None),
            Err(e) => return Err(format!("open: {e:?}")),
        };
        let early = marks(&db);
        let watch = watch(&hook, at as u64, pages - 1, plan);
        let whole = early.contains(&Quarantine::Database);
        let unread = || {
            let known = db.testing().probe().refusals();
            let w = watch.lock().unwrap();
            let seen = |p: &u64| w.read.contains(p) || known.contains_key(&Pid(*p));
            (Pid::FIRST_DATA.0..pages).find(|p| !whole && !seen(p))
        };
        let ended = || watch.lock().unwrap().ended;
        let mut done = whole;
        for _ in 0..SCRUB_NAPS {
            if done {
                break;
            }
            done = unread().is_none() && ended();
            common::sleep(
                &rt,
                &mut rt.io(),
                Duration::from_secs(if done { 1 } else { 60 }),
            );
        }
        if let Some(page) = unread() {
            return Err(format!("no pass read page {page} of {pages}"));
        }
        if !done {
            return Err("no pass ended after reading the page".to_owned());
        }
        let w = watch.lock().unwrap();
        let (touched, left) = (w.touched.clone(), w.plan.iter().filter(|f| **f).count());
        drop(w);
        let judged = judge(&db, &early)?;
        rot_close(db)?;
        Ok(Some(Pass {
            early,
            touched,
            left,
            judged,
        }))
    })??;
    let mut sim = Sim::with_files(SEED, Config::default(), sim.files());
    let reopened = finished_run(&mut sim, |rt| {
        match Database::open_in(rt, FILE, &rot_log(), &no_create(REINDEX)) {
            Ok(db) => rot_close(db),
            Err(_) if got.is_none() => Ok(()),
            Err(e) => Err(format!("reopen: {e:?}")),
        }
    });
    reopened??;
    Ok(got)
}

/// Returns the relation the first read of `object` names.
fn relation_of(object: &RotObject) -> &str {
    let named = object.reads[0].text.split_whitespace().nth(3);
    named
        .and_then(|n| n.split('.').next())
        .expect("from x in r")
}

/// Returns whether a finding of `db` names page `at`.
fn names_page<R: Runtime>(db: &Database<R>, at: usize) -> Result<bool, String> {
    let named = format!("where: \"page {at}: ");
    Ok(match rot_read(db, FINDINGS)? {
        Ok(rows) => rows.iter().any(|r| r.contains(&named)),
        Err(_) => db.testing().held_findings() > 0,
    })
}

/// Fail unless the scrub filed damage of page `at` of `object` as a read would: one finding naming
/// the page, `object` alone marked, a write to it refused naming it, reads of it refused naming it
/// unless it is an index, and every read of another relation exact. A database marked whole by its
/// open holds no finding.
#[rustfmt::skip]
fn filed_as_read<R: Runtime>(db: &Database<R>, early: &BTreeSet<Quarantine>, at: usize, rot: Rot, object: &RotObject, objects: &[RotObject]) -> Result<(), String> {
    let whole = object.mark == Quarantine::Database;
    let refused = Err(object.name.clone());
    if marks(db) != BTreeSet::from([object.mark]) {
        return Err(format!("marks {:?}", marks(db)));
    }
    let named = match rot {
        Rot::Flip => format!("where: \"page {at}: Checksum\""),
        Rot::Zeroed | Rot::Torn => format!("where: \"page {at}: "),
    };
    match rot_read(db, FINDINGS)? {
        Ok(rows) if rows.len() == 1 && rows[0].contains(&named) => {}
        Err(what) if whole && what == object.name && db.testing().held_findings() == usize::from(early.is_empty()) => {}
        other => return Err(format!("findings {other:?}")),
    }
    for o in objects {
        let read = rot_read(db, &o.reads[0].text)?;
        let same = relation_of(o) == relation_of(object);
        let fine = if whole || o.is_heap && o.name == object.name {
            read == refused
        } else {
            read == Ok(o.reads[0].want.clone()) || object.is_heap && same && read == refused
        };
        if !fine {
            return Err(format!("{} reads {read:.300?}", o.name));
        }
    }
    let insert = match relation_of(object) {
        "rows" => format!(r#"insert rows {{ id: rows.key({ROT_ROW}), name: "rotted" }}"#),
        "filler" => r#"insert filler { id: filler.key(999), body: "b" }"#.to_owned(),
        _ => r#"insert docs.Small { id: docs.key(99), tag: "t", note: "n" }"#.to_owned(),
    };
    let wrote = commit(db, &insert, || {}).map_err(|e| refused_what(e, "commit", &insert));
    if wrote != Err(Ok(object.name.clone())) {
        return Err(format!("a write to it {wrote:?}"));
    }
    Ok(())
}

/// What a scrub search saw: the open refusing, the open marking, the scrub alone naming the page.
#[derive(Default, Debug)]
struct Scrubbed {
    cases: usize,
    open_refused: usize,
    open_marked: usize,
    by_scrub: usize,
    unfound: usize,
}

fn rotted_image(pristine: &Files, at: usize, rot: Rot) -> Files {
    let mut image = pristine.clone();
    let page = &mut image.get_mut(FILE).unwrap()[at * PAGE..(at + 1) * PAGE];
    let bytes = rotted(page, at, rot);
    assert_ne!(page, &bytes[..], "page {at} {rot:?}: nothing was damaged");
    page.copy_from_slice(&bytes);
    image
}

#[test]
fn the_scrub_files_rot_on_a_page_no_call_reads_as_a_read_would() {
    hook_rot_panics();
    let (pristine, owners, objects) = scrub_fixture();
    let file = &pristine[FILE];
    let mut classes: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (at, page) in file.chunks(PAGE).enumerate() {
        let class = page_class(at, page, &owners);
        classes.entry(class).or_default().push(at);
    }
    let mut holder = BTreeMap::new();
    for o in &objects {
        holder.extend(pages_of(file, o).into_iter().map(|at| (at, o.clone())));
    }
    let (mut defects, mut table) = (Vec::new(), BTreeMap::<(String, Rot), Scrubbed>::new());
    let deep = profile::every_case();
    for (kind, rot) in ROTS.into_iter().enumerate() {
        let mut sample = if rot == Rot::Flip || deep {
            rot_sample(&classes, kind)
        } else {
            BTreeSet::new()
        };
        for o in sampled(&objects, kind) {
            let own = pages_of(file, &o);
            sample.insert(own[kind * own.len() / ROTS.len()]);
        }
        for at in sample {
            let class = page_class(at, &file[at * PAGE..(at + 1) * PAGE], &owners);
            let (image, object) = (rotted_image(&pristine, at, rot), holder.get(&at).cloned());
            let all = objects.clone();
            let label = format!("page {at} {class} {rot:?}");
            let answer = rot_bounded(ROT_CASE_LIMIT, move || {
                scrubbed(image, at, &[], |db, early| match &object {
                    Some(o) => filed_as_read(db, early, at, rot, o, &all).map(|()| (true, true)),
                    None => Ok((false, names_page(db, at)?)),
                })
            });
            let tally = table.entry((class.clone(), rot)).or_default();
            tally.cases += 1;
            match answer {
                Ok(None) => tally.open_refused += 1,
                Ok(Some(pass)) if pass.early.is_empty() && pass.judged.1 => tally.by_scrub += 1,
                Ok(Some(pass)) if !pass.early.is_empty() && !pass.judged.0 => {
                    tally.open_marked += 1;
                }
                Ok(Some(_)) if class == "file header" => tally.unfound += 1,
                Ok(Some(pass)) => {
                    defects.push(format!("{label}: unnamed, marks at open {:?}", pass.early));
                }
                Err(defect) => defects.push(format!("{label}: {defect:.900}")),
            }
        }
    }
    println!("pages {}\n{table:#?}", file.len() / PAGE);
    assert_eq!(Vec::<String>::new(), defects, "{} defects", defects.len());
    for ((class, rot), tally) in &table {
        let found = tally.open_refused + tally.open_marked + tally.by_scrub;
        if class == "file header" {
            assert_eq!(
                tally.cases, tally.unfound,
                "{rot:?}: no check reads the copy not used"
            );
        } else {
            assert_eq!(tally.cases, found, "{class} {rot:?}: every damage found");
        }
    }
    for class in SCRUB_ALONE {
        for rot in ROTS {
            let tally = table.get(&(class.to_owned(), rot));
            let floor = usize::from(rot == Rot::Flip || deep);
            let what = format!("{class} {rot:?}: found by the scrub alone");
            assert_reached(&what, tally.map_or(0, |t| t.by_scrub), floor);
        }
    }
}

/// Fail unless nothing is marked and every read of `objects` is exact.
fn all_exact<R: Runtime>(db: &Database<R>, objects: &[RotObject]) -> Result<(), String> {
    if !marks(db).is_empty() {
        return Err(format!("marks {:?}", marks(db)));
    }
    for o in objects {
        let read = rot_read(db, &o.reads[0].text)?;
        if read != Ok(o.reads[0].want.clone()) {
            return Err(format!("{} reads {read:.300?}", o.name));
        }
    }
    Ok(())
}

/// Fail unless nothing is marked and the only finding, if `unread`, names page `at` unreadable.
fn unread_filed<R: Runtime>(db: &Database<R>, at: usize, unread: bool) -> Result<(), String> {
    let rows = rot_read(db, FINDINGS)?.map_err(|what| format!("findings refused {what}"))?;
    let named = format!("where: \"page {at}: Unreadable\"");
    if !marks(db).is_empty()
        || rows.len() != usize::from(unread)
        || !rows.iter().all(|r| r.contains(&named))
    {
        return Err(format!("marks {:?}, findings {rows:?}", marks(db)));
    }
    Ok(())
}

#[test]
fn a_page_the_scrub_cannot_read_is_filed_unreadable_and_marks_nothing() {
    hook_rot_panics();
    let (pristine, _, objects) = scrub_fixture();
    let mut defects = Vec::new();
    #[rustfmt::skip]
    let plans: [(&[bool], bool); 4] = [(&[true], false), (&[true, true], false), (&[true, true, true], false), (&[false, true, true], true)];
    for o in sampled(&objects, 0) {
        let at = pages_of(&pristine[FILE], &o)[0];
        for (plan, rotten) in plans {
            let image = if rotten {
                rotted_image(&pristine, at, Rot::Flip)
            } else {
                pristine.clone()
            };
            let (all, label) = (objects.clone(), format!("page {at} {plan:?}"));
            let unread = plan.len() == 3;
            let answer = rot_bounded(ROT_CASE_LIMIT, move || {
                scrubbed(image, at, plan, |db, _| {
                    unread_filed(db, at, unread)?;
                    if rotten { Ok(()) } else { all_exact(db, &all) }
                })
            });
            let check: Vec<bool> = answer.as_ref().map_or(Vec::new(), |p| {
                p.as_ref()
                    .map_or(Vec::new(), |p| p.touched.iter().map(|t| t.1).collect())
            });
            match answer {
                Ok(Some(pass))
                    if pass.left == 0
                        && check.starts_with(plan)
                        && one_check(&pass.touched, plan.len()) => {}
                Ok(Some(pass)) => defects.push(format!("{label}: struck {:?}", pass.touched)),
                Ok(None) => defects.push(format!("{label}: open refused")),
                Err(defect) => defects.push(format!("{label}: {defect:.600}")),
            }
        }
    }
    assert_eq!(Vec::<String>::new(), defects, "{} defects", defects.len());
}

/// Returns whether the first `reads` reads of the page, and the read after a failure, ran within one
/// second, as one check of the page and its rechecks do.
fn one_check(touched: &[(Instant, bool)], reads: usize) -> bool {
    let reads = if reads < 3 { reads + 1 } else { reads };
    let span = touched
        .get(reads - 1)
        .map(|t| t.0.as_nanos() - touched[0].0.as_nanos());
    span.is_some_and(|ns| ns < 1_000_000_000)
}

/// Run `image` on a hooked runtime as `scrubbed` does until a crash at `crash`. Returns the files.
fn crashed(image: Files, crash: Instant) -> Result<Files, String> {
    let mut sim = Sim::with_files(SEED, Config::default(), image);
    sim.crash_at(crash);
    let outcome = sim.run(|rt| {
        let hook = Hooked::new(&rt);
        let _db = Database::open_in(hook.clone(), FILE, &rot_log(), &no_create(REINDEX))
            .expect("the image opens");
        let _watch = watch(&hook, 0, 0, &[]);
        loop {
            common::sleep(&rt, &mut rt.io(), Duration::from_secs(60));
        }
    });
    match outcome {
        Outcome::Crashed => Ok(sim.files()),
        other => Err(format!("the run ended {other:?}")),
    }
}

#[test]
fn a_crash_during_a_scrub_pass_leaves_the_damage_filed_after_the_next_pass() {
    hook_rot_panics();
    let (pristine, _, objects) = scrub_fixture();
    let (mut defects, mut kept, deep) = (Vec::new(), [0; 2], profile::every_case());
    let mut unmarked_whole = 0;
    for (kind, o) in sampled(&objects, 0).into_iter().enumerate() {
        let rot = ROTS[kind % ROTS.len()];
        let at = pages_of(&pristine[FILE], &o)[0];
        let image = rotted_image(&pristine, at, rot);
        let judge = |db: &_, early: &_| filed_as_read(db, early, at, rot, &o, &objects);
        let baseline = scrubbed(image.clone(), at, &[], judge);
        let baseline = baseline.and_then(|p| p.ok_or_else(|| "open refused".to_owned()));
        if let Ok(p) = &baseline
            && p.early.is_empty()
            && o.mark == Quarantine::Database
        {
            unmarked_whole += 1;
        }
        let first = match baseline.map(|p| p.touched.first().map(|t| t.0)) {
            Ok(Some(first)) => first.as_nanos(),
            Ok(_) => panic!("page {at}: the scrub never read it"),
            Err(defect) => {
                defects.push(format!("page {at} {rot:?} no crash: {defect:.600}"));
                continue;
            }
        };
        let take = if deep { CRASH_AFTER_MS.len() } else { 2 };
        for (k, ms) in CRASH_AFTER_MS.into_iter().take(take).enumerate() {
            let crash = Instant::from_nanos(first) + Duration::from_millis(ms);
            let (o, all) = (o.clone(), objects.clone());
            let answer = crashed(image.clone(), crash).and_then(|image| {
                rot_bounded(ROT_CASE_LIMIT, move || {
                    scrubbed(image, at, &[], |db, early| {
                        filed_as_read(db, early, at, rot, &o, &all)
                    })
                })
            });
            match answer {
                Ok(Some(pass)) => kept[usize::from(pass.early.is_empty())] += 1,
                Ok(None) => defects.push(format!("page {at} {rot:?} crash {k}: open refused")),
                Err(defect) => defects.push(format!("page {at} {rot:?} crash {k}: {defect:.600}")),
            }
        }
    }
    println!("crashes with the mark kept, and before it: {kept:?}");
    assert_eq!(Vec::<String>::new(), defects, "{} defects", defects.len());
    assert_reached("a crash with the mark kept", kept[0], 1);
    assert_eq!(
        0, kept[1],
        "the next open marks from the record the scrub forced"
    );
    assert_reached("a file mark filed with no mark at open", unmarked_whole, 1);
}
