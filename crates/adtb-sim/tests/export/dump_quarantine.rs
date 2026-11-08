//! Dumps of a quarantined database: a file quarantined whole dumps what it can read whole, and a
//! dump that cannot read a relation it holds fails with `Quarantined` and writes no dump.

use std::collections::BTreeSet;

use adtb::catalog::Catalog;
use adtb::storage::{Damage, Finding, HEADER_LEN, Options, Pager, Pid, Quarantine};
use adtb::testing::{Object, Plant};
use adtb::{ArchiveTarget, CallOptions, Database, Error, FatalReason, Log, RestorePoint};
use adtb_harness::damage::DataFile;
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_simkit::open;
use adtb_simkit::{api, harness};

const FILE: &str = "app.adt";
const COPY: &str = "copy.adt";
const APP: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
  index (name)
add type Shape { name: Text } = Circle { r: I64 } | Square { side: I64 }
add relation shapes : Shape key name
";
const ROWS: &str = r#"transact
  insert users { name: "ann" }
  insert users { name: "bo" }
  insert shapes.Circle { name: "c", r: 2 }
  insert shapes.Square { name: "s", side: 3 }"#;
const READS: [&str; 2] = ["app::users", "app::shapes"];
const DUMP: &str = "dump to \"d\"";
const PAGE: usize = 8192;

fn seeded(rt: &SimRuntime) -> (Database<SimRuntime>, Vec<Vec<String>>) {
    let db = open::opened(rt, FILE, &Log::new([APP]), &open::options(2));
    api::results(&db, ROWS);
    let rows = READS.map(|r| api::query(&db, r)).to_vec();
    (db, rows)
}

fn quarantined(got: Result<Vec<adtb::DynRows>, Error>) -> String {
    match got {
        Err(Error::Fatal {
            reason: FatalReason::Quarantined { what },
        }) => what,
        other => panic!("not Quarantined: {other:?}"),
    }
}

fn run(db: &Database<SimRuntime>, text: &str) -> Result<Vec<adtb::DynRows>, Error> {
    db.query_text(text, &[], &CallOptions::default())
}

fn names(sim: &Sim) -> BTreeSet<String> {
    sim.files().into_keys().collect()
}

/// Run `body` as one process of `sim`, asserting it leaves the names of the files as they were.
fn writes_nothing(sim: &mut Sim, body: impl FnOnce(SimRuntime)) {
    let before = names(sim);
    api::run(sim, body);
    assert_eq!(before, names(sim));
}

/// Returns a simulator over `FILE` holding [`ROWS`], and the page `pick` picks of it.
fn seeded_sim<T>(seed: u64, pick: impl FnOnce(&Database<SimRuntime>) -> T) -> (Sim, T) {
    let mut sim = Sim::new(seed, Config::default());
    let picked = api::run(&mut sim, |rt| {
        let (db, _) = seeded(&rt);
        let picked = pick(&db);
        db.close().unwrap();
        picked
    });
    (sim, picked)
}

fn last_status_page(db: &Database<SimRuntime>) -> Pid {
    *db.testing()
        .commit_log()
        .status()
        .values()
        .next_back()
        .unwrap()
}

#[test]
fn a_file_quarantined_whole_dumps_and_restores_every_row() {
    let mut sim = Sim::new(1, Config::default());
    let rows = api::run(&mut sim, |rt| {
        let (db, rows) = seeded(&rt);
        db.testing()
            .plant(Plant::Mark(Quarantine::Database))
            .unwrap();
        db.close().unwrap();
        rows
    });
    let before = names(&sim);
    api::run(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        run(&db, DUMP).unwrap();
        let refused = [
            "from u in users select u.name",
            "insert users { name: \"cy\" }",
            "print schema",
            "vacuum users",
            "backup to \"bk\"",
            "dump to \"e\"\nverify",
            "verify\ndump to \"e\"",
            "dump to \"e\"\nfrom u in users select u.name",
            "transact\n  dump to \"e\"",
        ];
        for text in refused {
            assert_eq!("the database", quarantined(run(&db, text)), "`{text}`");
        }
        db.close().unwrap();
        let archive = ArchiveTarget::path("d").unwrap();
        adtb::restore_in(
            rt.clone(),
            &archive,
            COPY,
            RestorePoint::Latest,
            &open::options(2),
        )
        .unwrap();
        let copy = open::opened(&rt, COPY, &Log::new([APP]), &open::options(2));
        assert_eq!(rows, READS.map(|r| api::query(&copy, r)).to_vec());
        copy.close().unwrap();
    });
    let written = names(&sim).into_iter().filter(|n| !before.contains(n));
    let stray = written.filter(|n| !n.starts_with("d/") && !n.starts_with(COPY));
    assert_eq!(Vec::<String>::new(), stray.collect::<Vec<_>>());
}

#[test]
fn a_damaged_heap_page_fails_the_dump_of_a_file_quarantined_whole() {
    let (mut sim, square) = seeded_sim(2, |db| {
        let t = db.testing();
        t.pages(t.heap("app::shapes", "Square").unwrap()).unwrap()[0]
    });
    writes_nothing(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        db.testing()
            .plant(Plant::Mark(Quarantine::Database))
            .unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page: square,
                damage: Damage::Checksum,
            })
            .unwrap();
        for _ in 0..2 {
            assert_eq!("app::shapes.Square", quarantined(run(&db, DUMP)));
        }
        db.close().unwrap();
    });
}

