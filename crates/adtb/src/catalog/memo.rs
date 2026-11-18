//! A bounded memo of values by key, safe to share between threads.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// At most `cap` values by key, least recently used first.
pub(super) struct Memo<K, V> {
    cap: usize,
    entries: Mutex<Vec<(K, Arc<V>)>>,
}

impl<K: PartialEq, V> Memo<K, V> {
    pub(super) const fn new(cap: usize) -> Memo<K, V> {
        Memo {
            cap,
            entries: Mutex::new(Vec::new()),
        }
    }

    /// Returns the value held for `key`, else the value `make` returns, held from then on when
    /// `make` succeeds. Holding a value past the bound evicts the least recently used one.
    ///
    /// `make` runs with the memo unlocked, so two threads missing on one key may both run it.
    pub(super) fn get_or_try_insert<E>(
        &self,
        key: K,
        make: impl FnOnce() -> Result<V, E>,
    ) -> Result<Arc<V>, E> {
        if let Some(v) = self.touched(&key) {
            return Ok(v);
        }
        let v = Arc::new(make()?);
        if let Some(held) = self.touched(&key) {
            return Ok(held);
        }
        let mut entries = self.entries();
        entries.push((key, v.clone()));
        if entries.len() > self.cap {
            entries.remove(0);
        }
        Ok(v)
    }

    /// Returns the value held for `key`, made the most recently used.
    fn touched(&self, key: &K) -> Option<Arc<V>> {
        let mut entries = self.entries();
        let at = entries.iter().position(|(k, _)| k == key)?;
        let hit = entries.remove(at);
        let v = hit.1.clone();
        entries.push(hit);
        Some(v)
    }

    fn entries(&self) -> MutexGuard<'_, Vec<(K, Arc<V>)>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries().len()
    }
}
