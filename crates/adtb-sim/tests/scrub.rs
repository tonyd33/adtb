//! The background scrub: one thread per engine that reads every page of the file over each
//! `ScrubInterval`, without a call touching them.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use adtb::runtime::{Clock, Runtime, Threads};
use adtb::storage::{Options, PageKind, Pager, Pid, Quarantine};
use adtb::{CreateOptions, Database, Error, FatalReason, Log, OpenOptions};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::harness::finished;
use adtb_simkit::{api, common};
use adtb_sys::os::{Backend, OsRuntime};
use adtb_testkit::profile;

const FILE: &str = "scrub.adt";
const PAGE: usize = 8192;
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Other = { id: Key others }
add relation others : Other key id generate sequence
";
const USERS: &str = "from u in users select u.id";
const OTHERS: &str = "from o in others select o.id";
const FINDINGS: &str = "from f in sys::verify_findings select f";
const JOBS: &str = "from j in sys::jobs select j";

type Files = BTreeMap<String, Vec<u8>>;

fn app() -> Log {
    Log::new([APP])
}

fn open(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt, FILE, &app(), &api::options(2)).expect("the file opens")
}

fn nap(rt: &SimRuntime, d: Duration) {
    common::sleep(rt, &mut rt.io(), d);
}

fn set_interval(db: &Database<SimRuntime>, every: &str) {
    api::results(
        db,
        &format!("set setting ScrubInterval = {{ every: {every} }}"),
    );
}

fn set_io(db: &Database<SimRuntime>, io: &str) {
    api::results(
        db,
        &format!("set setting VacuumBudget = {{ io: {io}, cpu: 1.0 }}"),
    );
}

fn insert(db: &Database<SimRuntime>, from: i64, to: i64) {
    for i in from..to {
        api::results(
            db,
            &format!(r#"insert users {{ id: users.key({i}), name: "user" }}"#),
        );
    }
}

fn users_page<R: Runtime>(db: &Database<R>) -> Pid {
    let w = db.write_path();
    let pages = w
        .heap_named("app::users", "User")
        .expect("the heap")
        .pages(w.pager())
        .expect("its pages");
    *pages.last().expect("a heap has a page")
}

fn last_page(rt: &SimRuntime) -> Pid {
    let mut io = rt.io();
    let (_, len) = common::open(rt, &mut io, FILE, false).expect("the file");
    Pid(len / PAGE as u64 - 1)
}

/// Flip a byte of the file's copy of `page`, as the device might, with no call of the engine.
fn rot(rt: &SimRuntime, page: Pid) {
    let mut io = rt.io();
    let (file, _) = common::open(rt, &mut io, FILE, false).expect("the file");
    let at = page.0 * PAGE as u64;
    let mut bytes = common::read(rt, &mut io, file, at, PAGE).expect("the page");
    bytes[300] ^= 0x10;
    common::write(rt, &mut io, file, at, &bytes).expect("the write");
}

fn flush<R: Runtime>(db: &Database<R>) {
    let pager = db.write_path().pager();
    pager.flush_log();
    pager.flush_pages(usize::MAX);
    pager.checkpoint();
}

fn found(db: &Database<SimRuntime>) -> Vec<String> {
    api::sorted(api::result(db, FINDINGS))
}

fn quarantined<T: std::fmt::Debug>(got: Result<T, Error>) -> String {
    match got {
        Err(Error::Fatal {
            reason: FatalReason::Quarantined { what },
        }) => what,
        other => panic!("not Quarantined: {other:?}"),
    }
}

fn reads(db: &Database<SimRuntime>, text: &str) -> Result<Vec<adtb::DynRows>, Error> {
    db.query_text(text, &[], &adtb::CallOptions::default())
}

/// Returns a closed database file with `users` rows and the given settings, and a page of `users`.
fn seeded(seed: u64, users: i64, settings: &[(&str, &str)]) -> (Files, Pid) {
    let mut sim = Sim::new(seed, Config::default());
    let mut page = None;
    finished(&mut sim, |rt| {
        let db = open(rt);
        for (every, io) in settings {
            set_interval(&db, every);
            set_io(&db, io);
        }
        insert(&db, 0, users);
        page = Some(users_page(&db));
        db.close().expect("the close");
    });
    (sim.files(), page.expect("the page"))
}

#[test]
fn rot_is_found_and_marked_within_an_interval_with_no_call_touching_it() {
    let mut sim = Sim::new(1, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        set_interval(&db, "Some { value: 1h }");
        insert(&db, 0, 5);
        flush(&db);
        let page = users_page(&db);
        rot(&rt, page);
        assert_eq!(Vec::<String>::new(), found(&db), "no pass has run yet");
        nap(&rt, Duration::from_secs(3600 + 60));
        let rows = found(&db);
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(rows[0].starts_with("Checksum"), "{rows:?}");
        assert!(rows[0].contains(&format!("page {}", page.0)), "{rows:?}");
        assert!(
            db.write_path()
                .pager()
                .quarantined()
                .iter()
                .any(|q| matches!(q, Quarantine::Heap(_))),
            "the heap holding the page is marked"
        );
        assert!(quarantined(reads(&db, USERS)).contains("users"));
        reads(&db, OTHERS).expect("what does not hold the page still reads");
    });
}

