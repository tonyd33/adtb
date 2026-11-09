use adtb::testing::{Damage, Failed, Object, PageError, Pid, Plant, Quarantine, page::HEADER_LEN};
use adtb_harness::process;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::Value as Val;
use adtb::key::{Entry, Tid, Value};
use adtb::runtime::{Clock, Io, Op, Runtime, Threads};
use adtb::{
    CallOptions, Called, Database, Error, FatalReason, Log, OpenOptions, Registration,
    TxnCallOptions, TxnOptions,
};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};

use adtb_harness::damage::DataFile;
use adtb_simkit::api;
use adtb_simkit::open;
use adtb_sys::os::{Backend, OsRuntime};

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
const DATA: DataFile = DataFile::new(FILE, API_PAGE);
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
        ..open::options(2)
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
    let t = db.testing();
    t.pages(t.heap(relation, ctor).unwrap()).unwrap()[0]
}

fn index_page(db: &Database<SimRuntime>, relation: &str, clause: &str) -> Pid {
    let t = db.testing();
    *t.pages(t.index(relation, clause).unwrap())
        .unwrap()
        .last()
        .unwrap()
}

/// Returns a simulator over a seeded file with page `pick` picks flipped on disk.
fn flipped(seed_no: u64, pick: impl FnOnce(&Database<SimRuntime>) -> Pid) -> Sim {
    let mut sim = Sim::new(seed_no, Config::default());
    let page = process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let page = pick(&db);
        db.close().unwrap();
        page
    });
    let mut files = sim.files();
    DATA.flip(&mut files, page, 300 - HEADER_LEN);
    Sim::with_files(seed_no, Config::default(), files)
}

fn apple(db: &Database<SimRuntime>) -> Pid {
    heap_page(db, "app::identities", "Apple")
}

fn google(db: &Database<SimRuntime>) -> Pid {
    heap_page(db, "app::identities", "Google")
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
    db.testing().probe().marks()
}

fn apple_mark(db: &Database<SimRuntime>) -> Quarantine {
    let t = db.testing();
    t.own_mark(t.heap("app::identities", "Apple").unwrap())
        .unwrap()
}

fn user_index_mark(db: &Database<SimRuntime>) -> Quarantine {
    let t = db.testing();
    t.own_mark(t.index("app::identities", USER_INDEX).unwrap())
        .unwrap()
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(12, api::query(&db, SIDES).len());
    });
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_damaged_index_page_costs_only_plans_and_stops_writes_it_covers() {
    let mut sim = flipped(13, user_index);
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
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
        db.testing()
            .plant(Plant::Refuse {
                page: index_page(&db, "app::identities", "index (gid) in Google"),
                damage: Damage::Checksum,
            })
            .unwrap();
        let what = quarantined(&call(&db, "by_gid", &gid).unwrap_err()).to_owned();
        assert!(what.contains("(gid)"), "{what}");
        assert_eq!(before, call(&db, "by_gid", &gid).unwrap());
        assert!(!shown(&db).contains("Probe"), "{}", shown(&db));
    });
}

#[test]
fn a_key_lookup_runs_from_the_heaps_while_its_index_is_quarantined() {
    let mut sim = Sim::new(16, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let text = "from n in identities where n.id == identities.key(11) select n.user";
        let deref = "from n in [identities.key(11)] select n.id";
        let before = (api::query(&db, text), api::query(&db, deref));
        let page = index_page(&db, "app::identities", "key id");
        db.testing()
            .plant(Plant::Refuse {
                page,
                damage: Damage::Checksum,
            })
            .unwrap();
        let what = failed_on(&db, text);
        assert!(what.contains("key id"), "{what}");
        assert_eq!(before, (api::query(&db, text), api::query(&db, deref)));
    });
}

#[test]
fn an_io_failure_sets_no_mark_and_stays_a_storage_failure() {
    let mut sim = Sim::new(17, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.testing()
            .plant(Plant::Refuse {
                page: apple(&db),
                damage: Damage::Unreadable,
            })
            .unwrap();
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
    for seed_no in adtb_harness::seeds::sweep(20..22) {
        let mut sim = Sim::new(seed_no, Config::default());
        process::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            seed(&db);
            db.testing()
                .plant(Plant::Refuse {
                    page: apple(&db),
                    damage: Damage::Checksum,
                })
                .unwrap();
            let before = db.testing().probe().durable();
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
            assert!(db.testing().probe().durable() > before, "seed {seed_no}");
            let once = db.testing().probe().durable();
            db.testing().plant(Plant::Mark(apple_mark(&db))).unwrap();
            assert_eq!(once, db.testing().probe().durable(), "seed {seed_no}");
        });
    }
}

