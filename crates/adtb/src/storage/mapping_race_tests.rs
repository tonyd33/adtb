//! The damage mapper racing a thread that frees, restarts, allocates or retires the page it maps.

use crate::testing::Probe;

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::runtime::{Entropy, Threads};
use crate::storage::order::{Ordered, Orders, ordered_for_test};
use crate::storage::page;
use crate::storage::policy::{Holder, Phase};
use crate::storage::{Damage, Lsn, Options, PageKind, Pager, Pid, Quarantine};
use crate::test_world as harness;
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::damage::Files;
use adtb_testkit::seeds::{assert_reached, share, sweep};

use harness::finished;

const SEEDS: Range<u64> = 0..64;
/// The most steps the racer waits for the mapper's walk, and then before its change.
const WAIT: u64 = 4096;
/// The most steps a seed's racer waits once the walk returned.
const LATE: u64 = 512;

fn options() -> Options {
    Options {
        threads: 3,
        ..harness::options(16)
    }
}

fn switching(seed: u64, files: Files) -> Sim {
    let config = Config {
        switch_rate: 0.5,
        ..Config::default()
    };
    Sim::with_files(seed, config, files)
}

/// Returns the holder of `object`, a relation's heap or index.
fn holder(object: Quarantine) -> BTreeSet<Holder> {
    BTreeSet::from([Holder::Relation {
        object,
        engine_module: false,
    }])
}

/// Returns the floor of a sweep's count of a case reached `count` times in [`SEEDS`], at least 1.
fn floor(count: usize) -> usize {
    share(SEEDS, count).max(1)
}

/// The pages of [`pages`]: three allocated, and one allocated and freed, the lowest free page.
#[derive(Clone, Copy, Debug)]
struct Pages {
    p: Pid,
    q: Pid,
    r: Pid,
    freed: Pid,
}

/// Returns a closed file holding [`Pages`].
fn pages() -> (Files, Pages) {
    let mut sim = Sim::new(1, Config::default());
    let mut pages = None;
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options()).unwrap();
        let new = || pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let (p, q, r, freed) = (new(), new(), new(), new());
        pager.free(freed).unwrap();
        pager.close();
        pages = Some(Pages { p, q, r, freed });
    });
    (sim.files(), pages.unwrap())
}

/// A source of commit order that waits a seeded number of steps before each hold.
struct Yielding<'a>(&'a SimRuntime);

impl Orders for Yielding<'_> {
    fn ordered<T>(&self, f: impl FnOnce(Ordered<'_>) -> T) -> T {
        for _ in 0..self.0.next_u64() % LATE {
            self.0.yield_now();
        }
        f(ordered_for_test())
    }
}

/// What the racer does once the mapper's walk returned.
#[derive(Clone, Copy, Debug)]
enum Racer {
    /// Free the damaged heap page.
    Free,
    /// Overwrite the damaged heap page with another's contents, starting its chain again.
    Restart,
    /// Allocate the damaged page, free when the walk began.
    Allocate,
    /// Retire the damaged index page and swap another root in, releasing the index's mark, as a
    /// reindex does.
    Swap,
}

impl Racer {
    /// Returns the object the mapper maps the damaged page to.
    fn object(self) -> Quarantine {
        match self {
            Racer::Swap => Quarantine::Index(1),
            Racer::Free | Racer::Restart | Racer::Allocate => Quarantine::Heap(1),
        }
    }

    /// Returns the damaged page of `pages`.
    fn damaged(self, pages: Pages) -> Pid {
        match self {
            Racer::Allocate => pages.freed,
            Racer::Free | Racer::Restart | Racer::Swap => pages.p,
        }
    }

    /// Change the damaged page of `pages`. Returns the LSN of the change.
    fn run(self, pager: &Pager<SimRuntime>, pages: Pages) -> Lsn {
        let Pages { p, q, r, freed } = pages;
        match self {
            Racer::Free => pager.free(p).unwrap(),
            Racer::Restart => {
                pager.replace_contents(p, q, None).unwrap();
                page::lsn(&pager.read(p).unwrap())
            }
            Racer::Allocate => {
                let allocated = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
                assert_eq!(freed, allocated, "the racer allocates the free page");
                page::lsn(&pager.read(freed).unwrap())
            }
            Racer::Swap => {
                pager.retire([p]);
                let index = Some(Quarantine::Index(1));
                pager.replace_contents(q, r, index).unwrap();
                page::lsn(&pager.read(q).unwrap())
            }
        }
    }
}

/// A span one thread marks for another: begun once `start` is set, over once `end` is.
#[derive(Clone, Default)]
struct Span {
    start: Arc<AtomicBool>,
    end: Arc<AtomicBool>,
}

impl Span {
    fn begun(&self) -> bool {
        self.start.load(Ordering::Acquire)
    }

    fn over(&self) -> bool {
        self.end.load(Ordering::Acquire)
    }
}

/// Wait for `span` to begin, at most [`WAIT`] steps, then a seeded number of steps under `most`.
fn wait_into(rt: &SimRuntime, span: &Span, most: u64) {
    for _ in 0..WAIT {
        if span.begun() {
            break;
        }
        rt.yield_now();
    }
    for _ in 0..rt.next_u64() % most {
        rt.yield_now();
    }
}

