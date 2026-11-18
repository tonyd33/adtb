//! A step that fails after its change's main update. After a transaction's durable commit the
//! caller is told it succeeded and the next open finishes the step; before it, the call fails,
//! nothing of it is seen, every release still ran, and every count of a large value equals the
//! versions pointing at it.

use std::collections::{BTreeMap, BTreeSet};

use adtb_harness::process::run;
use adtb_sim::{Config, Sim, SimRuntime};

use super::root::{Changing, Root};
use super::{Catalog, Read, Reader, Transactions};
use crate::faults::Step;
use crate::faults::armed::{self, Run, Site};
use crate::storage::{self, Pager};
use crate::test_world::api::{printed, query};
use crate::test_world::load::{FILE, opened, options};
use crate::verify::{self, Depth, Problem};
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

/// Returns the overcounted heads verify finds on the closed file once an open ended any change
/// on the catalog root, failing unless it read every structure and found nothing else but
/// unsettled directory entries.
fn overcounted(rt: &SimRuntime, what: &str) -> usize {
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
    let mut over = 0;
    for f in &report.findings {
        match f.problem {
            Problem::Overcounted { .. } => over += 1,
            Problem::Unsettled(_) => {}
            ref other => panic!("{what}: {other:?}"),
        }
    }
    over
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
/// the overcounted heads verify found after a reopen.
struct Ran {
    runs: Vec<Run>,
    ok: bool,
    overcounted: usize,
}

/// Open the file at [`NOTES`] with [`ROWS`] rows, then at [`RANKED`], failing `fail`.
fn migrated(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("migration {fail:?}");
    let mut sim = Sim::new(SEED, Config::default());
    run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES]), &options(1));
        write(&db, &inserts(""));
        db.close().expect("the database closes");
    });
    let runs = run(&mut sim, |rt| {
        arm(fail);
        let db = opened(&rt, FILE, &log(&[NOTES, RANKED]), &options(1));
        let runs = armed::reached();
        assert_eq!(fail.is_some(), changing(db.tested_pager()), "{what}");
        db.close().expect("the database closes");
        runs
    });
    let overcounted = run(&mut sim, |rt| {
        reads(&rt, ROWS, &what);
        overcounted(&rt, &what)
    });
    Ran {
        runs,
        ok: true,
        overcounted,
    }
}

/// Settle at open the pending drop of a unique whose holder no open registers, failing `fail`.
fn settled(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("pending drop {fail:?}");
    let mut sim = Sim::new(SEED, Config::default());
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
    let overcounted = run(&mut sim, |rt| {
        let (pager, catalog, _) = catalog_at(&rt);
        assert!(catalog.pending_drops().is_empty(), "{what}");
        pager.close();
        overcounted(&rt, &what)
    });
    Ran {
        runs,
        ok: true,
        overcounted,
    }
}

/// Insert [`ROWS`] rows into a file at [`RANKED`], failing `fail`.
fn inserted(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("insert {fail:?}");
    let mut sim = Sim::new(SEED, Config::default());
    let (runs, ok) = run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[NOTES, RANKED]), &options(1));
        arm(fail);
        let ok = call(&db, &inserts(", rank: 7")).is_ok();
        let runs = armed::reached();
        db.close().expect("the database closes");
        (runs, ok)
    });
    let overcounted = run(&mut sim, |rt| {
        reads(&rt, if ok { ROWS } else { 0 }, &what);
        overcounted(&rt, &what)
    });
    Ran {
        runs,
        ok,
        overcounted,
    }
}

/// Insert [`ROWS`] rows into a file at [`RANKED`], vacuum, then update each keeping its body,
/// failing `fail`.
fn updated(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("update {fail:?}");
    let mut sim = Sim::new(SEED, Config::default());
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
    let overcounted = run(&mut sim, |rt| {
        reads(&rt, if ok { 0 } else { ROWS }, &what);
        overcounted(&rt, &what)
    });
    Ran {
        runs,
        ok,
        overcounted,
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
    let mut sim = Sim::new(SEED, Config::default());
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
    let overcounted = run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(&[WIDE]), &options(1));
        let rows = query(&db, "from w in wides select w.id");
        assert_eq!(if ok { ROWS } else { 0 }, rows.len(), "{what}");
        assert_eq!(
            Vec::<String>::new(),
            printed(db.verify_holding().expect("verify runs")),
            "{what}"
        );
        db.close().expect("the database closes");
        overcounted(&rt, &what)
    });
    Ran {
        runs,
        ok,
        overcounted,
    }
}

/// Delete every row of a file at [`RANKED`], then vacuum, failing `fail`.
fn vacuumed(fail: Option<(Site, usize)>) -> Ran {
    let what = format!("vacuum {fail:?}");
    let mut sim = Sim::new(SEED, Config::default());
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
    let overcounted = run(&mut sim, |rt| {
        reads(&rt, 0, &what);
        overcounted(&rt, &what)
    });
    Ran {
        runs,
        ok,
        overcounted,
    }
}

type Case = (
    &'static str,
    fn(Option<(Site, usize)>) -> Ran,
    &'static [Step],
    usize,
);

const AFTER_COMMIT: &[Step] = &[Step::FreeLeaked, Step::EndChange];
const BEFORE_COMMIT: &[Step] = &[Step::Release, Step::Settle];
const BEFORE_UPDATE: &[Step] = &[Step::ClearBit, Step::Reflect, Step::Write];
const BEFORE_UPDATE_WIDE: &[Step] = &[Step::Retain, Step::Reflect, Step::Write];

/// The heads the migration leaves counting a version of the heap it rewrote, one per row.
const REWRITE_LEAKS: usize = ROWS;

const CASES: &[Case] = &[
    ("migration", migrated, AFTER_COMMIT, REWRITE_LEAKS),
    ("pending drop", settled, AFTER_COMMIT, 0),
    ("insert", inserted, BEFORE_COMMIT, 0),
    ("vacuum", vacuumed, BEFORE_COMMIT, 0),
    ("update", updated, BEFORE_UPDATE, 0),
    ("wide insert", widened, BEFORE_UPDATE_WIDE, 0),
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
    for (case, run_case, steps, leaked) in CASES {
        let clean = run_case(None);
        assert!(clean.ok, "{case} fails with nothing failed");
        assert_eq!(*leaked, clean.overcounted, "{case} with nothing failed");
        let after = steps == &AFTER_COMMIT;
        for (site, n) in ends(&clean.runs, steps) {
            let got = run_case(Some((site, n)));
            assert!(
                got.runs.contains(&(site, n, true)),
                "{case}: run {n} of {site:?} armed and not failed: {:?}",
                got.runs
            );
            assert_eq!(after, got.ok, "{case}: run {n} of {site:?}");
            let skipped = usize::from(site.step() == Step::Release);
            assert_eq!(
                clean.overcounted + skipped,
                got.overcounted,
                "{case}: run {n} of {site:?}"
            );
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
