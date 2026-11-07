//! Opening a file cut short: open returns on every length, and what the lost pages held is refused.
#![expect(
    clippy::disallowed_methods,
    reason = "a real thread bounds a run that may never return; removing it needs the simulator to bound a run by its own steps"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use adtb::key::Tid;
use adtb::runtime::{Clock, Runtime, Threads};
use adtb::storage::{HEADER_LEN, Pid, Quarantine};
use adtb::{CallOptions, CreateOptions, Database, Error, FatalReason, Log, OpenError, OpenOptions};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::harness::finished;
use adtb_simkit::{api, common};
use adtb_testkit::profile::{assert_reached, every_case, share, sweep};

const FILE: &str = "truncated_open.adt";
const PAGE: usize = 8192;
const SECTOR: usize = 512;
const LIMIT: Duration = Duration::from_secs(5);
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";
const SELECT: &str = "from u in users select u.id";
const INSERT: &str = r#"insert users { id: users.key(9), name: "x" }"#;
const ROW: &str = "users.key(1)";

type Files = BTreeMap<String, Vec<u8>>;

struct Seeded {
    files: Files,
    roles: Pid,
    users: Pid,
}

#[derive(Debug)]
struct Opened {
    marks: BTreeSet<Quarantine>,
    select: Result<Vec<String>, Error>,
    verify: Vec<String>,
    insert: Result<(), Error>,
}

fn open(rt: SimRuntime) -> Result<Database<SimRuntime>, OpenError> {
    Database::open_in(rt, FILE, &Log::new([APP]), &api::options(2))
}

fn seeded() -> Seeded {
    let mut sim = Sim::new(1, Config::default());
    let mut pages = None;
    finished(&mut sim, |rt| {
        let db = open(rt).unwrap();
        api::results(&db, r#"insert users { id: users.key(1), name: "a" }"#);
        let w = db.write_path();
        let first = |relation, ctor| {
            w.heap_named(relation, ctor)
                .unwrap()
                .pages(w.pager())
                .unwrap()[0]
        };
        pages = Some((first("catalog::roles", "Role"), first("app::users", "User")));
        db.close().unwrap();
    });
    let (roles, users) = pages.unwrap();
    Seeded {
        files: sim.files(),
        roles,
        users,
    }
}

fn cut(files: &Files, len: usize) -> Files {
    let mut files = files.clone();
    files.get_mut(FILE).unwrap().truncate(len);
    files
}

fn zeroed(files: &Files, page: Pid) -> Files {
    let mut files = files.clone();
    let at = page.0 as usize * PAGE;
    files.get_mut(FILE).unwrap()[at..at + PAGE].fill(0);
    files
}

fn call(db: &Database<SimRuntime>) -> Opened {
    let none = CallOptions::default();
    let select = db
        .query_text(SELECT, &[], &none)
        .map(|mut r| api::printed(r.remove(0)));
    let insert = if select.is_ok() {
        Ok(())
    } else {
        db.query_text(INSERT, &[], &none).map(drop)
    };
    Opened {
        marks: db.testing().probe().marks(),
        select,
        verify: api::printed(db.verify(None).unwrap()),
        insert,
    }
}

/// Returns what `body` returns. Panics when it takes longer than `LIMIT`.
fn within<T: Send + 'static>(what: &str, body: impl FnOnce() -> T + Send + 'static) -> T {
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || send.send(body()).unwrap());
    recv.recv_timeout(LIMIT)
        .unwrap_or_else(|e| panic!("{what}: did not return: {e}"))
}

/// Returns what open and calls on the opened database answer, each of `rounds` opens over the
/// files the one before left.
fn opened(files: Files, rounds: usize, what: &str) -> Vec<Result<Opened, OpenError>> {
    within(what, move || {
        let mut sim = Sim::with_files(1, Config::default(), files);
        let mut got = Vec::new();
        for _ in 0..rounds {
            finished(&mut sim, |rt| {
                got.push(open(rt).map(|db| {
                    let opened = call(&db);
                    db.close().unwrap();
                    opened
                }));
            });
        }
        got
    })
}