#[test]
fn a_bad_catalog_page_marks_the_database() {
    let mut sim = Sim::new(18, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.testing()
            .plant(Plant::Refuse {
                page: heap_page(&db, "catalog::roles", "Role"),
                damage: Damage::Checksum,
            })
            .unwrap();
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
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
fn a_damaged_commit_log_page_marks_the_database_and_verify_runs() {
    let mut sim = Sim::new(30, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let page = *db
            .testing()
            .commit_log()
            .status()
            .values()
            .next_back()
            .unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page,
                damage: Damage::Checksum,
            })
            .unwrap();
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let t = db.testing();
        let ix = t.index("app::identities", USER_INDEX).unwrap();
        let gone = t.entries(ix, &[Value::I64(5)]).unwrap().remove(0);
        t.plant(Plant::DropEntry {
            index: ix,
            entry: gone,
        })
        .unwrap();
        let found = api::printed(db.verify(Some("identities")).unwrap());
        assert!(found[0].starts_with("Missing"), "{found:?}");
        assert_eq!(BTreeSet::from([user_index_mark(&db)]), marks(&db));
        assert_eq!(1, api::query(&db, BY_USER).len());
    });
    let mut sim = Sim::new(34, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let t = db.testing();
        let heap = t.heap("app::identities", "Google").unwrap();
        let page = t.pages(heap).unwrap()[0];
        let ix = t.index("app::identities", USER_INDEX).unwrap();
        let ctor = *t.ctors(ix).unwrap().iter().next().unwrap();
        let orphan = Entry {
            key: vec![Value::I64(1000)],
            ctor,
            tid: Tid::new(page.0, 400).unwrap(),
        };
        t.plant(Plant::AddEntry {
            index: ix,
            entry: orphan,
        })
        .unwrap();
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Orphan")), "{found:?}");
        assert_eq!(BTreeSet::from([user_index_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_verify_checksum_finding_marks_what_holds_the_page() {
    let mut sim = flipped(35, apple);
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        let page = apple(&db);
        let found = api::printed(db.verify(None).unwrap());
        let at = format!("where: \"page {}: Checksum\" }}", page.0);
        assert_eq!(1, found.len(), "{found:?}");
        assert!(found[0].starts_with("Checksum {"), "{found:?}");
        assert!(found[0].ends_with(&at), "{found:?}");
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
        assert_eq!("app::identities.Apple", failed_on(&db, ALL));
        assert_eq!(12, api::query(&db, SIDES).len());
    });
}

#[test]
fn verify_of_a_marked_database_still_runs() {
    let mut sim = Sim::new(36, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.testing()
            .plant(Plant::Mark(Quarantine::Database))
            .unwrap();
        db.close().unwrap();
    });
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let marks_page = db.testing().root(Object::Marks).unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: marks_page,
                damage: Damage::Checksum,
            })
            .unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: apple(&db),
                damage: Damage::Checksum,
            })
            .unwrap();
        let what = failed_on(&db, ALL);
        assert!(what.starts_with("app::identities.Apple"), "{what}");
        assert!(what.contains("only in memory"), "{what}");
        let apple = apple_mark(&db);
        assert_eq!(BTreeSet::from([apple, Quarantine::Database]), marks(&db));
        assert_eq!("the database", failed_on(&db, ALL));
    });
}

#[test]
fn a_mark_fails_a_call_of_an_open_transaction_and_leaves_it_open() {
    let mut sim = Sim::new(40, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        assert_eq!(18, api::in_txn(&mut t, ALL).len());
        db.testing().plant(Plant::Mark(apple_mark(&db))).unwrap();
        let got = t.query_text(ALL, &[], &TxnCallOptions::default());
        assert_eq!("app::identities.Apple", quarantined(&got.unwrap_err()));
        assert_eq!(12, api::in_txn(&mut t, SIDES).len());
        t.commit(None).unwrap();
    });
}

#[test]
fn a_damaged_large_value_page_marks_the_database() {
    let mut sim = Sim::new(41, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        let long = "x".repeat(40_000);
        api::results(
            &db,
            &format!(
                r#"insert users {{ id: users.key(1), name: "n" }}
                   insert identities.Apple {{ id: identities.key(1), user: users.key(1), sub: "{long}" }}"#
            ),
        );
        let t = db.testing();
        let large = t.large("app::identities", "Apple").unwrap();
        let pages = t.owned(large).unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: *pages.last().unwrap(),
                damage: Damage::Checksum,
            })
            .unwrap();
        assert_eq!("the database", failed_on(&db, ROWS));
    });
}

