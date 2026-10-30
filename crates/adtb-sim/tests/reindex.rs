//! `reindex`: its job row, rebuilds under writers and across crashes, and the release of a
//! quarantined index.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use adtb::key::{Entry, Value};
use adtb::runtime::{Clock, Threads};
use adtb::storage::{Damage, Pid, Quarantine};
use adtb::{Database, Log, OpenOptions};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::{api, crash, seeds};

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
  | Apple     { sub: Text }
add relation identities : Identity key id generate sequence
  unique (email) in EmailPass
  unique (gid) in Google
  unique (sub) in Apple
  index (user) in (Google | Apple)
";

const FILE: &str = "reindex.adt";
const API_PAGE: usize = 8192;
const USER_INDEX: &str = "index (user) in (Google | Apple)";
const GID_INDEX: &str = "unique (gid) in Google";
const USER_REINDEX: &str = "reindex identities index (user) in (Google | Apple)";
const PHASES: [&str; 6] = ["waiting", "scan", "build", "catch up", "swap", "reclaim"];

fn open_app(rt: SimRuntime) -> Database<SimRuntime> {
    Database::open_in(rt, FILE, &Log::new([APP]), &api::options(2)).expect("the database opens")
}

fn user_text(i: u32) -> String {
    format!(
        r#"insert users {{ id: users.key({i}), name: "n{i}" }}
               insert identities.EmailPass {{ id: identities.key({}), user: users.key({i}), email: "e{i}" }}
               insert identities.Google {{ id: identities.key({}), user: users.key({i}), gid: "g{i}" }}
               insert identities.Apple {{ id: identities.key({}), user: users.key({i}), sub: "a{i}" }}"#,
        i * 10,
        i * 10 + 1,
        i * 10 + 2,
    )
}

fn insert_user(db: &Database<SimRuntime>, i: u32) {
    api::results(db, &user_text(i));
}

fn seed_rows(db: &Database<SimRuntime>, rows: u32) {
    for i in 1..=rows {
        insert_user(db, i);
    }
}

fn wait_for_jobs(rt: &SimRuntime, db: &Database<SimRuntime>) {
    while !api::printed(api::result(db, "from j in sys::jobs select j.id")).is_empty() {
        for _ in 0..256 {
            rt.yield_now();
        }
    }
}

fn verified(db: &Database<SimRuntime>) -> Vec<String> {
    api::printed(db.verify_holding().expect("verify runs"))
}

fn verified_live(db: &Database<SimRuntime>) -> Vec<String> {
    api::printed(db.verify(None).expect("verify runs"))
}

fn findings(db: &Database<SimRuntime>) -> Vec<String> {
    api::printed(api::result(db, "from f in sys::verify_findings select f"))
}

fn marks(db: &Database<SimRuntime>) -> BTreeSet<Quarantine> {
    db.write_path().pager().quarantined()
}

fn user_mark(db: &Database<SimRuntime>) -> Quarantine {
    let w = db.write_path();
    Quarantine::Index(
        w.index_named("app::identities", USER_INDEX)
            .unwrap()
            .tree()
            .owner(),
    )
}

fn user_index_pages(db: &Database<SimRuntime>) -> Vec<Pid> {
    let w = db.write_path();
    let index = w.index_named("app::identities", USER_INDEX).unwrap();
    index.tree().pages(w.pager()).unwrap()
}

/// Returns the entries `index` of `identities` holds, from a walk of the tree.
fn entries(db: &Database<SimRuntime>, clause: &str) -> Vec<Entry> {
    let w = db.write_path();
    let index = w.index_named("app::identities", clause).unwrap();
    index
        .range(
            w.pager(),
            std::ops::Bound::Unbounded,
            std::ops::Bound::Unbounded,
        )
        .unwrap()
}

