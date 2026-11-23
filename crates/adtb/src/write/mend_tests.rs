//! The mend pass an open schedules when the log it recovered ends at no clean close: it runs once
//! on the background worker beside writers, a clean open schedules none, it leaves no head
//! overcounted, no directory unsettled and no split unfinished after a crash anywhere in a write,
//! a vacuum or the pass itself.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adtb_sim::{Config, Match, Sim, SimRuntime};
use adtb_testkit::common::sleep;
use adtb_testkit::process::run;
use adtb_testkit::profile::assert_reached;
use adtb_testkit::search::{Files, each_crash_image_under};

use crate::btree::BTree;
use crate::catalog::Catalog;
use crate::heap::create_directories_with_fanout;
use crate::mvcc::Transactions;
use crate::runtime::{Runtime, Threads};
use crate::storage::{self, Pager};
use crate::test_world::api::query;
use crate::test_world::load::{FILE, opened, options};
use crate::testing::Knobs;
use crate::verify::{self, Depth, Mended, Problem};
use crate::{CallOptions, Database, Log, OpenOptions};

const SEED: u64 = 47;

const WIDE: &str = r"module app
add module app
add type Wide = { id: Key wides, a: Text, b: Text, tag: Text }
add relation wides : Wide key id generate sequence
add index (tag) in wides
";

/// The entries of each directory page of a heap or store, few enough that a handful of pages
/// gives a directory an inner level.
const FANOUT: u16 = 4;

/// The rows written with two values out of line each.
const ROWS: usize = 3;

/// The versions each planted head counts besides those that point at it.
const PLANTED: u32 = 2;

/// The rows written with only a long tag, enough to split the tag index's leaves.
const TAGGED: usize = 120;

fn log() -> Log {
    Log::new([WIDE])
}

fn quiet() -> OpenOptions {
    OpenOptions {
        testing: Knobs::without_background_vacuum(),
        ..options(1)
    }
}

fn large(c: char) -> String {
    format!("\"{}\"", c.to_string().repeat(5000))
}

fn tag(n: usize, len: usize) -> String {
    format!("\"{n:04}{}\"", "t".repeat(len))
}

fn wide_rows(rows: usize) -> String {
    (0..rows)
        .map(|n| {
            let (a, b) = (large('a'), large('b'));
            format!("  insert wides {{ a: {a}, b: {b}, tag: \"w{n}\" }}\n")
        })
        .collect()
}

fn tagged_rows(from: usize, rows: usize, len: usize) -> String {
    (from..from + rows)
        .map(|n| {
            format!(
                "  insert wides {{ a: \"\", b: \"\", tag: {} }}\n",
                tag(n, len)
            )
        })
        .collect()
}

fn write(db: &Database<SimRuntime>, rows: &str) {
    db.query_text(&format!("transact\n{rows}"), &[], &CallOptions::default())
        .expect("the rows write");
}

/// Run `body` on the closed file's pager and catalog, leaving the log ending as it found it: at a
/// clean close or not. Returns what `body` returns.
fn at_rest<T>(
    rt: &SimRuntime,
    body: impl FnOnce(&Pager<SimRuntime>, &Catalog, &Transactions<SimRuntime>) -> T,
) -> T {
    let (pager, unclean) = pager_at(rt);
    let out = with_catalog(rt, &pager, body);
    pager.close_owing(unclean);
    out
}

/// Returns the pager of the closed file and whether its log ends at no clean close.
fn pager_at(rt: &SimRuntime) -> (Pager<SimRuntime>, bool) {
    let options = storage::Options {
        page_size: 8192,
        ..storage::Options::new(FILE)
    };
    let (pager, opening) = Pager::open(rt.clone(), &options).expect("the file reopens");
    let unclean = opening.unclean();
    (pager, unclean)
}

fn with_catalog<T>(
    rt: &SimRuntime,
    pager: &Pager<SimRuntime>,
    body: impl FnOnce(&Pager<SimRuntime>, &Catalog, &Transactions<SimRuntime>) -> T,
) -> T {
    let (catalog, txns) = Catalog::load(rt.clone(), pager).expect("the catalog loads");
    body(pager, &catalog, &txns)
}