#[test]
fn the_mark_of_a_large_value_store_is_the_one_its_damage_sets() {
    let mut sim = Sim::new(41, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        let long = "x".repeat(40_000);
        api::results(
            &db,
            &format!(
                r#"insert users {{ id: users.key(1), name: "n" }}
                   insert identities.Apple {{ id: identities.key(1), user: users.key(1), sub: "{long}" }}"#
            ),
        );
        let t = db.testing();
        let large = t.large("app::identities", "Apple").unwrap();
        let page = *t.owned(large).unwrap().last().unwrap();
        t.plant(Plant::Refuse {
            page,
            damage: Damage::Checksum,
        })
        .unwrap();
        failed_on(&db, ROWS);
        assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.testing().plant(Plant::Mark(mark(&db))).unwrap();
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
            let t = db.testing();
            t.own_mark(t.heap("app::users", "User").unwrap()).unwrap()
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        db.testing()
            .plant(Plant::Mark(Quarantine::Database))
            .unwrap();
        db.close().unwrap();
    });
    process::run(&mut sim, |rt| {
        let later = "module app\nadd type Z = { id: Key zs }\nadd relation zs : Z key id generate sequence\n";
        let options = OpenOptions {
            registrations: vec![reads()],
            ..open::options(2)
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
    adtb::version_hash(&schema, &source)
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
    let t = db.testing();
    let index = t.index("app::identities", USER_INDEX).unwrap();
    let pages = t.pages(index).unwrap();
    assert!(pages.len() >= 3, "{pages:?}");
    (t.root(index).unwrap(), pages)
}

fn runs(db: &Database<SimRuntime>, text: &str) {
    if let Err(e) = db.query_text(text, &[], &CallOptions::default()) {
        panic!("`{text}` runs: {e:?}");
    }
}

#[test]
fn two_finders_of_different_objects_keep_both_marks() {
    for s in adtb_harness::seeds::sweep(0..6) {
        let mut sim = Sim::new(s, Config::default());
        process::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            let finders: Vec<_> = (0..2u32)
                .map(|i| {
                    let db = db.clone();
                    rt.spawn(&format!("marker {i}"), move || {
                        let t = db.testing();
                        t.plant(Plant::Mark(Quarantine::Heap(1000 + i))).unwrap();
                    })
                })
                .collect();
            for t in finders {
                rt.join(t);
            }
            assert_eq!(2, marks(&db).len(), "seed {s}: {:?}", marks(&db));
            db.close().unwrap();
        });
        process::run(&mut sim, |rt| {
            let db = open_app(rt);
            assert_eq!(2, marks(&db).len(), "seed {s}: {:?}", marks(&db));
        });
    }
}

#[test]
fn a_second_finder_of_one_object_returns_with_the_mark_durable() {
    for s in adtb_harness::seeds::sweep(0..6) {
        let mut sim = Sim::new(s, Config::default());
        process::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            let finders: Vec<_> = (0..3u32)
                .map(|i| {
                    let db = db.clone();
                    rt.spawn(&format!("marker {i}"), move || {
                        let t = db.testing();
                        t.plant(Plant::Mark(Quarantine::Heap(1000))).unwrap();
                        let marks_page = t.root(Object::Marks).unwrap();
                        let at = adtb::testing::page::lsn(&t.probe().page(marks_page).unwrap());
                        assert!(t.probe().durable() > at, "seed {s} marker {i}");
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        seed_many(&db, 900);
        let (_, pages) = index_pages(&db);
        db.testing()
            .plant(Plant::Refuse {
                page: pages[pages.len() - 1],
                damage: Damage::Checksum,
            })
            .unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: pages[pages.len() - 2],
                damage: Damage::Checksum,
            })
            .unwrap();
        db.verify(Some("identities")).unwrap();
        assert_eq!(BTreeSet::from([user_index_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_damaged_root_and_leaf_of_one_index_mark_the_index() {
    let mut sim = Sim::new(62, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        seed_many(&db, 900);
        let (root, pages) = index_pages(&db);
        let leaf = *pages.iter().rev().find(|p| **p != root).unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: root,
                damage: Damage::Checksum,
            })
            .unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: leaf,
                damage: Damage::Checksum,
            })
            .unwrap();
        db.verify(None).unwrap();
        assert_eq!(BTreeSet::from([user_index_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_walk_stopped_by_damage_marks_its_heap() {
    let mut sim = Sim::new(63, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let t = db.testing();
        let heap = t.heap("app::identities", "Apple").unwrap();
        let owned = t.owned(heap).unwrap();
        for pid in owned {
            db.testing()
                .plant(Plant::Refuse {
                    page: pid,
                    damage: Damage::Checksum,
                })
                .unwrap();
        }
        db.verify(Some("identities")).unwrap();
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
    });
}

#[test]
fn an_io_failure_in_the_walk_marks_nothing() {
    let mut sim = Sim::new(64, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let t = db.testing();
        let heap = t.heap("app::identities", "Apple").unwrap();
        let unheld = t
            .plant(Plant::Orphan {
                kind: adtb::testing::PageKind::Quarantine,
                owner: 0,
            })
            .unwrap()
            .page()
            .unwrap();
        let root = t.owned(heap).unwrap()[0];
        db.testing()
            .plant(Plant::Refuse {
                page: root,
                damage: Damage::Unreadable,
            })
            .unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: unheld,
                damage: Damage::Checksum,
            })
            .unwrap();
        let found = api::printed(db.verify(None).unwrap());
        let checksum = format!("where: \"page {}: Checksum\"", unheld.0);
        assert!(found.iter().any(|r| r.contains(&checksum)), "{found:?}");
        assert!(marks(&db).is_empty(), "{:?}", marks(&db));
    });
}

#[test]
fn rot_on_a_free_page_marks_nothing() {
    let mut sim = Sim::new(51, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let t = db.testing();
        let pid = t
            .plant(Plant::Orphan {
                kind: adtb::testing::PageKind::Quarantine,
                owner: 0,
            })
            .unwrap()
            .page()
            .unwrap();
        t.plant(Plant::Free(pid)).unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: pid,
                damage: Damage::Checksum,
            })
            .unwrap();
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
        assert_eq!(BTreeSet::new(), marks(&db));
    });
}

#[test]
fn an_allocated_page_every_walk_misses_marks_the_database() {
    let mut sim = Sim::new(53, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let pid = db
            .testing()
            .plant(Plant::Orphan {
                kind: adtb::testing::PageKind::Quarantine,
                owner: 0,
            })
            .unwrap()
            .page()
            .unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: pid,
                damage: Damage::Checksum,
            })
            .unwrap();
        db.verify(None).unwrap();
        assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
    });
}