#[test]
fn rot_on_a_page_the_pool_holds_is_found_from_the_file() {
    let mut sim = Sim::new(2, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        set_interval(&db, "Some { value: 1h }");
        insert(&db, 0, 5);
        flush(&db);
        api::query(&db, USERS);
        rot(&rt, users_page(&db));
        api::query(&db, USERS);
        nap(&rt, Duration::from_secs(3600 + 60));
        let rows = found(&db);
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(rows[0].starts_with("Checksum"), "{rows:?}");
    });
}

#[test]
fn a_lost_write_is_a_stale_page_finding() {
    let (older, page) = seeded(3, 3, &[("Some { value: 1h }", "0.1")]);
    let mut sim = Sim::with_files(3, Config::default(), older.clone());
    finished(&mut sim, |rt| {
        let db = open(rt);
        api::results(
            &db,
            r#"update u in users where key u == users.key(1) set User { u with name: "newer" }"#,
        );
        db.close().expect("the close");
    });
    let mut files = sim.files();
    let at = page.0 as usize * PAGE;
    files.get_mut(FILE).expect("the file")[at..at + PAGE]
        .copy_from_slice(&older[FILE][at..at + PAGE]);
    let mut sim = Sim::with_files(3, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        nap(&rt, Duration::from_secs(3600 + 60));
        let rows = found(&db);
        assert!(rows.iter().any(|r| r.starts_with("StalePage")), "{rows:?}");
    });
}

#[test]
fn a_second_pass_duplicates_no_row_and_a_healed_scope_is_left_to_its_own_run() {
    let mut sim = Sim::new(4, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        set_interval(&db, "Some { value: 10min }");
        insert(&db, 0, 5);
        flush(&db);
        rot(&rt, users_page(&db));
        nap(&rt, Duration::from_secs(660));
        let once = found(&db);
        assert_eq!(1, once.len(), "{once:?}");
        nap(&rt, Duration::from_secs(1800));
        assert_eq!(once, found(&db), "later passes keep the row as it is");
        let by_verify = api::printed(db.verify(None).expect("the run"));
        assert!(!by_verify.is_empty());
        nap(&rt, Duration::from_secs(1800));
        let rows = found(&db);
        assert_eq!(
            once.len() + by_verify.len(),
            rows.len(),
            "each run replaces only its own scope: {rows:?}"
        );
        api::printed(db.verify(None).expect("the run"));
        assert_eq!(once.len() + by_verify.len(), found(&db).len());
    });
}

#[test]
fn findings_of_a_scrub_are_kept_across_a_reopen_with_no_second_row() {
    let (files, page) = seeded(5, 5, &[("Some { value: 10min }", "0.1")]);
    let mut sim = Sim::with_files(5, Config::default(), files);
    let mut before = Vec::new();
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        rot(&rt, page);
        nap(&rt, Duration::from_secs(660));
        before = found(&db);
        assert_eq!(1, before.len(), "{before:?}");
        db.close().expect("the close");
    });
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        assert_eq!(before, found(&db), "the row is kept");
        nap(&rt, Duration::from_secs(1800));
        assert_eq!(before, found(&db), "the pass after a reopen adds no row");
    });
}

#[test]
fn a_full_pass_drops_a_row_it_no_longer_finds() {
    let (files, page) = seeded(16, 5, &[("Some { value: 10min }", "1.0")]);
    let clean = files[FILE].clone();
    let mut sim = Sim::with_files(16, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        rot(&rt, page);
        nap(&rt, Duration::from_secs(660));
        assert_eq!(1, found(&db).len());
        db.close().expect("the close");
    });
    let mut files = sim.files();
    let at = page.0 as usize * PAGE;
    files.get_mut(FILE).expect("the file")[at..at + PAGE].copy_from_slice(&clean[at..at + PAGE]);
    let mut sim = Sim::with_files(16, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        assert_eq!(1, found(&db).len(), "the row outlives the reopen");
        nap(&rt, Duration::from_secs(660));
        assert_eq!(Vec::<String>::new(), found(&db), "the page reads clean now");
    });
}

