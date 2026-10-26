use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Error, Exceeded};

/// The query memory of a process: what the grants of running queries may hold together.
#[derive(Debug)]
pub struct Budget {
    limit: usize,
    granted: AtomicUsize,
}

impl Budget {
    pub fn new(limit: usize) -> Arc<Budget> {
        Arc::new(Budget {
            limit,
            granted: AtomicUsize::new(0),
        })
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Returns the bytes the grants now admitted hold.
    pub fn granted(&self) -> usize {
        self.granted.load(Ordering::Acquire)
    }

    /// Admit a query whose grant is `bytes`. Returns `None`, admitting nothing, when the grants
    /// of running queries and this one together would exceed the budget.
    pub fn admit(self: &Arc<Budget>, bytes: usize) -> Option<Arc<Grant>> {
        let mut now = self.granted.load(Ordering::Acquire);
        loop {
            let next = now.checked_add(bytes).filter(|n| *n <= self.limit)?;
            match self
                .granted
                .compare_exchange(now, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(seen) => now = seen,
            }
        }
        Some(Arc::new(Grant {
            budget: self.clone(),
            limit: bytes,
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }))
    }
}

/// The memory one query may hold, returned to its budget when the query's last holder drops it.
///
/// Invariant: `used` never exceeds `limit`.
#[derive(Debug)]
pub struct Grant {
    budget: Arc<Budget>,
    limit: usize,
    used: AtomicUsize,
    peak: AtomicUsize,
}

impl Grant {
    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    /// Returns the most the query held at once.
    pub fn peak(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }

    /// Take `bytes` of the grant until the returned arena is dropped. Fails with the statement
    /// limit, taking nothing, when the grant has less than `bytes` left.
    pub fn take(self: &Arc<Grant>, bytes: usize) -> Result<Arena, Error> {
        self.reserve(bytes)?;
        Ok(Arena {
            grant: self.clone(),
            bytes,
        })
    }

    fn reserve(&self, bytes: usize) -> Result<(), Error> {
        let mut now = self.used.load(Ordering::Acquire);
        loop {
            let next = now.checked_add(bytes).filter(|n| *n <= self.limit).ok_or(
                Error::StatementLimit(Exceeded::Memory { limit: self.limit }),
            )?;
            match self
                .used
                .compare_exchange(now, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => {
                    self.peak.fetch_max(next, Ordering::AcqRel);
                    break;
                }
                Err(seen) => now = seen,
            }
        }
        Ok(())
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        self.budget.granted.fetch_sub(self.limit, Ordering::AcqRel);
    }
}

/// Memory taken from a grant, returned to it when dropped.
#[derive(Debug)]
pub struct Arena {
    grant: Arc<Grant>,
    bytes: usize,
}

impl Arena {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Take `bytes` more into this arena.
    pub fn grow(&mut self, bytes: usize) -> Result<(), Error> {
        self.grant.reserve(bytes)?;
        self.bytes += bytes;
        Ok(())
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        self.grant.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
