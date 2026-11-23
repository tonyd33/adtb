use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Waker;

use super::sched::{Admission, Entry, Priority, Refusal};
use super::{Error, Exceeded};
use crate::runtime::Instant;

/// The query memory of a process: what the grants of running queries may hold together, and
/// the queue of calls waiting for a grant that does not fit.
#[derive(Debug)]
pub struct Budget {
    limit: usize,
    bound: Admission,
    queue: Mutex<Queue>,
}

/// Highest class first, then highest rank, then the oldest transaction, then the earliest
/// queued.
type Order = (Reverse<Priority>, Reverse<u8>, Instant, u64);

#[derive(Debug, Default)]
struct Queue {
    granted: usize,
    next: u64,
    waiting: BTreeMap<Order, Waiting>,
    /// The sum of the waiting calls' [`Entry::bytes`].
    held: usize,
    /// The grants admitted to calls that left the queue and have not taken them.
    admitted: BTreeMap<u64, Arc<Grant>>,
}

#[derive(Debug)]
struct Waiting {
    grant: usize,
    bytes: usize,
    waker: Option<Waker>,
}

/// How a request for a grant ended.
#[derive(Debug)]
pub enum Requested {
    Granted(Arc<Grant>),
    /// The call waits in the budget's queue until the ticket yields its grant.
    Queued(Ticket),
}

/// A call's place in a budget's queue, given up when dropped.
#[derive(Debug)]
pub struct Ticket {
    budget: Arc<Budget>,
    order: Order,
}

impl Budget {
    /// Returns a budget of `limit` bytes whose queue is bounded as [`Admission::default`] is.
    pub fn new(limit: usize) -> Arc<Budget> {
        Budget::bounded(limit, Admission::default())
    }