#[test]
fn a_pass_resumed_after_a_restart_keeps_what_its_first_part_found() {
    let (mut rotted, page) = seeded(17, 5, &[("Some { value: 1h }", "0.0001")]);
    let at = page.0 as usize * PAGE;
    rotted.get_mut(FILE).expect("the file")[at + 300] ^= 0x10;
    let crash = time_cursor_passes(17, &rotted, page) + Duration::from_secs(1);
    let mut sim = Sim::with_files(17, Config::default(), rotted);
    sim.crash_at(adtb::runtime::Instant::from_nanos(crash.as_nanos() as u64));
    let outcome = sim.run(|rt| {
        let db = open(rt.clone());
        loop {
            nap(&rt, Duration::from_secs(60));
            db.saved_scrub_page();
        }
    });
    assert_eq!(Outcome::Crashed, outcome);
    let mut sim = Sim::with_files(17, Config::default(), sim.files());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        let before = found(&db);
        assert_eq!(1, before.len(), "the row was kept: {before:?}");
        let resumed = db.saved_scrub_page().flatten().expect("an unfinished pass");
        assert!(resumed > page.0, "the page lies behind the cursor");
        set_io(&db, "1.0");
        nap(&rt, Duration::from_secs(30));
        assert_eq!(None, db.saved_scrub_page().flatten(), "the pass has ended");
        assert_eq!(before, found(&db), "the rest of the pass does not drop it");
    });
}

#[test]
fn a_database_marked_whole_writes_nothing_past_the_mark() {
    let mut sim = Sim::new(6, Config::default());
    let mut marks_page = None;
    let mut status_page = None;
    let mut held = 0;
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        set_interval(&db, "Some { value: 10min }");
        insert(&db, 0, 3);
        let status = db
            .write_path()
            .pager()
            .allocate(1, PageKind::HeapSlotted, 7)
            .expect("a page no relation holds");
        flush(&db);
        marks_page = Some(db.write_path().pager().quarantine_page());
        status_page = Some(status);
        rot(&rt, status);
        nap(&rt, Duration::from_secs(660));
        assert!(
            db.write_path().pager().is_quarantined(Quarantine::Database),
            "the page no relation holds marks the database"
        );
        held = db.held_findings();
        assert_eq!(
            1, held,
            "the finding alone, held in memory with no failed write"
        );
    });
    let marked = sim.files();
    let status = status_page.expect("the page");
    let mut users = None;
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        users = Some(users_page(&db));
        let pager = db.write_path().pager();
        let commits = pager.logged_commits();
        let marks_before = pager.quarantined();
        rot(&rt, users_page(&db));
        nap(&rt, Duration::from_secs(3600));
        assert!(db.held_findings() >= 2, "another pass finds them both");
        assert_eq!(
            BTreeSet::from([Quarantine::Database]),
            marks_before,
            "the whole mark"
        );
        assert_eq!(
            marks_before,
            pager.quarantined(),
            "no mark past the whole mark"
        );
        assert_eq!(
            commits,
            pager.logged_commits(),
            "no log batch past the mark"
        );
    });
    let later = sim.files();
    let rotted = [status.0, users.expect("the page").0];
    let marks = marks_page.expect("the page");
    for (n, (a, b)) in marked[FILE]
        .chunks(PAGE)
        .zip(later[FILE].chunks(PAGE))
        .enumerate()
    {
        if n > 1 && !rotted.contains(&(n as u64)) {
            assert!(
                a == b,
                "page {n} changed past the mark, the marks are {marks:?}"
            );
        }
    }
}

/// Returns the simulated time at which a pass over `files` first saves a cursor past `page`.
fn time_cursor_passes(seed: u64, files: &Files, page: Pid) -> Duration {
    let mut sim = Sim::with_files(seed, Config::default(), files.clone());
    let mut at = None;
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        while at.is_none() {
            nap(&rt, Duration::from_secs(60));
            if db.saved_scrub_page().flatten().is_some_and(|p| p > page.0) {
                at = Some(rt.now());
            }
        }
    });
    Duration::from_nanos(at.expect("the cursor passed it").as_nanos())
}

#[test]
fn a_restart_in_the_middle_of_a_pass_resumes_at_or_before_and_finds_rot_behind_it() {
    let (files, page) = seeded(7, 5, &[("Some { value: 1h }", "0.0001")]);
    let crash = time_cursor_passes(7, &files, page) + Duration::from_secs(1);
    let mut sim = Sim::with_files(7, Config::default(), files);
    let mut saved = None;
    sim.crash_at(adtb::runtime::Instant::from_nanos(crash.as_nanos() as u64));
    let outcome = sim.run(|rt| {
        let db = open(rt.clone());
        loop {
            nap(&rt, Duration::from_secs(60));
            saved = db.saved_scrub_page().flatten().or(saved);
        }
    });
    assert_eq!(Outcome::Crashed, outcome);
    let saved = saved.expect("a long pass saves its cursor while it runs");
    let mut files = sim.files();
    let at = page.0 as usize * PAGE;
    files.get_mut(FILE).expect("the file")[at + 300] ^= 0x10;
    let mut sim = Sim::with_files(7, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        let resumed = db
            .saved_scrub_page()
            .expect("the cursor is in the file")
            .expect("the pass is unfinished");
        assert!(
            resumed >= saved,
            "a cursor saved stays saved: {resumed} {saved}"
        );
        assert!(resumed > page.0, "the rot lies behind the cursor");
        set_io(&db, "1.0");
        nap(&rt, Duration::from_secs(10));
        assert_eq!(
            Vec::<String>::new(),
            found(&db),
            "this pass is past the page"
        );
        nap(&rt, Duration::from_secs(3700));
        let rows = found(&db);
        assert!(
            rows.iter().any(|r| r.contains(&format!("page {}", page.0))),
            "the next pass finds it: {rows:?}"
        );
    });
}

