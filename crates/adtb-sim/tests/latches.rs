use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use adtb::runtime::{Entropy, Threads};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_sys::pages::PageMemory;

use adtb_simkit::seeds;

use seeds::{share, sweep};

const PAGE: usize = 4096;
const PAGES: u64 = 4;
const CHUNK: usize = 512;

/// The latch protocol driven with a switch point between every chunk a writer stores or a
/// reader copies, so optimistic readers meet half-written pages.
struct Shared {
    seed: u64,
    mem: PageMemory,
    /// Contents of evicted pages, standing in for the file.
    disk: Mutex<BTreeMap<u64, Vec<u8>>>,
    /// Increments made to each page's counter.
    made: Mutex<BTreeMap<u64, u64>>,
    /// Optimistic reads that validated.
    validated: Mutex<u64>,
}

fn uniform(page: &[u8]) -> Option<u64> {
    let first = u64::from_le_bytes(page[..8].try_into().unwrap());
    page.chunks(8)
        .all(|w| u64::from_le_bytes(w.try_into().unwrap()) == first)
        .then_some(first)
}

fn fill_if_evicted(s: &Shared, g: &mut adtb_sys::pages::Exclusive<'_>) {
    if !g.resident() {
        let bytes = s.disk.lock().unwrap().get(&g.pid()).cloned();
        g.write(0, &bytes.unwrap_or_else(|| vec![0; PAGE]));
    }
}

fn increment(s: &Shared, rt: &SimRuntime, mut g: adtb_sys::pages::Exclusive<'_>) {
    fill_if_evicted(s, &mut g);
    let n = uniform(&g).unwrap_or_else(|| panic!("seed {}: a latched page is torn", s.seed)) + 1;
    for at in (0..PAGE).step_by(CHUNK) {
        g.write(at, &n.to_le_bytes().repeat(CHUNK / 8));
        rt.yield_now();
    }
    *s.made.lock().unwrap().entry(g.pid()).or_default() += 1;
}

fn writer(s: &Shared, rt: &SimRuntime, steps: usize) {
    for _ in 0..steps {
        let pid = rt.next_u64() % PAGES;
        if rt.next_u64().is_multiple_of(2) {
            increment(s, rt, s.mem.lock_exclusive(pid, rt));
        } else if let Some(o) = s.mem.optimistic(pid)
            && let Some(g) = o.upgrade()
        {
            increment(s, rt, g);
        }
    }
}

fn reader(s: &Shared, rt: &SimRuntime, steps: usize) {
    let mut copy = vec![0; PAGE];
    for _ in 0..steps {
        let pid = rt.next_u64() % PAGES;
        if rt.next_u64().is_multiple_of(3) {
            if let Some(g) = s.mem.lock_shared(pid, rt) {
                let before = uniform(&g);
                rt.yield_now();
                assert_eq!(
                    before,
                    uniform(&g),
                    "seed {}: a page changed under a shared latch",
                    s.seed
                );
                assert!(
                    before.is_some(),
                    "seed {}: a shared latch saw a torn page",
                    s.seed
                );
            }
            continue;
        }
        let Some(o) = s.mem.optimistic(pid) else {
            rt.yield_now();
            continue;
        };
        for at in (0..PAGE).step_by(CHUNK) {
            o.read(at, &mut copy[at..at + CHUNK]);
            rt.yield_now();
        }
        if o.validate() {
            assert!(
                uniform(&copy).is_some(),
                "seed {}: a validated read of page {pid} is torn",
                s.seed
            );
            *s.validated.lock().unwrap() += 1;
        }
    }
}

fn evictor(s: &Shared, rt: &SimRuntime, steps: usize) {
    for _ in 0..steps {
        let pid = rt.next_u64() % PAGES;
        if s.mem.mark(pid)
            && let Some(g) = s.mem.try_lock_exclusive(pid)
        {
            s.disk.lock().unwrap().insert(pid, g.to_vec());
            g.evict();
        }
        rt.yield_now();
    }
}

