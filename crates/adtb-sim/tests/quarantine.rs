use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::btree::{self, BTree};
use adtb::exec::Val;
use adtb::key::{Entry, Tid, Value};
use adtb::runtime::{Clock, Io, Op, Runtime, Threads};
use adtb::storage::{Damage, Options, PageError, Pager, Pid, Quarantine, Stopped};
use adtb::{
    CallOptions, Called, Database, Error, FatalReason, Log, OpenOptions, Registration,
    TxnCallOptions, TxnOptions,
};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

use adtb_simkit::{api, harness};
use adtb_sys::os::{Backend, OsRuntime};

use harness::{NAME, PAGE, finished};

fn options() -> Options {
    Options {
        segment_size: 64 * 1024,
        max_pages: 1024,
        ..harness::options(16)
    }
}

fn create(sim: &mut Sim) {
    finished(sim, |rt| {
        Pager::create(rt, &options()).unwrap();
    });
}

fn with_pager<T>(sim: &mut Sim, body: impl FnOnce(&Pager<SimRuntime>) -> T) -> T {
    let mut out = None;
    finished(sim, |rt| {
        let (p, _) = Pager::open(rt, &options()).unwrap();
        out = Some(body(&p));
    });
    out.unwrap()
}

fn with_named_pager<T>(sim: &mut Sim, body: impl FnOnce(&Pager<SimRuntime>) -> T) -> T {
    let mut out = None;
    finished(sim, |rt| {
        let options = Options {
            page_size: API_PAGE as u32,
            threads: 2,
            ..Options::new(FILE)
        };
        let (p, _) = Pager::open(rt, &options).unwrap();
        out = Some(body(&p));
    });
    out.unwrap()
}

#[test]
fn a_new_database_quarantines_nothing() {
    let mut sim = Sim::new(1, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| {
        assert!(p.quarantined().is_empty());
        assert!(!p.is_quarantined(Quarantine::Heap(1)));
    });
}

#[test]
fn marks_survive_a_restart_until_released() {
    let mut sim = Sim::new(2, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| {
        p.quarantine(Quarantine::Heap(7)).unwrap();
        p.quarantine(Quarantine::Index(3)).unwrap();
        p.quarantine(Quarantine::Heap(7)).unwrap();
        assert!(p.is_quarantined(Quarantine::Heap(7)));
        assert!(
            !p.is_quarantined(Quarantine::Heap(3)),
            "heap and index ids are apart"
        );
    });
    with_pager(&mut sim, |p| {
        assert_eq!(
            BTreeSet::from([Quarantine::Heap(7), Quarantine::Index(3)]),
            p.quarantined()
        );
        p.release(Quarantine::Heap(7)).unwrap();
    });
    with_pager(&mut sim, |p| {
        assert_eq!(BTreeSet::from([Quarantine::Index(3)]), p.quarantined());
        assert!(p.verify().findings.is_empty());
    });
}

#[test]
fn a_mark_is_durable_once_quarantine_returns() {
    for seed in 0..8 {
        let mut sim = Sim::new(seed, Config::default());
        create(&mut sim);
        sim.crash_at(sim.now() + Duration::from_secs(1));
        let outcome = sim.run(|rt| {
            let (p, _) = Pager::open(rt.clone(), &options()).unwrap();
            p.quarantine(Quarantine::Heap(40 + seed as u32)).unwrap();
            let mut io = rt.io();
            io.submit(Op::Timer {
                at: rt.now() + Duration::from_secs(10),
            });
            loop {
                rt.park();
            }
        });
        assert_eq!(Outcome::Crashed, outcome);
        with_pager(&mut sim, |p| {
            assert!(
                p.is_quarantined(Quarantine::Heap(40 + seed as u32)),
                "seed {seed}"
            );
        });
    }
}

#[test]
fn a_database_mark_covers_every_object() {
    let mut sim = Sim::new(3, Config::default());
    create(&mut sim);
    with_pager(&mut sim, |p| p.quarantine(Quarantine::Database).unwrap());
    with_pager(&mut sim, |p| {
        assert!(p.is_quarantined(Quarantine::Heap(9)));
        assert!(p.is_quarantined(Quarantine::Index(9)));
    });
}

#[test]
fn a_damaged_mark_page_quarantines_the_database() {
    let mut sim = Sim::new(4, Config::default());
    create(&mut sim);
    let page = with_pager(&mut sim, |p| {
        p.quarantine(Quarantine::Index(5)).unwrap();
        p.quarantine_page()
    });
    with_pager(&mut sim, |_| ());
    let mut files = sim.files();
    files.get_mut(NAME).unwrap()[page.0 as usize * PAGE + 200] ^= 0x10;
    let mut sim = Sim::with_files(4, Config::default(), files);
    with_pager(&mut sim, |p| {
        assert!(p.is_quarantined(Quarantine::Database));
        assert!(p.quarantine(Quarantine::Heap(1)).is_err());
    });
}

const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
  | Apple     { sub: Text }
add relation identities : Identity key id generate sequence
  index (user) in (Google | Apple)
  index (gid) in Google
add type Note = { id: Key notes, about: Key users }
add relation notes : Note key id generate sequence
  index (about)
add type Label = { id: Key tags, owner: Key users }
add relation tags : Label key id generate sequence
  index (owner)
add constraint no_apple_tags =
  from t in tags
  from a in identities.Apple
  where a.user == t.owner
  select t
";

const FILE: &str = "quarantine.adt";
const API_PAGE: usize = 8192;
const SIDES: &str = "from i in identities.(EmailPass | Google) select i.id";
const ALL: &str = "from i in identities select i.id";
const BY_USER: &str = "from i in identities.Google where i.user == users.key(3) select i.gid";
const ROWS: &str = "from i in identities select i";
const USER_INDEX: &str = "index (user) in (Google | Apple)";

fn reads() -> Registration {
    Registration::new(
        "reads",
        "import app
query by_key (k: Key identities) -> Set (Key identities) =
  from i in identities where i.id == k select i.id
query by_gid (g: Text) -> Set (Key identities) =
  from i in identities.Google where i.gid == g select i.id
query google_of (u: Key users) -> Set Text =
  from i in identities.Google where i.user == u select i.gid
fn every_id () -> Set (Key identities) = from i in identities select i.id
query through_fn () -> Set (Key identities) = every_id()
",
    )
}

fn open_app(rt: SimRuntime) -> Database<SimRuntime> {
    let options = OpenOptions {
        registrations: vec![reads()],
        ..api::options(2)
    };
    Database::open_in(rt, FILE, &Log::new([APP]), &options).expect("the database opens")
}

fn seed(db: &Database<SimRuntime>) {
    for i in 1..=6 {
        api::results(
            db,
            &format!(
                r#"insert users {{ id: users.key({i}), name: "n{i}" }}
                   insert identities.EmailPass {{ id: identities.key({}), user: users.key({i}), email: "e{i}" }}
                   insert identities.Google {{ id: identities.key({}), user: users.key({i}), gid: "g{i}" }}
                   insert identities.Apple {{ id: identities.key({}), user: users.key({i}), sub: "a{i}" }}"#,
                i * 10,
                i * 10 + 1,
                i * 10 + 2,
            ),
        );
    }
}

fn heap_page(db: &Database<SimRuntime>, relation: &str, ctor: &str) -> Pid {
    let w = db.write_path();
    w.heap_named(relation, ctor)
        .unwrap()
        .pages(w.pager())
        .unwrap()[0]
}

fn index_page(db: &Database<SimRuntime>, relation: &str, clause: &str) -> Pid {
    let w = db.write_path();
    let index = w.index_named(relation, clause).unwrap();
    *index.tree().pages(w.pager()).unwrap().last().unwrap()
}

/// Returns a simulator over a seeded file with page `pick` picks flipped on disk.
fn flipped(seed_no: u64, pick: impl FnOnce(&Database<SimRuntime>) -> Pid) -> Sim {
    let mut sim = Sim::new(seed_no, Config::default());
    let page = api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let page = pick(&db);
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    files.get_mut(FILE).unwrap()[page.0 as usize * API_PAGE + 300] ^= 0x10;
    Sim::with_files(seed_no, Config::default(), files)
}

fn apple(db: &Database<SimRuntime>) -> Pid {
    heap_page(db, "app::identities", "Apple")
}

fn user_index(db: &Database<SimRuntime>) -> Pid {
    index_page(db, "app::identities", USER_INDEX)
}

fn quarantined(e: &Error) -> &str {
    match e {
        Error::Fatal {
            reason: FatalReason::Quarantined { what },
        } => what,
        other => panic!("not Quarantined: {other:?}"),
    }
}

fn failed_on(db: &Database<SimRuntime>, text: &str) -> String {
    quarantined(&api::failure(db, text)).to_owned()
}

fn marks<R: Runtime>(db: &Database<R>) -> BTreeSet<Quarantine> {
    db.write_path().pager().quarantined()
}

fn apple_mark(db: &Database<SimRuntime>) -> Quarantine {
    let w = db.write_path();
    Quarantine::Heap(w.heap_named("app::identities", "Apple").unwrap().id().0)
}

fn user_index_mark(db: &Database<SimRuntime>) -> Quarantine {
    let w = db.write_path();
    Quarantine::Index(
        w.index_named("app::identities", USER_INDEX)
            .unwrap()
            .tree()
            .owner(),
    )
}

fn call(db: &Database<SimRuntime>, query: &str, args: &[Val]) -> Result<Vec<String>, Error> {
    let bound = db.registered("reads").unwrap();
    let ordinal = bound.ordinal(query).unwrap();
    match bound.call(ordinal, args, &CallOptions::default())? {
        Called::Rows(rows) => Ok(api::sorted(rows)),
        other => panic!("{query} returns rows: {other:?}"),
    }
}