/// What a crash leaves that the mend pass mends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Leftovers {
    overcounted: usize,
    unsettled: usize,
    unfinished: usize,
}

impl Leftovers {
    fn none(self) -> bool {
        self == Leftovers::default()
    }
}

/// Returns what the closed file holds for the mend pass, failing unless verify read every
/// structure whole and found nothing else but a page a crashed change leaked.
fn leftovers(rt: &SimRuntime) -> Leftovers {
    at_rest(rt, |pager, catalog, txns| {
        let clog = txns.commit_log();
        let structures = catalog.structures();
        let inventory = structures.inventory(Some(&clog));
        let report = verify::verify_with(pager, &inventory, Depth::FULL);
        assert!(report.complete, "{:?}", report.findings);
        let mut found = Leftovers::default();
        for f in &report.findings {
            match f.problem {
                Problem::Overcounted { .. } => found.overcounted += 1,
                Problem::Unsettled(_) => found.unsettled += 1,
                Problem::Leaked => {}
                ref other => panic!("page {:?}: {other:?}", f.pid),
            }
        }
        assert!(report.log.is_empty(), "{:?}", report.log);
        for tree in trees(&inventory) {
            found.unfinished += tree.unfinished(pager).expect("the tree reads").len();
        }
        found
    })
}

fn trees(inventory: &verify::Inventory) -> Vec<BTree> {
    let indexes = inventory.sums.iter().flat_map(|s| &s.indexes);
    indexes
        .map(|i| i.tree())
        .chain(inventory.trees.iter().copied())
        .collect()
}

/// Returns whether the closed file's log ends at no clean close.
fn unclean(rt: &SimRuntime) -> bool {
    let (pager, unclean) = pager_at(rt);
    pager.close_owing(unclean);
    unclean
}

/// Returns a file whose directories hold [`FANOUT`] entries a page, with [`ROWS`] rows of values
/// out of line and [`TAGGED`] rows with long tags, closed cleanly.
fn written() -> Sim {
    let mut sim = Sim::new(SEED, Config::default());
    run(&mut sim, |rt| {
        create_directories_with_fanout(Some(FANOUT));
        let db = opened(&rt, FILE, &log(), &quiet());
        write(&db, &wide_rows(ROWS));
        write(&db, &tagged_rows(0, TAGGED, 300));
        db.close().expect("the database closes");
    });
    sim
}

/// Returns a file holding what a crash leaves for the mend pass: each head of [`ROWS`] rows
/// counting [`PLANTED`] versions too many, an unsettled directory of the store and an unfinished
/// split of the tag index, its log ending at no clean close.
fn planted() -> Files {
    let mut sim = written();
    run(&mut sim, |rt| {
        let (pager, _) = pager_at(&rt);
        with_catalog(&rt, &pager, |pager, catalog, txns| {
            let clog = txns.commit_log();
            let structures = catalog.structures();
            let inventory = structures.inventory(Some(&clog));
            let mut unsettled = false;
            for (store, heaps) in verify::stores(&inventory) {
                for heap in heaps {
                    let pointers = heap.out_of_line(pager).expect("the heap reads");
                    for p in pointers.expect("the heap is sound") {
                        store
                            .recount(pager, p.first, 1 + PLANTED)
                            .expect("the head recounts");
                    }
                }
                unsettled |= store.unsettle(pager).expect("the store reads");
            }
            assert!(unsettled);
            let tall = inventory
                .sums
                .iter()
                .flat_map(|s| &s.indexes)
                .map(|i| i.tree())
                .max_by_key(|t| t.height(pager).expect("the tree reads"))
                .expect("an index");
            assert!(tall.height(pager).expect("the tree reads") >= 1);
            let entries = tall
                .range(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded)
                .collect(pager)
                .expect("the tree reads");
            tall.split_unfinished(pager, &entries[entries.len() / 2])
                .expect("the tree reads")
                .expect("the leaf splits");
        });
        pager.close_owing(true);
    });
    let planted = sim.files();
    let mut sim = Sim::with_files(SEED, Config::default(), planted.clone());
    let found = run(&mut sim, |rt| leftovers(&rt));
    assert_eq!(2 * ROWS, found.overcounted);
    assert!(found.unsettled > 0, "{found:?}");
    assert_eq!(1, found.unfinished);
    planted
}

