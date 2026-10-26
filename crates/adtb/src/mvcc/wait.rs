use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::{Runtime, ThreadId};

use super::{Cause, SerializationFailure, Xid};
use adtb_sys::lock::Lock;

#[derive(Debug, Default)]
struct State {
    running: BTreeSet<Xid>,
    parked: BTreeMap<Xid, Vec<ThreadId>>,
    waits_for: BTreeMap<Xid, Xid>,
}

/// Waits of one transaction for another to end, parked through the runtime.
///
/// Preconditions on the caller:
///
/// - `started` runs before any version names the transaction, and `ended` after the commit log
///   records its end.
/// - No latch is held across `wait`.
#[derive(Debug)]
pub struct Waits<R: Runtime> {
    rt: R,
    state: Lock<State>,
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

    /// Wake every transaction waiting for `x`.
    pub fn ended(&self, x: Xid) {
        let parked = {
            let mut st = self.state.lock(&self.rt);
            st.running.remove(&x);
            st.waits_for.remove(&x);
            st.parked.remove(&x).unwrap_or_default()
        };
        for t in parked {
            self.rt.unpark(t);
        }
    }

    /// Block until `on` has ended. `me` is the waiting transaction, if it has an xid.
    ///
    /// Fails, without waiting, if `on` already waits for `me` directly or through others.
    pub fn wait(&self, me: Option<Xid>, on: Xid) -> Result<(), SerializationFailure> {
        {
            let mut st = self.state.lock(&self.rt);
            if !st.running.contains(&on) {
                return Ok(());
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
            st.parked.entry(on).or_default().push(self.rt.current());
        }
        loop {
            self.rt.park();
            let mut st = self.state.lock(&self.rt);
            if !st.running.contains(&on) {
                if let Some(me) = me {
                    st.waits_for.remove(&me);
                }
                return Ok(());
            }
        }
    }
}