#[test]
fn a_damaged_constructor_page_stops_only_the_calls_that_read_it() {
    let mut sim = flipped(10, apple);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(12, api::query(&db, SIDES).len());
        assert!(
            marks(&db).is_empty(),
            "a read that skips the page finds nothing"
        );
        assert_eq!("app::identities.Apple", failed_on(&db, ROWS));
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
        assert_eq!(12, api::query(&db, SIDES).len());
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
        assert_eq!(6, api::query(&db, "from u in users select u.id").len());
        db.close().unwrap();
    });
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(
            BTreeSet::from([apple_mark(&db)]),
            marks(&db),
            "across restart"
        );
        assert_eq!(12, api::query(&db, SIDES).len());
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
    });
}

#[test]
fn a_crash_before_the_mark_finds_the_damage_again() {
    let mut sim = flipped(11, apple);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(12, api::query(&db, SIDES).len());
    });
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert!(marks(&db).is_empty());
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
    });
}

#[test]
fn the_mark_of_a_discovering_call_survives_a_crash() {
    let mut sim = flipped(12, apple);
    sim.crash_at(sim.now() + Duration::from_secs(20));
    let outcome = sim.run(|rt| {
        let db = open_app(rt.clone());
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
        let mut io = rt.io();
        io.submit(Op::Timer {
            at: rt.now() + Duration::from_secs(60),
        });
        loop {
            rt.park();
        }
    });
    assert_eq!(Outcome::Crashed, outcome);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_damaged_index_page_costs_only_plans_and_stops_writes_it_covers() {
    let mut sim = flipped(13, user_index);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let what = failed_on(&db, BY_USER);
        assert!(what.contains(USER_INDEX), "{what}");
        assert_eq!(BTreeSet::from([user_index_mark(&db)]), marks(&db));
        assert_eq!(1, api::query(&db, BY_USER).len(), "the read runs");
        let insert = |c: &str, n: u32| {
            format!(
                r"insert identities.{c} {{ id: identities.key({n}), user: users.key(1), {} }}",
                match c {
                    "Google" => r#"gid: "g-2""#,
                    _ => r#"email: "x""#,
                }
            )
        };
        let what = failed_on(&db, &insert("Google", 900));
        assert!(what.contains(USER_INDEX), "{what}");
        api::results(&db, &insert("EmailPass", 901));
        db.close().unwrap();
    });
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(1, api::query(&db, BY_USER).len(), "across restart");
        let what = failed_on(
            &db,
            r#"insert identities.Google { id: identities.key(902), user: users.key(1), gid: "g-3" }"#,
        );
        assert!(what.contains(USER_INDEX), "{what}");
    });
}

#[test]
fn a_registered_call_fails_whichever_arm_its_arguments_reach() {
    let mut sim = flipped(14, apple);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let google = Val::Int(11);
        assert_eq!(
            1,
            call(&db, "by_key", std::slice::from_ref(&google))
                .unwrap()
                .len()
        );
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
        let got = call(&db, "by_key", std::slice::from_ref(&google)).unwrap_err();
        assert_eq!("app::identities.Apple", quarantined(&got));
        let got = call(&db, "through_fn", &[]).unwrap_err();
        assert_eq!(
            "app::identities.Apple",
            quarantined(&got),
            "through a function"
        );
        assert_eq!(1, call(&db, "google_of", &[Val::Int(1)]).unwrap().len());
    });
}

#[test]
fn a_plan_over_the_index_is_replanned_with_the_same_rows() {
    let mut sim = Sim::new(15, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let gid = [Val::text("g3")];
        let before = call(&db, "by_gid", &gid).unwrap();
        assert_eq!(1, before.len());
        let shown = |db: &Database<SimRuntime>| {
            api::last(
                db,
                r#"explain from i in identities.Google where i.gid == "g3" select i.id"#,
            )
            .join("\n")
        };
        assert!(shown(&db).contains("Probe"), "{}", shown(&db));
        db.damage_page(
            index_page(&db, "app::identities", "index (gid) in Google"),
            Damage::Checksum,
        );
        let what = quarantined(&call(&db, "by_gid", &gid).unwrap_err()).to_owned();
        assert!(what.contains("(gid)"), "{what}");
        assert_eq!(before, call(&db, "by_gid", &gid).unwrap());
        assert!(!shown(&db).contains("Probe"), "{}", shown(&db));
    });
}

#[test]
fn a_key_lookup_runs_from_the_heaps_while_its_index_is_quarantined() {
    let mut sim = Sim::new(16, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let text = "from n in identities where n.id == identities.key(11) select n.user";
        let deref = "from n in [identities.key(11)] select n.id";
        let before = (api::query(&db, text), api::query(&db, deref));
        let page = index_page(&db, "app::identities", "key id");
        db.damage_page(page, Damage::Checksum);
        let what = failed_on(&db, text);
        assert!(what.contains("key id"), "{what}");
        assert_eq!(before, (api::query(&db, text), api::query(&db, deref)));
    });
}

#[test]
fn an_io_failure_sets_no_mark_and_stays_a_storage_failure() {
    let mut sim = Sim::new(17, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.damage_page(apple(&db), Damage::Unreadable);
        assert!(
            matches!(
                api::failure(&db, ALL),
                Error::Fatal {
                    reason: FatalReason::Storage { .. }
                }
            ),
            "{:?}",
            api::failure(&db, ALL)
        );
        assert!(marks(&db).is_empty());
        let found = api::printed(db.verify(Some("identities")).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
        assert!(marks(&db).is_empty(), "verify marks no I/O failure");
    });
}

#[test]
fn concurrent_readers_of_one_damaged_page_set_one_mark() {
    for seed_no in adtb_simkit::seeds::sweep(20..22) {
        let mut sim = Sim::new(seed_no, Config::default());
        api::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            seed(&db);
            db.damage_page(apple(&db), Damage::Checksum);
            let before = db.write_path().pager().durable();
            let readers: Vec<_> = (0..4)
                .map(|i| {
                    let db = db.clone();
                    rt.spawn(&format!("reader {i}"), move || {
                        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
                    })
                })
                .collect();
            for r in readers {
                rt.join(r);
            }
            assert_eq!(
                BTreeSet::from([apple_mark(&db)]),
                marks(&db),
                "seed {seed_no}"
            );
            assert!(db.write_path().pager().durable() > before, "seed {seed_no}");
            let once = db.write_path().pager().durable();
            db.write_path().pager().quarantine(apple_mark(&db)).unwrap();
            assert_eq!(once, db.write_path().pager().durable(), "seed {seed_no}");
        });
    }
}

#[test]
fn a_bad_catalog_page_marks_the_database() {
    let mut sim = Sim::new(18, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.damage_page(heap_page(&db, "catalog::roles", "Role"), Damage::Checksum);
        assert_eq!(
            "the database",
            failed_on(&db, "from r in catalog::roles select r.name")
        );
        assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
        assert_eq!(
            "the database",
            failed_on(&db, "from u in users select u.id")
        );
        assert_eq!(
            "the database",
            quarantined(&db.begin(&TxnOptions::default()).err().unwrap())
        );
        db.close().unwrap();
    });
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(
            "the database",
            failed_on(&db, "from u in users select u.id")
        );
        assert_eq!(
            "the database",
            failed_on(&db, r#"insert users { id: users.key(99), name: "x" }"#)
        );
        db.verify(None).unwrap();
    });
}

#[test]
fn a_file_with_a_damaged_catalog_page_opens_for_verify() {
    let mut sim = flipped(19, |db| heap_page(db, "catalog::roles", "Role"));
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(
            "the database",
            failed_on(&db, "from u in users select u.id")
        );
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
    });
}

#[test]
fn a_file_with_a_damaged_catalog_root_opens_and_keeps_the_mark() {
    let mut sim = flipped(19, |db| db.write_path().pager().catalog_root());
    api::run(&mut sim, |rt| {
        let got = Database::open_in(rt, FILE, &Log::new([APP]), &api::options(2));
        got.unwrap().close().unwrap();
    });
    with_named_pager(&mut sim, |p| {
        assert_eq!(BTreeSet::from([Quarantine::Database]), p.quarantined());
    });
}

#[test]
fn a_damaged_commit_log_page_marks_the_database_and_verify_runs() {
    let mut sim = Sim::new(30, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let clog = db.write_path().txns().commit_log();
        let page = *clog.status_pages().values().next_back().unwrap();
        db.damage_page(page, Damage::Checksum);
        let got = api::failure(&db, r#"insert users { id: users.key(99), name: "x" }"#);
        assert_eq!("the database", quarantined(&got));
        assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
        assert_eq!(
            "the database",
            failed_on(&db, "from u in users select u.id")
        );
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
    });
}

#[test]
fn a_reference_check_reads_the_target_and_a_delete_reads_the_referrers() {
    let mut sim = flipped(31, |db| heap_page(db, "app::users", "User"));
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let what = failed_on(&db, "from u in users select u");
        assert_eq!("app::users.User", what);
        assert_eq!(
            what,
            failed_on(
                &db,
                "insert notes { id: notes.key(1), about: users.key(1) }"
            ),
            "the insert names only notes"
        );
        assert_eq!(1, api::query(&db, "from t in [1] select t").len());
    });
    let mut sim = flipped(38, |db| {
        api::results(db, "insert notes { id: notes.key(1), about: users.key(1) }");
        heap_page(db, "app::notes", "Note")
    });
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let what = failed_on(&db, "from n in notes select n");
        assert_eq!("app::notes.Note", what);
        api::results(&db, r#"insert users { id: users.key(50), name: "lone" }"#);
        assert_eq!(
            what,
            failed_on(&db, "delete u in users where key u == users.key(50)"),
        );
    });
}