/// Open the file, run `then` beside the mend pass an unclean file is owed, and close it once the
/// pass has run. Returns what the pass did, `None` when the open found the file clean.
fn mend_pass_then(rt: &SimRuntime, then: impl FnOnce(&Database<SimRuntime>)) -> Option<Mended> {
    let db: Database<SimRuntime> = opened(rt, FILE, &log(), &quiet());
    then(&db);
    let pass = mended(rt, &db);
    db.close().expect("the database closes");
    pass
}

fn mend_pass(rt: &SimRuntime) -> Option<Mended> {
    mend_pass_then(rt, |_| {})
}

#[test]
fn an_unclean_open_mends_on_the_background_worker_and_a_clean_open_schedules_none() {
    let mut sim = Sim::with_files(SEED, Config::default(), planted());
    assert!(run(&mut sim, |rt| unclean(&rt)));
    let first = run(&mut sim, |rt| mend_pass(&rt)).expect("an unclean open mends");
    assert!(first.finished(), "{first:?}");
    assert_eq!((2 * ROWS, 1), (first.heads(), first.splits()));
    assert!(first.directories() > 0, "{first:?}");
    assert_eq!(Leftovers::default(), run(&mut sim, |rt| leftovers(&rt)));
    assert!(
        !run(&mut sim, |rt| unclean(&rt)),
        "the close ends the log cleanly"
    );
    assert_eq!(None, run(&mut sim, |rt| mend_pass(&rt)));
}

#[test]
fn inserts_into_an_index_beside_the_pass_and_the_pass_both_finish() {
    let files = planted();
    let mut beside = 0;
    for seed in SEED..SEED + 4 {
        let mut sim = Sim::with_files(seed, Config::default(), files.clone());
        let (mended, overlapped) = run(&mut sim, |rt| {
            let mut overlapped = 0;
            let mended = mend_pass_then(&rt, |db| {
                for n in 0..8 {
                    overlapped += usize::from(db.testing().mended().is_none());
                    write(db, &tagged_rows(TAGGED + 4 * n, 4, 300));
                }
            });
            (mended, overlapped)
        });
        let mended = mended.expect("an unclean open mends");
        assert!(mended.finished(), "seed {seed}: {mended:?}");
        beside += overlapped;
        assert_eq!(Leftovers::default(), run(&mut sim, |rt| leftovers(&rt)));
        let rows = run(&mut sim, |rt| {
            let db = opened(&rt, FILE, &log(), &quiet());
            let rows = query(&db, "from w in wides select w.id").len();
            db.close().expect("the database closes");
            rows
        });
        assert_eq!(ROWS + TAGGED + 32, rows, "seed {seed}");
    }
    assert_reached("an insert while the pass ran", beside, 1);
}

/// Returns, by kind, how many of `found` hold something to mend.
fn reach(found: &[Leftovers]) -> Leftovers {
    let mut reached = Leftovers::default();
    for f in found {
        reached.overcounted += usize::from(f.overcounted > 0);
        reached.unsettled += usize::from(f.unsettled > 0);
        reached.unfinished += usize::from(f.unfinished > 0);
    }
    reached
}

/// Wait until the mend pass of `db` has run, if one is owed. Returns what it did.
fn mended(rt: &SimRuntime, db: &Database<SimRuntime>) -> Option<Mended> {
    let mut io = rt.io();
    for _ in 0..50 {
        if let Some(m) = db.testing().mended() {
            return Some(m);
        }
        sleep(rt, &mut io, Duration::from_millis(100));
    }
    db.testing().mended()
}

