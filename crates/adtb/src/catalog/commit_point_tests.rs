//! A step that fails after its change's main update. After a transaction's durable commit the
//! caller is told it succeeded and the next open finishes the step; before it, the call fails,
//! nothing of it is seen, every release still ran, and every count of a large value equals the
//! versions pointing at it. Damage an open meets after its migrations committed quarantines the
//! database, and the migrations stay applied. A refused change whose cleanup fails is told its
//! refusal and leaves its mark on the catalog root for the next open.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};

use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process::run;
use adtb_testkit::profile;
use adtb_testkit::search::Files;

use super::codec::testing;
use super::root::{Changing, Root};
use super::{Catalog, Read, Reader, Rewrite, Transactions};
use crate::faults::Step;
use crate::faults::armed::{self, Run, Site};
use crate::heap::{self, Heap, Tested, Undo};
use crate::key::{CtorId, Tid, Value};
use crate::mvcc::{Begun, Fate, StatusError, Writers};
use crate::storage::{self, Pager};
use crate::storage::{Quarantine, Walk};
use crate::test_world;
use crate::test_world::api::{printed, query};
use crate::test_world::fixture::Storage;
use crate::test_world::grants;
use crate::test_world::load::{FILE, open_in, opened, options};
use crate::test_world::rows::{EMAIL, key_of, row_type};
use crate::testing::Plant;
use crate::verify::{self, Depth, Finding, Inventory, Problem, Sum};
use crate::{
    CallOptions, Database, Error, FatalReason, Log, OpenError, OpenOptions, Registration,
    RejectReason,
};

const SEED: u64 = 71;

const NOTES: &str = r"module app
add module app
add type Note = { id: Key notes, title: Text, body: Text }
add relation notes : Note key id generate sequence
add index (title) in notes
";

const RANKED: &str = "module app\ndrop index (title) in notes\nadd field Note.rank: I64 fill 7\n";

const UNIQUE: &str = "module app\nadd unique (title) in notes\n";
const DROP_UNIQUE: &str = "module app\ndrop unique (title) in notes\n";

const ROWS: usize = 4;

const WIDE: &str = r"module app
add module app
add type Wide = { id: Key wides, a: Text, b: Text, c: Text, d: Text }
add relation wides : Wide key id generate sequence
";

/// The length of each field of a row of `wides`: one value to a store page.
const WIDE_LEN: usize = 5000;

fn body(i: usize) -> String {
    format!("note-{i}-{}", "x".repeat(3000))
}

/// Returns the operator log of the migrations `texts`.
fn log(texts: &[&str]) -> Log {
    Log::new(texts.iter().copied())
}

fn call(db: &Database<SimRuntime>, text: &str) -> Result<(), Error> {
    db.query_text(text, &[], &CallOptions::default()).map(drop)
}

fn write(db: &Database<SimRuntime>, text: &str) {
    call(db, text).unwrap_or_else(|e| panic!("`{}` fails: {e:?}", &text[..text.len().min(60)]));
}

/// Returns the text inserting [`ROWS`] rows into `notes`, with the fields `more` writes beside
/// each title and body.
fn inserts(more: &str) -> String {
    let rows: String = (0..ROWS)
        .map(|i| {
            format!(
                "  insert notes {{ title: \"t{i}\", body: \"{}\"{more} }}\n",
                body(i)
            )
        })
        .collect();
    format!("transact\n{rows}")
}

