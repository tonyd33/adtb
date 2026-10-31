use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::task::{Wake, Waker};

use crate::runtime::{Runtime, ThreadId};

use super::{Cause, SerializationFailure, Xid};
use adtb_sys::lock::Lock;

#[derive(Debug, Default)]
struct State {
    running: BTreeSet<Xid>,
    /// The wakers registered on each running transaction, each with the transaction it waits as.
    watchers: BTreeMap<Xid, Vec<(Option<Xid>, Waker)>>,
    waits_for: BTreeMap<Xid, Xid>,
}

/// What a registration with [`Waits::watch`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watch {
    /// The transaction had already ended; nothing was registered.
    Ended,
    /// The waker is registered and is woken once the transaction ends.
    Pending,
}

/// Waits of one transaction for another to end, as wakers woken when it ends.
///
/// Preconditions on the caller:
///
/// - `started` runs before any version names the transaction, and `ended` after the commit log
///   records its end.
/// - No latch is held across `wait`, nor while a call registered by `watch` is suspended.
#[derive(Debug)]
pub struct Waits<R: Runtime> {
    rt: R,
    state: Lock<State>,
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
            state: Lock::default(),
        }
    }

    pub fn started(&self, x: Xid) {
        self.state.lock(&self.rt).running.insert(x);
    }

    /// Wake every waker registered on `x`, outside the state lock.
    pub fn ended(&self, x: Xid) {
        let watchers = {
            let mut st = self.state.lock(&self.rt);
            st.running.remove(&x);
            st.waits_for.remove(&x);
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
    /// has an xid.
    ///
    /// Fail, registering nothing, if `on` already waits for `me` directly or through others.
    ///
    /// Postconditions: on `Pending`, `me` waits for `on`, and unless the registration is
    /// withdrawn first, `waker` is woken once `on` has ended.
    pub fn watch(
        &self,
        me: Option<Xid>,
        on: Xid,
        waker: &Waker,
    ) -> Result<Watch, SerializationFailure> {
        let mut st = self.state.lock(&self.rt);
        if !st.running.contains(&on) {
            return Ok(Watch::Ended);
        }
        if let Some(me) = me {
            let mut cycle = vec![on];
            while let Some(&next) = st.waits_for.get(cycle.last().expect("starts at `on`")) {
                if next == me {
                    return Err(SerializationFailure::new(Cause::Deadlock, cycle));
                }
                cycle.push(next);
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
    pub fn wait(&self, me: Option<Xid>, on: Xid) -> Result<(), SerializationFailure> {
        let waker = Waker::from(Arc::new(Unpark {
            rt: self.rt.clone(),
            thread: self.rt.current(),
        }));
        if self.watch(me, on, &waker)? == Watch::Ended {
            return Ok(());
        }
        while self.state.lock(&self.rt).running.contains(&on) {
            self.rt.park();
        }
        Ok(())
    }

    /// Wait as `me` until `on` has ended, running `pause` between looks. Fail as
    /// [`watch`](Waits::watch) does, or with the failure of `pause`, which withdraws the wait.
    pub fn wait_pausing<E: From<SerializationFailure>>(
        &self,
        me: Xid,
        on: Xid,
        mut pause: impl FnMut() -> Result<(), E>,
    ) -> Result<(), E> {
        let waker = Waker::noop();
        if self.watch(Some(me), on, waker)? == Watch::Ended {
            return Ok(());
        }
        while self.state.lock(&self.rt).running.contains(&on) {
            if let Err(e) = pause() {
                self.withdraw(Some(me), on, waker);
                return Err(e);
            }
        }
        Ok(())
    }
}