#[test]
fn a_constraint_check_reads_what_its_query_reads() {
    let mut sim = flipped(39, apple);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!("app::identities.Apple", failed_on(&db, ROWS));
        assert_eq!(
            "app::identities.Apple",
            failed_on(&db, "insert tags { id: tags.key(1), owner: users.key(1) }")
        );
        api::results(
            &db,
            "insert notes { id: notes.key(1), about: users.key(1) }",
        );
    });
}

#[test]
fn a_write_to_a_sum_reads_its_one_key_over_every_constructor() {
    let mut sim = flipped(32, apple);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
        assert_eq!(
            "app::identities.Apple",
            failed_on(
                &db,
                r#"insert identities.Google { id: identities.key(800), user: users.key(1), gid: "x" }"#
            )
        );
    });
}

#[test]
fn verify_findings_mark_the_index_they_name() {
    let mut sim = Sim::new(33, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let w = db.write_path();
        let ix = w.index_named("app::identities", USER_INDEX).unwrap();
        let gone = ix.probe(w.pager(), &[Value::I64(5)]).unwrap().remove(0);
        assert!(ix.delete(w.pager(), &gone).unwrap());
        let found = api::printed(db.verify(Some("identities")).unwrap());
        assert!(found[0].starts_with("Missing"), "{found:?}");
        assert_eq!(BTreeSet::from([user_index_mark(&db)]), marks(&db));
        assert_eq!(1, api::query(&db, BY_USER).len());
    });
    let mut sim = Sim::new(34, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Google").unwrap();
        let page = heap.pages(w.pager()).unwrap()[0];
        let ix = w.index_named("app::identities", USER_INDEX).unwrap();
        let ctor = *ix.ctors().iter().next().unwrap();
        let orphan = Entry {
            key: vec![Value::I64(1000)],
            ctor,
            tid: Tid::new(page.0, 400).unwrap(),
        };
        assert!(ix.insert(w.pager(), &orphan).unwrap());
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Orphan")), "{found:?}");
        assert_eq!(BTreeSet::from([user_index_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_verify_checksum_finding_marks_what_holds_the_page() {
    let mut sim = flipped(35, apple);
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
        assert_eq!(12, api::query(&db, SIDES).len());
    });
}

#[test]
fn verify_of_a_marked_database_still_runs() {
    let mut sim = Sim::new(36, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.write_path()
            .pager()
            .quarantine(Quarantine::Database)
            .unwrap();
        db.close().unwrap();
    });
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!("the database", failed_on(&db, ALL));
        let rows = api::printed(db.verify(Some("users")).unwrap());
        assert_eq!(1, rows.len(), "{rows:?}");
        assert!(rows[0].contains("quarantined whole"), "{rows:?}");
        assert_eq!("the database", failed_on(&db, "vacuum users"));
        assert_eq!(
            "the database",
            failed_on(&db, "explain from u in users select u.id")
        );
    });
}

#[test]
fn a_failed_mark_does_not_hide_the_damage() {
    let mut sim = Sim::new(37, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let pager = db.write_path().pager();
        db.damage_page(pager.quarantine_page(), Damage::Checksum);
        db.damage_page(apple(&db), Damage::Checksum);
        let what = failed_on(&db, ALL);
        assert!(what.starts_with("app::identities.Apple"), "{what}");
        assert!(what.contains("only in memory"), "{what}");
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
    });
}

#[test]
fn a_mark_fails_a_call_of_an_open_transaction_and_leaves_it_open() {
    let mut sim = Sim::new(40, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        assert_eq!(18, api::in_txn(&mut t, ALL).len());
        db.write_path().pager().quarantine(apple_mark(&db)).unwrap();
        let got = t.query_text(ALL, &[], &TxnCallOptions::default());
        assert_eq!("app::identities.Apple", quarantined(&got.unwrap_err()));
        assert_eq!(12, api::in_txn(&mut t, SIDES).len());
        t.commit(None).unwrap();
    });
}

#[test]
fn a_damaged_large_value_page_marks_the_database() {
    let mut sim = Sim::new(41, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let long = "x".repeat(40_000);
        api::results(
            &db,
            &format!(
                r#"insert users {{ id: users.key(1), name: "n" }}
                   insert identities.Apple {{ id: identities.key(1), user: users.key(1), sub: "{long}" }}"#
            ),
        );
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Apple").unwrap();
        let pages = heap.large().owned_pages(w.pager()).unwrap();
        db.damage_page(*pages.last().unwrap(), Damage::Checksum);
        assert_eq!("the database", failed_on(&db, ROWS));
    });
}

/// Run `body` over a seeded database with `mark` of it set and no page damaged, so that only the
/// comparison of a call to the marks can fail the call.
fn marked_undamaged(
    seed_no: u64,
    mark: impl FnOnce(&Database<SimRuntime>) -> Quarantine,
    body: impl FnOnce(&Database<SimRuntime>),
) {
    let mut sim = Sim::new(seed_no, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.write_path().pager().quarantine(mark(&db)).unwrap();
        body(&db);
    });
}

#[test]
fn a_mark_fails_a_call_before_it_reads_anything() {
    marked_undamaged(42, apple_mark, |db| {
        let apple = "app::identities.Apple";
        assert_eq!(apple, failed_on(db, ALL));
        let got = call(db, "by_key", &[Val::Int(11)]).unwrap_err();
        assert_eq!(
            apple,
            quarantined(&got),
            "arguments that reach no Apple row"
        );
        let got = call(db, "through_fn", &[]).unwrap_err();
        assert_eq!(apple, quarantined(&got), "through a function");
        assert_eq!(
            apple,
            failed_on(db, "insert tags { id: tags.key(1), owner: users.key(1) }"),
            "a constraint reads it"
        );
        assert_eq!(
            apple,
            failed_on(
                db,
                r#"insert identities.Google { id: identities.key(800), user: users.key(1), gid: "x" }"#
            ),
            "the key spans the sum"
        );
        assert_eq!(12, api::query(db, SIDES).len());
    });
}

#[test]
fn a_reference_mark_fails_a_write_that_names_only_the_referrer() {
    marked_undamaged(
        43,
        |db| {
            let w = db.write_path();
            Quarantine::Heap(w.heap_named("app::users", "User").unwrap().id().0)
        },
        |db| {
            assert_eq!(
                "app::users.User",
                failed_on(db, "insert notes { id: notes.key(1), about: users.key(1) }")
            );
        },
    );
}

#[test]
fn an_index_mark_fails_a_write_it_covers_with_no_page_damaged() {
    marked_undamaged(44, user_index_mark, |db| {
        let what = failed_on(
            db,
            r#"insert identities.Google { id: identities.key(800), user: users.key(1), gid: "x" }"#,
        );
        assert!(what.contains(USER_INDEX), "{what}");
        assert_eq!(1, api::query(db, BY_USER).len());
    });
}

#[test]
fn opening_a_marked_database_applies_no_migration() {
    let mut sim = Sim::new(45, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        db.write_path()
            .pager()
            .quarantine(Quarantine::Database)
            .unwrap();
        db.close().unwrap();
    });
    api::run(&mut sim, |rt| {
        let later = "module app\nadd type Z = { id: Key zs }\nadd relation zs : Z key id generate sequence\n";
        let options = OpenOptions {
            registrations: vec![reads()],
            ..api::options(2)
        };
        let db = Database::open_in(rt, FILE, &Log::new([APP, later]), &options).unwrap();
        assert_eq!("the database", failed_on(&db, "from z in zs select z"));
    });
}

fn hash_log() -> String {
    use adtb_lang::evolve::Source;
    use adtb_lang::parse::{Options as ParseOptions, parse_migration};
    let file = parse_migration(APP, ParseOptions::default()).ok().unwrap();
    let source = Source::default().migrate(&file, APP).unwrap();
    let schema = source.schema().unwrap();
    adtb::catalog::version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn seed_many(db: &Database<SimRuntime>, n: i64) {
    let mut text = String::new();
    for i in 100..100 + n {
        text.push_str(&format!(
            "insert users {{ id: users.key({i}), name: \"n{i}\" }}\n insert identities.Google {{ id: identities.key({i}), user: users.key({i}), gid: \"g{i}\" }}\n"
        ));
        if i % 50 == 0 {
            api::results(db, &text);
            text.clear();
        }
    }
    if !text.is_empty() {
        api::results(db, &text);
    }
}

fn index_pages(db: &Database<SimRuntime>) -> (Pid, Vec<Pid>) {
    let w = db.write_path();
    let index = w.index_named("app::identities", USER_INDEX).unwrap();
    let pages = index.tree().pages(w.pager()).unwrap();
    assert!(pages.len() >= 3, "{pages:?}");
    (index.tree().root(), pages)
}

fn runs(db: &Database<SimRuntime>, text: &str) {
    if let Err(e) = db.query_text(text, &[], &CallOptions::default()) {
        panic!("`{text}` runs: {e:?}");
    }
}

#[test]
fn two_finders_of_different_objects_keep_both_marks() {
    for s in adtb_simkit::seeds::sweep(0..6) {
        let mut sim = Sim::new(s, Config::default());
        api::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            let finders: Vec<_> = (0..2u32)
                .map(|i| {
                    let db = db.clone();
                    rt.spawn(&format!("marker {i}"), move || {
                        let p = db.write_path().pager();
                        p.quarantine(Quarantine::Heap(1000 + i)).unwrap();
                    })
                })
                .collect();
            for t in finders {
                rt.join(t);
            }
            assert_eq!(2, marks(&db).len(), "seed {s}: {:?}", marks(&db));
            db.close().unwrap();
        });
        api::run(&mut sim, |rt| {
            let db = open_app(rt);
            assert_eq!(2, marks(&db).len(), "seed {s}: {:?}", marks(&db));
        });
    }
}