/// Assert that a probe of the user index finds the Google and Apple rows of each of `users`.
fn assert_every_row_found(db: &Database<SimRuntime>, users: impl Iterator<Item = u32>, what: &str) {
    for i in users {
        let got = api::query(
            db,
            &format!("from g in identities.Google where g.user == users.key({i}) select g.gid"),
        );
        assert_eq!(vec![format!(r#""g{i}""#)], got, "{what}: user {i}");
        let held = entries(db, USER_INDEX)
            .iter()
            .filter(|e| e.key == vec![Value::I64(i64::from(i))])
            .count();
        assert!(held >= 2, "{what}: user {i} has {held} entries");
    }
}

#[test]
fn the_result_names_the_job_and_the_row_shows_its_phase_while_it_runs() {
    api::once(1, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 3);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (shown, reader) = (seen.clone(), db.clone());
        db.on_reindex_phase(Arc::new(move |phase| {
            if phase == "build" {
                *shown.lock().unwrap() = api::printed(api::result(
                    &reader,
                    "from j in sys::jobs select { k: j.kind, t: j.target, p: j.phase, d: j.done }",
                ));
            }
        }));
        let got = api::printed(api::results(&db, USER_REINDEX).remove(0));
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].starts_with("{ job: "), "{got:?}");
        assert_eq!(
            1,
            api::printed(api::result(&db, "from j in sys::jobs select j.id")).len(),
            "the row is there when the result returns"
        );
        wait_for_jobs(&rt, &db);
        let row = seen.lock().unwrap().clone();
        assert_eq!(1, row.len(), "{row:?}");
        for part in ["Reindex", "identities", "build"] {
            assert!(row[0].contains(part), "{row:?}");
        }
        db.close().unwrap();
    });
}

#[test]
fn a_panic_in_the_job_removes_its_row_and_keeps_the_old_index_and_a_finding() {
    api::once(2, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 3);
        let before = entries(&db, USER_INDEX);
        db.on_reindex_phase(Arc::new(|phase| {
            assert_ne!("catch up", phase, "the injected failure");
        }));
        api::results(&db, USER_REINDEX);
        wait_for_jobs(&rt, &db);
        assert_eq!(before, entries(&db, USER_INDEX));
        let got = findings(&db);
        assert_eq!(1, got.len(), "{got:?}");
        assert!(got[0].contains("Structure"), "{got:?}");
        assert!(got[0].contains("identities"), "{got:?}");
        assert_eq!(Vec::<String>::new(), verified(&db));
        db.close().unwrap();
    });
}

#[test]
fn a_rebuild_under_writers_loses_no_row() {
    for seed in seeds::sweep(0..1) {
        api::once(seed, |rt| {
            let db = open_app(rt.clone());
            seed_rows(&db, 80);
            let stop = Arc::new(AtomicBool::new(false));
            let (writer, stopping) = (db.clone(), stop.clone());
            let last = Arc::new(Mutex::new(80));
            let wrote = last.clone();
            let write = rt.spawn("writer", move || {
                let mut i = 81;
                while !stopping.load(Ordering::Relaxed) && i < 200 {
                    insert_user(&writer, i);
                    *wrote.lock().unwrap() = i;
                    i += 1;
                }
            });
            api::results(&db, "reindex identities");
            wait_for_jobs(&rt, &db);
            stop.store(true, Ordering::Relaxed);
            rt.join(write);
            let last = *last.lock().unwrap();
            assert_every_row_found(&db, 1..=last, &format!("seed {seed}"));
            assert_eq!(Vec::<String>::new(), verified_live(&db), "seed {seed}");
            db.close().unwrap();
        });
    }
}

#[test]
fn a_commit_landing_in_each_phase_is_in_the_new_index() {
    api::once(3, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 10);
        let next = Arc::new(Mutex::new(100));
        let (writer, counter) = (db.clone(), next.clone());
        db.on_reindex_phase(Arc::new(move |_| {
            let mut n = counter.lock().unwrap();
            insert_user(&writer, *n);
            *n += 1;
        }));
        api::results(&db, USER_REINDEX);
        wait_for_jobs(&rt, &db);
        let end = *next.lock().unwrap();
        assert!(end >= 100 + PHASES.len() as u32 - 1, "{end}");
        assert_every_row_found(&db, (1..=10).chain(100..end), "after the rebuild");
        assert_eq!(Vec::<String>::new(), verified_live(&db));
        db.close().unwrap();
    });
}