/// Returns a simulation whose file a create at the migrations `texts` left closed. The files of
/// each list of migrations are made once.
fn created_at(texts: &[&'static str]) -> Sim {
    static FILES: Mutex<BTreeMap<Vec<&'static str>, Files>> = Mutex::new(BTreeMap::new());
    let files = FILES
        .lock()
        .expect("no panic while the files are held")
        .entry(texts.to_vec())
        .or_insert_with(|| {
            let mut sim = Sim::new(SEED, Config::default());
            run(&mut sim, |rt| {
                let db = opened(&rt, FILE, &log(texts), &options(1));
                db.close().expect("the database closes");
            });
            sim.files()
        })
        .clone();
    Sim::with_files(SEED, Config::default(), files)
}

/// Arm `fail` and forget the runs of steps so far on this thread.
fn arm(fail: Option<(Site, usize)>) {
    armed::reached();
    if let Some((site, n)) = fail {
        armed::fail(site, n);
    }
}

fn catalog_at(rt: &SimRuntime) -> (Pager<SimRuntime>, Catalog, Transactions<SimRuntime>) {
    let options = storage::Options {
        page_size: 8192,
        ..storage::Options::new(FILE)
    };
    let (pager, _) = Pager::open(rt.clone(), &options).expect("the file reopens");
    let (catalog, txns) = Catalog::open(rt.clone(), &pager).expect("the catalog opens");
    (pager, catalog, txns)
}

fn changing(pager: &Pager<SimRuntime>) -> bool {
    let root = pager
        .read(pager.catalog_root())
        .expect("the catalog root reads");
    Root::read(&root).changing() != Changing::None
}

/// What verify finds on the closed file once an open ended any change on the catalog root.
#[derive(Debug, PartialEq, Eq)]
struct Findings {
    overcounted: usize,
    unsettled: usize,
}

/// Returns the overcounted heads and unsettled directory entries verify finds on the closed file
/// once an open ended any change on the catalog root, failing unless it read every structure and
/// found nothing else.
fn findings(rt: &SimRuntime, what: &str) -> Findings {
    let (pager, catalog, txns) = catalog_at(rt);
    assert!(!changing(&pager), "{what}: the open ends the change");
    let clog = txns.commit_log();
    let depth = Depth {
        log: true,
        indexes: true,
    };
    let report = verify::verify_with(&pager, &catalog.structures().inventory(Some(&clog)), depth);
    pager.close();
    assert!(report.complete, "{what}: {:?}", report.findings);
    counted(&report.findings, what)
}

/// Returns the overcounted heads and unsettled directory entries among `findings`, failing on
/// any other.
fn counted(findings: &[Finding], what: &str) -> Findings {
    let mut found = Findings {
        overcounted: 0,
        unsettled: 0,
    };
    for f in findings {
        match f.problem {
            Problem::Overcounted { .. } => found.overcounted += 1,
            Problem::Unsettled(_) => found.unsettled += 1,
            ref other => panic!("{what}: {other:?}"),
        }
    }
    found
}

/// What one run of a change did: the runs of steps it reached, the failure the caller was told,
/// whether a reopen finds the change, and what verify found after the reopen.
struct Ran {
    runs: Vec<Run>,
    failed: Option<String>,
    /// The failure of a read of `notes` on the database an open answered.
    answered: Option<Error>,
    /// `None` for a change with no commit, which fails at every failed step.
    durable: Option<bool>,
    found: Findings,
    /// The heads verify finds overcounted before an open ends a change on the catalog root, and
    /// whether the root records one.
    unmended: (usize, bool),
}

/// Returns the heads verify finds overcounted on the closed file before any open ends a change
/// the catalog root records, and whether it records one.
fn unmended(rt: &SimRuntime) -> (usize, bool) {
    let options = storage::Options {
        page_size: 8192,
        ..storage::Options::new(FILE)
    };
    let (pager, _) = Pager::open(rt.clone(), &options).expect("the file reopens");
    let (catalog, txns) = Catalog::load(rt.clone(), &pager).expect("the catalog loads");
    let clog = txns.commit_log();
    let depth = Depth {
        log: true,
        indexes: true,
    };
    let report = verify::verify_with(&pager, &catalog.structures().inventory(Some(&clog)), depth);
    let left = changing(&pager);
    pager.close();
    assert!(report.complete, "{:?}", report.findings);
    let over = report.findings.iter();
    let over = over.filter(|f| matches!(f.problem, Problem::Overcounted { .. }));
    (over.count(), left)
}

/// Returns the number of migrations the file's catalog holds.
fn version(rt: &SimRuntime) -> usize {
    let (pager, catalog, _) = catalog_at(rt);
    let version = catalog.source().log().len();
    pager.close();
    version
}

/// Returns the notes ranked 7 an open at [`RANKED`] reads, failing unless verify answers no row.
fn ranked(rt: &SimRuntime, what: &str) -> usize {
    let db = opened(rt, FILE, &log(&[NOTES, RANKED]), &options(1));
    let ranked = query(&db, "from n in notes where n.rank == 7 select n.id");
    assert_eq!(
        Vec::<String>::new(),
        printed(db.verify_holding().expect("verify runs")),
        "{what}"
    );
    db.close().expect("the database closes");
    ranked.len()
}

/// The read [`told_at`] makes of the database an open answers.
const NOTE_IDS: &str = "from n in notes select n.id";

/// Open the file at the migrations `texts`. Returns the failure of the open, or of a read of
/// `notes` on the database it answers.
fn told_at(rt: &SimRuntime, texts: &[&str]) -> (Option<String>, Option<Error>) {
    match open_in(rt, FILE, &log(texts), &options(1)) {
        Ok(db) => {
            let answered = call(&db, NOTE_IDS).err();
            db.close().expect("the database closes");
            (None, answered)
        }
        Err(e) => (Some(e.to_string()), None),
    }
}

/// Returns the marks of `marks` the file holds as a recovered pager reads them.
fn marks_left(rt: &SimRuntime, marks: &BTreeSet<Quarantine>) -> BTreeSet<Quarantine> {
    let options = storage::Options {
        page_size: 8192,
        ..storage::Options::new(FILE)
    };
    let (pager, _) = Pager::open(rt.clone(), &options).expect("the file reopens");
    let left = pager.quarantined().intersection(marks).copied().collect();
    pager.close();
    left
}

/// Open the file at [`NOTES`] with [`ROWS`] rows and both its indexes marked, then at
/// [`RANKED`], which drops one and builds the other again, failing `fail`. Fail unless the marks
/// are gone exactly when the migration is durable, unless a sweep after the next open leaves the
/// store as large as before the migration, when `fail` is not a settle, or, when the open answers
/// a database quarantined whole, unless the next open answers it too.
fn migrated(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("migration {fail:?}");
    let mut sim = created_at(&[NOTES]);
    let marks = run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES]), &options(1));
        write(&db, &inserts(""));
        let t = db.testing();
        let marks: BTreeSet<Quarantine> = ["key id", "index (title)"]
            .into_iter()
            .map(|clause| {
                let index = t.index("app::notes", clause).expect("an index of notes");
                let mark = t.own_mark(index).expect("the index's mark");
                t.plant(Plant::Mark(mark)).expect("the mark is set");
                mark
            })
            .collect();
        db.close().expect("the database closes");
        marks
    });
    let held = run(&mut sim, |rt| swept(&rt));
    let (runs, failed, answered) = run(&mut sim, |rt| {
        arm(fail);
        let (failed, answered) = told_at(&rt, &[NOTES, RANKED]);
        (armed::reached(), failed, answered)
    });
    let left = run(&mut sim, |rt| marks_left(&rt, &marks));
    let unmended = run(&mut sim, |rt| unmended(&rt));
    let durable = run(&mut sim, |rt| version(&rt) == 2);
    let expected = if durable { BTreeSet::new() } else { marks };
    assert_eq!(expected, left, "{what}: the marks of the indexes");
    if answered.is_some() {
        let again = run(&mut sim, |rt| told_at(&rt, &[NOTES, RANKED]));
        assert_eq!((None, Some(quarantined())), again, "{what}");
        return Ran {
            runs,
            failed,
            answered,
            durable: Some(durable),
            found: Findings {
                overcounted: 0,
                unsettled: 0,
            },
            unmended,
        };
    }
    let found = run(&mut sim, |rt| {
        assert_eq!(ROWS, ranked(&rt, &what), "{what}");
        findings(&rt, &what)
    });
    let swept = run(&mut sim, |rt| swept(&rt));
    if fail.is_none_or(|(site, _)| site.step() != Step::Settle) {
        assert_eq!(held, swept, "{what}");
    }
    Ran {
        runs,
        failed,
        answered,
        durable: Some(durable),
        found,
        unmended,
    }
}

/// Create the file at [`NOTES`], failing `fail`. Fail unless an open the failure quarantined
/// answers the next open quarantined too. A create that leaves no file made nothing durable.
fn created(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("create {fail:?}");
    let mut sim = Sim::new(SEED, Config::default());
    let (runs, failed, answered) = run(&mut sim, |rt| {
        arm(fail);
        let (failed, answered) = told_at(&rt, &[NOTES]);
        (armed::reached(), failed, answered)
    });
    if !sim.files().contains_key(FILE) {
        return Ran {
            runs,
            failed,
            answered,
            durable: Some(false),
            found: Findings {
                overcounted: 0,
                unsettled: 0,
            },
            unmended: (0, false),
        };
    }
    let unmended = run(&mut sim, |rt| unmended(&rt));
    let durable = run(&mut sim, |rt| version(&rt) == 1);
    let found = if answered.is_some() {
        let again = run(&mut sim, |rt| told_at(&rt, &[NOTES]));
        assert_eq!((None, Some(quarantined())), again, "{what}");
        Findings {
            overcounted: 0,
            unsettled: 0,
        }
    } else {
        run(&mut sim, |rt| findings(&rt, &what))
    };
    Ran {
        runs,
        failed,
        answered,
        durable: Some(durable),
        found,
        unmended,
    }
}