#[test]
fn a_second_finder_of_one_object_returns_with_the_mark_durable() {
    for s in adtb_simkit::seeds::sweep(0..6) {
        let mut sim = Sim::new(s, Config::default());
        api::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            let finders: Vec<_> = (0..3u32)
                .map(|i| {
                    let db = db.clone();
                    rt.spawn(&format!("marker {i}"), move || {
                        let p = db.write_path().pager();
                        p.quarantine(Quarantine::Heap(1000)).unwrap();
                        let at = adtb::storage::lsn(&p.read(p.quarantine_page()).unwrap());
                        assert!(p.durable() > at, "seed {s} marker {i}");
                    })
                })
                .collect();
            for t in finders {
                rt.join(t);
            }
        });
    }
}

#[test]
fn two_damaged_pages_of_one_index_mark_the_index() {
    let mut sim = Sim::new(50, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        seed_many(&db, 900);
        let (_, pages) = index_pages(&db);
        db.damage_page(pages[pages.len() - 1], Damage::Checksum);
        db.damage_page(pages[pages.len() - 2], Damage::Checksum);
        db.verify(Some("identities")).unwrap();
        assert_eq!(BTreeSet::from([user_index_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_damaged_root_and_leaf_of_one_index_mark_the_index() {
    let mut sim = Sim::new(62, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        seed_many(&db, 900);
        let (root, pages) = index_pages(&db);
        let leaf = *pages.iter().rev().find(|p| **p != root).unwrap();
        db.damage_page(root, Damage::Checksum);
        db.damage_page(leaf, Damage::Checksum);
        db.verify(None).unwrap();
        assert_eq!(BTreeSet::from([user_index_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_walk_stopped_by_damage_marks_its_heap() {
    let mut sim = Sim::new(63, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Apple").unwrap();
        let owned = heap.owned_pages(w.pager()).unwrap();
        for pid in owned {
            db.damage_page(pid, Damage::Checksum);
        }
        db.verify(Some("identities")).unwrap();
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
    });
}

#[test]
fn an_io_failure_in_the_walk_marks_nothing() {
    let mut sim = Sim::new(64, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Apple").unwrap();
        let large = heap.large();
        let long = "x".repeat(40_000);
        api::results(
            &db,
            &format!(
                r#"insert identities.Apple {{ id: identities.key(700), user: users.key(1), sub: "{long}" }}"#
            ),
        );
        let unheld = *large.owned_pages(w.pager()).unwrap().last().unwrap();
        let root = heap.owned_pages(w.pager()).unwrap()[0];
        db.damage_page(root, Damage::Unreadable);
        db.damage_page(unheld, Damage::Checksum);
        db.verify(Some("users")).unwrap();
        assert!(marks(&db).is_empty(), "{:?}", marks(&db));
    });
}

#[test]
fn rot_on_a_free_page_marks_nothing() {
    let mut sim = Sim::new(51, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let p = db.write_path().pager();
        let mut new = p.reserve();
        let pid = new.page(1, adtb::storage::PageKind::Quarantine, 0).unwrap();
        p.update_with(new, &[pid], |_| {}).unwrap();
        p.free(pid).unwrap();
        db.damage_page(pid, Damage::Checksum);
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
        assert_eq!(BTreeSet::new(), marks(&db));
    });
}

#[test]
fn a_page_vacuum_frees_during_the_walk_marks_nothing() {
    let mut sim = Sim::new(52, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let p = db.write_path().pager();
        let mut new = p.reserve();
        let pid = new.page(1, adtb::storage::PageKind::Quarantine, 0).unwrap();
        p.update_with(new, &[pid], |_| {}).unwrap();
        let held = p.holders_of(pid, Some(Quarantine::Database), |_| {
            p.free(pid).unwrap();
        });
        assert_eq!(BTreeSet::new(), held);
    });
}

#[test]
fn an_allocated_page_every_walk_misses_marks_the_database() {
    let mut sim = Sim::new(53, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let p = db.write_path().pager();
        let mut new = p.reserve();
        let pid = new.page(1, adtb::storage::PageKind::Quarantine, 0).unwrap();
        p.update_with(new, &[pid], |_| {}).unwrap();
        db.damage_page(pid, Damage::Checksum);
        db.verify(None).unwrap();
        assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
    });
}

#[test]
fn rot_in_the_kept_store_is_a_finding_with_no_mark() {
    let mut sim = Sim::new(54, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.damage_kept_store();
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
        assert_eq!(BTreeSet::new(), marks(&db));
    });
}

#[test]
fn a_failed_open_marks_the_heap_of_the_damage_it_met() {
    let mut sim = flipped(57, apple);
    let later = "module app\nadd constraint nope =\n  from a in identities.Apple\n  where a.sub == \"zz\"\n  select a\n";
    api::run(&mut sim, |rt| {
        let opts = OpenOptions {
            registrations: vec![reads()],
            ..api::options(2)
        };
        let got = Database::open_in(rt, FILE, &Log::new([APP, later]), &opts);
        assert!(
            matches!(got, Err(adtb::OpenError::Corrupt { .. })),
            "{:?}",
            got.err()
        );
    });
    with_named_pager(&mut sim, |p| {
        let marks = p.quarantined();
        assert_eq!(1, marks.len(), "{marks:?}");
        assert!(
            matches!(marks.first(), Some(Quarantine::Heap(_))),
            "{marks:?}"
        );
    });
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
        api::query(&db, SIDES);
    });
}

#[test]
fn a_failed_open_that_cannot_map_the_damage_writes_nothing() {
    let later = "module app\nadd constraint nope =\n  from u in users\n  where u.name == \"zz\"\n  select u\n";
    let mut sim = Sim::new(58, Config::default());
    let page = api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let long = "x".repeat(40_000);
        api::results(
            &db,
            &format!(r#"insert users {{ id: users.key(1), name: "{long}" }}"#),
        );
        let w = db.write_path();
        let heap = w.heap_named("app::users", "User").unwrap();
        let page = *heap.large().owned_pages(w.pager()).unwrap().last().unwrap();
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    files.get_mut(FILE).unwrap()[page.0 as usize * API_PAGE + 300] ^= 0x10;
    let mut sim = Sim::with_files(58, Config::default(), files);
    api::run(&mut sim, |rt| {
        let got = Database::open_in(rt, FILE, &Log::new([APP, later]), &api::options(2));
        assert!(
            matches!(got, Err(adtb::OpenError::Corrupt { .. })),
            "{:?}",
            got.err()
        );
    });
    with_named_pager(&mut sim, |p| assert_eq!(BTreeSet::new(), p.quarantined()));
}

fn commit_log_page(seed_no: u64, pick: impl FnOnce(&adtb::mvcc::CommitLog) -> Pid) -> Sim {
    let mut sim = Sim::new(seed_no, Config::default());
    let page = api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let page = pick(&db.write_path().txns().commit_log());
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    files.get_mut(FILE).unwrap()[page.0 as usize * API_PAGE + 300] ^= 0x10;
    Sim::with_files(seed_no, Config::default(), files)
}

#[test]
fn a_file_with_a_damaged_commit_log_status_page_opens_for_verify() {
    let mut sim = commit_log_page(58, |clog| {
        *clog.status_pages().values().next_back().unwrap()
    });
    for _ in 0..2 {
        api::run(&mut sim, |rt| {
            let db = open_app(rt);
            assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
            assert_eq!(
                "the database",
                failed_on(&db, "from u in users select u.id")
            );
            assert_eq!(
                "the database",
                failed_on(&db, r#"insert users { id: users.key(99), name: "x" }"#)
            );
            assert_eq!(
                "the database",
                quarantined(&db.begin(&TxnOptions::default()).err().unwrap())
            );
            let e = call(&db, "through_fn", &[]).expect_err("fails");
            assert_eq!("the database", quarantined(&e));
            let found = api::printed(db.verify(None).unwrap());
            assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
            db.close().unwrap();
        });
    }
}

#[test]
fn a_file_with_a_damaged_commit_log_root_opens_and_keeps_the_mark() {
    let mut sim = commit_log_page(59, adtb::mvcc::CommitLog::root);
    api::run(&mut sim, |rt| {
        let got = Database::open_in(rt, FILE, &Log::new([APP]), &api::options(2));
        got.unwrap().close().unwrap();
    });
    with_named_pager(&mut sim, |p| {
        assert_eq!(BTreeSet::from([Quarantine::Database]), p.quarantined());
    });
}

#[test]
fn a_delete_reads_no_target_of_its_row() {
    let mut sim = Sim::new(70, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        api::results(
            &db,
            "insert notes { id: notes.key(5), about: users.key(1) }",
        );
        let users = {
            let w = db.write_path();
            Quarantine::Heap(w.heap_named("app::users", "User").unwrap().id().0)
        };
        db.write_path().pager().quarantine(users).unwrap();
        runs(&db, "from i in identities.Google select i.user");
        runs(&db, "delete n in notes where key n == notes.key(5)");
        assert_eq!(
            "app::users.User",
            failed_on(
                &db,
                "insert notes { id: notes.key(6), about: users.key(1) }"
            )
        );
        assert_eq!(
            "app::users.User",
            failed_on(&db, "from n in notes select n.about.name")
        );
        assert_eq!(
            "app::users.User",
            failed_on(
                &db,
                "update n in notes where key n == notes.key(6) set Note { n with about: users.key(2) }"
            )
        );
    });
}

#[test]
fn every_gate_path_refuses_a_call_over_a_marked_constructor() {
    let mut sim = Sim::new(71, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.write_path().pager().quarantine(apple_mark(&db)).unwrap();
        for text in [
            "from i in identities select match i | EmailPass e -> 1 | Google g -> 2 | Apple a -> 3",
            r#"update i in identities.Google where key i == identities.key(11) set Apple { i with sub: "m" }"#,
            r#"insert identities.Apple { id: identities.key(801), user: users.key(1), sub: "x" }"#,
            "from i in [identities.key(11)] select i.user",
            "from x in (from i in identities select i.id) select x",
        ] {
            assert_eq!("app::identities.Apple", failed_on(&db, text), "{text}");
        }
    });
}

#[test]
fn a_marked_database_opens_with_a_newer_log_and_a_bundle_needing_it() {
    let mut sim = Sim::new(72, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        db.write_path()
            .pager()
            .quarantine(Quarantine::Database)
            .unwrap();
        db.close().unwrap();
    });
    api::run(&mut sim, |rt| {
        let later = "module app\nadd type Z = { id: Key zs }\nadd relation zs : Z key id generate sequence\n";
        let needs = Registration::new(
            "zr",
            "import app\nquery q () -> Set (Key zs) = from z in zs select z.id\n",
        );
        let opts = OpenOptions {
            registrations: vec![needs],
            ..api::options(2)
        };
        let db = Database::open_in(rt, FILE, &Log::new([APP, later]), &opts)
            .expect("a marked file opens");
        db.verify(None).unwrap();
        let e = db.registered("zr").expect_err("no call runs");
        assert_eq!("the database", quarantined(&e));
    });
}

#[test]
fn a_registered_call_on_a_reopened_marked_database_fails_with_quarantined() {
    let mut sim = Sim::new(73, Config::default());
    api::run(&mut sim, |rt| {
        let db = Database::open_in(rt, FILE, &Log::new([APP]), &api::options(2)).unwrap();
        seed(&db);
        db.write_path()
            .pager()
            .quarantine(Quarantine::Database)
            .unwrap();
        db.close().unwrap();
    });
    for _ in 0..2 {
        api::run(&mut sim, |rt| {
            let db = open_app(rt);
            let e = call(&db, "through_fn", &[]).expect_err("fails");
            assert_eq!("the database", quarantined(&e));
            db.close().unwrap();
        });
    }
}

#[test]
fn register_on_a_marked_database_fails_with_quarantined() {
    let mut sim = Sim::new(74, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        db.write_path()
            .pager()
            .quarantine(Quarantine::Database)
            .unwrap();
        let auth = db.role_authority();
        let session = auth.session("owner").unwrap();
        let text = format!(
            "bundle late-1 pins 1 ({}) edition \"2026-10\"\nimport app\nquery q () -> Set (Key users) = from u in users select u.id\n",
            hash_log()
        );
        let reg = Registration::bundle(text).expect("parses");
        let e = auth.register(&session, &reg).expect_err("refused");
        assert_eq!("the database", quarantined(&e));
    });
}