#[test]
fn short_opens_complete_a_pass() {
    let (files, _) = seeded(8, 5, &[("Some { value: 1h }", "0.1")]);
    let mut sim = Sim::with_files(8, Config::default(), files);
    finished(&mut sim, |rt| nap(&rt, Duration::from_secs(3700)));
    let mut seen = Vec::new();
    while !seen.iter().any(Option::is_none) {
        assert!(seen.len() < 30, "no pass ended: {seen:?}");
        finished(&mut sim, |rt| {
            let db = open(rt.clone());
            nap(&rt, Duration::from_secs(1));
            db.close().expect("the close");
        });
        finished(&mut sim, |rt| {
            let db = open(rt);
            seen.push(db.saved_scrub_page().flatten());
            db.close().expect("the close");
        });
    }
    assert!(
        seen.len() > 1,
        "one open does not finish the pass: {seen:?}"
    );
    let moving: Vec<u64> = seen.iter().flatten().copied().collect();
    assert!(moving.is_sorted(), "the cursor moves on: {moving:?}");
}

#[test]
fn a_scrub_set_to_none_reads_nothing_until_it_is_set_again() {
    let mut sim = Sim::new(9, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        set_interval(&db, "None");
        insert(&db, 0, 5);
        flush(&db);
        rot(&rt, users_page(&db));
        nap(&rt, Duration::from_secs(30 * 24 * 3600));
        assert_eq!(Vec::<String>::new(), found(&db), "no scrub runs under None");
        set_interval(&db, "Some { value: 1min }");
        nap(&rt, Duration::from_secs(120));
        assert_eq!(1, found(&db).len(), "back on with no reopen");
        set_interval(&db, "None");
        nap(&rt, Duration::from_secs(10));
        assert_eq!(Vec::<String>::new(), api::query(&db, JOBS));
    });
}

#[test]
fn a_file_opened_under_none_starts_no_worker_until_the_setting_changes() {
    let (files, page) = seeded(10, 5, &[("None", "0.1")]);
    let mut sim = Sim::with_files(10, Config::default(), files);
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        rot(&rt, page);
        nap(&rt, Duration::from_secs(3600));
        assert_eq!(Vec::<String>::new(), found(&db));
        set_interval(&db, "Some { value: 1s }");
        nap(&rt, Duration::from_secs(60));
        assert_eq!(1, found(&db).len());
    });
}

#[test]
fn a_shorter_interval_wakes_a_sleeping_worker() {
    let mut sim = Sim::new(11, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        insert(&db, 0, 5);
        flush(&db);
        rot(&rt, users_page(&db));
        nap(&rt, Duration::from_secs(60));
        assert_eq!(
            Vec::<String>::new(),
            found(&db),
            "seven days have not passed"
        );
        set_interval(&db, "Some { value: 1s }");
        nap(&rt, Duration::from_secs(60));
        assert_eq!(1, found(&db).len());
    });
}

#[test]
fn a_job_row_shows_only_while_a_pass_runs() {
    let mut sim = Sim::new(12, Config::default());
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        set_io(&db, "0.001");
        set_interval(&db, "Some { value: 1h }");
        assert_eq!(
            Vec::<String>::new(),
            api::query(&db, JOBS),
            "between passes"
        );
        nap(&rt, Duration::from_secs(3600 + 1));
        let rows = api::query(&db, JOBS);
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(rows[0].contains("Verify"), "{rows:?}");
        assert!(rows[0].contains("scrub"), "{rows:?}");
        set_io(&db, "1.0");
        nap(&rt, Duration::from_secs(1800));
        assert_eq!(
            Vec::<String>::new(),
            api::query(&db, JOBS),
            "after the pass"
        );
    });
}

/// Returns the simulated time a pass of the file of `seeded` takes at budget `io`, by watching its
/// row, and how many pages the file has.
fn pass_time(io: &str) -> (Duration, u64) {
    let (files, _) = seeded(13, 5, &[("Some { value: 1h }", io)]);
    let mut sim = Sim::with_files(13, Config::default(), files);
    let mut out = None;
    api::run(&mut sim, |rt| {
        let db = open(rt.clone());
        let step = Duration::from_millis(20);
        nap(&rt, Duration::from_secs(3599));
        while api::query(&db, JOBS).is_empty() {
            nap(&rt, step);
        }
        let began = rt.now();
        while !api::query(&db, JOBS).is_empty() {
            nap(&rt, step);
        }
        out = Some((
            rt.now().saturating_since(began),
            last_page(&rt).0 + 1 - Pid::FIRST_DATA.0,
        ));
    });
    out.expect("the pass ended")
}