fn assert_refused(got: &Result<(), Error>, expected: &str, what: &str) {
    match got {
        Err(Error::Fatal {
            reason: FatalReason::Quarantined { what: said },
        }) => assert_eq!(expected, said, "{what}"),
        other => panic!("{what}: not Quarantined: {other:?}"),
    }
}

fn assert_refused_rows(got: &Result<Vec<String>, Error>, expected: &str, what: &str) {
    assert_refused(
        &got.as_ref().map(drop).map_err(Clone::clone),
        expected,
        what,
    );
}

fn assert_for_verify_only(got: &Result<Opened, OpenError>, finding: &str, what: &str) {
    let opened = got
        .as_ref()
        .unwrap_or_else(|e| panic!("{what}: did not open: {e:?}"));
    assert!(opened.marks.contains(&Quarantine::Database), "{what}");
    assert_refused_rows(&opened.select, "the database", what);
    assert_refused(&opened.insert, "the database", what);
    assert!(
        opened.verify.iter().any(|f| f.starts_with(finding)),
        "{what}: {:?}",
        opened.verify
    );
}

/// Assert open refused, or whatever calls run see the row: none of it is lost without a report.
/// Returns whether the open succeeded.
#[must_use]
fn assert_nothing_silently_lost(got: &Result<Opened, OpenError>, what: &str) -> bool {
    let Ok(opened) = got else {
        return false;
    };
    match &opened.select {
        Ok(rows) => assert_eq!(vec![ROW.to_string()], *rows, "{what}"),
        Err(Error::Fatal {
            reason: FatalReason::Quarantined { .. },
        }) => assert!(!opened.marks.is_empty(), "{what}"),
        Err(e) => panic!("{what}: {e:?}"),
    }
    true
}

#[test]
fn a_file_cut_before_the_first_roles_page_opens_for_verify() {
    let s = seeded();
    let files = cut(&s.files, s.roles.0 as usize * PAGE);
    for (round, got) in opened(files, 2, "cut at roles").iter().enumerate() {
        assert_for_verify_only(got, "Checksum", &format!("round {round}"));
    }
}

#[test]
fn a_file_cut_inside_the_first_roles_page_opens_for_verify() {
    let s = seeded();
    let files = cut(&s.files, s.roles.0 as usize * PAGE + 100);
    assert_for_verify_only(
        &opened(files, 1, "cut in roles")[0],
        "Checksum",
        "cut in roles",
    );
}

#[test]
fn a_file_cut_before_its_first_bitmap_page_opens_for_verify() {
    let s = seeded();
    let files = cut(&s.files, 2 * PAGE);
    assert_for_verify_only(
        &opened(files, 1, "cut at bitmap")[0],
        "Checksum",
        "cut at bitmap",
    );
}

#[test]
fn a_file_cut_before_a_relations_first_page_quarantines_that_relation() {
    let s = seeded();
    let files = cut(&s.files, s.users.0 as usize * PAGE);
    for (round, got) in opened(files, 2, "cut at users").iter().enumerate() {
        let what = format!("round {round}");
        let opened = got
            .as_ref()
            .unwrap_or_else(|e| panic!("{what}: did not open: {e:?}"));
        assert!(!opened.marks.contains(&Quarantine::Database), "{what}");
        assert_refused_rows(&opened.select, "app::users.User", &what);
        assert_refused(&opened.insert, "app::users.User", &what);
    }
}

#[test]
fn a_zeroed_roles_page_opens_for_verify() {
    let s = seeded();
    let files = zeroed(&s.files, s.roles);
    assert_for_verify_only(
        &opened(files, 1, "zeroed roles")[0],
        "StalePage",
        "zeroed roles",
    );
}

#[test]
fn a_write_to_a_zeroed_page_of_a_relation_is_refused() {
    let s = seeded();
    let files = zeroed(&s.files, s.users);
    let got = within("insert", move || {
        let mut sim = Sim::with_files(1, Config::default(), files);
        let mut got = None;
        finished(&mut sim, |rt| {
            let db = open(rt).unwrap();
            let none = CallOptions::default();
            got = Some(db.query_text(INSERT, &[], &none).map(drop));
            db.close().unwrap();
        });
        got.unwrap()
    });
    assert_refused(&got, "app::users.User", "insert");
}