fn run(seed: u64) -> (BTreeMap<u64, u64>, u64) {
    let mut sim = Sim::new(seed, Config::default());
    let shared = Arc::new(Shared {
        seed,
        mem: PageMemory::reserve(PAGE, PAGES),
        disk: Mutex::default(),
        made: Mutex::default(),
        validated: Mutex::default(),
    });
    let s = shared.clone();
    let outcome = sim.run(move |rt| {
        let mut ts = Vec::new();
        for i in 0..6 {
            let (s, rt2) = (s.clone(), rt.clone());
            ts.push(rt.spawn(&format!("t{i}"), move || match i {
                0..=1 => writer(&s, &rt2, 60),
                2..=4 => reader(&s, &rt2, 80),
                _ => evictor(&s, &rt2, 200),
            }));
        }
        for t in ts {
            rt.join(t);
        }
        for pid in 0..PAGES {
            let mut g = s.mem.lock_exclusive(pid, &rt);
            fill_if_evicted(&s, &mut g);
            let made = s.made.lock().unwrap().get(&pid).copied().unwrap_or(0);
            assert_eq!(
                Some(made),
                uniform(&g),
                "seed {seed}: page {pid} lost an increment"
            );
        }
    });
    assert_eq!(Outcome::Finished, outcome, "seed {seed}");
    let made = shared.made.lock().unwrap().clone();
    let validated = *shared.validated.lock().unwrap();
    (made, validated)
}

#[test]
fn latches_keep_every_increment_and_validated_reads_whole() {
    const SEEDS: Range<u64> = 0..40;
    let mut validated = 0;
    for seed in sweep(SEEDS) {
        validated += run(seed).1;
    }
    assert!(
        validated > share(SEEDS, 100) as u64,
        "only {validated} optimistic reads validated"
    );
}

#[test]
fn a_seed_replays_the_same_interleaving() {
    assert_eq!(run(3), run(3));
}

/// Returns the optimistic reads that failed validation as one reader and one writer share a
/// page, each yielding only between its operations, with page memory switching at
/// `switch_rate`.
fn failed_validations(seed: u64, switch_rate: f64) -> u64 {
    let config = Config {
        switch_rate,
        ..Config::default()
    };
    let mut sim = Sim::new(seed, config);
    let mem = Arc::new(PageMemory::reserve(PAGE, 1));
    let failed = Arc::new(Mutex::new(0));
    let f = failed.clone();
    let outcome = sim.run(move |rt| {
        mem.lock_exclusive(0, &rt).write(0, &[0; PAGE]);
        let (m, rt2) = (mem.clone(), rt.clone());
        let writer = rt.spawn("writer", move || {
            for n in 0..100u64 {
                m.lock_exclusive(0, &rt2).write(0, &n.to_le_bytes());
                rt2.yield_now();
            }
        });
        let (m, rt2) = (mem.clone(), rt.clone());
        let reader = rt.spawn("reader", move || {
            let mut copy = vec![0; PAGE];
            for _ in 0..100 {
                if let Some(o) = m.optimistic(0) {
                    o.read(0, &mut copy);
                    if !o.validate() {
                        *f.lock().unwrap() += 1;
                    }
                }
                rt2.yield_now();
            }
        });
        rt.join(writer);
        rt.join(reader);
    });
    assert_eq!(Outcome::Finished, outcome, "seed {seed}");
    *failed.lock().unwrap()
}

#[test]
fn switch_points_fail_optimistic_reads_only_when_enabled() {
    const SEEDS: Range<u64> = 0..10;
    let off: u64 = sweep(SEEDS).map(|seed| failed_validations(seed, 0.0)).sum();
    let on: u64 = sweep(SEEDS).map(|seed| failed_validations(seed, 0.5)).sum();
    assert_eq!(off, 0);
    assert!(
        on >= share(SEEDS, 10) as u64,
        "only {on} reads failed validation"
    );
}