#[test]
fn a_write_keeps_its_uniqueness_check_across_a_rebuild() {
    api::once(4, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 5);
        let refused = Arc::new(Mutex::new(Vec::new()));
        let (writer, out) = (db.clone(), refused.clone());
        db.on_reindex_phase(Arc::new(move |phase| {
            let e = api::failure(
                &writer,
                r#"insert identities.Google { id: identities.key(9000), user: users.key(1), gid: "g1" }"#,
            );
            out.lock().unwrap().push(format!("{phase}: {e:?}"));
        }));
        api::results(&db, "reindex identities index (gid) in Google");
        wait_for_jobs(&rt, &db);
        assert!(refused.lock().unwrap().len() >= 4, "{refused:?}");
        db.close().unwrap();
    });
}

/// Returns the instant the run of seed `seed` reaches each phase.
fn phase_times(seed: u64) -> Vec<(String, adtb::runtime::Instant)> {
    let times = Arc::new(Mutex::new(Vec::new()));
    let out = times.clone();
    let mut sim = Sim::new(seed, Config::default());
    api::run(&mut sim, move |rt| {
        let db = crash_body(&rt);
        let (clock, seen) = (rt.clone(), out);
        let flushing = db.clone();
        db.on_reindex_phase(Arc::new(move |phase| {
            seen.lock().unwrap().push((phase.to_owned(), clock.now()));
            flushing.write_path().pager().flush_log();
        }));
        api::results(&db, USER_REINDEX);
        wait_for_jobs(&rt, &db);
        db.close().unwrap();
    });
    times.lock().unwrap().clone()
}

fn crash_body(rt: &SimRuntime) -> Database<SimRuntime> {
    let db = open_app(rt.clone());
    seed_rows(&db, 40);
    db.on_reindex_phase(Arc::new(|_| {}));
    db
}

#[test]
fn a_crash_at_each_stage_keeps_an_index_that_finds_every_row_and_leaks_nothing() {
    for seed in seeds::sweep(0..1) {
        let times = phase_times(seed);
        assert!(times.len() >= 4, "seed {seed}: {times:?}");
        crash::silence_fatal_panics();
        for (phase, at) in times {
            let mut sim = Sim::new(seed, Config::default());
            sim.crash_at(at);
            let ran = sim.run(|rt| {
                let db = crash_body(&rt);
                let flushing = db.clone();
                db.on_reindex_phase(Arc::new(move |_| {
                    flushing.write_path().pager().flush_log();
                }));
                api::results(&db, USER_REINDEX);
                wait_for_jobs(&rt, &db);
                db.close().unwrap();
            });
            assert_eq!(Outcome::Crashed, ran, "seed {seed} phase {phase}");
            api::run(&mut sim, |rt| {
                let db = open_app(rt);
                assert_every_row_found(&db, 1..=40, &format!("seed {seed} phase {phase}"));
                assert_eq!(
                    Vec::<String>::new(),
                    verified(&db),
                    "seed {seed} phase {phase}"
                );
                db.close().unwrap();
            });
        }
    }
}

/// Returns a simulator over a seeded file with the last page of the user index flipped on disk.
fn with_damaged_user_index(seed: u64, top: bool) -> Sim {
    let mut sim = Sim::new(seed, Config::default());
    let page = api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed_rows(&db, 300);
        let pages = user_index_pages(&db);
        assert!(pages.len() > 1, "{pages:?}");
        let page = if top {
            pages[0]
        } else {
            *pages.last().unwrap()
        };
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    files.get_mut(FILE).unwrap()[page.0 as usize * API_PAGE + 300] ^= 0x10;
    Sim::with_files(seed, Config::default(), files)
}

fn quarantine_user_index(db: &Database<SimRuntime>) {
    api::failure(
        db,
        "from g in identities.Google where g.user == users.key(300) select g.gid",
    );
    assert_eq!(BTreeSet::from([user_mark(db)]), marks(db));
}

