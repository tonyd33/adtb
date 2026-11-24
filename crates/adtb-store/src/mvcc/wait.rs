use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::task::{Wake, Waker};

use crate::runtime::{Runtime, ThreadId};

use super::conflict::deadlock;
use super::{Conflict, Xid};
use adtb_sys::lock::Ranked;

#[derive(Debug, Default)]
struct State {
    running: BTreeSet<Xid>,
    /// The wakers registered on each running transaction, each with the transaction it waits as.
    watchers: BTreeMap<Xid, Vec<(Option<Xid>, Waker)>>,
    waits_for: BTreeMap<Xid, Xid>,
    /// The transactions whose statements hold a memory grant, with how many they hold.
    granted: BTreeMap<Xid, usize>,
    /// The transactions waiting in the budget's queue for a grant. Each waits for every
    /// transaction in `granted`.
    queued: BTreeSet<Xid>,
    /// The waiters refused to break a cycle through a grant wait, with the cycle.
    refused: BTreeMap<Xid, Vec<Xid>>,
    /// The thread each running transaction began on.
    #[cfg(any(test, feature = "testing"))]
    begun_on: BTreeMap<Xid, ThreadId>,
}

/// What a registration with [`Waits::watch`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watch {
    /// The transaction had already ended; nothing was registered.
    Ended,
    /// The waker is registered and is woken once the transaction ends.
    Pending,
}

/// A pseudo-transaction no begin assigns, running while a [`Gated`] of it lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Running while an offline rebuild holds writes to a relation.
    Offline,
    /// Running while a thread holds the vacuum lock.
    VacuumHolder,
}

impl Gate {
    /// Returns the id waiters wait on.
    pub fn xid(self) -> Xid {
        match self {
            Gate::Offline => Xid::SENTINELS[1],
            Gate::VacuumHolder => Xid::SENTINELS[0],
        }
    }
}

/// A gate started in the waits, ended when dropped.
#[must_use]
#[derive(Debug)]
pub struct Gated<'a, R: Runtime> {
    waits: &'a Waits<R>,
    gate: Gate,
}

impl<R: Runtime> Drop for Gated<'_, R> {
    fn drop(&mut self) {
        self.waits.ended(self.gate.xid());
    }
}

/// The waits of one transaction for another to end, as wakers woken when it ends.
///
/// Preconditions on the caller:
///
/// - `started` runs before any version names the transaction, and `ended` after the commit log
///   records its end or its end is kept unresolved. [`Transactions`](super::Transactions) runs
///   both for every transaction of its waits.
/// - No latch is held across `wait`, nor while a call registered by `watch` is suspended.
#[derive(Debug)]
pub struct Waits<R: Runtime> {
    rt: R,
    state: Ranked<State>,
}

struct Unpark<R: Runtime> {
    rt: R,
    thread: ThreadId,
}

impl<R: Runtime> Wake for Unpark<R> {
    fn wake(self: Arc<Self>) {
        self.rt.unpark(self.thread);
    }
}

impl<R: Runtime> Waits<R> {
    pub fn new(rt: R) -> Waits<R> {
        Waits {
            rt,
            state: Ranked::new(crate::ranks::WAITS, State::default()),
        }
    }

    pub(super) fn started(&self, x: Xid) {
        let mut st = self.state.lock(&self.rt);
        st.running.insert(x);
        #[cfg(any(test, feature = "testing"))]
        st.begun_on.insert(x, self.rt.current());
    }

    /// Returns the transactions started and not yet ended.
    #[cfg(any(test, feature = "testing"))]
    pub fn running(&self) -> BTreeSet<Xid> {
        self.state.lock(&self.rt).running.clone()
    }

    /// Returns the transactions some waker is registered on.
    #[cfg(any(test, feature = "testing"))]
    pub fn watched(&self) -> BTreeSet<Xid> {
        self.state.lock(&self.rt).watchers.keys().copied().collect()
    }

