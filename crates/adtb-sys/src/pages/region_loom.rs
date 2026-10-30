use loom::cell::UnsafeCell;

use crate::sync::{AtomicU64, Ordering};

/// Zeroed model atomics standing in for reserved memory, each word with a cell that records the
/// plain accesses production code makes to it through page bytes.
pub(super) struct Region {
    words: Box<[AtomicU64]>,
    plain: Box<[UnsafeCell<()>]>,
}

impl Region {
    pub(super) fn reserve(len: usize) -> Region {
        Region {
            words: (0..len / 8).map(|_| AtomicU64::new(0)).collect(),
            plain: (0..len / 8).map(|_| UnsafeCell::new(())).collect(),
        }
    }

    pub(super) fn words(&self) -> &[AtomicU64] {
        &self.words
    }

    pub(super) fn release(&self, offset: usize, len: usize) {
        for w in offset / 8..(offset + len) / 8 {
            self.words[w].store(0, Ordering::Relaxed);
            self.written(w);
        }
    }

    /// Record a store to word `w` that a plain read of it must not race.
    pub(super) fn written(&self, w: usize) {
        self.plain[w].with_mut(|_| ());
    }

    /// Returns word `w` as a plain read through page bytes would see it.
    pub(super) fn plain_read(&self, w: usize) -> u64 {
        self.plain[w].with(|_| self.words[w].load(Ordering::Relaxed))
    }
}