/// Insert a role into `catalog::roles` of a file at [`NOTES`] as a deployment write, failing
/// `fail`.
fn deployed(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("deploy {fail:?}");
    let mut sim = created_at(&[NOTES]);
    let (runs, failed) = run(&mut sim, |rt| {
        let (pager, catalog, txns) = catalog_at(&rt);
        let (relation, c) = grants::catalog_relation(catalog.schema(), "roles", "Role");
        let role = testing::row(
            catalog.schema(),
            relation,
            c,
            &[
                ("id", Value::I64(1)),
                ("name", Value::Text("web".into())),
                ("created", Value::Instant(0)),
            ],
        );
        let rewrite = Rewrite {
            relation,
            deleted: Vec::new(),
            inserted: vec![(c, role)],
        };
        arm(fail);
        let failed = catalog.deploy(&pager, &txns, &[rewrite]).err();
        let runs = armed::reached();
        pager.close();
        (runs, failed.map(|e| e.to_string()))
    });
    let unmended = run(&mut sim, |rt| unmended(&rt));
    let durable = run(&mut sim, |rt| {
        let (pager, catalog, txns) = catalog_at(&rt);
        let roles = catalog
            .committed_roles(&pager, &txns)
            .expect("the roles read");
        pager.close();
        roles.values().any(|name| name == "web")
    });
    let found = run(&mut sim, |rt| findings(&rt, &what));
    Ran {
        runs,
        failed,
        answered: None,
        durable: Some(durable),
        found,
        unmended,
    }
}

/// Settle at open the pending drop of a unique whose holder no open registers, failing `fail`.
fn settled(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("pending drop {fail:?}");
    let mut sim = created_at(&[NOTES, UNIQUE]);
    run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES, UNIQUE]), &options(1));
        db.close().expect("the database closes");
        let (pager, mut catalog, txns) = catalog_at(&rt);
        let reader = Reader {
            bundle: "web-2".into(),
            version: 2,
            hides_new: false,
            reads: vec![Read::Relation("app::notes".into())],
            writes: Vec::new(),
        };
        catalog.register(reader).expect("the reader registers");
        catalog
            .migrate(&pager, &txns, DROP_UNIQUE)
            .expect("the drop applies");
        assert_eq!(1, catalog.pending_drops().len());
        pager.close();
    });
    let (runs, failed) = run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        arm(fail);
        let failed = catalog.settle(&pager, &txns).err().map(|e| e.to_string());
        let runs = armed::reached();
        pager.close();
        (runs, failed)
    });
    let unmended = run(&mut sim, |rt| unmended(&rt));
    let durable = run(&mut sim, |rt| {
        let (pager, catalog, _) = catalog_at(&rt);
        let durable = catalog.pending_drops().is_empty();
        pager.close();
        durable
    });
    let found = run(&mut sim, |rt| findings(&rt, &what));
    Ran {
        runs,
        failed,
        answered: None,
        durable: Some(durable),
        found,
        unmended,
    }
}

/// Run `call` on a file a create at `texts` left, opened at `texts` after `before` wrote its
/// rows, failing `fail`. Returns the runs of steps it reached and the failure it was told.
fn called(
    sim: &mut Sim,
    texts: &[&str],
    before: &[String],
    call: &str,
    fail: Option<(Site, usize)>,
) -> (Vec<Run>, Option<String>) {
    run(sim, |rt| {
        let db = opened(&rt, FILE, &log(texts), &options(1));
        for text in before {
            write(&db, text);
        }
        arm(fail);
        let failed = self::call(&db, call).err().map(|e| e.to_string());
        let runs = armed::reached();
        db.close().expect("the database closes");
        (runs, failed)
    })
}

/// Insert [`ROWS`] rows into a file at [`RANKED`], failing `fail`.
fn inserted(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("insert {fail:?}");
    let mut sim = created_at(&[NOTES, RANKED]);
    let (runs, failed) = called(&mut sim, &[NOTES, RANKED], &[], &inserts(", rank: 7"), fail);
    let unmended = run(&mut sim, |rt| unmended(&rt));
    let rows = run(&mut sim, |rt| ranked(&rt, &what));
    assert!(rows == 0 || rows == ROWS, "{what}: {rows} rows");
    let found = run(&mut sim, |rt| findings(&rt, &what));
    Ran {
        runs,
        failed,
        answered: None,
        durable: Some(rows == ROWS),
        found,
        unmended,
    }
}

/// Insert [`ROWS`] rows into a file at [`RANKED`], vacuum, then update each keeping its body,
/// failing `fail`.
fn updated(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("update {fail:?}");
    let mut sim = created_at(&[NOTES, RANKED]);
    let before = [inserts(", rank: 7"), "vacuum notes".into()];
    let update = "update n in notes set { n with rank: 8 }";
    let (runs, failed) = called(&mut sim, &[NOTES, RANKED], &before, update, fail);
    let unmended = run(&mut sim, |rt| unmended(&rt));
    let rows = run(&mut sim, |rt| ranked(&rt, &what));
    assert!(rows == 0 || rows == ROWS, "{what}: {rows} rows");
    let found = run(&mut sim, |rt| findings(&rt, &what));
    Ran {
        runs,
        failed,
        answered: None,
        durable: Some(rows == 0),
        found,
        unmended,
    }
}

/// Insert [`ROWS`] rows of four large values each into `wides`, failing `fail`.
fn widened(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("wide insert {fail:?}");
    let field = |c: char| format!("\"{}\"", c.to_string().repeat(WIDE_LEN));
    let row = format!(
        "  insert wides {{ a: {}, b: {}, c: {}, d: {} }}\n",
        field('a'),
        field('b'),
        field('c'),
        field('d')
    );
    let mut sim = created_at(&[WIDE]);
    let insert = format!("transact\n{}", row.repeat(ROWS));
    let (runs, failed) = called(&mut sim, &[WIDE], &[], &insert, fail);
    if fail.is_none() {
        let retains = runs.iter().filter(|r| r.0.step() == Step::Retain);
        assert_eq!(
            2 * ROWS,
            retains.count(),
            "each row's counts take two updates"
        );
    }
    let unmended = run(&mut sim, |rt| unmended(&rt));
    let rows = run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[WIDE]), &options(1));
        let rows = query(&db, "from w in wides select w.id");
        assert_eq!(
            Vec::<String>::new(),
            printed(db.verify_holding().expect("verify runs")),
            "{what}"
        );
        db.close().expect("the database closes");
        rows.len()
    });
    assert!(rows == 0 || rows == ROWS, "{what}: {rows} rows");
    let found = run(&mut sim, |rt| findings(&rt, &what));
    Ran {
        runs,
        failed,
        answered: None,
        durable: Some(rows == ROWS),
        found,
        unmended,
    }
}