#[test]
fn the_budget_holds_on_the_simulated_clock() {
    let (fast, pages) = pass_time("1.0");
    let (slow, _) = pass_time("0.05");
    assert!(
        slow.as_secs_f64() >= fast.as_secs_f64() * 15.0,
        "reading {pages} pages: {slow:?} at 5% and {fast:?} at 100%"
    );
    assert!(
        slow.as_secs_f64() <= fast.as_secs_f64() * 30.0,
        "the pace follows the time spent reading: {slow:?} at 5% and {fast:?} at 100%"
    );
}

#[test]
fn a_process_ending_with_the_worker_asleep_does_not_move_the_clock() {
    let mut sim = Sim::new(14, Config::default());
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        nap(&rt, Duration::from_secs(1));
        db.close().expect("the close");
    });
    assert!(
        sim.now().as_nanos() < Duration::from_secs(3600).as_nanos() as u64,
        "{:?}",
        sim.now()
    );
}

#[test]
fn a_saved_cursor_of_an_idle_database_writes_nothing_on_close() {
    let (files, _) = seeded(15, 2, &[]);
    let mut sim = Sim::with_files(15, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open(rt);
        assert_eq!(None, db.saved_scrub_page());
        db.close().expect("the close");
    });
    finished(&mut sim, |rt| {
        let db = open(rt);
        assert_eq!(None, db.saved_scrub_page(), "nothing has run to save");
        db.close().expect("the close");
    });
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("scrub")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).expect("the directory");
    dir
}

fn host_options() -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(2),
        ..OpenOptions::default()
    }
}

fn text(db: &Database, query: &str) -> Vec<String> {
    let mut out = db
        .query_text(query, &[], &adtb::CallOptions::default())
        .unwrap_or_else(|e| panic!("`{query}` fails: {e:?}"));
    if out.is_empty() {
        return Vec::new();
    }
    let rows = out.remove(0);
    let form = rows.form().clone();
    rows.map(|r| adtb_lang::textual::print(&r.expect("a row"), &form).expect("a row prints"))
        .collect()
}

#[test]
fn a_real_file_rots_and_the_worker_finds_it() {
    let dir = scratch("rot");
    let path = dir.join("db");
    let db = Database::open(&path, &app(), &host_options()).expect("the file opens");
    text(&db, r#"insert users { id: users.key(1), name: "a" }"#);
    text(&db, "set setting VacuumBudget = { io: 1.0, cpu: 1.0 }");
    flush(&db);
    let page = users_page(&db);
    let at = page.0 * PAGE as u64;
    let mut bytes = std::fs::read(&path).expect("the file");
    bytes[at as usize + 300] ^= 0x10;
    std::fs::write(&path, &bytes).expect("the rot");
    text(
        &db,
        "set setting ScrubInterval = { every: Some { value: 1ms } }",
    );
    let rows = (0..2000)
        .find_map(|_| {
            std::thread::sleep(Duration::from_millis(5));
            Some(text(&db, FINDINGS)).filter(|rows| !rows.is_empty())
        })
        .expect("a finding within ten seconds");
    assert!(rows[0].starts_with("Checksum"), "{rows:?}");
}

#[test]
fn a_dropped_database_releases_a_real_file_while_the_worker_runs() {
    let dir = scratch("drop");
    let path = dir.join("db");
    for round in 0..5 {
        let db = Database::open(&path, &app(), &host_options())
            .unwrap_or_else(|e| panic!("round {round}: {e:?}"));
        text(
            &db,
            "set setting ScrubInterval = { every: Some { value: 0s } }",
        );
        text(&db, "set setting VacuumBudget = { io: 1.0, cpu: 1.0 }");
        std::thread::sleep(Duration::from_millis(2));
        drop(db);
    }
}

fn attempt(db: &Database, query: &str, done: &AtomicU64) {
    match db.query_text(query, &[], &adtb::CallOptions::default()) {
        Ok(_) => {
            done.fetch_add(1, Ordering::Relaxed);
        }
        Err(Error::Fatal { reason }) => panic!("`{query}` fails: {reason:?}"),
        Err(Error::Rejected { reason }) => assert!(
            !format!("{reason:?}").contains("Invalid"),
            "`{query}` fails: {reason:?}"
        ),
        Err(_) => {}
    }
}

#[test]
fn scrubbing_under_real_writers_finds_nothing() {
    let dir = scratch("stress");
    let path = dir.join("db");
    let small_pool = OpenOptions {
        pool_bytes: Some(32 * PAGE as u64),
        ..host_options()
    };
    let db = Database::open(&path, &app(), &small_pool).expect("the file opens");
    text(&db, "set setting VacuumBudget = { io: 1.0, cpu: 1.0 }");
    text(
        &db,
        "set setting ScrubInterval = { every: Some { value: 0s } }",
    );
    let stop = AtomicBool::new(false);
    let writes = AtomicU64::new(0);
    std::thread::scope(|scope| {
        for w in 0..3i64 {
            let (db, stop, writes) = (&db, &stop, &writes);
            scope.spawn(move || {
                let mut i = 0;
                while !stop.load(Ordering::Relaxed) {
                    let key = w * 1_000_000 + i % 100;
                    let name = "x".repeat(usize::try_from(i % 40).expect("small") * 100);
                    attempt(
                        db,
                        &format!(r#"insert users {{ id: users.key({key}), name: "{name}" }}"#),
                        writes,
                    );
                    attempt(
                        db,
                        &format!(
                            r#"update u in users where key u == users.key({key}) set User {{ u with name: "y" }}"#
                        ),
                        writes,
                    );
                    if i % 3 == 0 {
                        attempt(
                            db,
                            &format!("delete u in users where key u == users.key({key})"),
                            writes,
                        );
                    }
                    i += 1;
                }
            });
        }
        for round in 0..20 {
            attempt(&db, "vacuum users", &writes);
            attempt(&db, "reindex users", &writes);
            std::thread::sleep(Duration::from_millis(50));
            assert_eq!(Vec::<String>::new(), text(&db, FINDINGS), "round {round}");
        }
        stop.store(true, Ordering::Relaxed);
    });
    assert!(writes.load(Ordering::Relaxed) > 100, "the writers wrote");
    assert_eq!(Vec::<String>::new(), text(&db, FINDINGS));
    assert!(db.saved_scrub_page().is_some(), "the worker made passes");
}

#[test]
#[should_panic(expected = "deadlock")]
fn a_parked_thread_beside_an_open_database_is_a_deadlock() {
    let mut sim = Sim::new(40, Config::default());
    sim.run(|rt| {
        let db = open(rt.clone());
        rt.park();
        drop(db);
    });
}

#[test]
fn a_database_never_closed_does_not_move_the_clock_at_the_end() {
    let mut sim = Sim::new(41, Config::default());
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        nap(&rt, Duration::from_secs(1));
        std::mem::forget(db);
    });
    assert!(
        sim.now().as_nanos() < Duration::from_secs(3600).as_nanos() as u64,
        "{:?}",
        sim.now()
    );
}