#[test]
fn open_returns_on_a_file_cut_at_every_sector_of_its_first_pages() {
    let s = seeded();
    let mut opens = 0;
    for sectors in sweep(1..65) {
        let len = sectors as usize * SECTOR;
        let what = format!("cut at {len} bytes");
        let got = opened(cut(&s.files, len), 1, &what);
        opens += usize::from(assert_nothing_silently_lost(&got[0], &what));
    }
    assert_reached("a cut file that opened", opens, share(1..65, 30));
}

#[test]
fn open_returns_on_a_file_cut_at_pages_across_it() {
    let s = seeded();
    let pages = s.files[FILE].len() / PAGE;
    let lengths = sweep(0..64);
    let stride = pages.div_ceil(lengths.end as usize);
    let mut opens = 0;
    for i in lengths {
        let len = ((i as usize * stride + 1) * PAGE).min(pages * PAGE);
        let what = format!("cut at {len} bytes");
        let got = opened(cut(&s.files, len), 1, &what);
        opens += usize::from(assert_nothing_silently_lost(&got[0], &what));
    }
    assert_reached("a cut file that opened", opens, share(0..64, 32));
}

#[test]
fn open_returns_on_a_file_cut_at_each_of_its_last_pages() {
    let s = seeded();
    let pages = s.files[FILE].len() / PAGE;
    let mut opens = 0;
    for page in s.roles.0 as usize..pages {
        let what = format!("cut at page {page}");
        let got = opened(cut(&s.files, page * PAGE), 1, &what);
        opens += usize::from(assert_nothing_silently_lost(&got[0], &what));
    }
    assert_reached("a cut file that opened", opens, 1);
}

fn select_answer(db: &Database<SimRuntime>, text: &str) -> Result<Vec<String>, Error> {
    db.query_text(text, &[], &CallOptions::default())
        .map(|mut r| api::printed(r.remove(0)))
}

type Round = (Result<Vec<String>, Error>, BTreeSet<Quarantine>, Quarantine);

/// Returns, for each of two opens of `files`, how `text` ends, the marks after it, and the mark of
/// the users heap.
fn selected_twice(files: Files, text: &'static str, what: &'static str) -> Vec<Round> {
    within(what, move || {
        let mut sim = Sim::with_files(1, Config::default(), files);
        let mut got = Vec::new();
        for _ in 0..2 {
            finished(&mut sim, |rt| {
                let db = open(rt).unwrap();
                let w = db.write_path();
                let users = Quarantine::Heap(w.heap_named("app::users", "User").unwrap().id().0);
                got.push((
                    select_answer(&db, text),
                    db.testing().probe().marks(),
                    users,
                ));
                db.close().unwrap();
            });
        }
        got
    })
}

fn assert_select_refused(files: Files, text: &'static str, what: &'static str) {
    let got = selected_twice(files, text, what);
    for (round, (answer, marks, _)) in got.iter().enumerate() {
        let what = format!("{what}, round {round}");
        assert_refused_rows(answer, "app::users.User", &what);
        assert_eq!(1, marks.len(), "{what}: {marks:?}");
        assert!(!marks.contains(&Quarantine::Database), "{what}: {marks:?}");
    }
}

#[test]
fn a_select_over_a_zeroed_page_is_refused_and_marks_the_heap() {
    let s = seeded();
    assert_select_refused(zeroed(&s.files, s.users), SELECT, "scan");
}

#[test]
fn a_select_by_key_over_a_zeroed_page_is_refused_and_marks_the_heap() {
    let s = seeded();
    assert_select_refused(
        zeroed(&s.files, s.users),
        "from u in users where key u == users.key(1) select u.id",
        "probe",
    );
}

const TWO: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Other = { id: Key others, name: Text }
add relation others : Other key id generate sequence
";

fn open_two(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt, FILE, &Log::new([TWO]), &api::options(2)).unwrap()
}