/// Delete every row of a file at [`RANKED`], then vacuum, failing `fail`.
fn vacuumed(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("vacuum {fail:?}");
    let mut sim = created_at(&[NOTES, RANKED]);
    let before = [inserts(", rank: 7"), "delete n in notes".into()];
    let (runs, failed) = called(&mut sim, &[NOTES, RANKED], &before, "vacuum notes", fail);
    let unmended = run(&mut sim, |rt| unmended(&rt));
    assert_eq!(0, run(&mut sim, |rt| ranked(&rt, &what)), "{what}");
    let found = run(&mut sim, |rt| findings(&rt, &what));
    Ran {
        runs,
        failed,
        answered: None,
        durable: None,
        found,
        unmended,
    }
}

/// Apply [`VIOLATED`] to the file at [`NOTES`] with [`ROWS`] rows, failing `fail`: its
/// constraint refuses it after its rewrite made heaps, which the abort frees.
fn aborted(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("aborted migration {fail:?}");
    let mut sim = created_at(&[NOTES]);
    run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES]), &options(1));
        write(&db, &inserts(""));
        db.close().expect("the database closes");
    });
    let (runs, failed) = run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        arm(fail);
        let failed = catalog.migrate(&pager, &txns, VIOLATED).err();
        let runs = armed::reached();
        pager.close();
        (runs, failed.map(|e| e.to_string()))
    });
    let unmended = run(&mut sim, |rt| unmended(&rt));
    let durable = run(&mut sim, |rt| version(&rt) == 2);
    let found = run(&mut sim, |rt| findings(&rt, &what));
    Ran {
        runs,
        failed,
        answered: None,
        durable: Some(durable),
        found,
        unmended,
    }
}

/// Returns the site of the free after a migration's commit.
fn freeing_site() -> Site {
    static SITE: OnceLock<Site> = OnceLock::new();
    *SITE.get_or_init(|| {
        let runs = migrated(None).runs;
        let free = runs.iter().find(|r| r.0.step() == Step::FreeLeaked);
        free.expect("a migration frees what it left").0
    })
}

/// Open the catalog of a file whose migration to [`RANKED`] left its change on the catalog root,
/// failing `fail`. The open's change is the end of the migration's.
fn mended(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("mend at open {fail:?}");
    let left = freeing_site();
    let mut sim = created_at(&[NOTES]);
    run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES]), &options(1));
        write(&db, &inserts(""));
        db.close().expect("the database closes");
        arm(Some((left, 0)));
        assert_eq!((None, None), told_at(&rt, &[NOTES, RANKED]));
        armed::reached();
    });
    let (runs, failed) = run(&mut sim, |rt| {
        let options = storage::Options {
            page_size: 8192,
            ..storage::Options::new(FILE)
        };
        let (pager, _) = Pager::open(rt.clone(), &options).expect("the file reopens");
        assert!(changing(&pager), "{what}: the migration leaves its change");
        arm(fail);
        let opened = Catalog::open(rt, &pager);
        let runs = armed::reached();
        pager.close();
        (runs, opened.err().map(|e| e.to_string()))
    });
    let unmended = run(&mut sim, |rt| unmended(&rt));
    assert_eq!(2, run(&mut sim, |rt| version(&rt)), "{what}");
    let found = run(&mut sim, |rt| findings(&rt, &what));
    Ran {
        runs,
        failed,
        answered: None,
        durable: Some(!unmended.1),
        found,
        unmended,
    }
}

/// A change the search fails at every step it reaches.
struct Case {
    name: &'static str,
    run: fn(Option<(Site, usize)>) -> Ran,
    /// The failure the change is told with nothing failed.
    refused: Option<&'static str>,
    /// The failure the change is told when a failed step keeps it from committing.
    failed: &'static str,
}