/// What one race of the mapper against the racer gave.
#[derive(Debug)]
struct Raced {
    marks: BTreeSet<Quarantine>,
    /// Whether the marks page changed, which only a mark the mapper set starts.
    marked: bool,
    /// The LSN of the last change of the marks page.
    marked_at: Lsn,
    /// The LSN of the racer's change.
    changed_at: Lsn,
    /// Whether the racer's change began once the walk returned.
    after_walk: bool,
    /// Whether the racer's change began once the walk returned and before the mapping did.
    in_window: bool,
}

/// Damage a page of `pages` and map it to the racer's object under `seed`'s interleaving, while a
/// second thread changes it as `racer` says, from a seeded step after the walk returns.
fn race(seed: u64, files: &Files, pages: Pages, racer: Racer) -> Raced {
    let mut sim = switching(seed, files.clone());
    let mut out = None;
    let (object, damaged) = (racer.object(), racer.damaged(pages));
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options()).unwrap();
        pager.damage(damaged, Damage::Checksum);
        let marks_page = || page::lsn(&pager.read(pager.quarantine_page()).unwrap());
        let unmarked_at = marks_page();
        let after_walk = Span::default();
        let (pager2, rt2, span) = (pager.clone(), rt.clone(), after_walk.clone());
        let changed = Arc::new(Mutex::new(None));
        let into = changed.clone();
        let t = rt.spawn("racer", move || {
            wait_into(&rt2, &span, LATE);
            let (after_walk, in_window) = (span.begun(), span.begun() && !span.over());
            let changed_at = racer.run(&pager2, pages);
            *into.lock().unwrap() = Some((changed_at, after_walk, in_window));
        });
        pager.map_damage(
            |_| {
                after_walk.start.store(true, Ordering::Release);
                Some(holder(object))
            },
            &Yielding(&rt),
        );
        after_walk.end.store(true, Ordering::Release);
        rt.join(t);
        let (changed_at, after_walk, in_window) = changed.lock().unwrap().take().unwrap();
        let marked_at = marks_page();
        out = Some(Raced {
            marks: Probe::of(&pager).marks(),
            marked: marked_at > unmarked_at,
            marked_at,
            changed_at,
            after_walk,
            in_window,
        });
    });
    out.unwrap()
}

/// Race the mapper against `racer` on every seed. Check a mark set was set before the racer's
/// change and that a swap leaves no mark on the index it repaired. Check that changes begun after
/// the walk followed a mark, and that ones begun before the mapping's end preceded it.
fn every_mark_precedes_the_change_of_its_page(racer: Racer) {
    let (files, pages) = pages();
    let (mut marked, mut unmarked) = (0, 0);
    for seed in sweep(SEEDS) {
        let r = race(seed, &files, pages, racer);
        if let Racer::Swap = racer {
            assert_eq!(BTreeSet::new(), r.marks, "seed {seed}: {r:?}");
        }
        if r.marked {
            if !matches!(racer, Racer::Swap) {
                assert!(r.marked_at < r.changed_at, "seed {seed}: {r:?}");
                assert_eq!(
                    BTreeSet::from([racer.object()]),
                    r.marks,
                    "seed {seed}: {r:?}"
                );
            }
            marked += usize::from(r.after_walk);
        } else {
            assert_eq!(BTreeSet::new(), r.marks, "seed {seed}: {r:?}");
            unmarked += usize::from(r.in_window);
        }
    }
    assert_reached(
        "a mark set before a change begun after the walk",
        marked,
        floor(4),
    );
    assert_reached(
        "a change begun after the walk and before the mapping's end, then no mark",
        unmarked,
        floor(4),
    );
}

#[test]
fn a_free_racing_the_mapper_leaves_no_mark_set_after_it() {
    every_mark_precedes_the_change_of_its_page(Racer::Free);
}

#[test]
fn a_restart_racing_the_mapper_leaves_no_mark_set_after_it() {
    every_mark_precedes_the_change_of_its_page(Racer::Restart);
}

#[test]
fn an_allocation_racing_the_mapper_leaves_no_mark_set_after_it() {
    every_mark_precedes_the_change_of_its_page(Racer::Allocate);
}

#[test]
fn a_swap_racing_the_mapper_leaves_no_mark_on_the_repaired_index() {
    every_mark_precedes_the_change_of_its_page(Racer::Swap);
}

#[test]
fn a_page_free_before_the_walk_and_allocated_during_it_is_held_free() {
    let (files, Pages { freed: p, .. }) = pages();
    let mut sim = Sim::with_files(1, Config::default(), files);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        let held = pager.holders_of(p, Phase::Running, |_| {
            let allocated = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
            assert_eq!(p, allocated, "the walk allocates the free page");
        });
        assert_eq!(Some(BTreeSet::from([Holder::Free])), held);
    });
}

#[test]
fn a_page_retired_during_the_walk_is_held_retired() {
    let (files, Pages { p, .. }) = pages();
    let mut sim = Sim::with_files(1, Config::default(), files);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        let held = pager.holders_of(p, Phase::Running, |_| pager.retire([p]));
        assert_eq!(Some(BTreeSet::from([Holder::Retired])), held);
    });
}