#[test]
fn a_cut_page_ordinary_writes_grew_the_file_over_stays_refused() {
    let mut sim = Sim::new(1, Config::default());
    let mut last = None;
    finished(&mut sim, |rt| {
        let db = open_two(rt);
        let name = "x".repeat(1000);
        for i in 0..400 {
            api::results(
                &db,
                &format!(r#"insert users {{ id: users.key({i}), name: "{name}" }}"#),
            );
        }
        let w = db.write_path();
        let pages = w
            .heap_named("app::users", "User")
            .unwrap()
            .pages(w.pager())
            .unwrap();
        last = pages.last().copied();
        db.close().unwrap();
    });
    let last = last.unwrap();
    let mut sim = Sim::with_files(
        1,
        Config::default(),
        cut(&sim.files(), last.0 as usize * PAGE),
    );
    finished(&mut sim, |rt| {
        let db = open_two(rt);
        let name = "y".repeat(1000);
        for i in 0..400 {
            api::results(
                &db,
                &format!(r#"insert others {{ id: others.key({i}), name: "{name}" }}"#),
            );
        }
        db.close().unwrap();
    });
    let files = sim.files();
    let at = last.0 as usize * PAGE;
    assert!(
        files[FILE][at..at + PAGE].iter().all(|&b| b == 0),
        "the file grew over the cut page"
    );
    let mut sim = Sim::with_files(1, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open_two(rt);
        let answer = select_answer(&db, SELECT);
        assert_refused_rows(&answer, "app::users.User", "after growth");
        assert!(!db.testing().probe().marks().is_empty());
        db.close().unwrap();
    });
}

#[test]
fn a_cut_that_loses_a_later_bitmap_group_refuses_the_select() {
    adtb_heap_cap::raise_cap(4 << 30);
    let mut sim = Sim::new(1, Config::default());
    let span = (PAGE - HEADER_LEN) * 8;
    let mut high = None;
    finished(&mut sim, |rt| {
        let db = open(rt).unwrap();
        let p = db.write_path().pager();
        p.update(&[Pid::FIRST_DATA], |v| v[0][HEADER_LEN..].fill(0xff))
            .unwrap();
        let name = "x".repeat(2500);
        for i in 0..40 {
            api::results(
                &db,
                &format!(r#"insert users {{ id: users.key({i}), name: "{name}" }}"#),
            );
        }
        let w = db.write_path();
        high = w
            .heap_named("app::users", "User")
            .unwrap()
            .pages(w.pager())
            .unwrap()
            .last()
            .copied();
        db.close().unwrap();
    });
    assert!(high.unwrap().0 > span as u64, "a page in the second group");
    let files = cut(&sim.files(), (span - 100) * PAGE);
    let got = selected_twice(files, SELECT, "lost group");
    for (round, (answer, marks, users)) in got.iter().enumerate() {
        let what = format!("lost group, round {round}");
        assert_refused_rows(answer, "the database", &what);
        assert_eq!(
            BTreeSet::from([Quarantine::Database, *users]),
            *marks,
            "{what}"
        );
    }
}

#[test]
fn a_zeroed_large_value_page_refuses_the_select() {
    let mut sim = Sim::new(1, Config::default());
    let mut page = None;
    finished(&mut sim, |rt| {
        let db = open(rt).unwrap();
        let name = "x".repeat(4 * PAGE);
        api::results(
            &db,
            &format!(r#"insert users {{ id: users.key(1), name: "{name}" }}"#),
        );
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        page = heap.large().pages(w.pager()).unwrap().first().copied();
        db.close().unwrap();
    });
    let files = zeroed(&sim.files(), page.unwrap());
    let got = within("large", move || {
        let mut sim = Sim::with_files(1, Config::default(), files);
        let mut got = None;
        finished(&mut sim, |rt| {
            let db = open(rt).unwrap();
            got = Some(select_answer(&db, "from u in users select u.name"));
            db.close().unwrap();
        });
        got.unwrap()
    });
    assert_refused_rows(&got, "the database", "large");
}

#[test]
fn a_zeroed_directory_page_refuses_the_select() {
    let s = seeded();
    let mut sim = Sim::with_files(1, Config::default(), s.files);
    let mut node = None;
    finished(&mut sim, |rt| {
        let db = open(rt).unwrap();
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let owned = heap.owned_pages(w.pager()).unwrap();
        node = owned.first().copied();
        db.close().unwrap();
    });
    assert_select_refused(zeroed(&sim.files(), node.unwrap()), SELECT, "directory");
}

fn open_small(rt: SimRuntime) -> Database<SimRuntime> {
    let options = OpenOptions {
        pool_bytes: Some(64 * PAGE as u64),
        ..api::options(2)
    };
    Database::open_in(rt, FILE, &Log::new([APP]), &options).unwrap()
}

fn stressed_config() -> Config {
    Config {
        switch_rate: 0.3,
        ..Config::default()
    }
}

fn assert_not_refused<T: std::fmt::Debug>(got: &Result<T, Error>, what: &str) {
    assert!(
        !matches!(
            got,
            Err(Error::Fatal {
                reason: FatalReason::Quarantined { .. }
            })
        ),
        "{what}: {got:?}"
    );
}

/// Grow pages, free them and read them back from a pool the writes overrun: new pages named
/// before their first write out, pages vacuum frees under a scan's listing.
fn churn(rt: &SimRuntime, db: &Database<SimRuntime>, what: &str) {
    let none = CallOptions::default();
    let stop = Arc::new(AtomicBool::new(false));
    let scans = {
        let (db, stop, what, probe) = (db.clone(), stop.clone(), what.to_owned(), rt.clone());
        rt.spawn("scans", move || {
            let none = CallOptions::default();
            while !stop.load(Ordering::Relaxed) {
                for text in [
                    SELECT,
                    "from u in users where key u == users.key(3) select u.id",
                ] {
                    assert_not_refused(&db.query_text(text, &[], &none), &what);
                }
                common::sleep(&probe, &mut probe.io(), Duration::from_millis(1));
            }
        })
    };
    let name = "x".repeat(900);
    for round in 0..3i64 {
        let low = round * 100;
        let inserts: String = (low..low + 40)
            .map(|i| format!("insert users {{ id: users.key({i}), name: \"{name}\" }}\n"))
            .collect();
        assert_not_refused(&db.query_text(&inserts, &[], &none), what);
        let gone = format!(
            "delete u in users where key u >= users.key({low}) and key u < users.key({})",
            low + 40
        );
        assert_not_refused(&db.query_text(&gone, &[], &none), what);
        assert_not_refused(&db.query_text("vacuum users", &[], &none), what);
    }
    stop.store(true, Ordering::Relaxed);
    rt.join(scans);
    assert_eq!(BTreeSet::new(), db.testing().probe().marks(), "{what}");
}

fn no_refusal_after_reopening(seed: u64, files: Files, what: &str) {
    let mut sim = Sim::with_files(seed, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open_small(rt);
        let got = select_answer(&db, SELECT);
        assert!(got.is_ok(), "{what}: {got:?}");
        assert_eq!(BTreeSet::new(), db.testing().probe().marks(), "{what}");
        db.close().unwrap();
    });
}

#[test]
fn no_healthy_page_reads_zero_through_writers_vacuum_frees_and_crashes() {
    for seed in sweep(0..1) {
        let mut sim = Sim::new(seed, stressed_config());
        let mut took = None;
        finished(&mut sim, |rt| {
            let db = open_small(rt.clone());
            let began = rt.now();
            churn(&rt, &db, &format!("seed {seed}"));
            took = Some((began, rt.now()));
            db.close().unwrap();
        });
        no_refusal_after_reopening(seed, sim.files(), &format!("seed {seed}"));
        let (began, ended) = took.unwrap();
        let (began, ended) = (began.as_nanos(), ended.as_nanos());
        for step in 1..8u64 {
            let at = began + (ended - began) / 8 * step;
            let what = format!("seed {seed} crash at step {step}");
            let mut sim = Sim::new(seed, stressed_config());
            sim.crash_at(adtb::runtime::Instant::from_nanos(at));
            let outcome = sim.run(|rt| {
                let db = open_small(rt.clone());
                churn(&rt, &db, &what);
                db.close().unwrap();
            });
            assert_eq!(Outcome::Crashed, outcome, "{what}");
            no_refusal_after_reopening(seed, sim.files(), &what);
        }
    }
}

#[test]
fn a_scan_of_a_listing_skips_the_pages_vacuum_freed_since() {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let db = open_small(rt);
        api::results(&db, INSERT);
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let listing = heap.pages(w.pager()).unwrap();
        api::results(&db, "delete u in users");
        api::results(&db, "vacuum users");
        assert!(
            w.pager().is_free(listing[0]).unwrap(),
            "vacuum freed the page"
        );
        let mut seen = 0;
        heap.scan_pages(w.pager(), &listing, |_| seen += 1).unwrap();
        assert_eq!(0, seen);
        assert_eq!(BTreeSet::new(), db.testing().probe().marks());
        db.close().unwrap();
    });
}