const CASES: &[Case] = &[
    Case {
        name: "create",
        run: created,
        refused: None,
        failed: "page 0 is unreadable",
    },
    Case {
        name: "migration",
        run: migrated,
        refused: None,
        failed: "page 0 is unreadable",
    },
    Case {
        name: "aborted migration",
        run: aborted,
        refused: Some(r#"constraint app::unranked is violated by "t0", "t1", "t2", "t3""#),
        failed: "page 0 is unreadable",
    },
    Case {
        name: "mend at open",
        run: mended,
        refused: None,
        failed: "page 0 is unreadable",
    },
    Case {
        name: "deploy",
        run: deployed,
        refused: None,
        failed: "page 0 is unreadable",
    },
    Case {
        name: "pending drop",
        run: settled,
        refused: None,
        failed: "page 0 is unreadable",
    },
    Case {
        name: "insert",
        run: inserted,
        refused: None,
        failed: "fatal: storage failure: page 0 is unreadable",
    },
    Case {
        name: "vacuum",
        run: vacuumed,
        refused: None,
        failed: "fatal: storage failure: page 0 is unreadable",
    },
    Case {
        name: "update",
        run: updated,
        refused: None,
        failed: "fatal: storage failure: page 0 is unreadable",
    },
    Case {
        name: "wide insert",
        run: widened,
        refused: None,
        failed: "fatal: storage failure: page 0 is unreadable",
    },
];

/// Returns the failure every call on a database quarantined whole is told.
fn quarantined() -> Error {
    Error::Fatal {
        reason: FatalReason::Quarantined {
            what: "the database".into(),
        },
    }
}

/// Returns the first and the last run of each site among `runs`.
fn ends(runs: &[Run]) -> BTreeSet<(Site, usize)> {
    let mut last: BTreeMap<Site, usize> = BTreeMap::new();
    for (site, n, _) in runs {
        last.insert(*site, *n);
    }
    last.into_iter()
        .flat_map(|(s, n)| [(s, 0), (s, n)])
        .collect()
}

/// Returns the runs among `runs` the search fails: every one under the deep profile, else the
/// first and the last of each site.
fn searched(runs: &[Run]) -> BTreeSet<(Site, usize)> {
    if profile::every_case() {
        return runs.iter().map(|&(site, n, _)| (site, n)).collect();
    }
    ends(runs)
}

/// Returns whether `got`, a run of `case`, left its refusal's mark on the catalog root, failing
/// unless it was told the refusal.
fn refusal_kept(case: &Case, got: &Ran, what: &str) -> bool {
    let kept = got.unmended.1 && got.durable == Some(false) && case.refused.is_some();
    if kept {
        assert_eq!(case.refused, got.failed.as_deref(), "{what}");
    }
    kept
}

/// Returns the place of `step` in [`Step::EVERY`].
fn listed(step: Step) -> usize {
    match step {
        Step::Retain => 0,
        Step::ClearBit => 1,
        Step::Reflect => 2,
        Step::Write => 3,
        Step::FreeLeaked => 4,
        Step::ReadLeaked => 5,
        Step::EndChange => 6,
        Step::Settle => 7,
        Step::Release => 8,
        Step::Opening => 9,
    }
}

#[test]
fn a_change_tells_its_caller_success_exactly_when_it_is_durable_whatever_step_fails() {
    let every: Vec<usize> = Step::EVERY.iter().map(|s| listed(*s)).collect();
    assert_eq!((0..Step::EVERY.len()).collect::<Vec<_>>(), every);
    let settled = Findings {
        overcounted: 0,
        unsettled: 0,
    };
    let mut reached: BTreeSet<(&str, Step)> = BTreeSet::new();
    let mut lowered = 0;
    let mut kept_refused = 0;
    let mut wrong = Vec::new();
    for case in CASES {
        let name = case.name;
        let clean = (case.run)(None);
        assert_eq!(
            case.refused,
            clean.failed.as_deref(),
            "{name} with nothing failed"
        );
        assert_eq!(None, clean.answered, "{name} with nothing failed");
        assert_eq!(
            case.refused.is_some(),
            clean.durable == Some(false),
            "{name} with nothing failed"
        );
        assert_eq!(settled, clean.found, "{name} with nothing failed");
        for (site, n) in searched(&clean.runs) {
            let what = format!("{name}: run {n} of {site:?}");
            let got = (case.run)(Some((site, n)));
            assert!(
                got.runs.contains(&(site, n, true)),
                "{what} armed and not failed: {:?}",
                got.runs
            );
            let answered = (site.step() == Step::Opening).then(quarantined);
            let durable = got.durable == Some(true);
            let told: Vec<Option<&str>> = match (got.durable, case.refused) {
                (Some(true), _) => vec![None],
                (Some(false), Some(refused)) => vec![Some(case.failed), Some(refused)],
                (Some(false), None) | (None, _) => vec![Some(case.failed)],
            };
            if !told.contains(&got.failed.as_deref()) || answered != got.answered {
                wrong.push(format!(
                    "{what}: durable {durable}, told {:?}, answered {:?}",
                    got.failed, got.answered
                ));
            }
            if site.step() == Step::Opening {
                assert!(durable, "{what}");
            }
            let (unmended, left) = got.unmended;
            kept_refused += usize::from(refusal_kept(case, &got, &what));
            let skipped = usize::from(!left && site.step() == Step::Release);
            assert_eq!(skipped, got.found.overcounted, "{what}");
            if site.step() != Step::Settle {
                assert_eq!(0, got.found.unsettled, "{what}");
            }
            if site.step() == Step::ReadLeaked && unmended > 0 {
                lowered += 1;
            }
            reached.insert((name, site.step()));
        }
    }
    assert_eq!(Vec::<String>::new(), wrong);
    assert!(
        kept_refused > 0,
        "no failed cleanup of a refused migration kept its mark"
    );
    assert!(
        lowered > 0,
        "no failed read of a leaked page left a head overcounted for the open to lower"
    );
    let steps: BTreeSet<Step> = reached.iter().map(|r| r.1).collect();
    assert_eq!(BTreeSet::from(Step::EVERY), steps);
    assert!(
        reached.contains(&("create", Step::Write)),
        "create writes no page"
    );
    for name in ["migration", "aborted migration", "mend at open"] {
        assert!(
            reached.contains(&(name, Step::ReadLeaked)),
            "{name} reads no leaked page"
        );
    }
}

const DOCS: &str = r"module app
add module app
add type Doc { id: Key docs } = Draft { body: Text } | Plain { body: Text }
add relation docs : Doc key id generate sequence
";

const DEPRECATED: &str = "module app\ndeprecate docs.Draft\n";
const REMOVED: &str = "module app\nremove constructor Doc.Draft\n";
const DROPPED: &str = "module app\ndrop relation notes\ndrop type Note\n";
const VIOLATED: &str = r"module app
add field Note.rank: I64 fill 7
add constraint unranked = from n in notes where n.rank == 7 select n.title
";

const UNRANKED: &[&str] =
    &["constraint app::unranked is violated by \"t0\", \"t1\", \"t2\", \"t3\""];

/// Sweep the store of the closed file. Returns the pages it holds after.
fn swept(rt: &SimRuntime) -> usize {
    let (pager, catalog, txns) = catalog_at(rt);
    catalog
        .large
        .sweep(
            &crate::vacuum::Vacuuming::unheld(&pager, &txns),
            |x| txns.resolved(&pager, x),
            &mut || true,
        )
        .expect("the sweep runs");
    let pages = catalog.large.pages(&pager).expect("the store reads").len();
    pager.close();
    pages
}

/// A change that frees a heap: its name, the migrations before it, the writes that fill them,
/// its migrations, the refusal the last ends in, and whether the store keeps a value of each row.
struct Freeing {
    name: &'static str,
    before: &'static [&'static str],
    writes: fn() -> Vec<String>,
    changes: &'static [&'static str],
    refused: Option<&'static [&'static str]>,
    kept: bool,
}

fn notes() -> Vec<String> {
    vec![inserts("")]
}

fn deleted() -> Vec<String> {
    vec![inserts(""), "delete n in notes".into()]
}

fn retagged() -> Vec<String> {
    let rows: String = (0..ROWS)
        .map(|i| format!("  insert docs.Draft {{ body: \"{}\" }}\n", body(i)))
        .collect();
    vec![
        format!("transact\n{rows}"),
        "update d in docs.Draft set Plain { body: d.body }".into(),
    ]
}

const FREEINGS: &[Freeing] = &[
    Freeing {
        name: "rewrite",
        before: &[NOTES],
        writes: notes,
        changes: &[RANKED],
        refused: None,
        kept: true,
    },
    Freeing {
        name: "aborted rewrite",
        before: &[NOTES],
        writes: notes,
        changes: &[VIOLATED],
        refused: Some(UNRANKED),
        kept: true,
    },
    Freeing {
        name: "dropped relation",
        before: &[NOTES],
        writes: deleted,
        changes: &[DROPPED],
        refused: None,
        kept: false,
    },
    Freeing {
        name: "removed constructor",
        before: &[DOCS],
        writes: retagged,
        changes: &[DEPRECATED, REMOVED],
        refused: None,
        kept: true,
    },
];

#[test]
fn a_change_freeing_a_heap_counts_its_versions_off_every_value_they_point_at() {
    let mut found = Vec::new();
    let mut expected = Vec::new();
    for f in FREEINGS {
        let what = f.name;
        let mut sim = Sim::new(SEED, Config::default());
        run(&mut sim, |rt| {
            let db = opened(&rt, FILE, &log(f.before), &options(1));
            for text in (f.writes)() {
                write(&db, &text);
            }
            db.close().expect("the database closes");
        });
        let held = run(&mut sim, |rt| swept(&rt));
        assert!(held > 0, "{what}: the rows hold large values");
        run(&mut sim, |rt| {
            let (pager, mut catalog, txns) = catalog_at(&rt);
            let (last, earlier) = f.changes.split_last().expect("a change migrates");
            for text in earlier {
                catalog
                    .migrate(&pager, &txns, text)
                    .expect("the migration applies");
            }
            let got = catalog.migrate(&pager, &txns, last);
            match (f.refused, got) {
                (None, Ok(())) => {}
                (Some(why), Err(super::Error::Refused(got))) => assert_eq!(why, got, "{what}"),
                (_, got) => panic!("{what}: {got:?}"),
            }
            pager.close();
        });
        let over = run(&mut sim, |rt| findings(&rt, what).overcounted);
        let left = run(&mut sim, |rt| swept(&rt));
        found.push((what, over, left));
        expected.push((what, 0, if f.kept { held } else { 0 }));
    }
    assert_eq!(expected, found);
}

