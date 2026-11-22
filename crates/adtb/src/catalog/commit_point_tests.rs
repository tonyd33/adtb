//! A step that fails after its change's main update. After a transaction's durable commit the
//! caller is told it succeeded and the next open finishes the step; before it, the call fails,
//! nothing of it is seen, every release still ran, and every count of a large value equals the
//! versions pointing at it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process::run;
use adtb_testkit::profile;
use adtb_testkit::search::Files;

use super::root::{Changing, Root};
use super::{Catalog, Read, Reader, Transactions};
use crate::faults::Step;
use crate::faults::armed::{self, Run, Site};
use crate::heap::{self, Heap, Tested, Undo};
use crate::key::{CtorId, Tid, Value};
use crate::mvcc::{Begun, Fate, StatusError, Writers};
use crate::storage::Walk;
use crate::storage::{self, Pager};
use crate::test_world;
use crate::test_world::api::{printed, query};
use crate::test_world::fixture::Storage;
use crate::test_world::load::{FILE, opened, options};
use crate::test_world::rows::{EMAIL, key_of, row_type};
use crate::verify::{self, Depth, Finding, Inventory, Problem, Sum};
use crate::{CallOptions, Database, Error, Log};

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
    let report = verify::verify_with(&pager, &catalog.structures().inventory(&clog), depth);
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

/// Fail unless an open at [`RANKED`] answers no row from verify and reads `rows` notes ranked 7.
fn reads(rt: &SimRuntime, rows: usize, what: &str) {
    let db = opened(rt, FILE, &log(&[NOTES, RANKED]), &options(1));
    let ranked = query(&db, "from n in notes where n.rank == 7 select n.id");
    assert_eq!(rows, ranked.len(), "{what}");
    assert_eq!(
        Vec::<String>::new(),
        printed(db.verify_holding().expect("verify runs")),
        "{what}"
    );
    db.close().expect("the database closes");
}

/// What one run of a change did: the runs of steps it reached, whether the call succeeded, and
/// what verify found after a reopen.
struct Ran {
    runs: Vec<Run>,
    ok: bool,
    found: Findings,
}

/// Open the file at [`NOTES`] with [`ROWS`] rows, then at [`RANKED`], failing `fail`. Fail
/// unless a sweep after the next open leaves the store as large as before the migration.
fn migrated(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("migration {fail:?}");
    let mut sim = created_at(&[NOTES]);
    run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES]), &options(1));
        write(&db, &inserts(""));
        db.close().expect("the database closes");
    });
    let held = run(&mut sim, |rt| swept(&rt));
    let runs = run(&mut sim, |rt| {
        arm(fail);
        let db = opened(&rt, FILE, &log(&[NOTES, RANKED]), &options(1));
        let runs = armed::reached();
        assert_eq!(fail.is_some(), changing(db.tested_pager()), "{what}");
        db.close().expect("the database closes");
        runs
    });
    let found = run(&mut sim, |rt| {
        reads(&rt, ROWS, &what);
        findings(&rt, &what)
    });
    assert_eq!(held, run(&mut sim, |rt| swept(&rt)), "{what}");
    Ran {
        runs,
        ok: true,
        found,
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
    let runs = run(&mut sim, |rt| {
        let (pager, mut catalog, txns) = catalog_at(&rt);
        arm(fail);
        catalog.settle(&pager, &txns).expect("the settle succeeds");
        let runs = armed::reached();
        assert!(catalog.pending_drops().is_empty(), "{what}");
        assert_eq!(fail.is_some(), changing(&pager), "{what}");
        pager.close();
        runs
    });
    let found = run(&mut sim, |rt| {
        let (pager, catalog, _) = catalog_at(&rt);
        assert!(catalog.pending_drops().is_empty(), "{what}");
        pager.close();
        findings(&rt, &what)
    });
    Ran {
        runs,
        ok: true,
        found,
    }
}

