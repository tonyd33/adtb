//! An open of a database quarantined whole: a log with migrations past the file's is refused with
//! the quarantine, a diverged log with `Diverged`, a missing or changed host function as at any
//! open, and the file's own log opens.

use adtb_harness::damage::DataFile;
use adtb_harness::process::{self, finished};
use adtb_sim::{Config, Sim, SimRuntime};

use crate::batch::Batch;
use crate::storage::{Pid, Quarantine};
use crate::test_world::load as open;
use crate::testing::{Object, Plant};
use crate::{
    CallOptions, Database, Determinism, Error, FatalReason, HostFns, Log, OpenError, OpenOptions,
};

const FILE: &str = "marked.adt";
const DATA: DataFile = DataFile::new(FILE, 8192);
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";
const MORE: &str = r"module more
add module more
add type T = { id: Key ts }
add relation ts : T key id generate sequence
";
const OTHER: &str = r"module more
add module more
add type U = { id: Key us }
add relation us : U key id generate sequence
";
const ELSE: &str = r"module app
add module app
add type User = { id: Key people, name: Text }
add relation people : User key id generate sequence
";
const HOSTED: &str = r#"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
define fn label (t: Text) -> Text = host "label" version "1.0" deterministic
add constraint labelled =
  from u in users
  where label(u.name) != u.name
  select u
"#;
const AGED: &str = "module app\nadd field User.age: I64 fill 0\n";
const RELABEL: &str = r#"module app
redefine fn label (t: Text) -> Text = host "label" version "2.0" deterministic
"#;
const INSERT: &str = r#"insert users { name: "ann" }"#;
const SELECT: &str = "from u in users select u.name";

fn the_database() -> Error {
    Error::Fatal {
        reason: FatalReason::Quarantined {
            what: "the database".into(),
        },
    }
}

fn run(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

fn modules(db: &Database<SimRuntime>) -> Vec<String> {
    let s = db.schema();
    let mut names: Vec<String> = s
        .relations
        .iter()
        .map(|r| s.module(r.module).name.to_string())
        .filter(|m| m == "app" || m == "more")
        .collect();
    names.dedup();
    names
}

/// Returns host functions holding `label` at `version`, answering its argument.
fn label(version: &str) -> HostFns {
    let mut fns = HostFns::new();
    fns.register("label", version, Determinism::Deterministic, |b: &Batch| {
        Ok(b.clone())
    });
    fns
}

fn hosted(fns: HostFns) -> OpenOptions {
    OpenOptions {
        host_fns: fns,
        ..open::options(2)
    }
}

/// Returns a simulator over a file holding `log` and one row of `users`, opened with `options`,
/// marked whole by `mark`.
fn marked(seed: u64, mark: Mark, log: &[&str], options: &OpenOptions) -> Sim {
    let log = Log::new(log.iter().copied());
    let mut sim = Sim::new(seed, Config::default());
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &log, options);
        run(&db, INSERT).unwrap();
        if mark == Mark::Planted {
            db.testing()
                .plant(Plant::Mark(Quarantine::Database))
                .unwrap();
        }
        db.close().unwrap();
    });
    if mark == Mark::Planted {
        return sim;
    }
    let mut files = sim.files();
    DATA.zero(&mut files, Pid::FIRST_DATA);
    let mut sim = Sim::with_files(seed, Config::default(), files);
    finished(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &log, options);
        assert_eq!(Err(the_database()), run(&db, INSERT), "{mark:?}");
        assert!(
            db.testing().probe().marked(Quarantine::Database),
            "{mark:?}"
        );
        db.close().unwrap();
    });
    sim
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mark {
    /// The mark set alone, with no damage recorded.
    Planted,
    /// The mark a zeroed bitmap page gives, its damage recorded.
    Damaged,
}

const MARKS: [(u64, Mark); 2] = [(1, Mark::Planted), (2, Mark::Damaged)];

#[test]
fn an_open_under_a_whole_mark_refuses_migrations_past_the_files_and_opens_with_its_own() {
    let mut ran = 0;
    for (seed, mark) in MARKS {
        let mut sim = marked(seed, mark, &[APP], &open::options(2));
        finished(&mut sim, |rt| {
            let got =
                Database::open_in(rt.clone(), FILE, &Log::new([APP, MORE]), &open::options(2));
            let Err(OpenError::MigrationRefused { version, reason }) = got else {
                panic!(
                    "{mark:?}: expected the quarantine refusing migration 2, found {:?}",
                    got.map(drop)
                );
            };
            assert_eq!((2, the_database()), (version, reason), "{mark:?}");
            let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
            assert_eq!(vec!["app".to_owned()], modules(&db), "{mark:?}");
            assert_eq!(Err(the_database()), run(&db, SELECT), "{mark:?}");
            db.close().unwrap();
        });
        ran += 1;
    }
    assert_eq!(2, ran);
}