fn both_examples_run(db: &Database<SimRuntime>, id: u32) {
    let probe = "from g in identities.Google where g.user == users.key(3) select g.gid";
    let before = api::query(db, probe);
    assert!(before.contains(&r#""g3""#.to_owned()), "{before:?}");
    api::results(
        db,
        &format!(
            r#"insert identities.Google {{ id: identities.key({id}), user: users.key(3), gid: "g-{id}" }}"#
        ),
    );
    assert_eq!(before.len() + 1, api::query(db, probe).len());
}

#[test]
fn a_quarantined_index_is_restored_whichever_page_is_damaged() {
    for (seed, top) in [(5, false), (6, true)] {
        let mut sim = with_damaged_user_index(seed, top);
        api::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            quarantine_user_index(&db);
            api::results(&db, USER_REINDEX);
            wait_for_jobs(&rt, &db);
            assert_eq!(BTreeSet::new(), marks(&db), "top {top}");
            both_examples_run(&db, 9000);
            assert_every_row_found(&db, 1..=2, "restored");
            assert_eq!(Vec::<String>::new(), verified(&db), "top {top}");
            db.close().unwrap();
        });
        api::run(&mut sim, |rt| {
            let db = open_app(rt);
            assert_eq!(BTreeSet::new(), marks(&db), "the release survives restart");
            assert_every_row_found(&db, 1..=2, "after restart");
            both_examples_run(&db, 9001);
            assert_eq!(Vec::<String>::new(), verified(&db), "top {top}");
            db.close().unwrap();
        });
    }
}

/// Seed a file, damage the root of the user index with its records still in the log, restore it
/// and return the instant the rebuild reclaims, if `report` is given.
fn restore_damaged_root(
    rt: &SimRuntime,
    report: Option<Arc<Mutex<Option<adtb::runtime::Instant>>>>,
) {
    let db = open_app(rt.clone());
    seed_rows(&db, 5);
    let root = user_index_pages(&db)[0];
    db.damage_page(root, Damage::Checksum);
    api::failure(
        &db,
        "from g in identities.Google where g.user == users.key(3) select g.gid",
    );
    assert_eq!(BTreeSet::from([user_mark(&db)]), marks(&db));
    let clock = rt.clone();
    db.on_reindex_phase(Arc::new(move |phase| {
        if phase == "reclaim"
            && let Some(at) = &report
        {
            *at.lock().unwrap() = Some(clock.now());
        }
    }));
    api::results(&db, USER_REINDEX);
    wait_for_jobs(rt, &db);
    db.close().unwrap();
}

#[test]
fn a_crash_after_overwriting_a_damaged_root_replays_its_image() {
    let at = Arc::new(Mutex::new(None));
    let mut sim = Sim::new(19, Config::default());
    let report = at.clone();
    api::run(&mut sim, move |rt| restore_damaged_root(&rt, Some(report)));
    let at = at.lock().unwrap().expect("the rebuild reclaims");
    crash::silence_fatal_panics();
    let mut sim = Sim::new(19, Config::default());
    sim.crash_at(at);
    assert_eq!(
        Outcome::Crashed,
        sim.run(|rt| restore_damaged_root(&rt, Some(Arc::new(Mutex::new(None)))))
    );
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(BTreeSet::new(), marks(&db));
        assert_every_row_found(&db, 1..=5, "after the crash");
        assert_eq!(Vec::<String>::new(), verified(&db));
        db.close().unwrap();
    });
}

#[test]
fn the_mark_of_a_verify_finding_is_released_by_a_rebuild() {
    api::once(7, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 10);
        let w = db.write_path();
        let index = w.index_named("app::identities", USER_INDEX).unwrap();
        let entry = entries(&db, USER_INDEX).remove(0);
        assert!(index.delete(w.pager(), &entry).unwrap());
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|f| f.contains("Missing")), "{found:?}");
        assert_eq!(BTreeSet::from([user_mark(&db)]), marks(&db));
        api::results(&db, "reindex identities");
        wait_for_jobs(&rt, &db);
        assert_eq!(BTreeSet::new(), marks(&db));
        assert!(entries(&db, USER_INDEX).contains(&entry));
        assert_eq!(Vec::<String>::new(), verified_live(&db));
        db.close().unwrap();
    });
}