/// Returns whether a pass is running.
fn in_pass(db: &Database<SimRuntime>) -> bool {
    !api::sorted(api::result(db, JOBS)).is_empty()
}

#[test]
fn a_pass_reading_an_old_log_does_not_refuse_a_page_healed_since() {
    let key = |db: &Database<SimRuntime>| {
        let w = db.write_path();
        let index = w
            .index_named("app::users", "key id")
            .expect("the key index");
        index.tree().root()
    };
    let mut root = None;
    let mut sim = Sim::new(42, Config::default());
    finished(&mut sim, |rt| {
        let db = open(rt);
        set_interval(&db, "None");
        insert(&db, 0, 3);
        root = Some(key(&db));
        db.close().expect("the close");
    });
    let root = root.expect("the root");
    let older = sim.files();
    finished(&mut sim, |rt| {
        let db = open(rt);
        insert(&db, 3, 4);
        db.close().expect("the close");
    });
    let at = root.0 as usize * PAGE;
    let mut files = sim.files();
    files.get_mut(FILE).expect("the file")[at..at + PAGE]
        .copy_from_slice(&older[FILE][at..at + PAGE]);
    let mut sim = Sim::with_files(42, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        insert(&db, 4, 5);
        flush(&db);
        set_io(&db, "0.00001");
        set_interval(&db, "Some { value: 0s }");
        while !in_pass(&db) {
            nap(&rt, Duration::from_millis(1));
        }
        api::results(&db, "verify");
        assert!(
            found(&db).iter().any(|r| r.starts_with("StalePage")),
            "verify finds the lost write"
        );
        assert!(
            !db.write_path().pager().quarantined().is_empty(),
            "the mark"
        );
        api::results(&db, "reindex users");
        for _ in 0..1000 {
            if db.write_path().pager().quarantined().is_empty() {
                break;
            }
            nap(&rt, Duration::from_millis(10));
        }
        assert_eq!(
            BTreeSet::new(),
            db.write_path().pager().quarantined(),
            "the rebuild released the index"
        );
        assert!(
            db.saved_scrub_page().flatten().is_none_or(|p| p <= root.0),
            "the pass has not passed the root yet"
        );
        set_interval(&db, "Some { value: 1h }");
        set_io(&db, "1.0");
        nap(&rt, Duration::from_secs(10));
        assert_eq!(
            Some(None),
            db.saved_scrub_page(),
            "the pass reached its end"
        );
        assert_eq!(
            BTreeSet::new(),
            db.write_path().pager().quarantined(),
            "no pass marks the page reindex healed"
        );
        assert!(!db.write_path().pager().damaged().contains_key(&root));
        api::results(&db, "verify");
        assert_eq!(Vec::<String>::new(), found(&db));
    });
}