#[test]
fn rot_in_the_kept_store_is_a_finding_with_no_mark() {
    let mut sim = Sim::new(54, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.testing().plant(Plant::RefuseKeptStore).unwrap();
        let found = api::printed(db.verify(None).unwrap());
        assert!(found.iter().any(|r| r.starts_with("Checksum")), "{found:?}");
        assert_eq!(BTreeSet::new(), marks(&db));
    });
}

#[test]
fn a_delete_reads_no_target_of_its_row() {
    let mut sim = Sim::new(70, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        api::results(
            &db,
            "insert notes { id: notes.key(5), about: users.key(1) }",
        );
        let users = {
            let t = db.testing();
            t.own_mark(t.heap("app::users", "User").unwrap()).unwrap()
        };
        db.testing().plant(Plant::Mark(users)).unwrap();
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.testing().plant(Plant::Mark(apple_mark(&db))).unwrap();
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        db.testing()
            .plant(Plant::Mark(Quarantine::Database))
            .unwrap();
        db.close().unwrap();
    });
    process::run(&mut sim, |rt| {
        let later = "module app\nadd type Z = { id: Key zs }\nadd relation zs : Z key id generate sequence\n";
        let needs = Registration::new(
            "zr",
            "import app\nquery q () -> Set (Key zs) = from z in zs select z.id\n",
        );
        let opts = OpenOptions {
            registrations: vec![needs],
            ..open::options(2)
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
    process::run(&mut sim, |rt| {
        let db = Database::open_in(rt, FILE, &Log::new([APP]), &open::options(2)).unwrap();
        seed(&db);
        db.testing()
            .plant(Plant::Mark(Quarantine::Database))
            .unwrap();
        db.close().unwrap();
    });
    for _ in 0..2 {
        process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        db.testing()
            .plant(Plant::Mark(Quarantine::Database))
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.testing().plant(Plant::Mark(apple_mark(&db))).unwrap();
        assert_eq!("app::identities.Apple", failed_on(&db, "vacuum identities"));
        runs(&db, "vacuum users");
    });
}

#[test]
fn vacuum_of_a_relation_with_a_marked_index_fails_with_quarantined() {
    let mut sim = Sim::new(76, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        db.testing()
            .plant(Plant::Mark(user_index_mark(&db)))
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        with_history(&db);
        let t = db.testing();
        let past = t.history("app::identities", "Apple").unwrap();
        let page = *t.owned(past).unwrap().last().unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page,
                damage: Damage::Checksum,
            })
            .unwrap();
        db.verify(None).unwrap();
        assert_eq!(BTreeSet::from([apple_mark(&db)]), marks(&db));
    });
}

#[test]
fn a_damaged_history_index_page_marks_its_key_index() {
    let mut sim = Sim::new(78, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        with_history(&db);
        let t = db.testing();
        let past = t.history_index("app::identities").unwrap();
        let page = *t.pages(past).unwrap().last().unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page,
                damage: Damage::Checksum,
            })
            .unwrap();
        db.verify(None).unwrap();
        let key = t.index("app::identities", "key id").unwrap();
        assert_eq!(BTreeSet::from([t.own_mark(key).unwrap()]), marks(&db));
    });
}