/// The rows [`wide`] inserts, and one in how many of them a prune keeps.
const WIDE_ROWS: i64 = 48;
const PRUNE_KEPT: i64 = 4;

/// Returns the row of key `key`: a wide text and a large value.
fn wide_row(key: i64) -> Vec<Value> {
    vec![
        Value::I64(key),
        Value::Text("w".repeat(600)),
        Value::Bytes(vec![key as u8; 5000]),
    ]
}

/// A heap filled with [`WIDE_ROWS`] rows of [`wide_row`] over several pages, its directory nodes
/// two entries wide, and a heap of the same sum holding one row.
struct Wide {
    pager: Pager<SimRuntime>,
    txns: Transactions<SimRuntime>,
    heap: Heap,
    other: Heap,
    /// The TID of each row, by key.
    tids: Vec<Tid>,
    /// The TID of the row of `other`.
    other_tid: Tid,
    /// The running transaction of the heap writes the tests make.
    writer: Begun,
}

fn wide(rt: &SimRuntime) -> Wide {
    let mut s = Storage::create(rt, &test_world::options(64));
    s.commit_log(rt);
    let large = s.large();
    s.heap(row_type(EMAIL), large, 2);
    s.heap(row_type(EMAIL), large, 2);
    let (pager, txns, heaps, _) = s.split();
    let txns = txns.expect("the commit log just made");
    let [heap, other] = <[Heap; 2]>::try_from(heaps).expect("the two heaps just made");
    let filler = txns.begin(&pager).expect("a transaction begins");
    let tids = (0..WIDE_ROWS)
        .map(|key| {
            heap.insert(&pager, filler.xid(), &wide_row(key))
                .expect("the row inserts")
        })
        .collect();
    let other_tid = other
        .insert(&pager, filler.xid(), &wide_row(WIDE_ROWS + 10))
        .expect("the row inserts");
    txns.commit(&pager, filler).expect("the rows commit").done();
    let pages = heap.pages(&pager).expect("the heap reads").len();
    assert!(pages > 4, "the rows fill {pages} pages");
    let writer = txns.begin(&pager).expect("a transaction begins");
    Wide {
        pager,
        txns,
        heap,
        other,
        tids,
        other_tid,
        writer,
    }
}

impl Wide {
    /// Returns the overcounted heads and unsettled directory entries verify finds, failing on any
    /// other finding. With `dropped`, the first heap is left out and its pages are not findings.
    fn findings(&self, dropped: bool, what: &str) -> Findings {
        let clog = self.txns.commit_log();
        let mut heaps = vec![(CtorId(1), &self.other)];
        if !dropped {
            heaps.push((CtorId(0), &self.heap));
        }
        let inventory = Inventory {
            commit_log: Some(&clog),
            sums: vec![Sum {
                heaps,
                indexes: Vec::new(),
            }],
            ..Inventory::default()
        };
        let report = verify::verify(&self.pager, &inventory);
        assert!(report.complete, "{what}: {:?}", report.findings);
        let owner = |pid| self.pager.read(pid).expect("a leaked page reads").owner();
        let kept: Vec<Finding> = report
            .findings
            .into_iter()
            .filter(|f| {
                !(dropped && f.problem == Problem::Leaked && owner(f.pid) == self.heap.id().0)
            })
            .collect();
        counted(&kept, what)
    }

    /// Returns the rows of the versions of `heap` no transaction deleted, by key.
    fn live(&self, heap: &Heap) -> Vec<Vec<Value>> {
        let mut rows = Vec::new();
        heap.scan(&self.pager, |v| {
            if !v.header.writers.deleted() {
                rows.push(v.row);
            }
        })
        .expect("the heap reads");
        rows.sort_by_key(|r| key_of(r));
        rows
    }

    /// Delete as the writer all but one row in [`PRUNE_KEPT`].
    fn delete_most(&self) {
        for (key, &tid) in (0..).zip(&self.tids) {
            if key % PRUNE_KEPT != 0 {
                self.heap
                    .delete(&self.pager, tid, self.writer.xid())
                    .kept(&Tested)
                    .expect("the row deletes")
                    .unchecked(&Tested);
            }
        }
    }

    /// Returns a judge removing the versions the writer deleted and keeping any other.
    fn removed(&self) -> impl FnMut(&Writers) -> Result<Fate, StatusError> + use<> {
        let writer = self.writer.xid();
        move |w: &Writers| {
            Ok(if w.deleted_by(writer) {
                Fate::Remove
            } else {
                Fate::Keep
            })
        }
    }

    /// Prune each page of the heap by [`Wide::removed`], up to the first that fails.
    fn prune_all(&self) -> Result<(), heap::Refused> {
        let pages = self.heap.pages(&self.pager).expect("the heap reads");
        for pid in pages {
            self.heap
                .prune(&self.pager, pid, self.removed(), &[], false)?;
        }
        Ok(())
    }

    /// Delete as the writer every other row on the page of `tid` but the one at `tid`, so that a
    /// write finding that page full prunes it by [`Wide::removed`] and frees large values.
    fn make_room(&self, tid: Tid) {
        let on_page = self
            .tids
            .iter()
            .filter(|t| t.page() == tid.page() && **t != tid);
        for &t in on_page.step_by(2) {
            self.heap
                .delete(&self.pager, t, self.writer.xid())
                .kept(&Tested)
                .expect("the row deletes")
                .unchecked(&Tested);
        }
    }

    /// Returns the TID of the last row, on the page the heap's next insert tries first.
    fn last(&self) -> Tid {
        *self.tids.last().expect("the rows inserted")
    }
}

/// The live rows of both heaps of a [`Wide`].
type Both = (Vec<Vec<Value>>, Vec<Vec<Value>>);

/// The take-backs a statement recorded, each with whether its version is in the second heap.
type Recorded = Vec<(bool, Undo)>;

/// One run of a heap entry over a [`Wide`], failing `fail` once the entry's setup is done:
/// the runs of steps it reached and whether it succeeded.
type HeapEntry = fn(&Wide, Option<(Site, usize)>, &str) -> (Vec<Run>, bool);

