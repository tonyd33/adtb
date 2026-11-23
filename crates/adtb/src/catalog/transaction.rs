//! The one write transaction of the catalog: its id, its snapshot hold, the structures it makes,
//! and its commit or abandon.

use crate::mvcc::{Begun, Committed, Scope, Transactions};
use crate::runtime::Runtime;
use crate::storage::{Pager, Pid};

use super::{Error, Migrating};

/// A catalog transaction under way.
pub(super) struct Change<'a, R: Runtime> {
    begun: Begun,
    m: Migrating<'a, R>,
}

impl<'a, R: Runtime> Change<'a, R> {
    /// Begin a transaction that reads the catalog at a snapshot holding `scope`.
    ///
    /// Fail as [`Transactions::begin`] and [`Transactions::take`] do, abandoning the
    /// transaction when the hold fails.
    pub(super) fn begin(
        pager: &'a Pager<R>,
        txns: &'a Transactions<R>,
        scope: Scope,
    ) -> Result<Change<'a, R>, Error> {
        let begun = txns.begin(pager)?;
        let hold = match txns.take(scope) {
            Ok(hold) => hold,
            Err(e) => {
                txns.abandon_failed(pager, begun)?;
                return Err(e.into());
            }
        };
        let m = Migrating {
            pager,
            txns,
            x: begun.xid(),
            hold,
            made: Default::default(),
            top: Default::default(),
        };
        Ok(Change { begun, m })
    }

    /// Returns the writes of this transaction, read at its snapshot.
    pub(super) fn writes(&self) -> &Migrating<'a, R> {
        &self.m
    }

    /// Release the snapshot and commit, changing the page `pid` through `also` in the log batch
    /// of the commit record, as [`Transactions::commit_changing`] does.
    pub(super) fn commit_changing(
        self,
        pid: Pid,
        also: impl FnOnce(&mut [u8]),
    ) -> Result<Committed<'a>, Error> {
        let Change { begun, m } = self;
        let (pager, txns) = (m.pager, m.txns);
        drop(m);
        Ok(txns.commit_changing(pager, begun, pid, also)?)
    }

    /// Release the snapshot and abandon the transaction whole after its work failed with `e`.
    /// Returns `e`, or the failure of the abort record in its place.
    pub(super) fn abandon(self, e: Error) -> Error {
        let Change { begun, m } = self;
        let (pager, txns) = (m.pager, m.txns);
        drop(m);
        match txns.abandon_failed(pager, begun) {
            Ok(()) => e,
            Err(unrecorded) => unrecorded.into(),
        }
    }
}