fn refused_on_zeroed_users(text: &'static str) {
    let s = seeded();
    let files = zeroed(&s.files, s.users);
    let got = within(text, move || {
        let mut sim = Sim::with_files(1, Config::default(), files);
        let mut got = None;
        finished(&mut sim, |rt| {
            let db = open(rt).unwrap();
            got = Some(db.query_text(text, &[], &CallOptions::default()).map(drop));
            db.close().unwrap();
        });
        got.unwrap()
    });
    assert_refused(&got, "app::users.User", text);
}

#[test]
fn a_vacuum_over_a_zeroed_page_is_refused() {
    refused_on_zeroed_users("vacuum users");
}

#[test]
fn a_reindex_over_a_zeroed_page_is_refused() {
    const INDEXED: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
  index (name)
";
    let reopen = |rt| Database::open_in(rt, FILE, &Log::new([INDEXED]), &api::options(2)).unwrap();
    let mut sim = Sim::new(1, Config::default());
    let mut page = None;
    finished(&mut sim, |rt| {
        let db = reopen(rt);
        api::results(&db, INSERT);
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        page = heap.pages(w.pager()).unwrap().first().copied();
        db.close().unwrap();
    });
    let mut sim = Sim::with_files(1, Config::default(), zeroed(&sim.files(), page.unwrap()));
    finished(&mut sim, |rt| {
        let rt2 = rt.clone();
        let db = reopen(rt);
        api::results(&db, "reindex users");
        let marked = (0..600).any(|_| {
            common::sleep(&rt2, &mut rt2.io(), Duration::from_millis(100));
            !db.testing().probe().marks().is_empty()
        });
        assert!(marked, "the rebuild marked what it could not read");
        let got = select_answer(&db, SELECT);
        assert_refused_rows(&got, "app::users.User", "reindex");
        db.close().unwrap();
    });
}