#[test]
fn a_crash_at_each_stage_of_a_release_leaves_the_mark_and_old_index_or_neither() {
    let files = {
        let sim = with_damaged_user_index(8, false);
        sim.files()
    };
    let times = Arc::new(Mutex::new(Vec::new()));
    let seen = times.clone();
    let mut sim = Sim::with_files(8, Config::default(), files.clone());
    api::run(&mut sim, move |rt| {
        let db = open_app(rt.clone());
        quarantine_user_index(&db);
        let clock = rt.clone();
        db.on_reindex_phase(Arc::new(move |phase| {
            seen.lock().unwrap().push((phase.to_owned(), clock.now()));
        }));
        api::results(&db, USER_REINDEX);
        wait_for_jobs(&rt, &db);
        db.close().unwrap();
    });
    let times = times.lock().unwrap().clone();
    assert!(times.iter().any(|(p, _)| p == "reclaim"), "{times:?}");
    crash::silence_fatal_panics();
    for (phase, at) in times {
        let mut sim = Sim::with_files(8, Config::default(), files.clone());
        sim.crash_at(at);
        let ran = sim.run(|rt| {
            let db = open_app(rt.clone());
            quarantine_user_index(&db);
            db.on_reindex_phase(Arc::new(|_| {}));
            api::results(&db, USER_REINDEX);
            wait_for_jobs(&rt, &db);
            db.close().unwrap();
        });
        assert_eq!(Outcome::Crashed, ran, "phase {phase}");
        api::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            if marks(&db).is_empty() {
                assert!(
                    ["sweep", "reclaim"].contains(&phase.as_str()),
                    "the swap had not run at {phase}"
                );
                assert_every_row_found(&db, 1..=2, &phase);
            } else {
                assert_eq!(BTreeSet::from([user_mark(&db)]), marks(&db), "{phase}");
                api::results(&db, USER_REINDEX);
                wait_for_jobs(&rt, &db);
                assert_eq!(BTreeSet::new(), marks(&db), "{phase}");
                assert_every_row_found(&db, 1..=2, &phase);
            }
            db.close().unwrap();
        });
        api::run(&mut sim, |rt| {
            let db = open_app(rt);
            assert_eq!(BTreeSet::new(), marks(&db), "{phase}");
            assert_eq!(Vec::<String>::new(), verified(&db), "{phase}");
            db.close().unwrap();
        });
    }
}

#[test]
fn a_page_of_the_old_index_damaged_during_the_build_does_not_stop_it() {
    api::once(9, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 300);
        let page = *user_index_pages(&db).last().unwrap();
        let hit = db.clone();
        db.on_reindex_phase(Arc::new(move |phase| {
            if phase == "build" {
                hit.damage_page(page, Damage::Checksum);
            }
        }));
        api::results(&db, USER_REINDEX);
        wait_for_jobs(&rt, &db);
        assert_eq!(Vec::<String>::new(), findings(&db));
        assert_every_row_found(&db, (1..=300).step_by(20), "after the build");
        assert_eq!(Vec::<String>::new(), verified(&db));
        db.close().unwrap();
    });
}

/// Commit a second Google row with `gid` by a raw write to the heap, past the unique index.
fn raw_duplicate(db: &Database<SimRuntime>, gid: &str) {
    let w = db.write_path();
    let heap = w.heap_named("app::identities", "Google").unwrap();
    let txns = w.txns();
    let x = txns.begin(w.pager()).unwrap();
    let row = vec![Value::I64(7777), Value::I64(1), Value::Text(gid.to_owned())];
    heap.insert(w.pager(), x.0, &row).unwrap();
    txns.commit(w.pager(), x).unwrap();
}

#[test]
fn a_unique_duplicate_fails_the_rebuild_keeping_the_old_index_mark_and_no_new_page() {
    api::once(10, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 5);
        raw_duplicate(&db, "g1");
        let w = db.write_path();
        let owner = w
            .index_named("app::identities", GID_INDEX)
            .unwrap()
            .tree()
            .owner();
        w.pager().quarantine(Quarantine::Index(owner)).unwrap();
        let before = entries(&db, GID_INDEX);
        let allocated = w.pager().allocated().unwrap();
        api::results(&db, "reindex identities index (gid) in Google");
        wait_for_jobs(&rt, &db);
        assert_eq!(before, entries(&db, GID_INDEX));
        assert_eq!(BTreeSet::from([Quarantine::Index(owner)]), marks(&db));
        assert!(
            allocated.len() + 2 >= w.pager().allocated().unwrap().len(),
            "the new pages are freed"
        );
        let got = findings(&db);
        assert_eq!(1, got.len(), "{got:?}");
        for part in ["Structure", "app::identities", GID_INDEX, "duplicate"] {
            assert!(got[0].contains(part), "{part}: {got:?}");
        }
        db.close().unwrap();
    });
}