#[test]
fn an_open_under_a_whole_mark_refuses_a_diverged_log_of_any_length() {
    let mut ran = 0;
    for (seed, mark) in MARKS {
        let mut sim = marked(seed, mark, &[APP, MORE], &open::options(2));
        finished(&mut sim, |rt| {
            for (log, first) in [
                (vec![APP, OTHER], 2),
                (vec![ELSE], 1),
                (vec![APP, OTHER, MORE], 2),
            ] {
                let n = log.len();
                let got = Database::open_in(rt.clone(), FILE, &Log::new(log), &open::options(2));
                let Err(OpenError::Diverged { version }) = got else {
                    panic!(
                        "{mark:?}, {n} migrations: expected Diverged, found {:?}",
                        got.map(drop)
                    );
                };
                assert_eq!(first, version, "{mark:?}, {n} migrations");
            }
            let db = open::opened(&rt, FILE, &Log::new([APP, MORE]), &open::options(2));
            assert_eq!(
                vec!["app".to_owned(), "more".to_owned()],
                modules(&db),
                "{mark:?}"
            );
            db.close().unwrap();
        });
        ran += 1;
    }
    assert_eq!(2, ran);
}

#[test]
fn an_open_under_a_whole_mark_binds_host_functions_as_on_any_file() {
    let mut ran = 0;
    for (seed, mark) in MARKS {
        let mut sim = marked(seed, mark, &[HOSTED], &hosted(label("1.0")));
        finished(&mut sim, |rt| {
            let log = Log::new([HOSTED]);
            let got = Database::open_in(rt.clone(), FILE, &log, &open::options(2));
            let Err(OpenError::HostFnMissing { name }) = got else {
                panic!(
                    "{mark:?}: expected label missing, found {:?}",
                    got.map(drop)
                );
            };
            assert_eq!("label", name, "{mark:?}");
            let got = Database::open_in(rt.clone(), FILE, &log, &hosted(label("2.0")));
            let Err(OpenError::HostFnMismatch { name }) = got else {
                panic!(
                    "{mark:?}: expected label mismatched, found {:?}",
                    got.map(drop)
                );
            };
            assert_eq!("label", name, "{mark:?}");
            let relabelled = Log::new([HOSTED, RELABEL]);
            let got = Database::open_in(rt.clone(), FILE, &relabelled, &hosted(label("2.0")));
            let Err(OpenError::MigrationRefused { version, reason }) = got else {
                panic!(
                    "{mark:?}: expected the quarantine refusing the redefinition, found {:?}",
                    got.map(drop)
                );
            };
            assert_eq!((2, the_database()), (version, reason), "{mark:?}");
            open::opened(&rt, FILE, &log, &hosted(label("1.0")))
                .close()
                .unwrap();
        });
        ran += 1;
    }
    assert_eq!(2, ran);
}

#[test]
fn a_migration_an_open_cannot_commit_for_damage_is_refused_with_the_quarantine() {
    let mut sim = Sim::new(3, Config::default());
    let pages = process::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        let name = "x".repeat(30_000);
        run(&db, &format!("insert users {{ name: \"{name}\" }}")).unwrap();
        let t = db.testing();
        let Some(Object::Heap(users)) = t.heap("app::users", "User") else {
            panic!("the heap of users");
        };
        let pages = t.pages(Object::Large(users)).unwrap();
        db.close().unwrap();
        pages
    });
    let mut files = sim.files();
    for page in &pages {
        DATA.flip(&mut files, *page, 100);
    }
    let mut sim = Sim::with_files(3, Config::default(), files);
    finished(&mut sim, |rt| {
        let got = Database::open_in(rt.clone(), FILE, &Log::new([APP, AGED]), &open::options(2));
        let Err(OpenError::MigrationRefused { version, reason }) = got else {
            panic!(
                "expected the quarantine refusing migration 2, found {:?}",
                got.map(drop)
            );
        };
        assert_eq!((2, the_database()), (version, reason));
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        assert_eq!(vec!["app".to_owned()], modules(&db));
        assert_eq!(Err(the_database()), run(&db, SELECT));
        db.close().unwrap();
    });
}