#[test]
fn rot_on_a_free_page_is_no_finding() {
    let mut sim = Sim::new(43, Config::default());
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        set_interval(&db, "Some { value: 10min }");
        let pager = db.write_path().pager();
        let page = pager.allocate(1, PageKind::HeapSlotted, 7).expect("a page");
        pager.free(page).expect("the free");
        flush(&db);
        rot(&rt, page);
        nap(&rt, Duration::from_secs(1300));
        assert_eq!(0, db.held_findings());
        assert_eq!(BTreeSet::new(), pager.quarantined());
    });
}

#[test]
fn rot_on_a_free_page_then_reused_is_no_finding_and_no_mark() {
    for seed in 44..48 {
        let mut sim = Sim::new(seed, Config::default());
        finished(&mut sim, |rt| {
            let db = open(rt.clone());
            set_interval(&db, "None");
            let pager = db.write_path().pager();
            let page = pager.allocate(1, PageKind::HeapSlotted, 7).expect("a page");
            pager.free(page).expect("the free");
            flush(&db);
            rot(&rt, page);
            let mut next = 0;
            while pager.is_free(page).expect("the bitmap") {
                insert(&db, next, next + 1);
                next += 1;
                assert!(next < 1000, "seed {seed}: no insert reused page {page:?}");
            }
            set_interval(&db, "Some { value: 0s }");
            set_io(&db, "1.0");
            nap(&rt, Duration::from_secs(1));
            assert_eq!(0, db.held_findings(), "seed {seed}");
            assert_eq!(BTreeSet::new(), pager.quarantined(), "seed {seed}");
            assert_eq!(
                next,
                api::sorted(api::result(&db, USERS)).len() as i64,
                "seed {seed}"
            );
        });
    }
}

#[test]
fn a_budget_too_small_to_compute_waits_and_rechecks() {
    for (seed, io) in [(48, "1e-300"), (52, "1e-20")] {
        let mut sim = Sim::new(seed, Config::default());
        finished(&mut sim, |rt| {
            let db = open(rt.clone());
            insert(&db, 0, 3);
            set_interval(&db, "Some { value: 1min }");
            set_io(&db, io);
            nap(&rt, Duration::from_secs(600));
            assert_eq!(
                3,
                api::sorted(api::result(&db, USERS)).len(),
                "{io}: the engine runs"
            );
            assert_eq!(0, db.held_findings(), "{io}: {:?}", found(&db));
            set_io(&db, "1.0");
            nap(&rt, Duration::from_secs(120));
            assert_eq!(
                Some(None),
                db.saved_scrub_page(),
                "{io}: a larger budget finishes the pass"
            );
        });
    }
}

#[test]
fn a_budget_too_small_to_compute_reads_little() {
    let mut sim = Sim::new(49, Config::default());
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        set_io(&db, "1e-300");
        set_interval(&db, "Some { value: 1h }");
        nap(&rt, Duration::from_secs(3 * 3600 + 600));
        let saved = db.saved_scrub_page().flatten();
        assert!(
            saved.is_none_or(|p| p <= Pid::FIRST_DATA.0 + 3),
            "one page each hour: {saved:?}"
        );
    });
}

#[test]
fn a_panic_in_the_worker_leaves_a_finding_and_the_engine_running() {
    let mut sim = Sim::new(50, Config::default());
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        insert(&db, 0, 3);
        set_io(&db, "0.001");
        set_interval(&db, "Some { value: 0s }");
        while !in_pass(&db) {
            nap(&rt, Duration::from_millis(1));
        }
        db.fail_background("boom");
        nap(&rt, Duration::from_secs(1));
        let rows = found(&db);
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(
            rows[0].starts_with("Structure") && rows[0].contains("boom"),
            "{rows:?}"
        );
        assert!(!in_pass(&db), "the job row went with the worker");
        insert(&db, 3, 4);
        assert_eq!(4, api::sorted(api::result(&db, USERS)).len());
        set_interval(&db, "Some { value: 1h }");
        db.close().expect("the close");
    });
}

#[test]
#[should_panic(expected = "fatal I/O error")]
fn a_fatal_io_error_in_the_worker_still_crashes_the_process() {
    let mut sim = Sim::new(51, Config::default());
    sim.run(|rt| {
        let db = open(rt.clone());
        set_interval(&db, "Some { value: 0s }");
        db.fail_background("fatal I/O error: write: a test");
        nap(&rt, Duration::from_secs(1));
    });
}

#[test]
fn a_caught_panic_keeps_what_the_pass_found_beside_its_finding_across_a_reopen() {
    let mut sim = Sim::new(52, Config::default());
    let two_rows = |rows: Vec<String>| {
        assert_eq!(2, rows.len(), "{rows:?}");
        assert!(rows.iter().any(|r| r.starts_with("Checksum")), "{rows:?}");
        assert!(
            rows.iter()
                .any(|r| r.starts_with("Structure") && r.contains("boom")),
            "{rows:?}"
        );
    };
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        set_interval(&db, "Some { value: 1h }");
        insert(&db, 0, 3);
        flush(&db);
        rot(&rt, users_page(&db));
        nap(&rt, Duration::from_secs(3600 + 60));
        assert_eq!(1, found(&db).len(), "the rot is found first");
        db.fail_background("boom");
        nap(&rt, Duration::from_secs(3600 + 60));
        two_rows(found(&db));
        db.close().expect("the close");
    });
    finished(&mut sim, |rt| {
        let db = open(rt);
        two_rows(found(&db));
    });
}