#[test]
fn a_read_of_a_version_on_a_zeroed_page_is_refused() {
    let s = seeded();
    let mut sim = Sim::with_files(1, Config::default(), zeroed(&s.files, s.users));
    finished(&mut sim, |rt| {
        let db = open(rt).unwrap();
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let got = heap.read(w.pager(), Tid::new(s.users.0, 0).unwrap());
        assert_eq!(s.users, got.unwrap_err().pid);
        db.close().unwrap();
    });
}

#[test]
fn a_large_value_write_over_a_zeroed_page_of_the_store_is_refused() {
    let mut sim = Sim::new(1, Config::default());
    let mut page = None;
    finished(&mut sim, |rt| {
        let db = open(rt).unwrap();
        let name = "x".repeat(PAGE / 3);
        api::results(
            &db,
            &format!(r#"insert users {{ id: users.key(1), name: "{name}" }}"#),
        );
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        page = heap.large().pages(w.pager()).unwrap().last().copied();
        db.close().unwrap();
    });
    let files = zeroed(&sim.files(), page.unwrap());
    let got = within("large insert", move || {
        let mut sim = Sim::with_files(1, Config::default(), files);
        let mut got = None;
        finished(&mut sim, |rt| {
            let db = open(rt).unwrap();
            let name = "y".repeat(PAGE / 3);
            let text = format!(r#"insert users {{ id: users.key(2), name: "{name}" }}"#);
            got = Some(db.query_text(&text, &[], &CallOptions::default()).map(drop));
            db.close().unwrap();
        });
        got.unwrap()
    });
    assert_refused(&got, "the database", "large insert");
}

const LOG_PRESENT: &str = "the log directory holds a log that no file here wrote";
const NO_HEADER_CREATE: &str = "neither copy of the file header verifies";
const NO_HEADER: &str = "NoHeader";
const HEADER_COPY: usize = 4096;

fn with_create(create: bool) -> OpenOptions {
    OpenOptions {
        create: create.then(CreateOptions::default),
        ..api::options(2)
    }
}

fn refusal(got: &Result<(), OpenError>) -> Option<&str> {
    match got {
        Err(OpenError::Corrupt { detail }) => Some(detail),
        _ => None,
    }
}

/// Cut the file to each length up to and past both header copies with its log kept, and open
/// with and without `create`. While no header copy is whole open refuses, an empty file as one
/// whose log is no create's, and no file or directory changes, appears or goes.
#[test]
fn a_file_cut_short_beside_its_log_never_opens_as_a_new_database() {
    let s = seeded();
    let mut lengths = vec![0, 1, PAGE, PAGE + 1, 2 * PAGE];
    if every_case() {
        lengths.extend(
            (0..=2 * PAGE + SECTOR)
                .step_by(SECTOR)
                .flat_map(|n| [n, n + 1]),
        );
    }
    let (mut logs, mut headers, mut opens) = (0, 0, 0);
    for len in lengths {
        for create in [true, false] {
            let what = format!("cut at {len} bytes, create {create}");
            let files = cut(&s.files, len);
            let mut sim = Sim::with_files(1, Config::default(), files.clone());
            let dirs = sim.dirs();
            let mut got = None;
            finished(&mut sim, |rt| {
                let opened = Database::open_in(rt, FILE, &Log::new([APP]), &with_create(create));
                got = Some(opened.map(|db| db.close().unwrap()));
            });
            let got = got.unwrap();
            let expected = match (len, create) {
                (0, _) => Some(LOG_PRESENT),
                (..HEADER_COPY, true) => Some(NO_HEADER_CREATE),
                (..HEADER_COPY, false) => Some(NO_HEADER),
                _ => None,
            };
            assert_eq!(expected, refusal(&got), "{what}: {got:?}");
            match expected {
                Some(LOG_PRESENT) => logs += 1,
                Some(_) => headers += 1,
                None => opens += 1,
            }
            if expected.is_some() {
                assert_eq!(files, sim.files(), "{what}");
                assert_eq!(dirs, sim.dirs(), "{what}");
            } else {
                got.unwrap_or_else(|e| panic!("{what}: {e:?}"));
            }
        }
    }
    assert_reached("an empty file refused for its log", logs, 2);
    assert_reached("a file refused for its header", headers, 2);
    assert_reached("a file with a header copy that opened", opens, 2);
}

/// On the host, a file cut to nothing beside its log refuses with and without `create`, and no
/// file or directory changes, appears or goes.
#[test]
fn a_real_file_cut_to_nothing_beside_its_log_refuses() {
    let files = cut(&seeded().files, 0);
    for create in [true, false] {
        let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("truncated_open")
            .join(format!("create-{create}"));
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
            _ => {}
        }
        for (path, bytes) in &files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let got = Database::open(dir.join(FILE), &Log::new([APP]), &with_create(create)).map(drop);
        assert_eq!(Some(LOG_PRESENT), refusal(&got), "create {create}: {got:?}");
        assert_eq!(files, read_tree(&dir, &dir), "create {create}");
    }
}

/// Returns every entry under `at` by its path from `root`: a file's bytes, a directory's empty.
fn read_tree(root: &std::path::Path, at: &std::path::Path) -> Files {
    let mut out = Files::new();
    for entry in std::fs::read_dir(at).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(read_tree(root, &path));
        } else {
            let name = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            out.insert(name, std::fs::read(&path).unwrap());
        }
    }
    out
}