#[test]
fn a_damaged_commit_log_fails_the_dump_that_meets_it() {
    let (mut sim, page) = seeded_sim(3, last_status_page);
    writes_nothing(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        db.testing()
            .plant(Plant::Refuse {
                page,
                damage: Damage::Checksum,
            })
            .unwrap();
        assert_eq!("the database", quarantined(run(&db, DUMP)));
        let insert = run(&db, "insert users { name: \"cy\" }");
        assert_eq!("the database", quarantined(insert));
        assert_eq!("the database", quarantined(run(&db, DUMP)));
        db.close().unwrap();
    });
}

#[test]
fn a_quarantined_heap_fails_the_dump_and_a_quarantined_index_does_not() {
    let (mut sim, ()) = seeded_sim(4, |db| {
        let t = db.testing();
        let index = t
            .mark_of(t.index("app::users", "index (name)").unwrap())
            .unwrap();
        t.plant(Plant::Mark(index)).unwrap();
        run(db, DUMP).unwrap();
        let square = t.mark_of(t.heap("app::shapes", "Square").unwrap()).unwrap();
        t.plant(Plant::Mark(square)).unwrap();
    });
    writes_nothing(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        let got = run(&db, "dump to \"e\"");
        assert_eq!("app::shapes.Square", quarantined(got));
        db.close().unwrap();
    });
}

/// Returns `sim` with page `page` of `FILE` flipped on disk, or set to its bytes in `from`.
fn rewritten(seed: u64, sim: &Sim, page: Pid, from: Option<&Sim>) -> Sim {
    let mut files = sim.files();
    let data = DataFile::new(FILE, PAGE);
    match from {
        Some(old) => data.replace(&mut files, page, data.page(&old.files(), page)),
        None => data.flip(&mut files, page, 300 - HEADER_LEN),
    }
    Sim::with_files(seed, Config::default(), files)
}

#[test]
fn a_file_whose_catalog_cannot_be_read_refuses_the_dump() {
    let (sim, root) = seeded_sim(5, |db| db.testing().root(Object::Catalog).unwrap());
    let mut sim = rewritten(5, &sim, root, None);
    writes_nothing(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        assert_eq!("the database", quarantined(run(&db, DUMP)));
        db.close().unwrap();
    });
}

#[test]
fn a_damaged_page_of_the_deployment_facts_fails_the_dump_as_storage_and_marks_nothing() {
    let (mut sim, ()) = seeded_sim(6, |_| ());
    let mut roots = None;
    harness::finished(&mut sim, |rt| {
        let options = Options {
            page_size: PAGE as u32,
            threads: 2,
            ..Options::new(FILE)
        };
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        let (catalog, _) = Catalog::open(rt, &pager).unwrap();
        roots = Some(catalog.deployment().roots().owned_pages(&pager).unwrap()[0]);
    });
    writes_nothing(&mut sim, |rt| {
        let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
        let page = roots.unwrap();
        db.testing()
            .plant(Plant::Refuse {
                page,
                damage: Damage::Checksum,
            })
            .unwrap();
        let detail = format!("PageError {{ pid: Pid({}), damage: Checksum }}", page.0);
        match run(&db, DUMP) {
            Err(Error::Fatal {
                reason: FatalReason::Storage { detail: got },
            }) => assert_eq!(detail, got),
            other => panic!("not Storage: {other:?}"),
        }
        assert!(db.testing().probe().marks().is_empty());
        db.close().unwrap();
    });
}

/// A change to a page the schema or row visibility is read from. Returns the page.
type Change = fn(&Database<SimRuntime>) -> Pid;

const CHANGES: [Change; 2] = [
    |db| {
        api::results(db, "insert users { name: \"cy\" }");
        last_status_page(db)
    },
    |db| {
        db.role_authority().create_role("later").unwrap();
        let t = db.testing();
        *t.pages(t.heap("catalog::roles", "Role").unwrap())
            .unwrap()
            .last()
            .unwrap()
    },
];

#[test]
fn a_lost_write_to_the_commit_log_or_catalog_refuses_the_dump_of_a_file_quarantined_whole() {
    for (n, change) in CHANGES.into_iter().enumerate() {
        let (mut sim, ()) = seeded_sim(7, |_| ());
        let before = Sim::with_files(7, Config::default(), sim.files());
        let page = api::run(&mut sim, |rt| {
            let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
            let page = change(&db);
            db.testing()
                .plant(Plant::Mark(Quarantine::Database))
                .unwrap();
            db.close().unwrap();
            page
        });
        let mut sim = rewritten(7, &sim, page, Some(&before));
        writes_nothing(&mut sim, |rt| {
            let db = open::opened(&rt, FILE, &Log::new([APP]), &open::options(2));
            assert_eq!("the database", quarantined(run(&db, DUMP)), "change {n}");
            let found = db.testing().verify_pages().findings;
            let lost = |f: &Finding| matches!(f, Finding::Page { pid, .. } if *pid == page);
            assert!(found.iter().any(lost), "change {n}: {found:?}");
            db.close().unwrap();
        });
    }
}