#[test]
fn vacuum_of_a_marked_constructor_fails_with_quarantined() {
    let mut sim = Sim::new(75, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.write_path().pager().quarantine(apple_mark(&db)).unwrap();
        assert_eq!("app::identities.Apple", failed_on(&db, "vacuum identities"));
        runs(&db, "vacuum users");
    });
}

#[test]
fn vacuum_of_a_relation_with_a_marked_index_fails_with_quarantined() {
    let mut sim = Sim::new(76, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.write_path()
            .pager()
            .quarantine(user_index_mark(&db))
            .unwrap();
        assert_eq!(
            format!("app::identities {USER_INDEX}"),
            failed_on(&db, "vacuum identities")
        );
        runs(&db, "vacuum users");
    });
}

fn with_history(db: &Database<SimRuntime>) {
    runs(db, "set setting RetentionWindow = { length: 2h }");
    for i in 1..=6 {
        api::results(
            db,
            &format!(
                r#"update i in identities.Apple where key i == identities.key({}) set Apple {{ i with sub: "z" }}"#,
                i * 10 + 2
            ),
        );
    }
    runs(db, "vacuum identities");
}

#[test]
fn a_damaged_history_heap_page_marks_its_constructor() {
    let mut sim = Sim::new(77, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        with_history(&db);
        let w = db.write_path();
        let past = w.history_heap_named("app::identities", "Apple").unwrap();
        let page = *past.owned_pages(w.pager()).unwrap().last().unwrap();
        db.damage_page(page, Damage::Checksum);
        db.verify(None).unwrap();
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_damaged_history_index_page_marks_its_key_index() {
    let mut sim = Sim::new(78, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        with_history(&db);
        let w = db.write_path();
        let past = w.history_index_named("app::identities").unwrap();
        let page = *past.tree().pages(w.pager()).unwrap().last().unwrap();
        db.damage_page(page, Damage::Checksum);
        db.verify(None).unwrap();
        let key = w.index_named("app::identities", "key id").unwrap();
        assert_eq!(
            BTreeSet::from([Quarantine::Index(key.tree().owner())]),
            marks(&db)
        );
    });
}

#[test]
fn an_io_failure_on_the_marks_page_fails_the_load_and_marks_nothing() {
    let mut sim = Sim::new(83, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let p = db.write_path().pager();
        let page = p.quarantine_page();
        p.quarantine(Quarantine::Index(5)).unwrap();
        db.damage_page(page, Damage::Unreadable);
        let e = p.load_marks(page).expect_err("the page cannot be read");
        assert_eq!(Damage::Unreadable, e.damage);
        assert_eq!(BTreeSet::from([Quarantine::Index(5)]), p.quarantined());
    });
}

const BOUNDED: &str = "from i in identities where i.id == identities.key(11) select i.user";

fn key_index_marked(db: &Database<SimRuntime>) -> btree::Index {
    let w = db.write_path();
    let key = w.index_named("app::identities", "key id").unwrap();
    w.pager()
        .quarantine(Quarantine::Index(key.tree().owner()))
        .unwrap();
    key
}

#[test]
fn a_probe_from_the_heaps_stops_on_the_check_of_its_call() {
    let mut sim = Sim::new(79, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let before = api::query(&db, BOUNDED);
        key_index_marked(&db);
        assert_eq!(before, api::query(&db, BOUNDED));
        let w = db.write_path();
        let (id, routes) = w.key_probe("app::identities").unwrap();
        let snapshot = w.begin_verify().unwrap();
        let store = w.store_at(snapshot.txn());
        let grant = adtb::exec::Budget::new(1 << 30).admit(1 << 30).unwrap();
        let check = || Err(adtb::exec::Error::Cancelled);
        let got = store.probe(
            id,
            &[],
            &routes,
            adtb::exec::ProbeBounds {
                grant: &grant,
                check: &check,
            },
        );
        assert!(matches!(got, Err(adtb::exec::Error::Cancelled)), "{got:?}");
    });
}

#[test]
fn a_probe_from_the_heaps_charges_the_versions_it_scans_to_the_grant() {
    let mut sim = Sim::new(80, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        key_index_marked(&db);
        let w = db.write_path();
        let (id, routes) = w.key_probe("app::identities").unwrap();
        let snapshot = w.begin_verify().unwrap();
        let store = w.store_at(snapshot.txn());
        let grant = adtb::exec::Budget::new(8).admit(8).unwrap();
        let check = || Ok(());
        let got = store.probe(
            id,
            &[Value::I64(i64::MAX)],
            &routes,
            adtb::exec::ProbeBounds {
                grant: &grant,
                check: &check,
            },
        );
        assert!(
            matches!(got, Err(adtb::exec::Error::StatementLimit(_))),
            "{got:?}"
        );
    });
}

#[test]
fn a_fallback_gives_the_same_rows_after_updates_retags_and_deletes() {
    let mut sim = Sim::new(81, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        api::results(
            &db,
            r#"update u in users where key u == users.key(2) set User { u with name: "renamed" }"#,
        );
        api::results(
            &db,
            "delete n in identities where key n == identities.key(21)",
        );
        api::results(
            &db,
            r#"update i in identities.Google where key i == identities.key(31) set Apple { i with sub: "moved" }"#,
        );
        let texts = [
            "from i in identities select { i: i.id, n: i.user.name }",
            "from i in identities where i.id == identities.key(31) select i",
            "from i in identities.Apple where i.id == identities.key(31) select i.sub",
            "from i in identities.Google where i.id == identities.key(31) select i.gid",
            "from u in users where u.id == users.key(2) select u.name",
            "from n in [users.key(2), users.key(3)] select n.name",
        ];
        let before: Vec<_> = texts
            .iter()
            .map(|t| api::sorted(api::result(&db, t)))
            .collect();
        for rel in ["app::users", "app::identities"] {
            let w = db.write_path();
            let key = w.index_named(rel, "key id").unwrap();
            w.pager()
                .quarantine(Quarantine::Index(key.tree().owner()))
                .unwrap();
        }
        let after: Vec<_> = texts
            .iter()
            .map(|t| api::sorted(api::result(&db, t)))
            .collect();
        assert_eq!(before, after);
        api::results(
            &db,
            "insert notes { id: notes.key(1), about: users.key(2) }",
        );
        let dangling = api::failure(
            &db,
            "insert notes { id: notes.key(2), about: users.key(9999) }",
        );
        assert!(!matches!(dangling, Error::Fatal { .. }), "{dangling:?}");
    });
}

#[test]
fn a_fallback_finds_a_large_row_and_an_open_transactions_own_row() {
    let mut sim = Sim::new(82, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let big = "x".repeat(40_000);
        api::results(
            &db,
            &format!(
                r#"insert identities.EmailPass {{ id: identities.key(900), user: users.key(1), email: "{big}" }}"#
            ),
        );
        let text = "from i in identities where i.id == identities.key(900) select i.user";
        let before = api::query(&db, text);
        db.write_path()
            .pager()
            .quarantine(user_index_mark(&db))
            .unwrap();
        key_index_marked(&db);
        assert_eq!(before, api::query(&db, text));
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        api::in_txn_results(
            &mut t,
            "insert notes { id: notes.key(901), about: users.key(1) }",
        );
        let got = api::in_txn(
            &mut t,
            "from n in notes where n.id == notes.key(901) select n.about",
        );
        assert_eq!(1, got.len());
    });
}

const SAPP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
  index (name)