/// Run `entry` on `w`, failing `fail`, recording its writes as a statement does. Returns the
/// runs of steps it reached and its result; when it fails, takes back what it recorded and fails
/// unless the live rows of both heaps are those it found.
fn statement(
    w: &Wide,
    fail: Option<(Site, usize)>,
    what: &str,
    entry: impl FnOnce(&mut Recorded) -> Result<(), heap::Error>,
) -> (Vec<Run>, bool) {
    let found: Both = (w.live(&w.heap), w.live(&w.other));
    let mut list = Vec::new();
    arm(fail);
    let ok = entry(&mut list).is_ok();
    let runs = armed::reached();
    if !ok {
        for (in_other, u) in list.into_iter().rev() {
            let heap = if in_other { &w.other } else { &w.heap };
            heap.take_back(&w.pager, u)
                .expect("the write is taken back");
        }
        assert_eq!(found, (w.live(&w.heap), w.live(&w.other)), "{what}");
    }
    (runs, ok)
}

/// Returns the row of the version of `heap` at `tid` with a new text.
fn rewritten(w: &Wide, heap: &Heap, tid: Tid) -> Vec<Value> {
    let v = heap.read(&w.pager, tid).expect("the heap reads");
    let mut row = v.expect("the row").row;
    row[1] = Value::Text("u".repeat(600));
    row
}

fn insert(w: &Wide, fail: Option<(Site, usize)>, what: &str) -> (Vec<Run>, bool) {
    w.make_room(w.last());
    statement(w, fail, what, |list| {
        let row = wide_row(WIDE_ROWS);
        let put = w
            .heap
            .insert_pruning(&w.pager, w.writer.xid(), &row, &mut w.removed());
        put.record(list, false, false).map(drop)
    })
}

fn insert_all(w: &Wide, fail: Option<(Site, usize)>, _: &str) -> (Vec<Run>, bool) {
    let rows = (WIDE_ROWS..WIDE_ROWS + 3).map(|k| Ok::<_, heap::Error>(wide_row(k)));
    let mut placed = 0;
    w.make_room(w.last());
    arm(fail);
    let ok = w
        .heap
        .insert_all_pruning(
            &w.pager,
            w.writer.xid(),
            rows,
            &mut w.removed(),
            |_, _| {
                placed += 1;
                Ok(())
            },
            &Tested,
        )
        .is_ok();
    let runs = armed::reached();
    if ok {
        assert_eq!(3, placed, "every row is placed");
    }
    (runs, ok)
}

/// Update the row at `tid` to `row`, as a statement does. Fails unless a successful update is
/// heap-only exactly when `hot` holds.
fn update_at(
    w: &Wide,
    tid: Tid,
    row: Vec<Value>,
    hot: bool,
    fail: Option<(Site, usize)>,
    what: &str,
) -> (Vec<Run>, bool) {
    statement(w, fail, what, |list| {
        let put =
            w.heap
                .update_pruning(&w.pager, tid, w.writer.xid(), &row, &[0], &mut w.removed());
        let (updated, old) = put.record(list, false, false)?;
        old.unchecked(&Tested);
        assert_eq!(hot, updated.heap_only, "{what}");
        Ok(())
    })
}

/// Update the first row keeping its key, on its own page, which it finds full of heap-only
/// versions the writer superseded and prunes.
fn hot_update(w: &Wide, fail: Option<(Site, usize)>, what: &str) -> (Vec<Run>, bool) {
    w.delete_most();
    w.prune_all().expect("the prunes succeed");
    let target = w.tids[0];
    let mut chain = w.tids[PRUNE_KEPT as usize];
    assert_eq!(target.page(), chain.page(), "the two rows share a page");
    for text in 0.. {
        let mut row = rewritten(w, &w.heap, chain);
        row[1] = Value::Text(format!("{text}").repeat(600));
        let updated = w
            .heap
            .update(&w.pager, chain, w.writer.xid(), &row, &[0])
            .expect("the row updates");
        if !updated.heap_only {
            break;
        }
        chain = updated.tid;
    }
    update_at(w, target, rewritten(w, &w.heap, target), true, fail, what)
}

/// Update a middle row to a new key, onto the page of the last row, which it finds full and
/// prunes.
fn cold_update(w: &Wide, fail: Option<(Site, usize)>, what: &str) -> (Vec<Run>, bool) {
    let tid = w.tids[w.tids.len() / 2];
    w.make_room(w.last());
    let mut row = rewritten(w, &w.heap, tid);
    row[0] = Value::I64(WIDE_ROWS + 20);
    update_at(w, tid, row, false, fail, what)
}

/// Retag the row of the second heap into the first, onto the page of the last row, which it
/// finds full and prunes.
fn retag(w: &Wide, fail: Option<(Site, usize)>, what: &str) -> (Vec<Run>, bool) {
    let tid = w.other_tid;
    let row = rewritten(w, &w.other, tid);
    w.make_room(w.last());
    statement(w, fail, what, |list| {
        let put = w.other.retag_pruning(
            &w.pager,
            tid,
            w.writer.xid(),
            &w.heap,
            &row,
            &mut w.removed(),
        );
        let (_, old) = put.record(list, true, false)?;
        old.unchecked(&Tested);
        Ok(())
    })
}

fn delete(w: &Wide, fail: Option<(Site, usize)>, what: &str) -> (Vec<Run>, bool) {
    let tid = w.tids[w.tids.len() / 2];
    statement(w, fail, what, |list| {
        let old = w.heap.delete(&w.pager, tid, w.writer.xid());
        old.record(list, false, false)?.unchecked(&Tested);
        Ok(())
    })
}

/// Copy the row of the second heap into the first, onto the page of the last row, which it
/// finds full and prunes.
fn copy(w: &Wide, fail: Option<(Site, usize)>, _: &str) -> (Vec<Run>, bool) {
    let v = w
        .other
        .read(&w.pager, w.other_tid)
        .expect("the heap reads")
        .expect("the row");
    w.make_room(w.last());
    arm(fail);
    let copied = w.heap.copy_version_pruning(
        &crate::vacuum::Vacuuming::unheld(&w.pager, &w.txns),
        &w.other,
        &v,
        &mut w.removed(),
    );
    let runs = armed::reached();
    if let Ok(copied) = &copied {
        assert!(copied.is_some(), "the version is copied");
    }
    (runs, copied.is_ok())
}

fn prune(w: &Wide, fail: Option<(Site, usize)>, _: &str) -> (Vec<Run>, bool) {
    w.delete_most();
    arm(fail);
    let ok = w.prune_all().is_ok();
    (armed::reached(), ok)
}

fn sweep(w: &Wide, fail: Option<(Site, usize)>, _: &str) -> (Vec<Run>, bool) {
    w.delete_most();
    w.prune_all().expect("the prunes succeed");
    arm(fail);
    let swept = w.heap.large().sweep(
        &crate::vacuum::Vacuuming::unheld(&w.pager, &w.txns),
        |_| Ok(true),
        &mut || true,
    );
    let runs = armed::reached();
    if let Ok(walk) = &swept {
        assert!(
            matches!(walk, Walk::Done(n) if *n > 0),
            "the sweep frees chunks: {walk:?}"
        );
    }
    (runs, swept.is_ok())
}

