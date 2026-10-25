use std::collections::BTreeMap;

use crate::key::Tid;
use crate::storage::Pid;

const NODE_BYTES: usize = 64;

/// The TIDs vacuum has found dead, keyed by heap id and page, each page's slots a bitmap.
///
/// A page may be present with no dead slot: it is still to be pruned.
#[derive(Debug, Default)]
pub struct DeadSet {
    heaps: BTreeMap<u32, BTreeMap<Pid, Vec<u64>>>,
    bytes: usize,
    len: usize,
}

impl DeadSet {
    pub fn new() -> DeadSet {
        DeadSet::default()
    }

    /// Add page `pid` of heap `heap` with its dead `slots`.
    pub fn add(&mut self, heap: u32, pid: Pid, slots: &[u16]) {
        let pages = self.heaps.entry(heap).or_insert_with(|| {
            self.bytes += NODE_BYTES;
            BTreeMap::new()
        });
        let bits = pages.entry(pid).or_insert_with(|| {
            self.bytes += NODE_BYTES;
            Vec::new()
        });
        for &s in slots {
            let word = usize::from(s / 64);
            if bits.len() <= word {
                self.bytes += 8 * (word + 1 - bits.len());
                bits.resize(word + 1, 0);
            }
            let bit = 1 << (s % 64);
            if bits[word] & bit == 0 {
                bits[word] |= bit;
                self.len += 1;
            }
        }
    }

    pub fn contains(&self, heap: u32, tid: Tid) -> bool {
        let Some(bits) = self
            .heaps
            .get(&heap)
            .and_then(|pages| pages.get(&Pid(tid.page())))
        else {
            return false;
        };
        let s = usize::from(tid.slot());
        bits.get(s / 64).is_some_and(|w| w >> (s % 64) & 1 == 1)
    }

    /// Returns each page of heap `heap` in the set, in page order, with its dead slots.
    pub fn pages(&self, heap: u32) -> Vec<(Pid, Vec<u16>)> {
        let Some(pages) = self.heaps.get(&heap) else {
            return Vec::new();
        };
        pages
            .iter()
            .map(|(&pid, bits)| {
                let slots = (0..bits.len() * 64)
                    .filter(|&s| bits[s / 64] >> (s % 64) & 1 == 1)
                    .map(|s| s as u16)
                    .collect();
                (pid, slots)
            })
            .collect()
    }

    /// Returns the number of dead TIDs.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the set holds no page.
    pub fn is_empty(&self) -> bool {
        self.heaps.is_empty()
    }

    /// Returns an estimate of the memory the set takes.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn clear(&mut self) {
        *self = DeadSet::default();
    }
}