add type Identity { id: Key identities, user: Key users }
  = EmailPass { email: Text }
  | Google    { gid: Text }
add relation identities : Identity key id generate sequence
  index (user) in Google
  index (user)
";

#[test]
fn writers_and_vacuum_under_verify_set_no_false_mark() {
    for s in adtb_simkit::seeds::sweep(0..1) {
        let mut sim = Sim::new(
            s,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        api::run(&mut sim, |rt| {
            let options = OpenOptions {
                pool_bytes: Some(64 * 8192),
                ..api::options(2)
            };
            let db = Database::open_in(rt.clone(), "s.adt", &Log::new([SAPP]), &options).unwrap();
            for i in 1..=5 {
                api::results(
                    &db,
                    &format!(
                        r#"insert users {{ id: users.key({i}), name: "n{i}" }}
                           insert identities.Google {{ id: identities.key({i}), user: users.key({i}), gid: "g{i}" }}"#
                    ),
                );
            }
            let stop = Arc::new(AtomicBool::new(false));
            let writers: Vec<_> = (0..2i64)
                .map(|w| {
                    let (db, stop, rt2) = (db.clone(), stop.clone(), rt.clone());
                    rt.spawn(&format!("writer {w}"), move || {
                        let mut n = 0i64;
                        while !stop.load(Ordering::Relaxed) {
                            let row = 1000 + w * 100_000 + n;
                            for t in [
                                format!(
                                    r#"insert users {{ id: users.key({row}), name: "w{row}" }}
                                       insert identities.Google {{ id: identities.key({row}), user: users.key({row}), gid: "g" }}"#
                                ),
                                format!(
                                    r#"update u in users where key u == users.key({row}) set User {{ u with name: "r{row}" }}"#
                                ),
                                format!("delete i in identities where key i == identities.key({row})"),
                                "vacuum identities".to_owned(),
                            ] {
                                if let Err(e) = db.query_text(&t, &[], &CallOptions::default()) {
                                    assert!(e.is_serialization(), "seed {s}: {t}: {e:?}");
                                }
                            }
                            n += 1;
                            rt2.yield_now();
                        }
                    })
                })
                .collect();
            let got = api::printed(db.verify(None).unwrap());
            assert_eq!(Vec::<String>::new(), got, "seed {s}");
            stop.store(true, Ordering::Relaxed);
            for t in writers {
                rt.join(t);
            }
            assert_eq!(
                BTreeSet::new(),
                db.write_path().pager().quarantined(),
                "seed {s}"
            );
        });
    }
}

fn entry(i: u32) -> Vec<u8> {
    let mut v = i.to_be_bytes().to_vec();
    v.resize(500, 7);
    v
}

/// Returns a new scratch directory for a test on real threads.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("quarantine")
        .join(name);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Returns a pager on real threads and three trees of it.
fn real_trees(name: &str) -> (OsRuntime, Arc<Pager<OsRuntime>>, [BTree; 3]) {
    let rt = OsRuntime::new(scratch(name), Backend::Pool { threads: 4 });
    let mut o = Options::new("w.adt");
    o.threads = 70;
    o.pool_frames = 8192;
    o.page_size = 4096;
    let pager = Arc::new(Pager::create(rt.clone(), &o).unwrap());
    let trees = [41, 42, 43].map(|owner| BTree::create(&*pager, owner).unwrap());
    (rt, pager, trees)
}

/// Yield until `limit` passes.
fn wait(rt: &OsRuntime, limit: Duration) {
    let deadline = rt.now() + limit;
    while rt.now() < deadline {
        rt.yield_now();
    }
}

/// Run `mutate` and `map` on 64 threads until `limit` passes. Returns each mapping `map` answered,
/// with how many times.
fn mapped_under_churn<T: std::fmt::Debug>(
    rt: &OsRuntime,
    limit: Duration,
    mutate: impl FnOnce() + Send + 'static,
    map: impl Fn() -> T + Send + Sync + 'static,
) -> BTreeMap<String, u64> {
    let map = Arc::new(map);
    let stop = Arc::new(AtomicBool::new(false));
    let got = Arc::new(Mutex::new(BTreeMap::new()));
    let mappers: Vec<_> = (0..64)
        .map(|i| {
            let (map, stop, got) = (map.clone(), stop.clone(), got.clone());
            rt.spawn(&format!("mapper {i}"), move || {
                while !stop.load(Ordering::Relaxed) {
                    let held = map();
                    *got.lock()
                        .unwrap()
                        .entry(format!("{held:?}"))
                        .or_insert(0u64) += 1;
                }
            })
        })
        .collect();
    let mutator = rt.spawn("mutator", mutate);
    wait(rt, limit);
    stop.store(true, Ordering::Relaxed);
    for m in mappers {
        rt.join(m);
    }
    rt.join(mutator);
    Arc::try_unwrap(got).unwrap().into_inner().unwrap()
}

#[test]
fn mapping_a_page_of_one_tree_never_marks_a_tree_changing_beside_it() {
    let (rt, pager, [a, b, c]) = real_trees("beside");
    for i in 0..400 {
        a.insert(&*pager, &entry(i)).unwrap();
        c.insert(&*pager, &entry(i)).unwrap();
    }
    let damaged = *c.pages(&*pager).unwrap().last().unwrap();
    pager.damage(damaged, Damage::Checksum);
    let limit = Duration::from_millis(1200);
    let mutate = churn_tree(&rt, limit, &pager, a, b);
    let got = {
        let p = pager;
        mapped_under_churn(&rt, limit, mutate, move || {
            p.holders_of(damaged, Some(Quarantine::Database), |h| {
                h.walk(Some(Quarantine::Index(41)), || a.walk(&*p));
                h.walk(Some(Quarantine::Index(43)), || c.walk(&*p));
            })
        })
    };
    assert_eq!(1, got.len(), "{got:?}");
    assert!(got.contains_key("{Index(43)}"), "{got:?}");
}

/// Delete and insert the entries of `a`, with `b` taking the pages freed, until `limit` from now.
fn churn_tree(
    rt: &OsRuntime,
    limit: Duration,
    p: &Arc<Pager<OsRuntime>>,
    a: BTree,
    b: BTree,
) -> impl FnOnce() + Send + 'static {
    let deadline = rt.now() + limit;
    let (rt, p) = (rt.clone(), p.clone());
    move || {
        let mut n = 0;
        while rt.now() < deadline {
            for i in 0..400 {
                a.delete(&*p, &entry(i)).unwrap();
                if i % 2 == 0 {
                    b.insert(&*p, &entry(n)).unwrap();
                    n += 1;
                }
            }
            for i in 0..400 {
                a.insert(&*p, &entry(i)).unwrap();
            }
        }
    }
}

#[test]
fn a_walk_of_a_changing_tree_never_stops_on_damage() {
    let (rt, pager, [a, b, _]) = real_trees("walk");
    for i in 0..400 {
        a.insert(&*pager, &entry(i)).unwrap();
    }
    let limit = Duration::from_millis(1200);
    let mutate = churn_tree(&rt, limit, &pager, a, b);
    let got = {
        let p = pager;
        mapped_under_churn(&rt, limit, mutate, move || a.walk(&*p).map(|_| ()))
    };
    assert!(!got.contains_key("Err(Damage)"), "{got:?}");
}

#[test]
fn mapping_a_page_of_a_changing_tree_finds_that_tree_or_nothing() {
    let (rt, pager, [a, _, _]) = real_trees("within");
    for i in 0..400 {
        a.insert(&*pager, &entry(i)).unwrap();
    }
    let pages = a.pages(&*pager).unwrap();
    let damaged = pages[pages.len() - pages.len() / 2];
    pager.damage(damaged, Damage::Checksum);
    let deadline = rt.now() + Duration::from_millis(300);
    let mutate = {
        let p = pager.clone();
        let rt = rt.clone();
        move || {
            while rt.now() < deadline {
                for i in 0..400 {
                    match a.delete(&*p, &entry(i)) {
                        Ok(_) | Err(btree::Error::Page(_)) => {}
                        Err(e) => panic!("delete {i}: {e:?}"),
                    }
                }
                for i in 0..400 {
                    match a.insert(&*p, &entry(i)) {
                        Ok(_) | Err(btree::Error::Page(_)) => {}
                        Err(e) => panic!("insert {i}: {e:?}"),
                    }
                }
            }
        }
    };
    let got = {
        let p = pager;
        mapped_under_churn(&rt, Duration::from_millis(300), mutate, move || {
            p.holders_of(damaged, Some(Quarantine::Database), |h| {
                h.walk(Some(Quarantine::Index(41)), || a.walk(&*p));
            })
        })
    };
    assert!(!got.contains_key("{Database}"), "{got:?}");
    assert!(got.contains_key("{Index(41)}"), "{got:?}");
}

const LISTS: &str = r"module app
add module app
add type U = { id: Key us, name: Text }
add relation us : U key id generate sequence
add type L = { id: Key ls, s: Text }
add relation ls : L key id generate sequence
  index (s)
";

/// Insert and delete rows of `ls`, by `writer`, and vacuum `ls`, until `stop`. Fail on any error
/// but a serialization failure.
fn churn_lists(db: &Database, writer: i64, stop: &AtomicBool) {
    let run = |text: &str| match db.query_text(text, &[], &CallOptions::default()) {
        Ok(_) => {}
        Err(e) if e.is_serialization() => {}
        Err(e) => panic!("{text:.60}: {e:?}"),
    };
    let mut round = 0;
    while !stop.load(Ordering::Relaxed) {
        round += 1;
        let keys = (0..50).map(|i| writer * 1_000_000 + round * 1000 + i);
        let pad = "x".repeat(1500);
        let mut insert = String::new();
        let mut delete = String::new();
        for k in keys {
            insert.push_str(&format!(
                "insert ls {{ id: ls.key({k}), s: \"{k:08}{pad}\" }}\n"
            ));
            delete.push_str(&format!("delete l in ls where key l == ls.key({k})\n"));
        }
        run(&insert);
        run(&delete);
        run("vacuum ls");
    }
}