#[test]
fn an_io_failure_on_the_marks_page_fails_the_load_and_marks_nothing() {
    let mut sim = Sim::new(83, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        let t = db.testing();
        let page = t.root(Object::Marks).unwrap();
        t.plant(Plant::Mark(Quarantine::Index(5))).unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page,
                damage: Damage::Unreadable,
            })
            .unwrap();
        let e = match t.plant(Plant::LoadMarks(page)) {
            Err(Failed::Page(e)) => e,
            other => panic!("the page cannot be read: {other:?}"),
        };
        assert_eq!(Damage::Unreadable, e.damage);
        assert_eq!(BTreeSet::from([Quarantine::Index(5)]), t.probe().marks());
    });
}

fn key_index_marked(db: &Database<SimRuntime>) -> Object {
    let t = db.testing();
    let key = t.index("app::identities", "key id").unwrap();
    t.plant(Plant::Mark(t.own_mark(key).unwrap())).unwrap();
    key
}

#[test]
fn a_fallback_gives_the_same_rows_after_updates_retags_and_deletes() {
    let mut sim = Sim::new(81, Config::default());
    process::run(&mut sim, |rt| {
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
            let t = db.testing();
            let key = t.index(rel, "key id").unwrap();
            t.plant(Plant::Mark(t.own_mark(key).unwrap())).unwrap();
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
    process::run(&mut sim, |rt| {
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
        db.testing()
            .plant(Plant::Mark(user_index_mark(&db)))
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
    for s in adtb_harness::seeds::sweep(0..1) {
        let mut sim = Sim::new(
            s,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        process::run(&mut sim, |rt| {
            let options = OpenOptions {
                pool_bytes: Some(64 * 8192),
                ..open::options(2)
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
            assert_eq!(BTreeSet::new(), db.testing().probe().marks(), "seed {s}");
        });
    }
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

#[test]
fn a_large_value_page_fails_as_storage_unmarked_while_its_store_walk_stops_then_maps_to_the_database()
 {
    let mut sim = Sim::new(64, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        let long = "x".repeat(40_000);
        api::results(
            &db,
            &format!(
                r#"insert users {{ id: users.key(1), name: "n" }}
                   insert identities.Apple {{ id: identities.key(1), user: users.key(1), sub: "{long}" }}"#
            ),
        );
        let t = db.testing();
        let large = t.large("app::identities", "Apple").unwrap();
        let page = *t.owned(large).unwrap().last().unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page,
                damage: Damage::Checksum,
            })
            .unwrap();
        let flip = || {
            t.plant(Plant::FlipOwner(t.root(large).unwrap())).unwrap();
        };
        flip();
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
        flip();
        assert_eq!("the database", failed_on(&db, ROWS));
        assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
    });
}

const GOOGLE_901: &str =
    r#"insert identities.Google { id: identities.key(901), user: users.key(1), gid: "g901" }"#;
const GOOGLE_901_IDS: &str = "from i in identities.Google where i.gid == \"g901\" select i.id";

fn mark_named(db: &Database<SimRuntime>, relation: &str, ctor: &str) -> Quarantine {
    let t = db.testing();
    t.own_mark(t.heap(relation, ctor).unwrap()).unwrap()
}

/// Commit a transaction that runs `write`, with `mark` set after the write. Returns what the
/// commit fails with.
#[must_use]
fn refused_commit(db: &Database<SimRuntime>, write: &str, mark: Quarantine) -> String {
    let mut t = db.begin(&TxnOptions::default()).unwrap();
    api::in_txn_results(&mut t, write);
    db.testing().plant(Plant::Mark(mark)).unwrap();
    quarantined(&t.commit(None).unwrap_err()).to_owned()
}

#[test]
fn a_commit_after_a_mark_on_an_index_covering_its_write_fails_and_leaves_nothing() {
    let mut sim = Sim::new(90, Config::default());
    process::run(&mut sim, |rt| {
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
        process::run(&mut sim, |rt| {
            let db = open_app(rt);
            seed(&db);
            let mark = mark_named(&db, "app::identities", ctor);
            let what = refused_commit(&db, GOOGLE_901, mark);
            assert_eq!(format!("app::identities.{ctor}"), what);
            db.testing().plant(Plant::Unmark(mark)).unwrap();
            assert!(api::query(&db, GOOGLE_901_IDS).is_empty(), "{ctor}");
        });
    }
}

#[test]
fn a_commit_after_a_mark_its_write_checks_read_fails() {
    let mut sim = Sim::new(92, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let note = "insert notes { id: notes.key(901), about: users.key(1) }";
        let mark = mark_named(&db, "app::users", "User");
        assert_eq!("app::users.User", refused_commit(&db, note, mark));
        db.testing().plant(Plant::Unmark(mark)).unwrap();
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        api::in_txn_results(
            &mut t,
            "insert notes { id: notes.key(901), about: users.key(1) }",
        );
        db.testing().plant(Plant::Mark(apple_mark(&db))).unwrap();
        t.commit(None).unwrap();
        assert_eq!(1, api::query(&db, "from n in notes select n.id").len());
    });
}

#[test]
fn a_transaction_that_only_read_commits_under_a_mark_on_what_it_read() {
    let mut sim = Sim::new(94, Config::default());
    process::run(&mut sim, |rt| {
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
            db.testing().plant(Plant::Mark(mark)).unwrap();
            t.commit(None).unwrap();
            db.testing().plant(Plant::Unmark(mark)).unwrap();
        }
    });
}

#[test]
fn a_statement_that_failed_leaves_no_write_for_the_commit_to_refuse() {
    let mut sim = Sim::new(95, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        let repeat =
            r#"insert identities.Google { id: identities.key(11), user: users.key(1), gid: "r" }"#;
        t.query_text(repeat, &[], &TxnCallOptions::default())
            .unwrap_err();
        db.testing()
            .plant(Plant::Mark(user_index_mark(&db)))
            .unwrap();
        t.commit(None).unwrap();
    });
}

#[test]
fn transact_fails_once_when_a_mark_lands_before_its_commit() {
    let mut sim = Sim::new(96, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let mut attempts = 0;
        let got = db.transact(&TxnOptions::default(), 3, |t| {
            attempts += 1;
            api::in_txn_results(t, GOOGLE_901);
            db.testing()
                .plant(Plant::Mark(user_index_mark(&db)))
                .unwrap();
            Ok(())
        });
        assert!(quarantined(&got.err().unwrap()).contains(USER_INDEX));
        assert_eq!(1, attempts);
        assert!(api::query(&db, GOOGLE_901_IDS).is_empty());
    });
}

#[test]
fn a_mark_lsn_follows_every_commit_kept_before_it_and_refuses_every_commit_after() {
    let seeds = adtb_harness::seeds::sweep(0..2);
    let mut refusing = 0;
    for seed_no in seeds.clone() {
        let config = Config {
            switch_rate: 0.5,
            ..Config::default()
        };
        let mut sim = Sim::new(seed_no, config);
        process::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            seed(&db);
            let kept = Arc::new(Mutex::new(Vec::new()));
            let (between, refused) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
            let writers: Vec<_> = (0..4)
                .map(|w| {
                    let (db, clock, kept, between, refused) = (
                        db.clone(),
                        rt.clone(),
                        kept.clone(),
                        between.clone(),
                        refused.clone(),
                    );
                    rt.spawn(&format!("writer {w}"), move || {
                        while marks(&db).is_empty() {
                            let mut t = db.begin(&TxnOptions::default()).unwrap();
                            let write = "insert notes { about: users.key(1) }";
                            if t.query_text(write, &[], &TxnCallOptions::default())
                                .is_err()
                            {
                                continue;
                            }
                            between.fetch_add(1, Ordering::AcqRel);
                            let wait = adtb_harness::draw::below(&clock, 4) as u64;
                            let until = clock.now() + Duration::from_millis(wait);
                            while clock.now() < until {
                                clock.yield_now();
                            }
                            let got = t.commit(None);
                            between.fetch_sub(1, Ordering::AcqRel);
                            match got {
                                Ok(at) => kept.lock().unwrap().push(at.lsn),
                                Err(e) => {
                                    assert_eq!(
                                        "app::notes index (about)",
                                        quarantined(&e),
                                        "seed {seed_no}"
                                    );
                                    refused.fetch_add(1, Ordering::AcqRel);
                                }
                            }
                        }
                    })
                })
                .collect();
            let marker = {
                let (db, rt, kept) = (db.clone(), rt.clone(), kept.clone());
                rt.clone().spawn("marker", move || {
                    for _ in 0..4000 {
                        if kept.lock().unwrap().len() >= 8 && between.load(Ordering::Acquire) > 0 {
                            break;
                        }
                        rt.yield_now();
                    }
                    let page = index_page(&db, "app::notes", "index (about)");
                    db.testing()
                        .plant(Plant::Refuse {
                            page,
                            damage: Damage::Checksum,
                        })
                        .unwrap();
                    while marks(&db).is_empty() {
                        drop(db.verify(Some("notes")));
                    }
                })
            };
            for w in writers {
                rt.join(w);
            }
            rt.join(marker);
            let t = db.testing();
            let marks_page = t.root(Object::Marks).unwrap();
            let marked = adtb::testing::page::lsn(&t.probe().page(marks_page).unwrap());
            let kept = kept.lock().unwrap();
            for &at in kept.iter() {
                assert!(
                    at < marked.0,
                    "seed {seed_no}: commit at {at}, mark at {marked:?}"
                );
            }
            assert_eq!(
                kept.len(),
                api::query(&db, "from n in notes select n").len(),
                "seed {seed_no}: the notes are the kept commits"
            );
            adtb_harness::seeds::assert_reached(
                &format!("seed {seed_no}: commits before the mark"),
                kept.len(),
                8,
            );
            if refused.load(Ordering::Acquire) > 0 {
                refusing += 1;
            }
        });
    }
    adtb_harness::seeds::assert_reached(
        "seeds refusing a commit after the mark",
        refusing,
        seeds.count().div_ceil(2),
    );
}

