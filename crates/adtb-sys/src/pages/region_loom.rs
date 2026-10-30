use crate::sync::{AtomicU64, Ordering};

/// Zeroed model atomics standing in for reserved memory.
pub(super) struct Region {
    words: Box<[AtomicU64]>,
}

impl Region {
    pub(super) fn reserve(len: usize) -> Region {
        Region {
            words: (0..len / 8).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    pub(super) fn words(&self) -> &[AtomicU64] {
        &self.words
    }

    pub(super) fn release(&self, offset: usize, len: usize) {
        for w in &self.words[offset / 8..(offset + len) / 8] {
            w.store(0, Ordering::Relaxed);
        }
    }
}