    /// Returns whether some waker is registered on a transaction begun on `thread`.
    #[cfg(any(test, feature = "testing"))]
    pub fn watched_on(&self, thread: ThreadId) -> bool {
        let st = self.state.lock(&self.rt);
        st.watchers
            .keys()
            .any(|x| st.begun_on.get(x) == Some(&thread))
    }

    /// Start `gate`, running until the returned [`Gated`] drops.
    pub fn gate(&self, gate: Gate) -> Gated<'_, R> {
        self.started(gate.xid());
        Gated { waits: self, gate }
    }

    /// Start `x` as [`Transactions`](super::Transactions) starts a transaction it begins.
    #[cfg(any(test, feature = "testing"))]
    pub fn started_tested(&self, x: Xid) {
        self.started(x);
    }

    /// End `x` as [`Transactions`](super::Transactions) ends a transaction it begins.
    #[cfg(any(test, feature = "testing"))]
    pub fn ended_tested(&self, x: Xid) {
        self.ended(x);
    }

    /// Wake every waker registered on `x`, outside the state lock.
    pub(super) fn ended(&self, x: Xid) {
        let watchers = {
            let mut st = self.state.lock(&self.rt);
            st.running.remove(&x);
            #[cfg(any(test, feature = "testing"))]
            st.begun_on.remove(&x);
            st.waits_for.remove(&x);
            st.refused.remove(&x);
            st.queued.remove(&x);
            let watchers = st.watchers.remove(&x).unwrap_or_default();
            for (me, _) in &watchers {
                if let Some(me) = me
                    && st.waits_for.get(me) == Some(&x)
                {
                    st.waits_for.remove(me);
                }
            }
            watchers
        };
        for (_, w) in watchers {
            w.wake();
        }
    }

    /// Register `waker` to be woken once `on` has ended. `me` is the waiting transaction, if it
    /// has an xid, waiting to write a version of `heap`, or at a gate for `None`.
    ///
    /// Fail, registering nothing, if `on` already waits for `me` directly or through others.
    /// Panic, in debug builds and under `latch-checks`, if the calling thread holds a page latch
    /// or an index latch.
    ///
    /// Postconditions: on `Pending`, `me` waits for `on`, and unless the registration is
    /// withdrawn first, `waker` is woken once `on` has ended.
    pub fn watch(
        &self,
        me: Option<Xid>,
        on: Xid,
        heap: Option<u32>,
        waker: &Waker,
    ) -> Result<Watch, Conflict> {
        crate::ranks::unlatched(format_args!("a wait on {on:?} is registered"));
        let mut st = self.state.lock(&self.rt);
        if !st.running.contains(&on) {
            return Ok(Watch::Ended);
        }
        if let Some(me) = me {
            let mut cycle = vec![on];
            while let Some(&next) = st.waits_for.get(cycle.last().expect("starts at `on`")) {
                if next == me {
                    return Err(deadlock(cycle, heap));
                }
                cycle.push(next);
            }
            if cycle.last().is_some_and(|last| st.queued.contains(last))
                && st.granted.contains_key(&me)
            {
                return Err(deadlock(cycle, heap));
            }
            st.waits_for.insert(me, on);
        }
        st.watchers.entry(on).or_default().push((me, waker.clone()));
        Ok(Watch::Pending)
    }

    /// Withdraw the registration of `waker` on `on` made as `me`, if `on` has not ended.
    pub fn withdraw(&self, me: Option<Xid>, on: Xid, waker: &Waker) {
        let mut st = self.state.lock(&self.rt);
        if let Some(ws) = st.watchers.get_mut(&on) {
            ws.retain(|(m, w)| !(*m == me && w.will_wake(waker)));
            if ws.is_empty() {
                st.watchers.remove(&on);
            }
        }
        if let Some(me) = me
            && st.waits_for.get(&me) == Some(&on)
        {
            st.waits_for.remove(&me);
        }
    }

    /// Block until `on` has ended, failing as [`watch`](Waits::watch) does.
    pub fn wait(&self, me: Option<Xid>, on: Xid, heap: Option<u32>) -> Result<(), Conflict> {
        let waker = Waker::from(Arc::new(Unpark {
            rt: self.rt.clone(),
            thread: self.rt.current(),
        }));
        if self.watch(me, on, heap, &waker)? == Watch::Ended {
            return Ok(());
        }
        while self.state.lock(&self.rt).running.contains(&on) {
            self.rt.park();
        }
        Ok(())
    }

    /// Wait as `me` until `on` has ended, running `pause` between looks. Fail as
    /// [`watch`](Waits::watch) does, or with the failure of `pause`, which withdraws the wait.
    pub fn wait_pausing<E: From<Conflict>>(
        &self,
        me: Xid,
        on: Xid,
        heap: Option<u32>,
        mut pause: impl FnMut() -> Result<(), E>,
    ) -> Result<(), E> {
        let waker = Waker::noop();
        if self.watch(Some(me), on, heap, waker)? == Watch::Ended {
            return Ok(());
        }
        loop {
            {
                let mut st = self.state.lock(&self.rt);
                if let Some(cycle) = st.refused.remove(&me) {
                    drop(st);
                    self.withdraw(Some(me), on, waker);
                    return Err(deadlock(cycle, heap).into());
                }
                if !st.running.contains(&on) {
                    st.refused.remove(&me);
                    return Ok(());
                }
            }
            if let Err(e) = pause() {
                self.withdraw(Some(me), on, waker);
                return Err(e);
            }
        }
    }

    /// Count a memory grant the statements of `x` hold, until [`Waits::released_grant`].
    pub fn holds_grant(&self, x: Xid) {
        *self.state.lock(&self.rt).granted.entry(x).or_default() += 1;
    }

    pub fn released_grant(&self, x: Xid) {
        let mut st = self.state.lock(&self.rt);
        if let Some(n) = st.granted.get_mut(&x) {
            *n -= 1;
            if *n == 0 {
                st.granted.remove(&x);
            }
        }
    }

    /// Record `me` as waiting in the budget's queue, for every transaction holding a grant, until
    /// [`Waits::left_queue`], and break a cycle that wait closes, as [`Waits::break_cycle`] does.
    pub fn queued(&self, me: Xid) {
        self.state.lock(&self.rt).queued.insert(me);
        self.break_cycle(me);
    }

    pub fn left_queue(&self, me: Xid) {
        self.state.lock(&self.rt).queued.remove(&me);
    }

    /// Refuse, with a deadlock, one transaction holding a grant that waits, directly or through
    /// others, for `me`, queued for a grant, unless one refused still holds its grant. Its
    /// [`Waits::wait_pausing`] fails, and its grant goes back to the budget.
    pub fn break_cycle(&self, me: Xid) {
        let mut st = self.state.lock(&self.rt);
        let pending = st.refused.keys().any(|r| st.granted.contains_key(r));
        if pending {
            return;
        }
        let closed = st.granted.keys().filter(|h| **h != me).find_map(|&h| {
            let (mut cycle, mut at) = (vec![h], h);
            while let Some(&next) = st.waits_for.get(&at) {
                if next == me {
                    cycle.push(me);
                    return Some((h, cycle));
                }
                if cycle.contains(&next) {
                    return None;
                }
                cycle.push(next);
                at = next;
            }
            None
        });
        let Some((refused, mut cycle)) = closed else {
            return;
        };
        cycle.remove(0);
        let Some(&on) = st.waits_for.get(&refused) else {
            return;
        };
        let woken: Vec<Waker> = st.watchers.get(&on).map_or_else(Vec::new, |ws| {
            ws.iter()
                .filter(|(m, _)| *m == Some(refused))
                .map(|(_, w)| w.clone())
                .collect()
        });
        st.refused.insert(refused, cycle);
        drop(st);
        for w in woken {
            w.wake();
        }
    }
}
