//! Search crashes and failed writes and forces through `Database` while a vacuum, a reindex and a
//! migration run after acknowledged commits. Every expected row comes from a table written here.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use adtb::runtime::{Op, Runtime, Threads};
use adtb::{CallOptions, Database, Error, Log, OpenOptions, TxnCallOptions, TxnOptions};
use adtb_sim::{Config, Sim};
use adtb_simkit::api;
use adtb_simkit::crash::silence_fatal_panics;
use adtb_simkit::search::{self, Files, Hooked, WriteOrSync};
use adtb_testkit::profile::assert_reached;

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

fn query<R: Runtime>(db: &Database<R>, text: &str) -> Result<Vec<adtb::DynRows>, Error> {
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
            db.on_reindex_phase(Arc::new(move |phase| {
                seen.lock().unwrap().phases.insert(phase.to_owned());
            }));
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
        let findings = db.verify_holding().expect("verify runs");
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