#[test]
fn a_commit_waits_for_a_mark_being_set_and_is_refused() {
    let mut sim = Sim::new(100, Config::default());
    process::run(&mut sim, |rt| {
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
        db.testing()
            .plant(Plant::Refuse {
                page: user_index(&db),
                damage: Damage::Checksum,
            })
            .unwrap();
        let pause = |rt: &SimRuntime| {
            let until = rt.now() + Duration::from_millis(50);
            while rt.now() < until {
                rt.yield_now();
            }
        };
        let t = db.testing();
        let held = t.hold_page(t.root(Object::Marks).unwrap()).unwrap();
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
    let t = db.testing();
    let past = t.history_index("app::identities").unwrap();
    db.testing()
        .plant(Plant::Refuse {
            page: t.root(past).unwrap(),
            damage: Damage::Checksum,
        })
        .unwrap();
}

#[test]
fn a_read_of_the_past_runs_without_its_damaged_history_key_index() {
    let mut sim = Sim::new(97, Config::default());
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
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
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let (first, _) = apple_history(&db);
        let at_first = past_of(first, PAST_APPLE);
        damage_history_index(&db);
        failed_on(&db, &at_first);
        let t = db.testing();
        let heap = t.history("app::identities", "Apple").unwrap();
        for page in t.pages(heap).unwrap() {
            db.testing()
                .plant(Plant::Refuse {
                    page,
                    damage: Damage::Checksum,
                })
                .unwrap();
        }
        let what = failed_on(&db, &at_first);
        assert!(what.contains("app::identities.Apple"), "{what}");
        assert!(marks(&db).contains(&apple_mark(&db)));
        assert_eq!("app::identities.Apple", failed_on(&db, &at_first));
    });
}