#[test]
fn a_panic_in_a_kept_write_of_the_worker_ends_the_run_with_no_call_after_it() {
    let mut sim = Sim::new(53, Config::default());
    let after = AtomicBool::new(false);
    let run = catch_unwind(AssertUnwindSafe(|| {
        sim.run(|rt| {
            let db = open(rt.clone());
            set_interval(&db, "Some { value: 1h }");
            insert(&db, 0, 3);
            flush(&db);
            rot(&rt, users_page(&db));
            db.fail_kept_write("halfway");
            nap(&rt, Duration::from_secs(24 * 3600));
            after.store(true, Ordering::Relaxed);
        })
    }));
    let payload = run.expect_err("the panic ends the run");
    assert_eq!(
        Some("halfway".to_owned()),
        adtb::write::panic_text(&*payload)
    );
    assert!(!after.load(Ordering::Relaxed), "a call ran past the panic");
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        nap(&rt, Duration::from_secs(3600 + 60));
        assert_eq!(
            Vec::<String>::new(),
            found(&db),
            "recovery restored the page"
        );
        assert!(quarantined(reads(&db, USERS)).contains("users"));
        reads(&db, OTHERS).expect("what the mark does not cover still reads");
    });
}

#[test]
fn a_caught_panic_on_a_database_marked_whole_stays_in_memory() {
    let mut sim = Sim::new(54, Config::default());
    finished(&mut sim, |rt| {
        let db = open(rt.clone());
        set_interval(&db, "Some { value: 10min }");
        insert(&db, 0, 3);
        let stray = db
            .write_path()
            .pager()
            .allocate(1, PageKind::HeapSlotted, 7)
            .expect("a page no relation holds");
        flush(&db);
        rot(&rt, stray);
        nap(&rt, Duration::from_secs(660));
        assert!(db.write_path().pager().is_quarantined(Quarantine::Database));
        assert_eq!(1, db.held_findings());
        let before = db.write_path().pager().logged_commits();
        db.fail_background("boom");
        nap(&rt, Duration::from_secs(660));
        assert_eq!(2, db.held_findings());
        assert_eq!(before, db.write_path().pager().logged_commits());
    });
}

#[test]
fn a_real_file_survives_a_tiny_budget_and_a_panic_in_the_worker() {
    let dir = scratch("survive");
    let path = dir.join("db");
    let db = Database::open(&path, &app(), &host_options()).expect("the file opens");
    text(&db, "set setting VacuumBudget = { io: 1e-300, cpu: 1.0 }");
    text(
        &db,
        "set setting ScrubInterval = { every: Some { value: 0s } }",
    );
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(Vec::<String>::new(), text(&db, FINDINGS));
    db.fail_background("boom");
    for _ in 0..1000 {
        if db.held_findings() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(text(&db, FINDINGS)[0].contains("boom"));
    text(&db, r#"insert users { id: users.key(1), name: "n" }"#);
    db.close().expect("the close");
}

#[test]
fn a_page_checked_against_a_writer_on_real_threads_finds_nothing() {
    let rt = OsRuntime::new(scratch("hammer"), Backend::Pool { threads: 4 });
    let mut o = Options::new("h.adt");
    o.pool_frames = 64;
    o.page_size = 65536;
    let pager = Arc::new(Pager::create(rt.clone(), &o).expect("the pager"));
    let page = pager.allocate(1, PageKind::HeapSlotted, 7).expect("a page");
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (pager, stop) = (pager.clone(), stop.clone());
        rt.spawn("writer", move || {
            let mut i = 0u8;
            while !stop.load(Ordering::Relaxed) {
                i = i.wrapping_add(1);
                pager
                    .update(&[page], |p| p[0][1024..].fill(i))
                    .expect("an update");
                pager.flush_pages(1);
            }
        })
    };
    let limit = Duration::from_millis(profile::deep(1500));
    let deadline = rt.now() + limit;
    let checkers: Vec<_> = (0..4)
        .map(|c| {
            let (pager, rt2) = (pager.clone(), rt.clone());
            rt.spawn("checker", move || {
                let mut checks = 0u64;
                while rt2.now() < deadline {
                    let log = pager.read_log(true);
                    for _ in 0..200 {
                        let found = pager.check_page(page, &log);
                        assert_eq!(
                            Vec::<adtb::storage::Finding>::new(),
                            found,
                            "checker {c}, check {checks}"
                        );
                        checks += 1;
                    }
                }
            })
        })
        .collect();
    for c in checkers {
        rt.join(c);
    }
    stop.store(true, Ordering::Relaxed);
    rt.join(writer);
}