/// Open `files`, run `workload` with a thread writing the log as fast as it can, and close the
/// file, a crash armed throughout. Reopen each crash image at `most` sampled events and assert
/// the mend pass ran exactly when the image's log ends at no clean close and left nothing to
/// mend. Returns what each image held for the pass before it was reopened.
fn crashes_mend(
    files: &Files,
    most: u64,
    workload: impl FnOnce(&SimRuntime, &Arc<Database<SimRuntime>>) + 'static,
) -> Vec<Leftovers> {
    let mut found = Vec::new();
    let switching = Config {
        switch_rate: 0.3,
        ..Config::default()
    };
    each_crash_image_under(
        switching,
        files,
        SEED,
        most,
        2,
        Box::new(move |rt| {
            rt.arm(Match::any());
            let db = Arc::new(opened(rt, FILE, &log(), &quiet()));
            let done = Arc::new(AtomicBool::new(false));
            let (d, stop) = (db.clone(), done.clone());
            let r = rt.clone();
            let flusher = rt.spawn("flusher", move || {
                while !stop.load(Ordering::Relaxed) {
                    d.testing().flush_log();
                    r.yield_now();
                }
            });
            workload(rt, &db);
            done.store(true, Ordering::Relaxed);
            rt.join(flusher);
            Arc::into_inner(db)
                .expect("the flusher let go of the database")
                .close()
                .expect("the database closes");
            rt.disarm();
            Vec::new()
        }),
        &mut |at, image| {
            let mut sim = Sim::with_files(at as u64, Config::default(), image);
            let left = run(&mut sim, |rt| leftovers(&rt));
            let unclean = run(&mut sim, |rt| unclean(&rt));
            let pass = run(&mut sim, |rt| mend_pass(&rt));
            assert_eq!(unclean, pass.is_some(), "event {at}");
            if let Some(m) = &pass {
                assert!(m.finished(), "event {at}: {m:?}");
            }
            assert!(left.none() || unclean, "event {at}: {left:?}");
            assert_eq!(
                Leftovers::default(),
                run(&mut sim, |rt| leftovers(&rt)),
                "event {at}"
            );
            found.push(left);
        },
    );
    assert_reached("crash images", found.len(), 4);
    found
}

#[test]
fn a_crash_writing_values_out_of_line_leaves_what_the_pass_mends() {
    let files = written().files();
    let found = crashes_mend(&files, 24, |_, db| {
        for _ in 0..6 {
            write(db, &wide_rows(1));
        }
    });
    let reached = reach(&found);
    assert_reached("an unsettled directory", reached.unsettled, 1);
}

#[test]
fn a_crash_during_a_vacuum_leaves_what_the_pass_mends() {
    let mut sim = written();
    run(&mut sim, |rt| {
        let db = opened(&rt, FILE, &log(), &quiet());
        write(&db, "  delete w in wides where w.a != \"\"\n");
        db.close().expect("the database closes");
    });
    let found = crashes_mend(&sim.files(), 16, |_, db| {
        db.testing().vacuum("app::wides").expect("the vacuum runs");
    });
    assert_reached("an overcounted head", reach(&found).overcounted, 1);
}

#[test]
fn a_crash_during_the_mend_pass_leaves_a_file_the_next_open_finishes() {
    let files = planted();
    let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
    let planted = run(&mut sim, |rt| leftovers(&rt));
    let found = crashes_mend(&files, 16, |rt, db| {
        let pass = mended(rt, db).expect("an unclean open mends");
        assert!(pass.finished(), "{pass:?}");
    });
    let reached = reach(&found);
    let part = found.iter().filter(|f| !f.none() && **f != planted);
    assert_reached("an image holding overcounted heads", reached.overcounted, 1);
    assert_reached(
        "an image holding an unsettled directory",
        reached.unsettled,
        1,
    );
    assert_reached(
        "an image holding an unfinished split",
        reached.unfinished,
        1,
    );
    assert_reached("an image the first pass mended in part", part.count(), 1);
}