#[test]
fn a_commit_writing_a_heap_whose_damage_is_recorded_and_not_yet_mapped_is_refused() {
    let mut sim = Sim::new(96, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        api::in_txn_results(&mut t, GOOGLE_901);
        db.testing()
            .plant(Plant::Refuse {
                page: google(&db),
                damage: Damage::Checksum,
            })
            .unwrap();
        assert_eq!(BTreeSet::new(), marks(&db));
        let what = quarantined(&t.commit(None).unwrap_err()).to_owned();
        assert_eq!("app::identities.Google", what);
        let google = mark_named(&db, "app::identities", "Google");
        assert_eq!(BTreeSet::from([google]), marks(&db));
    });
}

#[test]
fn a_commit_whose_record_of_the_retention_window_is_damaged_is_refused_naming_the_database() {
    let mut sim = Sim::new(97, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt.clone());
        seed(&db);
        api::results(&db, "set setting RetentionWindow = { length: 1h }");
        api::results(&db, GOOGLE_901);
        let past = db.testing().root(Object::CommitTimes).unwrap();
        db.testing().flush();
        let mut io = rt.io();
        let (file, _) = adtb_harness::common::open(&rt, &mut io, FILE, false).unwrap();
        let at = DATA.range(past).start as u64;
        let mut page = adtb_harness::common::read(&rt, &mut io, file, at, API_PAGE).unwrap();
        page[300] ^= 0x10;
        adtb_harness::common::write(&rt, &mut io, file, at, &page).unwrap();
        adtb_harness::common::sync(&rt, &mut io, file).unwrap();
        for p in 0..4096 {
            db.testing()
                .probe()
                .page(Pid(1 << 16 | p))
                .expect("a page never written");
        }
        let mut t = db.begin(&TxnOptions::default()).unwrap();
        api::in_txn_results(&mut t, r#"insert users { name: "zed" }"#);
        assert_eq!(
            None,
            db.testing().probe().records().pages().get(&past),
            "nothing read the page yet"
        );
        let what = quarantined(&t.commit(None).unwrap_err()).to_owned();
        assert_eq!(
            Some(&Damage::Checksum),
            db.testing().probe().records().pages().get(&past)
        );
        assert_eq!("the database", what);
        assert_eq!(BTreeSet::from([Quarantine::Database]), marks(&db));
    });
}