fn real_lists(name: &str) -> (OsRuntime, Database) {
    let rt = OsRuntime::new(scratch(name), Backend::Pool { threads: 4 });
    let options = OpenOptions {
        create: Some(adtb::CreateOptions::default()),
        workers: Some(2),
        ..OpenOptions::default()
    };
    let db = Database::open_in(rt.clone(), "f.adt", &Log::new([LISTS]), &options).unwrap();
    (rt, db)
}

#[test]
fn mapping_a_damaged_heap_page_never_marks_an_index_that_is_changing() {
    let (rt, db) = real_lists("engine");
    db.query_text(
        r#"insert us { id: us.key(1), name: "a" }"#,
        &[],
        &CallOptions::default(),
    )
    .unwrap();
    let w = db.write_path();
    let page = w
        .heap_named("app::us", "U")
        .unwrap()
        .pages(w.pager())
        .unwrap()[0];
    db.damage_page(page, Damage::Checksum);
    let stop = Arc::new(AtomicBool::new(false));
    let writers: Vec<_> = (0..3)
        .map(|writer| {
            let (db, stop) = (db.clone(), stop.clone());
            rt.spawn(&format!("writer {writer}"), move || {
                churn_lists(&db, writer, &stop);
            })
        })
        .collect();
    let mappers: Vec<_> = (0..48)
        .map(|i| {
            let (db, stop) = (db.clone(), stop.clone());
            rt.spawn(&format!("mapper {i}"), move || {
                while !stop.load(Ordering::Relaxed) {
                    let e = db
                        .write_path()
                        .quarantining(adtb::write::Error::Page(PageError {
                            pid: page,
                            damage: Damage::Checksum,
                        }));
                    assert!(
                        matches!(&e, adtb::write::Error::Quarantined(what) if what == "app::us.U"),
                        "{e:?}"
                    );
                }
            })
        })
        .collect();
    wait(&rt, Duration::from_millis(1100));
    stop.store(true, Ordering::Relaxed);
    for t in writers.into_iter().chain(mappers) {
        rt.join(t);
    }
    let heap = Quarantine::Heap(w.heap_named("app::us", "U").unwrap().id().0);
    assert_eq!(BTreeSet::from([heap]), marks(&db));
}

#[test]
fn verify_of_indexes_that_are_changing_finds_and_marks_nothing() {
    let (rt, db) = real_lists("verify");
    let stop = Arc::new(AtomicBool::new(false));
    let writers: Vec<_> = (0..3)
        .map(|writer| {
            let (db, stop) = (db.clone(), stop.clone());
            rt.spawn(&format!("writer {writer}"), move || {
                churn_lists(&db, writer, &stop);
            })
        })
        .collect();
    let deadline = rt.now() + Duration::from_millis(800);
    let mut runs = 0;
    while rt.now() < deadline {
        let found = api::printed(db.verify(None).unwrap());
        assert_eq!(Vec::<String>::new(), found, "run {runs}");
        runs += 1;
    }
    stop.store(true, Ordering::Relaxed);
    for t in writers {
        rt.join(t);
    }
    assert!(runs > 0);
    assert_eq!(BTreeSet::new(), marks(&db));
}

/// Run `body` with a page of a fresh pager that no walk holds.
fn with_unheld_page(seed_no: u64, body: impl FnOnce(&Pager<SimRuntime>, Pid)) {
    let mut sim = Sim::new(seed_no, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let p = db.write_path().pager();
        let mut new = p.reserve();
        let pid = new.page(1, adtb::storage::PageKind::Quarantine, 0).unwrap();
        p.update_with(new, &[pid], |_| {}).unwrap();
        body(p, pid);
    });
}

/// Returns what holds `pid` when one walk of `object` gives each of `script` in turn, its last
/// again and again, and how many walks ran.
fn scripted(
    p: &Pager<SimRuntime>,
    pid: Pid,
    script: &[Result<Vec<Pid>, Stopped>],
) -> (BTreeSet<Quarantine>, usize) {
    let mut walks = 0;
    let held = p.holders_of(pid, Some(Quarantine::Database), |h| {
        h.walk(Some(Quarantine::Index(7)), || {
            walks += 1;
            script[(walks - 1).min(script.len() - 1)].clone()
        });
    });
    (held, walks)
}

#[test]
fn a_walk_that_never_follows_its_tree_leaves_the_page_unmapped() {
    with_unheld_page(60, |p, pid| {
        let (held, walks) = scripted(p, pid, &[Err(Stopped::Shape)]);
        assert_eq!(BTreeSet::new(), held);
        assert!((2..=64).contains(&walks), "{walks}");
    });
}

#[test]
fn a_walk_that_stops_on_a_shape_is_run_again() {
    with_unheld_page(61, |p, pid| {
        let index = BTreeSet::from([Quarantine::Index(7)]);
        let script = [Err(Stopped::Shape), Err(Stopped::Shape), Ok(vec![pid])];
        assert_eq!((index, 3), scripted(p, pid, &script));
        let script = [Err(Stopped::Shape), Ok(Vec::new())];
        let database = BTreeSet::from([Quarantine::Database]);
        assert_eq!((database, 2), scripted(p, pid, &script));
    });
}

#[test]
fn a_walk_marks_its_object_only_when_damage_stops_it_twice_in_a_row() {
    with_unheld_page(62, |p, pid| {
        let index = BTreeSet::from([Quarantine::Index(7)]);
        let database = BTreeSet::from([Quarantine::Database]);
        let script = [Err(Stopped::Damage)];
        assert_eq!((index.clone(), 2), scripted(p, pid, &script));
        let script = [Err(Stopped::Damage), Ok(Vec::new())];
        assert_eq!((database, 2), scripted(p, pid, &script));
        let script = [
            Err(Stopped::Damage),
            Err(Stopped::Shape),
            Err(Stopped::Damage),
            Err(Stopped::Damage),
        ];
        assert_eq!((index, 4), scripted(p, pid, &script));
        let script = [Err(Stopped::Damage), Err(Stopped::Shape)];
        assert_eq!(BTreeSet::new(), scripted(p, pid, &script).0);
    });
}

#[test]
fn a_walk_stopped_by_io_is_not_run_again_and_leaves_the_page_unmapped() {
    with_unheld_page(63, |p, pid| {
        let (held, walks) = scripted(p, pid, &[Err(Stopped::Io)]);
        assert_eq!((BTreeSet::new(), 1), (held, walks));
    });
}

#[test]
fn a_page_whose_holder_is_unknown_fails_storage_and_marks_nothing_until_the_walk_resolves() {
    let mut sim = Sim::new(64, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        let long = "x".repeat(40_000);
        api::results(
            &db,
            &format!(
                r#"insert users {{ id: users.key(1), name: "n" }}
                   insert identities.Apple {{ id: identities.key(1), user: users.key(1), sub: "{long}" }}"#
            ),
        );
        let w = db.write_path();
        let heap = w.heap_named("app::identities", "Apple").unwrap();
        let page = *heap.large().owned_pages(w.pager()).unwrap().last().unwrap();
        db.damage_page(page, Damage::Checksum);
        let index = w.index_named("app::identities", USER_INDEX).unwrap();
        let root = index.tree().root();
        index.tree().flip_owner(w.pager(), root).unwrap();
        for attempt in 0..2 {
            let e = api::failure(&db, ROWS);
            let Error::Fatal {
                reason: FatalReason::Storage { detail },
            } = &e
            else {
                panic!("attempt {attempt}: {e:?}");
            };
            assert!(detail.contains(&format!("{page:?}")), "{detail}");
            assert_eq!(BTreeSet::new(), marks(&db), "attempt {attempt}");
        }
        index.tree().flip_owner(w.pager(), root).unwrap();
        assert_eq!("the database", failed_on(&db, ROWS));
    });
}

const GOOGLE_901: &str =
    r#"insert identities.Google { id: identities.key(901), user: users.key(1), gid: "g901" }"#;
const GOOGLE_901_IDS: &str = "from i in identities.Google where i.gid == \"g901\" select i.id";

fn mark_named(db: &Database<SimRuntime>, relation: &str, ctor: &str) -> Quarantine {
    Quarantine::Heap(db.write_path().heap_named(relation, ctor).unwrap().id().0)
}

/// Commit a transaction that runs `write`, with `mark` set after the write. Returns what the
/// commit fails with.
fn refused_commit(db: &Database<SimRuntime>, write: &str, mark: Quarantine) -> String {
    let mut t = db.begin(&TxnOptions::default()).unwrap();
    api::in_txn_results(&mut t, write);
    db.write_path().pager().quarantine(mark).unwrap();
    quarantined(&t.commit(None).unwrap_err()).to_owned()
}

#[test]
fn a_commit_after_a_mark_on_an_index_covering_its_write_fails_and_leaves_nothing() {
    let mut sim = Sim::new(90, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let what = refused_commit(&db, GOOGLE_901, user_index_mark(&db));
        assert!(what.contains(USER_INDEX), "{what}");
        assert!(api::query(&db, GOOGLE_901_IDS).is_empty());
    });
}

#[test]
fn a_commit_after_a_mark_on_its_heap_or_a_sibling_heap_fails_and_leaves_nothing() {
    for ctor in ["Google", "Apple", "EmailPass"] {
        let mut sim = Sim::new(91, Config::default());
        api::run(&mut sim, |rt| {
            let db = open_app(rt);
            seed(&db);
            let mark = mark_named(&db, "app::identities", ctor);
            let what = refused_commit(&db, GOOGLE_901, mark);
            assert_eq!(format!("app::identities.{ctor}"), what);
            db.write_path().pager().release(mark).unwrap();
            assert!(api::query(&db, GOOGLE_901_IDS).is_empty(), "{ctor}");
        });
    }
}

