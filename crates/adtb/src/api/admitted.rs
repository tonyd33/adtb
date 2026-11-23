//! The gate every call of a program passes before it runs.

use std::sync::Arc;
use std::time::SystemTime;

use crate::exec::{self, Entry, Priority};
use crate::mvcc::Xid;
use crate::runtime::Runtime;
use crate::write::{Bounds, Stop};

use super::text::{Issuer, Prepared};
use super::{Engine, Error, TxnId};

/// The bytes a queued call's metadata counts for besides its text and arguments.
pub(super) const CALL_BYTES: usize = 256;

/// A checked program the database and the admission queue's byte bound admit, with where it
/// queues. Only [`Engine::admit_program`] builds one.
pub(super) struct Admitted {
    prepared: Prepared,
    entry: Entry,
}

impl Admitted {
    pub(super) fn prepared(&self) -> &Prepared {
        &self.prepared
    }

    pub(super) fn entry(&self) -> Entry {
        self.entry
    }

    pub(super) fn issuer_mut(&mut self) -> Option<&mut Issuer> {
        self.prepared.issuer.as_mut()
    }
}

/// Where a call queues: its class, its rank within the class, and when it stops waiting.
pub(super) struct Queued {
    class: Priority,
    rank: Option<u8>,
    deadline: Option<SystemTime>,
    /// The bytes the call holds while it waits.
    bytes: usize,
}

impl Queued {
    pub(super) fn new(
        class: Priority,
        rank: Option<u8>,
        deadline: Option<SystemTime>,
        bytes: usize,
    ) -> Queued {
        Queued {
            class,
            rank,
            deadline,
            bytes,
        }
    }
}

impl<R: Runtime> Engine<R> {
    /// Admit `p` to run as a call queued as `queued`.
    ///
    /// Refuse it: on a database quarantined whole with `Quarantined`, unless it is one run of
    /// `verify` or `dump` alone; and with the statement limit of the queue's byte bound when its
    /// bytes alone pass that bound.
    pub(super) fn admit_program(&self, p: Prepared, queued: &Queued) -> Result<Admitted, Error> {
        let prepared = self.admit_text(Ok(p))?;
        self.db
            .within_byte_bound(queued.bytes)
            .map_err(|e| self.write_error(TxnId(0), e))?;
        let rt = &self.rt;
        let deadline = queued.deadline.map(|d| {
            let left = d.duration_since(rt.wall()).unwrap_or_default();
            rt.now() + left
        });
        let entry = Entry {
            priority: queued.class,
            rank: queued.rank.unwrap_or(u8::MAX),
            start: rt.now(),
            bytes: queued.bytes,
            deadline,
        };
        Ok(Admitted { prepared, entry })
    }

    /// Returns `bounds` holding a grant of the call `admitted` of transaction `me`, taken from
    /// the budget in its place in the queue, the calling thread waiting there until the stops of
    /// `bounds` end the wait. A call that `bounds` already grants keeps its grant.
    pub(super) fn granted(
        &self,
        admitted: &Admitted,
        me: Option<Xid>,
        bounds: Bounds,
    ) -> Result<Bounds, Error> {
        if bounds.grant.is_some() {
            return Ok(bounds);
        }
        let stop = Stop::of(&bounds, None, None);
        let grant: Arc<exec::Grant> = self
            .db
            .admit_entry(&admitted.entry, me, &stop)
            .map_err(|e| self.write_error(TxnId(0), e))?;
        Ok(Bounds {
            grant: Some(grant),
            ..bounds
        })
    }
}