    /// Returns a budget of `limit` bytes whose queue holds at most `bound.entries` calls and
    /// `bound.bytes` of their bytes.
    pub fn bounded(limit: usize, bound: Admission) -> Arc<Budget> {
        Arc::new(Budget {
            limit,
            bound,
            queue: Mutex::default(),
        })
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Returns the bound of the queue's bytes.
    pub fn queue_bytes(&self) -> usize {
        self.bound.bytes
    }

    /// Returns the bytes the grants now admitted hold.
    #[cfg(any(test, feature = "testing"))]
    pub fn granted(&self) -> usize {
        self.lock().granted
    }

    /// Returns how many calls wait in the queue.
    #[cfg(any(test, feature = "testing"))]
    pub fn waiting(&self) -> usize {
        self.lock().waiting.len()
    }

    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn grant(self: &Arc<Budget>, bytes: usize) -> Arc<Grant> {
        Arc::new(Grant {
            budget: self.clone(),
            limit: bytes,
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            spilled: AtomicUsize::new(0),
        })
    }

    /// Admit a query whose grant is `bytes`. Returns `None`, admitting nothing, when a call
    /// waits in the queue, or when the grants of running queries and this one together would
    /// exceed the budget.
    pub fn admit(self: &Arc<Budget>, bytes: usize) -> Option<Arc<Grant>> {
        let mut q = self.lock();
        let next = q.granted.checked_add(bytes).filter(|n| *n <= self.limit)?;
        if !q.waiting.is_empty() {
            return None;
        }
        q.granted = next;
        drop(q);
        Some(self.grant(bytes))
    }

    /// Request a grant of `bytes` for the call `entry` describes: granted at once when it fits
    /// and no call waits, else queued by the entry's priority, rank and start.
    ///
    /// Refuse, queueing nothing: with `TooLarge` when the call's bytes alone pass the queue's
    /// byte bound or `bytes` pass the budget, and with `Overloaded` when the queue would pass its
    /// bounds.
    pub fn request(self: &Arc<Budget>, entry: &Entry, bytes: usize) -> Result<Requested, Refusal> {
        if entry.bytes > self.bound.bytes || bytes > self.limit {
            return Err(Refusal::TooLarge);
        }
        let mut q = self.lock();
        if q.waiting.is_empty() && q.granted + bytes <= self.limit {
            q.granted += bytes;
            drop(q);
            return Ok(Requested::Granted(self.grant(bytes)));
        }
        if q.waiting.len() >= self.bound.entries || q.held + entry.bytes > self.bound.bytes {
            return Err(Refusal::Overloaded);
        }
        let order = (
            Reverse(entry.priority),
            Reverse(entry.rank),
            entry.start,
            q.next,
        );
        q.next += 1;
        q.held += entry.bytes;
        q.waiting.insert(
            order,
            Waiting {
                grant: bytes,
                bytes: entry.bytes,
                waker: None,
            },
        );
        Ok(Requested::Queued(Ticket {
            budget: self.clone(),
            order,
        }))
    }

    /// Return `bytes` of released grants, then admit the head of the queue while its grant
    /// fits, waking each call admitted.
    fn release(self: &Arc<Budget>, bytes: usize) {
        let mut woken = Vec::new();
        {
            let mut q = self.lock();
            q.granted -= bytes;
            self.admit_heads(&mut q, &mut woken);
        }
        for w in woken {
            w.wake();
        }
    }

    fn admit_heads(self: &Arc<Budget>, q: &mut Queue, woken: &mut Vec<Waker>) {
        while let Some(entry) = q.waiting.first_entry() {
            let grant = entry.get().grant;
            if q.granted + grant > self.limit {
                break;
            }
            let (order, w) = entry.remove_entry();
            q.granted += grant;
            q.held -= w.bytes;
            q.admitted.insert(order.3, self.grant(grant));
            woken.extend(w.waker);
        }
    }
}

impl Ticket {
    /// Returns the grant the queue admitted the call to, once it has; else register `waker`
    /// to be woken when it does.
    pub fn poll(&self, waker: &Waker) -> Option<Arc<Grant>> {
        let mut q = self.budget.lock();
        if let Some(g) = q.admitted.remove(&self.order.3) {
            return Some(g);
        }
        if let Some(w) = q.waiting.get_mut(&self.order) {
            w.waker = Some(waker.clone());
        }
        None
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut woken = Vec::new();
        let unclaimed = {
            let mut q = self.budget.lock();
            if let Some(w) = q.waiting.remove(&self.order) {
                q.held -= w.bytes;
                self.budget.admit_heads(&mut q, &mut woken);
            }
            q.admitted.remove(&self.order.3)
        };
        drop(unclaimed);
        for w in woken {
            w.wake();
        }
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
    spilled: AtomicUsize,
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

    /// Returns the bytes the query wrote to its spill file.
    pub fn spilled(&self) -> usize {
        self.spilled.load(Ordering::Acquire)
    }

    pub(super) fn note_spill(&self, bytes: usize) {
        self.spilled.fetch_add(bytes, Ordering::AcqRel);
    }

    /// Returns an arena of this grant holding nothing.
    pub fn arena(self: &Arc<Grant>) -> Arena {
        Arena {
            grant: self.clone(),
            bytes: 0,
        }
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
        self.budget.release(self.limit);
    }
}

/// Memory taken from a grant, returned to it when dropped.
#[derive(Debug)]
pub struct Arena {
    grant: Arc<Grant>,
    bytes: usize,
}

impl Arena {
    pub(super) fn grant(&self) -> &Arc<Grant> {
        &self.grant
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Take `bytes` more into this arena.
    pub fn grow(&mut self, bytes: usize) -> Result<(), Error> {
        self.grant.reserve(bytes)?;
        self.bytes += bytes;
        Ok(())
    }

    /// Return up to `bytes` of this arena to the grant.
    pub fn release(&mut self, bytes: usize) {
        let n = bytes.min(self.bytes);
        self.bytes -= n;
        self.grant.used.fetch_sub(n, Ordering::AcqRel);
    }

    /// Take over what `other` holds.
    pub fn absorb(&mut self, mut other: Arena) {
        assert!(
            Arc::ptr_eq(&self.grant, &other.grant),
            "an arena absorbs only arenas of its own grant"
        );
        self.bytes += std::mem::take(&mut other.bytes);
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        self.grant.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// The bytes a value held in a grant is charged.
pub trait Footprint {
    fn footprint(&self) -> usize;
}

impl<A: Footprint, B: Footprint> Footprint for (A, B) {
    fn footprint(&self) -> usize {
        self.0.footprint() + self.1.footprint()
    }
}

impl Footprint for crate::key::Tid {
    fn footprint(&self) -> usize {
        size_of::<Self>()
    }
}

impl Footprint for u32 {
    fn footprint(&self) -> usize {
        size_of::<u32>()
    }
}

impl Footprint for adtb_lang::types::RelId {
    fn footprint(&self) -> usize {
        size_of::<Self>()
    }
}

/// A collection whose items are charged to a grant before they are held and returned to it
/// as they leave.
#[derive(Debug)]
pub struct Held<C> {
    items: C,
    arena: Arena,
}

impl Grant {
    /// Returns an empty collection of this grant.
    pub fn held<C: Default>(self: &Arc<Grant>) -> Held<C> {
        Held {
            items: C::default(),
            arena: self.arena(),
        }
    }
}

impl<C> std::ops::Deref for Held<C> {
    type Target = C;

    fn deref(&self) -> &C {
        &self.items
    }
}

impl<T: Footprint> Held<Vec<T>> {
    pub fn push(&mut self, t: T) -> Result<(), Error> {
        self.arena.grow(t.footprint())?;
        self.items.push(t);
        Ok(())
    }

    pub fn sort(&mut self)
    where
        T: Ord,
    {
        self.items.sort();
    }
}

impl<T: Ord + Footprint> Held<std::collections::BTreeSet<T>> {
    /// Add `t`. Returns whether it was absent.
    pub fn insert(&mut self, t: T) -> Result<bool, Error> {
        let n = t.footprint();
        self.arena.grow(n)?;
        let fresh = self.items.insert(t);
        if !fresh {
            self.arena.release(n);
        }
        Ok(fresh)
    }
}

impl<K: Ord + Footprint, V: Footprint> Held<BTreeMap<K, V>> {
    /// Map `k` to `v`, returning to the grant what the value `k` mapped to and the replaced key
    /// were charged.
    pub fn insert(&mut self, k: K, v: V) -> Result<(), Error> {
        let key = k.footprint();
        self.arena.grow(key + v.footprint())?;
        if let Some(old) = self.items.insert(k, v) {
            self.arena.release(key + old.footprint());
        }
        Ok(())
    }
}

impl<C> IntoIterator for Held<C>
where
    C: IntoIterator,
    C::Item: Footprint,
{
    type Item = C::Item;
    type IntoIter = Released<C::IntoIter>;

    fn into_iter(self) -> Released<C::IntoIter> {
        Released {
            items: self.items.into_iter(),
            arena: self.arena,
        }
    }
}

/// The items of a [`Held`] collection, each returned to the grant as it is taken.
#[derive(Debug)]
pub struct Released<I> {
    items: I,
    arena: Arena,
}

impl<I> Iterator for Released<I>
where
    I: Iterator,
    I::Item: Footprint,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<I::Item> {
        let t = self.items.next()?;
        self.arena.release(t.footprint());
        Some(t)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::task::Waker;

    use super::{Budget, Requested, Ticket};
    use crate::exec::sched::{Admission, Entry, Priority, Refusal};
    use crate::runtime::Instant;

    fn entry(priority: Priority, start: u64, bytes: usize) -> Entry {
        Entry {
            priority,
            rank: u8::MAX,
            start: Instant::from_nanos(start),
            bytes,
            deadline: None,
        }
    }

    fn queued(b: &Arc<Budget>, e: &Entry) -> Ticket {
        match b.request(e, 10) {
            Ok(Requested::Queued(t)) => t,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_released_grant_admits_the_queue_by_priority_then_start_within_its_bounds() {
        let b = Budget::bounded(
            20,
            Admission {
                entries: 3,
                bytes: 30,
            },
        );
        let held: Vec<_> = (0..2)
            .map(|i| match b.request(&entry(Priority::AdHoc, i, 0), 10) {
                Ok(Requested::Granted(g)) => g,
                other => panic!("{other:?}"),
            })
            .collect();
        let late = queued(&b, &entry(Priority::AdHoc, 5, 10));
        let early = queued(&b, &entry(Priority::AdHoc, 1, 10));
        let high = queued(&b, &entry(Priority::Regular, 9, 10));
        assert_eq!(
            Refusal::Overloaded,
            b.request(&entry(Priority::Regular, 0, 0), 10).unwrap_err()
        );
        assert_eq!(
            Refusal::TooLarge,
            b.request(&entry(Priority::Regular, 0, 31), 10).unwrap_err()
        );
        assert!(b.admit(10).is_none());
        let mut taken = Vec::new();
        let mut admitted = Vec::new();
        for g in held {
            drop(g);
            admitted.push([&high, &early, &late].map(|t| match t.poll(Waker::noop()) {
                Some(g) => {
                    taken.push(g);
                    true
                }
                None => false,
            }));
        }
        assert_eq!(vec![[true, false, false], [false, true, false]], admitted);
        drop(taken.pop());
        assert!(late.poll(Waker::noop()).is_some());
        assert_eq!(0, b.waiting());
    }
}
