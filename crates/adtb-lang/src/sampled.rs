//! Count the hits of the front end's caches and choose which of them to check against the
//! result computed again.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};

/// The share of hits checked: one in this many.
const EVERY: u64 = 16;

thread_local! {
    static EVERY_HIT: Cell<bool> = const { Cell::new(false) };
    /// The hits of each sampler on this thread, by its index.
    static HITS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// The hits of one cache, and how many of them were checked.
#[derive(Debug)]
pub struct Sampler {
    index: usize,
    checked: AtomicU64,
}

impl Sampler {
    const fn new(index: usize) -> Sampler {
        Sampler {
            index,
            checked: AtomicU64::new(0),
        }
    }

    /// Count a hit on this thread. Returns whether to check it: the first hit of this cache on
    /// the thread and every sixteenth after, and every hit inside [`checking_every_hit`].
    pub(crate) fn due(&self) -> bool {
        let n = HITS.with(|h| {
            let mut h = h.borrow_mut();
            if h.len() <= self.index {
                h.resize(self.index + 1, 0);
            }
            h[self.index] += 1;
            h[self.index] - 1
        });
        let due = n.is_multiple_of(EVERY) || EVERY_HIT.with(Cell::get);
        if due {
            self.checked.fetch_add(1, Ordering::Relaxed);
        }
        due
    }

    /// Returns how many hits this process checked.
    pub fn checked(&self) -> u64 {
        self.checked.load(Ordering::Relaxed)
    }
}

/// The hits of the build memo of a source.
pub static BUILDS: Sampler = Sampler::new(0);

/// The hits of the build memo of a migration's model.
pub static MODEL_BUILDS: Sampler = Sampler::new(1);

/// The hits of the checks a build holds.
pub static CHECKS: Sampler = Sampler::new(3);

/// The uses of the engine's fixed ids a build of the built-in declarations shares.
pub static ENGINE_IDS: Sampler = Sampler::new(2);

/// The uses of a logged migration's held replay.
pub static REPLAYS: Sampler = Sampler::new(4);

/// Run `f`, checking every hit this thread makes while it runs.
pub fn checking_every_hit<T>(f: impl FnOnce() -> T) -> T {
    let was = EVERY_HIT.with(|e| e.replace(true));
    let out = f();
    EVERY_HIT.with(|e| e.set(was));
    out
}