/// Insert [`ROWS`] rows into a file at [`RANKED`], failing `fail`.
fn inserted(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("insert {fail:?}");
    let mut sim = created_at(&[NOTES, RANKED]);
    let (runs, ok) = run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES, RANKED]), &options(1));
        arm(fail);
        let ok = call(&db, &inserts(", rank: 7")).is_ok();
        let runs = armed::reached();
        db.close().expect("the database closes");
        (runs, ok)
    });
    let found = run(&mut sim, |rt| {
        reads(&rt, if ok { ROWS } else { 0 }, &what);
        findings(&rt, &what)
    });
    Ran { runs, ok, found }
}

/// Insert [`ROWS`] rows into a file at [`RANKED`], vacuum, then update each keeping its body,
/// failing `fail`.
fn updated(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("update {fail:?}");
    let mut sim = created_at(&[NOTES, RANKED]);
    let (runs, ok) = run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES, RANKED]), &options(1));
        write(&db, &inserts(", rank: 7"));
        write(&db, "vacuum notes");
        arm(fail);
        let ok = call(&db, "update n in notes set { n with rank: 8 }").is_ok();
        let runs = armed::reached();
        db.close().expect("the database closes");
        (runs, ok)
    });
    let found = run(&mut sim, |rt| {
        reads(&rt, if ok { 0 } else { ROWS }, &what);
        findings(&rt, &what)
    });
    Ran { runs, ok, found }
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
    let (runs, ok) = run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[WIDE]), &options(1));
        arm(fail);
        let ok = call(&db, &format!("transact\n{}", row.repeat(ROWS))).is_ok();
        let runs = armed::reached();
        if fail.is_none() {
            let retains = runs.iter().filter(|r| r.0.step() == Step::Retain);
            assert_eq!(
                2 * ROWS,
                retains.count(),
                "each row's counts take two updates"
            );
        }
        db.close().expect("the database closes");
        (runs, ok)
    });
    let found = run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[WIDE]), &options(1));
        let rows = query(&db, "from w in wides select w.id");
        assert_eq!(if ok { ROWS } else { 0 }, rows.len(), "{what}");
        assert_eq!(
            Vec::<String>::new(),
            printed(db.verify_holding().expect("verify runs")),
            "{what}"
        );
        db.close().expect("the database closes");
        findings(&rt, &what)
    });
    Ran { runs, ok, found }
}

/// Delete every row of a file at [`RANKED`], then vacuum, failing `fail`.
fn vacuumed(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("vacuum {fail:?}");
    let mut sim = created_at(&[NOTES, RANKED]);
    let (runs, ok) = run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES, RANKED]), &options(1));
        write(&db, &inserts(", rank: 7"));
        write(&db, "delete n in notes");
        arm(fail);
        let ok = call(&db, "vacuum notes").is_ok();
        let runs = armed::reached();
        db.close().expect("the database closes");
        (runs, ok)
    });
    let found = run(&mut sim, |rt| {
        reads(&rt, 0, &what);
        findings(&rt, &what)
    });
    Ran { runs, ok, found }
}

type Case = (
    &'static str,
    fn(Option<(Site, usize)>) -> Ran,
    &'static [Step],
);

const AFTER_COMMIT: &[Step] = &[Step::FreeLeaked, Step::EndChange];
const AFTER_REWRITE: &[Step] = &[Step::FreeLeaked, Step::Release, Step::EndChange];
const BEFORE_COMMIT: &[Step] = &[Step::Release, Step::Settle];
const BEFORE_UPDATE: &[Step] = &[Step::ClearBit, Step::Reflect, Step::Write];
const BEFORE_UPDATE_WIDE: &[Step] = &[Step::Retain, Step::Reflect, Step::Write];

const CASES: &[Case] = &[
    ("migration", migrated, AFTER_REWRITE),
    ("pending drop", settled, AFTER_COMMIT),
    ("insert", inserted, BEFORE_COMMIT),
    ("vacuum", vacuumed, BEFORE_COMMIT),
    ("update", updated, BEFORE_UPDATE),
    ("wide insert", widened, BEFORE_UPDATE_WIDE),
];