#[test]
fn a_commit_after_a_mark_its_write_checks_read_fails() {
    let mut sim = Sim::new(92, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let note = "insert notes { id: notes.key(901), about: users.key(1) }";
        let mark = mark_named(&db, "app::users", "User");
        assert_eq!("app::users.User", refused_commit(&db, note, mark));
        db.write_path().pager().release(mark).unwrap();
        let tag = r#"insert users { id: users.key(99), name: "n" }
                     insert tags { id: tags.key(901), owner: users.key(99) }"#;
        let mark = apple_mark(&db);
        assert_eq!(
            "app::identities.Apple",
            refused_commit(&db, tag, mark),
            "a constraint reads Apple"
        );
    });
}

#[test]
fn a_commit_writing_nothing_a_mark_stops_runs() {
    let mut sim = Sim::new(93, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        api::in_txn_results(
            &mut t,
            "insert notes { id: notes.key(901), about: users.key(1) }",
        );
        db.write_path().pager().quarantine(apple_mark(&db)).unwrap();
        t.commit(None).unwrap();
        assert_eq!(1, api::query(&db, "from n in notes select n.id").len());
    });
}

#[test]
fn a_transaction_that_only_read_commits_under_a_mark_on_what_it_read() {
    let mut sim = Sim::new(94, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let read_only = TxnOptions {
            read_only: true,
            ..TxnOptions::default()
        };
        for opts in [read_only, TxnOptions::default()] {
            let mut t = db.begin(&opts).unwrap();
            assert_eq!(18, api::in_txn(&mut t, ALL).len());
            let mark = apple_mark(&db);
            db.write_path().pager().quarantine(mark).unwrap();
            t.commit(None).unwrap();
            db.write_path().pager().release(mark).unwrap();
        }
    });
}

#[test]
fn a_statement_that_failed_leaves_no_write_for_the_commit_to_refuse() {
    let mut sim = Sim::new(95, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        let repeat =
            r#"insert identities.Google { id: identities.key(11), user: users.key(1), gid: "r" }"#;
        t.query_text(repeat, &[], &TxnCallOptions::default())
            .unwrap_err();
        db.write_path()
            .pager()
            .quarantine(user_index_mark(&db))
            .unwrap();
        t.commit(None).unwrap();
    });
}

#[test]
fn transact_fails_once_when_a_mark_lands_before_its_commit() {
    let mut sim = Sim::new(96, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let mut attempts = 0;
        let got = db.transact(&TxnOptions::default(), 3, |t| {
            attempts += 1;
            api::in_txn_results(t, GOOGLE_901);
            db.write_path()
                .pager()
                .quarantine(user_index_mark(&db))
                .unwrap();
            Ok(())
        });
        assert!(quarantined(&got.err().unwrap()).contains(USER_INDEX));
        assert_eq!(1, attempts);
        assert!(api::query(&db, GOOGLE_901_IDS).is_empty());
    });
}

#[test]
fn every_commit_with_its_record_after_a_mark_is_refused() {
    for seed_no in adtb_simkit::seeds::sweep(0..1) {
        let config = Config {
            switch_rate: 0.5,
            ..Config::default()
        };
        let mut sim = Sim::new(seed_no, config);
        api::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            seed(&db);
            let kept = Arc::new(Mutex::new(Vec::new()));
            let writers: Vec<_> = (0..4)
                .map(|w| {
                    let (db, kept) = (db.clone(), kept.clone());
                    rt.spawn(&format!("writer {w}"), move || {
                        while marks(&db).is_empty() {
                            let mut t = db.begin(&TxnOptions::default()).unwrap();
                            let write = "insert notes { about: users.key(1) }";
                            if t.query_text(write, &[], &TxnCallOptions::default()).is_ok()
                                && let Ok(at) = t.commit(None)
                            {
                                kept.lock().unwrap().push(at.lsn);
                            }
                        }
                    })
                })
                .collect();
            let marker = {
                let (db, rt) = (db.clone(), rt.clone());
                rt.clone().spawn("marker", move || {
                    for _ in 0..40 {
                        rt.yield_now();
                    }
                    let page = index_page(&db, "app::notes", "index (about)");
                    db.damage_page(page, Damage::Checksum);
                    while marks(&db).is_empty() {
                        drop(db.verify(Some("notes")));
                    }
                })
            };
            for w in writers {
                rt.join(w);
            }
            rt.join(marker);
            let w = db.write_path();
            let pager = w.pager();
            let marked = adtb::storage::lsn(&pager.read(pager.quarantine_page()).unwrap());
            for &at in kept.lock().unwrap().iter() {
                assert!(
                    at < marked.0,
                    "seed {seed_no}: commit at {at}, mark at {marked:?}"
                );
            }
        });
    }
}

#[test]
fn a_commit_waits_for_a_mark_being_set_and_is_refused() {
    let mut sim = Sim::new(100, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt.clone());
        seed(&db);
        let (wrote, go, done) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        );
        let committer = {
            let (db, rt, wrote, go, done) = (
                db.clone(),
                rt.clone(),
                wrote.clone(),
                go.clone(),
                done.clone(),
            );
            rt.clone().spawn("committer", move || {
                let mut t = db.begin(&TxnOptions::default()).unwrap();
                api::in_txn_results(&mut t, GOOGLE_901);
                wrote.store(true, Ordering::Release);
                while !go.load(Ordering::Acquire) {
                    rt.yield_now();
                }
                let got = t.commit(None);
                done.store(true, Ordering::Release);
                assert!(quarantined(&got.unwrap_err()).contains(USER_INDEX));
            })
        };
        while !wrote.load(Ordering::Acquire) {
            rt.yield_now();
        }
        db.damage_page(user_index(&db), Damage::Checksum);
        let pause = |rt: &SimRuntime| {
            let until = rt.now() + Duration::from_millis(50);
            while rt.now() < until {
                rt.yield_now();
            }
        };
        let w = db.write_path();
        let pager = w.pager();
        let held = pager.exclusive(pager.quarantine_page()).unwrap();
        let finder = {
            let db = db.clone();
            rt.spawn("finder", move || {
                failed_on(&db, BY_USER);
            })
        };
        pause(&rt);
        go.store(true, Ordering::Release);
        pause(&rt);
        assert!(
            !done.load(Ordering::Acquire),
            "the commit passed a mark being set"
        );
        drop(held);
        rt.join(finder);
        rt.join(committer);
    });
}

const PAST_APPLE: &str = "from n in [identities.key(12)] select n.user";

fn past_of(at: adtb::Position, text: &str) -> String {
    format!(
        "at commit Position {{ epoch: {}, lsn: {} }} {{ {text} }}",
        at.epoch, at.lsn
    )
}

/// Keep history, commit an update of the Apple row of key 12, and move the old version to
/// history. Returns the position before the update and the one after it.
fn apple_history(db: &Database<SimRuntime>) -> (adtb::Position, adtb::Position) {
    runs(db, "set setting RetentionWindow = { length: 2h }");
    let commit = |text: &str| {
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        api::in_txn_results(&mut t, text);
        let at = t.commit(None).unwrap();
        api::query(db, ALL);
        at
    };
    let update = |user: i64| {
        format!(
            "update i in identities.Apple where key i == identities.key(12) \
             set Apple {{ i with user: users.key({user}) }}"
        )
    };
    let first = commit(&update(2));
    let second = commit(&update(3));
    commit(&update(4));
    runs(db, "vacuum identities");
    (first, second)
}

fn damage_history_index(db: &Database<SimRuntime>) {
    let w = db.write_path();
    let past = w.history_index_named("app::identities").unwrap();
    db.damage_page(past.tree().root(), Damage::Checksum);
}

#[test]
fn a_read_of_the_past_runs_without_its_damaged_history_key_index() {
    let mut sim = Sim::new(97, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let (first, second) = apple_history(&db);
        let (at_first, at_second) = (past_of(first, PAST_APPLE), past_of(second, PAST_APPLE));
        let (then_first, then_second) = (api::query(&db, &at_first), api::query(&db, &at_second));
        assert_ne!(then_first, then_second);
        damage_history_index(&db);
        let what = failed_on(&db, &at_first);
        assert!(what.contains("key id"), "{what}");
        assert_eq!(1, marks(&db).len());
        assert_eq!(then_first, api::query(&db, &at_first));
        assert_eq!(then_second, api::query(&db, &at_second));
    });
}

#[test]
fn a_read_of_the_past_with_no_row_then_fails_alike_without_the_history_key_index() {
    let mut sim = Sim::new(98, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let (first, _) = apple_history(&db);
        let gone = past_of(first, "from n in [identities.key(999)] select n.user");
        let before = format!("{:?}", api::failure(&db, &gone));
        damage_history_index(&db);
        failed_on(&db, &past_of(first, PAST_APPLE));
        assert_eq!(before, format!("{:?}", api::failure(&db, &gone)));
    });
}

#[test]
fn a_read_of_the_past_fails_on_a_damaged_history_heap_without_the_history_key_index() {
    let mut sim = Sim::new(99, Config::default());
    api::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let (first, _) = apple_history(&db);
        let at_first = past_of(first, PAST_APPLE);
        damage_history_index(&db);
        failed_on(&db, &at_first);
        let w = db.write_path();
        let heap = w.history_heap_named("app::identities", "Apple").unwrap();
        for page in heap.pages(w.pager()).unwrap() {
            db.damage_page(page, Damage::Checksum);
        }
        let what = failed_on(&db, &at_first);
        assert!(what.contains("app::identities.Apple"), "{what}");
        assert!(marks(&db).contains(&apple_mark(&db)));
        assert_eq!("app::identities.Apple", failed_on(&db, &at_first));
    });
}