#[test]
fn damage_another_thread_records_on_a_page_the_commits_walk_reads_marks_only_its_index() {
    let mut during = 0;
    for step in [0u64, 10, 20, 30, 40, 45, 50, 55, 57] {
        let mut sim = Sim::new(0, Config::default());
        process::run(&mut sim, |rt| {
            let db = open_app(rt.clone());
            seed(&db);
            let (google_page, leaf) = (google(&db), user_index(&db));
            let google = mark_named(&db, "app::identities", "Google");
            let mut t = db.begin(&TxnOptions::default()).unwrap();
            api::in_txn_results(&mut t, GOOGLE_901);
            db.testing().flush();
            for p in 0..4096 {
                db.testing()
                    .probe()
                    .page(Pid(1 << 16 | p))
                    .expect("a page never written");
            }
            db.testing()
                .plant(Plant::Refuse {
                    page: google_page,
                    damage: Damage::Checksum,
                })
                .unwrap();
            let before = db.testing().probe().walks();
            let walked_then = Arc::new(Mutex::new(None));
            let (other, rt2, then) = (db.clone(), rt.clone(), walked_then.clone());
            let detecting = rt.spawn("detecting", move || {
                let nap = Duration::from_micros(step * step * 40);
                adtb_harness::common::sleep(&rt2, &mut rt2.io(), nap);
                *then.lock().unwrap() = Some(other.testing().probe().walks());
                other
                    .testing()
                    .plant(Plant::Refuse {
                        page: leaf,
                        damage: Damage::Checksum,
                    })
                    .unwrap();
            });
            let what = quarantined(&t.commit(None).unwrap_err()).to_owned();
            rt.join(detecting);
            let then = walked_then.lock().unwrap().expect("the thread ran");
            if then == before + 1 {
                during += 1;
            }
            assert_ne!("the database", what, "step {step}");
            let index = user_index_mark(&db);
            assert_eq!(BTreeSet::from([google, index]), marks(&db), "step {step}");
            assert_eq!(
                Some(BTreeSet::from([index])),
                db.testing().probe().mapped_to(leaf),
                "step {step}"
            );
        });
    }
    println!("{during} of 9 runs recorded the damage during the commit's walk");
    adtb_harness::seeds::assert_reached("a record appended during the commit's walk", during, 1);
}

#[test]
fn a_commit_walks_no_record_whose_last_mapping_was_incomplete() {
    let mut sim = Sim::new(65, Config::default());
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let t = db.testing();
        let unheld = t
            .plant(Plant::Orphan {
                kind: adtb::testing::PageKind::Quarantine,
                owner: 0,
            })
            .unwrap()
            .page()
            .unwrap();
        let root = t
            .owned(t.heap("app::identities", "Apple").unwrap())
            .unwrap()[0];
        db.testing()
            .plant(Plant::Refuse {
                page: root,
                damage: Damage::Unreadable,
            })
            .unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: unheld,
                damage: Damage::Checksum,
            })
            .unwrap();
        let mut walked = vec![t.probe().walks()];
        for i in 0..3 {
            api::results(&db, &format!(r#"insert users {{ name: "w{i}" }}"#));
            walked.push(t.probe().walks());
        }
        assert_eq!(None, t.probe().mapped_to(unheld));
        assert_eq!(BTreeSet::new(), marks(&db));
        let first = walked[0] + 1;
        assert_eq!(vec![walked[0], first, first, first], walked);
        let detail = format!(
            "{:?}",
            PageError {
                pid: root,
                damage: Damage::Unreadable
            }
        );
        let failed = Error::Fatal {
            reason: FatalReason::Storage { detail },
        };
        assert_eq!(failed, api::failure(&db, ALL));
        assert_eq!(
            first + 1,
            t.probe().walks(),
            "a failing call walks it again"
        );
    });
}

#[test]
fn a_mark_a_crash_loses_before_it_is_written_is_set_again_by_the_next_open() {
    let mut sim = Sim::new(99, Config::default());
    let (pid, google) = process::run(&mut sim, |rt| {
        let db = open_app(rt);
        seed(&db);
        let pid = google(&db);
        let mark = mark_named(&db, "app::identities", "Google");
        db.close().unwrap();
        (pid, mark)
    });
    adtb_harness::common::run_then_crash(&mut sim, |rt, _| {
        let db = open_app(rt.clone());
        db.testing()
            .plant(Plant::Refuse {
                page: pid,
                damage: Damage::Checksum,
            })
            .unwrap();
        assert_eq!(BTreeSet::new(), marks(&db));
    });
    process::run(&mut sim, |rt| {
        let db = open_app(rt);
        assert_eq!(BTreeSet::from([google]), marks(&db));
        db.close().unwrap();
    });
}