/// Returns the first and the last run of each site of `steps` among `runs`.
fn ends(runs: &[Run], steps: &[Step]) -> BTreeSet<(Site, usize)> {
    let mut last: BTreeMap<Site, usize> = BTreeMap::new();
    for (site, n, _) in runs.iter().filter(|r| steps.contains(&r.0.step())) {
        last.insert(*site, *n);
    }
    last.into_iter()
        .flat_map(|(s, n)| [(s, 0), (s, n)])
        .collect()
}

#[test]
fn a_step_failing_after_a_commit_fails_no_caller_and_one_before_it_fails_the_call() {
    let mut struck: BTreeSet<Site> = BTreeSet::new();
    for (case, run_case, steps) in CASES {
        let clean = run_case(None);
        assert!(clean.ok, "{case} fails with nothing failed");
        let settled = Findings {
            overcounted: 0,
            unsettled: 0,
        };
        assert_eq!(settled, clean.found, "{case} with nothing failed");
        let after = steps.contains(&Step::FreeLeaked);
        for (site, n) in ends(&clean.runs, steps) {
            let got = run_case(Some((site, n)));
            assert!(
                got.runs.contains(&(site, n, true)),
                "{case}: run {n} of {site:?} armed and not failed: {:?}",
                got.runs
            );
            assert_eq!(after, got.ok, "{case}: run {n} of {site:?}");
            let skipped = usize::from(!after && site.step() == Step::Release);
            assert_eq!(
                skipped, got.found.overcounted,
                "{case}: run {n} of {site:?}"
            );
            if site.step() != Step::Settle {
                assert_eq!(0, got.found.unsettled, "{case}: run {n} of {site:?}");
            }
            struck.insert(site);
        }
    }
    let mut sites: BTreeMap<(&str, Step), usize> = BTreeMap::new();
    for s in struck {
        *sites.entry((s.file_name(), s.step())).or_default() += 1;
    }
    let counted = BTreeMap::from([
        (("migration.rs", Step::FreeLeaked), 1),
        (("migration.rs", Step::EndChange), 1),
        (("decode.rs", Step::Release), 1),
        (("pending.rs", Step::FreeLeaked), 1),
        (("pending.rs", Step::EndChange), 1),
        (("mod.rs", Step::Release), 1),
        (("mod.rs", Step::Settle), 1),
        (("put.rs", Step::Settle), 1),
        (("large.rs", Step::Settle), 5),
        (("large.rs", Step::Retain), 1),
        (("write.rs", Step::ClearBit), 1),
        (("write.rs", Step::Reflect), 1),
        (("write.rs", Step::Write), 1),
    ]);
    assert_eq!(counted, sites);
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
    txns.commit(&pager, filler).expect("the rows commit");
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

/// Run `case` on a fresh [`wide`] world, failing `fail`. Returns the runs of steps it reached,
/// whether it succeeded and what verify then finds.
fn heap_ran(case: &HeapCase, fail: Option<(Site, usize)>, what: &str) -> Ran {
    let mut sim = Sim::new(SEED, Config::default());
    run(&mut sim, |rt| {
        let w = wide(&rt);
        let (runs, ok) = (case.run)(&w, fail, what);
        let found = w.findings(case.dropped, what);
        Ran { runs, ok, found }
    })
}

/// Returns the runs of `steps` among `runs` the search fails: under the deep profile every one,
/// otherwise the first and the last of each site.
fn searched(runs: &[Run], steps: &[Step]) -> BTreeSet<(Site, usize)> {
    if !profile::every_case() {
        return ends(runs, steps);
    }
    runs.iter()
        .filter(|r| steps.contains(&r.0.step()))
        .map(|&(site, n, _)| (site, n))
        .collect()
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
        for (site, n) in searched(&clean.runs, case.steps) {
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
