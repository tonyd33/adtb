//! A step that fails after its change's main update. After a transaction's durable commit the
//! caller is told it succeeded and the next open finishes the step; before it, the call fails,
//! nothing of it is seen, every release still ran, and every count of a large value equals the
//! versions pointing at it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::process::run;
use adtb_testkit::search::Files;

use super::root::{Changing, Root};
use super::{Catalog, Read, Reader, Transactions};
use crate::faults::Step;
use crate::faults::armed::{self, Run, Site};
use crate::heap::{self, Heap, Tested, Unchecked, Undo};
use crate::key::{CtorId, Tid, Value};
use crate::mvcc::{Fate, Writers, Xid};
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

/// The rows [`wide_heap`] inserts, and one in how many of them [`pruned`] keeps.
const WIDE_ROWS: i64 = 48;
const PRUNE_KEPT: i64 = 4;

/// The transaction of the heap writes the tests make after [`wide_heap`].
const WRITER: Xid = Xid(2);

/// Returns the row of key `key`: a wide text and a large value.
fn wide_row(key: i64) -> Vec<Value> {
    vec![
        Value::I64(key),
        Value::Text("w".repeat(600)),
        Value::Bytes(vec![key as u8; 5000]),
    ]
}

/// Returns a heap whose directory nodes hold two entries, filled with [`WIDE_ROWS`] rows of
/// [`wide_row`] over several pages, and the TID of each row.
fn wide_heap(rt: &SimRuntime) -> (Pager<SimRuntime>, Heap, Vec<Tid>) {
    let mut s = Storage::create(rt, &test_world::options(64));
    let large = s.large();
    s.heap(row_type(EMAIL), large, 2);
    let (pager, _, heaps, _) = s.split();
    let heap = heaps.into_iter().next().expect("the heap just made");
    let tids = (0..WIDE_ROWS)
        .map(|key| {
            heap.insert(&pager, Xid(1), &wide_row(key))
                .expect("the row inserts")
        })
        .collect();
    let pages = heap.pages(&pager).expect("the heap reads").len();
    assert!(pages > 4, "the rows fill {pages} pages");
    (pager, heap, tids)
}

/// Returns what verify finds of `heap` that [`counted`] counts.
fn heap_findings(pager: &Pager<SimRuntime>, heap: &Heap, what: &str) -> Findings {
    let inventory = Inventory {
        sums: vec![Sum {
            heaps: vec![(CtorId(0), heap)],
            indexes: Vec::new(),
        }],
        ..Inventory::default()
    };
    counted(&verify::verify(pager, &inventory).findings, what)
}

/// Fill a [`wide_heap`], delete all but one row in [`PRUNE_KEPT`], then prune each page,
/// failing `fail`. Each prune leaves its page roomier than the entries above it say.
fn pruned(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("prune {fail:?}");
    let mut sim = Sim::new(SEED, Config::default());
    run(&mut sim, |rt| {
        let (pager, heap, tids) = wide_heap(&rt);
        for (key, tid) in (0..).zip(tids) {
            if key % PRUNE_KEPT != 0 {
                heap.delete(&pager, tid, WRITER)
                    .kept(&Tested)
                    .expect("the row deletes")
                    .unchecked(Unchecked::Test);
            }
        }
        let judge = |w: &Writers| {
            Ok(if w.deleted_by(WRITER) {
                Fate::Remove
            } else {
                Fate::Keep
            })
        };
        let pages = heap.pages(&pager).expect("the heap reads");
        arm(fail);
        let ok = pages
            .iter()
            .try_for_each(|&pid| heap.prune(&pager, pid, judge, &[], false).map(drop))
            .is_ok();
        let runs = armed::reached();
        Ran {
            runs,
            ok,
            found: heap_findings(&pager, &heap, &what),
        }
    })
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
    ("prune", pruned, BEFORE_COMMIT),
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
        (("write.rs", Step::Release), 1),
        (("write.rs", Step::Settle), 2),
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
            &crate::write::Vacuuming::unheld(&pager, &txns),
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

/// A write a statement makes to a heap, as `WRITER`, of the version at the TID given, its
/// take-back recorded on the list given.
type HeapWrite =
    fn(&Heap, &Pager<SimRuntime>, Tid, &mut Vec<((), Undo)>) -> Result<(), heap::Error>;

/// Returns the rows of the versions of `heap` no transaction deleted, by key.
fn live(pager: &Pager<SimRuntime>, heap: &Heap) -> Vec<Vec<Value>> {
    let mut rows = Vec::new();
    heap.scan(pager, |v| {
        if !v.header.writers.deleted() {
            rows.push(v.row);
        }
    })
    .expect("the heap reads");
    rows.sort_by_key(|r| key_of(r));
    rows
}

const HEAP_WRITES: &[(&str, HeapWrite)] = &[
    ("insert", |heap, pager, _, list| {
        let put = heap.insert_pruning(pager, WRITER, &wide_row(WIDE_ROWS), &mut |_| Ok(Fate::Keep));
        put.record(list, (), ()).map(drop)
    }),
    ("update", |heap, pager, tid, list| {
        let put = heap.update_pruning(pager, tid, WRITER, &wide_row(WIDE_ROWS), &[0], &mut |_| {
            Ok(Fate::Keep)
        });
        let (_, old) = put.record(list, (), ())?;
        old.unchecked(Unchecked::Test);
        Ok(())
    }),
    ("delete", |heap, pager, tid, list| {
        let old = heap.delete(pager, tid, WRITER).record(list, (), ())?;
        old.unchecked(Unchecked::Test);
        Ok(())
    }),
];

/// Write `write` over a [`wide_heap`], failing `fail`. Returns the runs of steps it reached and
/// its result; when it fails, takes back what it recorded and fails unless the heap's live rows
/// are those it found.
fn heap_written(write: HeapWrite, fail: Option<(Site, usize)>) -> (Vec<Run>, bool) {
    let what = format!("{fail:?}");
    let mut sim = Sim::new(SEED, Config::default());
    run(&mut sim, |rt| {
        let (pager, heap, tids) = wide_heap(&rt);
        let found = live(&pager, &heap);
        let mut list = Vec::new();
        arm(fail);
        let ok = write(&heap, &pager, tids[tids.len() / 2], &mut list).is_ok();
        let runs = armed::reached();
        if !ok {
            for ((), u) in list.into_iter().rev() {
                heap.take_back(&pager, u).expect("the write is taken back");
            }
            assert_eq!(found, live(&pager, &heap), "{what}");
        }
        (runs, ok)
    })
}

#[test]
fn a_heap_write_failing_after_its_update_returns_a_take_back_restoring_the_rows_it_found() {
    for (kind, write) in HEAP_WRITES {
        let (clean, ok) = heap_written(*write, None);
        assert!(ok, "{kind} fails with nothing failed");
        let sites = ends(&clean, BEFORE_COMMIT);
        assert!(
            sites.iter().any(|(s, _)| s.step() == Step::Settle),
            "{kind} reaches no settle: {clean:?}"
        );
        for (site, n) in sites {
            let (runs, ok) = heap_written(*write, Some((site, n)));
            assert!(
                runs.contains(&(site, n, true)),
                "{kind}: run {n} of {site:?} armed and not failed: {runs:?}"
            );
            assert!(!ok, "{kind}: run {n} of {site:?}");
        }
    }
}