fn release_page(w: &Wide, fail: Option<(Site, usize)>, _: &str) -> (Vec<Run>, bool) {
    let pages = w.heap.pages(&w.pager).expect("the heap reads");
    arm(fail);
    let mut counted = true;
    for pid in pages {
        counted &= w.heap.release_page(&w.pager, pid);
    }
    (armed::reached(), counted)
}

/// A heap entry the search fails at each run of a step after an update.
struct HeapCase {
    name: &'static str,
    run: HeapEntry,
    /// The steps after an update the entry reaches.
    steps: &'static [Step],
    /// The entry counts the first heap's versions off as a drop does, leaving it out of verify.
    dropped: bool,
    /// A failed settle leaves an entry unsettled.
    unsettles: bool,
}

const HEAP_CASES: &[HeapCase] = &[
    HeapCase {
        name: "insert",
        run: insert,
        steps: &[Step::Release, Step::Settle],
        dropped: false,
        unsettles: false,
    },
    HeapCase {
        name: "insert all",
        run: insert_all,
        steps: &[Step::Release, Step::Settle],
        dropped: false,
        unsettles: false,
    },
    HeapCase {
        name: "hot update",
        run: hot_update,
        steps: &[Step::Release, Step::Settle],
        dropped: false,
        unsettles: false,
    },
    HeapCase {
        name: "cold update",
        run: cold_update,
        steps: &[Step::Release, Step::Settle],
        dropped: false,
        unsettles: false,
    },
    HeapCase {
        name: "retag",
        run: retag,
        steps: &[Step::Release, Step::Settle],
        dropped: false,
        unsettles: false,
    },
    HeapCase {
        name: "delete",
        run: delete,
        steps: &[Step::Settle],
        dropped: false,
        unsettles: false,
    },
    HeapCase {
        name: "copy",
        run: copy,
        steps: &[Step::Release, Step::Settle],
        dropped: false,
        unsettles: false,
    },
    HeapCase {
        name: "prune",
        run: prune,
        steps: &[Step::Release, Step::Settle],
        dropped: false,
        unsettles: true,
    },
    HeapCase {
        name: "sweep",
        run: sweep,
        steps: &[Step::Settle],
        dropped: false,
        unsettles: false,
    },
    HeapCase {
        name: "release page",
        run: release_page,
        steps: &[Step::Release, Step::Settle],
        dropped: true,
        unsettles: false,
    },
];

/// What one run of a heap entry did: the runs of steps it reached, whether it succeeded, and what
/// verify then found.
struct HeapRan {
    runs: Vec<Run>,
    ok: bool,
    found: Findings,
}

/// Run `case` on a fresh [`wide`] world, failing `fail`.
fn heap_ran(case: &HeapCase, fail: Option<(Site, usize)>, what: &str) -> HeapRan {
    let mut sim = Sim::new(SEED, Config::default());
    run(&mut sim, |rt| {
        let w = wide(&rt);
        let (runs, ok) = (case.run)(&w, fail, what);
        let found = w.findings(case.dropped, what);
        HeapRan { runs, ok, found }
    })
}

/// Returns the runs of `steps` among `runs` the search fails.
fn searched_of(runs: &[Run], steps: &[Step]) -> BTreeSet<(Site, usize)> {
    let runs: Vec<Run> = runs
        .iter()
        .filter(|r| steps.contains(&r.0.step()))
        .copied()
        .collect();
    searched(&runs)
}

#[test]
fn a_heap_entry_failing_a_release_or_settle_after_its_update_fails_and_skips_no_other_step() {
    let mut retain_settles = 0;
    for case in HEAP_CASES {
        let name = case.name;
        let clean = heap_ran(case, None, name);
        assert!(clean.ok, "{name} fails with nothing failed");
        let settled = Findings {
            overcounted: 0,
            unsettled: 0,
        };
        assert_eq!(settled, clean.found, "{name} with nothing failed");
        let mut struck: BTreeSet<Site> = BTreeSet::new();
        let mut unsettled = false;
        for (site, n) in searched_of(&clean.runs, case.steps) {
            let what = format!("{name}: run {n} of {site:?}");
            let got = heap_ran(case, Some((site, n)), &what);
            assert!(
                got.runs.contains(&(site, n, true)),
                "{what} armed and not failed: {:?}",
                got.runs
            );
            assert!(!got.ok, "{what}");
            let released = site.step() == Step::Release;
            assert_eq!(usize::from(released), got.found.overcounted, "{what}");
            if released {
                assert_eq!(0, got.found.unsettled, "{what}");
            }
            unsettled |= got.found.unsettled > 0;
            struck.insert(site);
        }
        let steps: BTreeSet<Step> = case.steps.iter().copied().collect();
        let struck_steps: BTreeSet<Step> = struck.iter().map(|s| s.step()).collect();
        assert_eq!(steps, struck_steps, "{name}");
        // The settle of a raise of counts ahead of an update runs right after its Retain step.
        for pair in clean.runs.windows(2) {
            if let [(retain, ..), (settle, ..)] = pair
                && retain.step() == Step::Retain
            {
                assert_eq!(Step::Settle, settle.step(), "{name}: after {retain:?}");
                assert!(struck.contains(settle), "{name}: {settle:?} not failed");
                retain_settles += 1;
            }
        }
        if case.unsettles {
            assert!(
                unsettled,
                "{name}: no failed settle left an entry unsettled"
            );
        }
    }
    assert!(
        retain_settles > 0,
        "no entry raised counts ahead of its update"
    );
}

#[test]
fn a_registration_refused_after_a_migration_committed_fails_the_open_and_keeps_the_migration() {
    let refused = Registration::new(
        "notes-1",
        "import app\nquery ranks () -> Set I64 = from n in notes select n.grade\n",
    );
    let mut sim = created_at(&[NOTES]);
    run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES]), &options(1));
        write(&db, &inserts(""));
        db.close().expect("the database closes");
    });
    let got = run(&mut sim, |rt| {
        let options = OpenOptions {
            registrations: vec![refused.clone()],
            ..options(1)
        };
        open_in(&rt, FILE, &log(&[NOTES, RANKED]), &options).map(drop)
    });
    let Err(OpenError::RegistrationRefused { bundle, reason }) = got else {
        panic!("the open is no refused registration: {got:?}");
    };
    let invalid = Error::Rejected {
        reason: RejectReason::Invalid {
            detail: "query `ranks`: Note has no field `grade`".into(),
        },
    };
    assert_eq!(("notes-1".to_owned(), invalid), (bundle, reason));
    assert_eq!(2, run(&mut sim, |rt| version(&rt)));
    assert_eq!(ROWS, run(&mut sim, |rt| ranked(&rt, "after the refusal")));
}