#[test]
fn a_deleted_row_and_a_new_one_of_one_key_are_no_duplicate() {
    api::once(11, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 3);
        api::results(
            &db,
            "delete g in identities.Google where key g == identities.key(11)",
        );
        api::results(
            &db,
            r#"insert identities.Google { id: identities.key(5000), user: users.key(1), gid: "g1" }"#,
        );
        api::results(&db, "reindex identities");
        wait_for_jobs(&rt, &db);
        assert_eq!(Vec::<String>::new(), findings(&db));
        assert_eq!(Vec::<String>::new(), verified_live(&db));
        db.close().unwrap();
    });
}

/// Returns whether a write of a row and of a row's name went through during the rebuild.
fn writes_during(seed: u64, how: &str) -> bool {
    api::once(seed, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 5);
        let done = Arc::new(AtomicBool::new(false));
        let thread = Arc::new(Mutex::new(None));
        let (writer, finished, clock, held) =
            (db.clone(), done.clone(), rt.clone(), thread.clone());
        let during = Arc::new(Mutex::new(None));
        let saw = during.clone();
        db.on_reindex_phase(Arc::new(move |phase| {
            if phase == "build" {
                let (w, f) = (writer.clone(), finished.clone());
                *held.lock().unwrap() = Some(clock.spawn("writer", move || {
                    api::results(
                        &w,
                        "delete g in identities.Google where key g == identities.key(21)",
                    );
                    f.store(true, Ordering::Release);
                }));
                for _ in 0..2000 {
                    clock.yield_now();
                }
                *saw.lock().unwrap() = Some(finished.load(Ordering::Acquire));
            }
        }));
        api::results(&db, &format!("{USER_REINDEX} {how}"));
        wait_for_jobs(&rt, &db);
        let joined = thread.lock().unwrap().take().unwrap();
        rt.join(joined);
        assert!(
            done.load(Ordering::Acquire),
            "the write ends after the rebuild"
        );
        assert_eq!(Vec::<String>::new(), verified_live(&db));
        db.close().unwrap();
        during.lock().unwrap().unwrap()
    })
}

#[test]
fn offline_blocks_writes_until_the_rebuild_ends() {
    assert!(!writes_during(12, "offline"));
}

#[test]
fn online_does_not_block_writes() {
    assert!(writes_during(13, "online"));
}

#[test]
fn reindex_of_a_relation_rebuilds_its_key_index_too() {
    api::once(14, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 5);
        let w = db.write_path();
        let key = w.index_named("app::identities", "key id").unwrap();
        let mark = Quarantine::Index(key.tree().owner());
        w.pager().quarantine(mark).unwrap();
        api::results(&db, "reindex identities");
        wait_for_jobs(&rt, &db);
        assert_eq!(BTreeSet::new(), marks(&db));
        insert_user(&db, 99);
        assert_eq!(Vec::<String>::new(), verified_live(&db));
        db.close().unwrap();
    });
}

#[test]
fn a_clause_with_no_in_names_the_one_index_of_those_fields() {
    api::once(15, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 3);
        api::results(&db, "reindex identities index (user)");
        wait_for_jobs(&rt, &db);
        assert_eq!(Vec::<String>::new(), findings(&db));
        assert_every_row_found(&db, 1..=3, "rebuilt by fields");
        db.close().unwrap();
    });
}

const AMBIGUOUS: &str = r"module app
add module app
add type T = { id: Key ts, a: I64 }
add relation ts : T key id generate sequence
  index (a)
  unique (a)
";

#[test]
fn a_clause_with_no_in_over_several_indexes_is_invalid_and_names_in() {
    api::once(16, |rt| {
        let db = Database::open_in(
            rt,
            "ambiguous.adt",
            &Log::new([AMBIGUOUS]),
            &api::options(2),
        )
        .expect("the database opens");
        let e = api::failure(&db, "reindex ts index (a)");
        assert!(api::is_invalid(&e), "{e:?}");
        assert!(format!("{e:?}").contains("`in`"), "{e:?}");
        db.close().unwrap();
    });
}

#[test]
fn a_quarantined_heap_fails_reindex_and_a_database_mark_stays() {
    api::once(17, |rt| {
        let db = open_app(rt);
        seed_rows(&db, 3);
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Apple").unwrap();
        let mark = Quarantine::Heap(heap.id().0);
        w.pager().quarantine(mark).unwrap();
        let e = api::failure(&db, "reindex identities");
        assert!(format!("{e:?}").contains("Quarantined"), "{e:?}");
        assert_eq!(
            Vec::<String>::new(),
            api::printed(api::result(&db, "from j in sys::jobs select j.id"))
        );
        w.pager().release(mark).unwrap();
        w.pager().quarantine(Quarantine::Database).unwrap();
        let e = api::failure(&db, "reindex identities");
        assert!(format!("{e:?}").contains("Quarantined"), "{e:?}");
        assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
    });
}

#[test]
fn a_close_stops_a_running_job_before_its_swap_and_the_old_index_stays() {
    let mut sim = Sim::new(18, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt.clone());
        seed_rows(&db, 5);
        let phases = Arc::new(Mutex::new(Vec::new()));
        let (seen, clock) = (phases.clone(), rt.clone());
        let started = Arc::new(AtomicBool::new(false));
        let scanning = started.clone();
        db.on_reindex_phase(Arc::new(move |phase| {
            seen.lock().unwrap().push(phase.to_owned());
            if phase == "scan" {
                scanning.store(true, Ordering::Release);
                for _ in 0..2000 {
                    clock.yield_now();
                }
            }
        }));
        api::results(&db, USER_REINDEX);
        while !started.load(Ordering::Acquire) {
            rt.yield_now();
        }
        db.close().unwrap();
        let phases = phases.lock().unwrap().clone();
        assert!(!phases.iter().any(|p| p == "swap"), "{phases:?}");
    });
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_every_row_found(&db, 1..=5, "after the close");
        assert_eq!(Vec::<String>::new(), verified(&db));
        db.close().unwrap();
    });
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("reindex")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn real_options() -> OpenOptions {
    api::options(2)
}

#[test]
fn a_real_file_restores_a_damaged_index_and_keeps_it_across_a_reopen() {
    let path = scratch("damaged").join("app.adt");
    let path = path.to_str().unwrap().to_owned();
    let log = Log::new([APP]);
    let db = Database::open(&path, &log, &real_options()).unwrap();
    let run = |db: &Database, text: &str| {
        db.query_text(text, &[], &adtb::CallOptions::default())
            .unwrap()
    };
    for i in 1..=5 {
        run(
            &db,
            &format!(
                "insert users {{ id: users.key({i}), name: \"n{i}\" }}\ninsert identities.Google {{ id: identities.key({i}), user: users.key({i}), gid: \"g{i}\" }}\ninsert identities.Apple {{ id: identities.key({}), user: users.key({i}), sub: \"a{i}\" }}",
                i + 1000
            ),
        );
    }
    let (page, page_size) = {
        let w = db.write_path();
        let index = w.index_named("app::identities", USER_INDEX).unwrap();
        (
            *index.tree().pages(w.pager()).unwrap().last().unwrap(),
            w.pager().page_size(),
        )
    };
    db.close().unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[page.0 as usize * page_size + 300] ^= 0x10;
    std::fs::write(&path, bytes).unwrap();
    let db = Database::open(&path, &log, &real_options()).unwrap();
    let probe = "from g in identities.Google where g.user == users.key(5) select g.gid";
    assert!(
        db.query_text(probe, &[], &adtb::CallOptions::default())
            .is_err()
    );
    assert_eq!(1, db.write_path().pager().quarantined().len());
    run(&db, USER_REINDEX);
    let idle = |db: &Database| {
        let rows = run(db, "from j in sys::jobs select j.id").remove(0);
        rows.count() == 0
    };
    while !idle(&db) {
        std::thread::yield_now();
    }
    assert!(db.write_path().pager().quarantined().is_empty());
    run(
        &db,
        r#"insert identities.Google { id: identities.key(100001), user: users.key(5), gid: "gnew" }"#,
    );
    db.close().unwrap();
    let db = Database::open(&path, &log, &real_options()).unwrap();
    assert!(db.write_path().pager().quarantined().is_empty());
    run(&db, probe);
    db.close().unwrap();
}
