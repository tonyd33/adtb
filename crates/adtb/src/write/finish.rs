use crate::runtime::Runtime;
use crate::storage::Position;

use super::session::SerialEnd;
use super::{Database, Error, Held, Txn};

impl<R: Runtime> Database<R> {
    /// Commit `tx`. Returns its commit position once the log is durable past its commit and
    /// every commit it read; for a read-only transaction, the position of the latest commit it
    /// read, else of the log's durable end.
    ///
    /// Fail with `Quarantined` if a mark set before its commit record stops what `tx` wrote, by
    /// the rule that stops a statement. On any failure, abandon `tx` as [`Database::abandon`]
    /// does: fail with the commit's failure, or with the abort record's when it fails.
    pub fn commit(&self, tx: Txn) -> Result<Position, Error> {
        let (sid, upto) = (tx.sid, tx.taken.upto());
        let written = tx.written.take();
        let mut held = tx.end();
        let begun = held.begun.take();
        let committed = self.session.txns.commit_with(
            &self.pager,
            begun,
            upto,
            |ser| {
                self.map_committing(ser.ordered());
                self.admit_commit(ser, written)?;
                ser.precommit(sid).map_err(Error::from)
            },
            |ser| (self.session).end_serial(ser, &mut held, SerialEnd::Committed),
        );
        match committed {
            Ok(c) => {
                let changed = held.changed.take();
                drop(held);
                self.session.count_dead(&changed, true);
                Ok(self.pager.position(c.lsn()))
            }
            Err(e) => Err(either(self.abandon_held(held, || e, |r| r))),
        }
    }

    /// Abort `tx` and release what it holds.
    ///
    /// Fail when its abort record fails, as [`crate::mvcc::Transactions::abort`] fails. The commit
    /// log then still says `tx`'s id is in progress, and this process counts it aborted.
    pub fn abort(&self, tx: Txn) -> Result<(), Error> {
        self.abort_held(tx.end())
    }

    fn abort_held(&self, held: Held) -> Result<(), Error> {
        let changed = held.changed.take();
        let aborted = held.close();
        self.session.count_dead(&changed, false);
        aborted?;
        Ok(())
    }

    /// Abort `tx` after its call failed, or as the call releases it open, releasing what it
    /// holds, and count its abort record if it fails. Returns the call's reply: `Ok` with
    /// `cause()` once the record is written, else `Err` with the record's failure as `reply`
    /// makes it.
    pub fn abandon<E>(
        &self,
        tx: Txn,
        cause: impl FnOnce() -> E,
        reply: impl FnOnce(Error) -> E,
    ) -> Result<E, E> {
        self.abandon_held(tx.end(), cause, reply)
    }

    fn abandon_held<E>(
        &self,
        held: Held,
        cause: impl FnOnce() -> E,
        reply: impl FnOnce(Error) -> E,
    ) -> Result<E, E> {
        match self.abort_held(held) {
            Ok(()) => Ok(cause()),
            Err(e) => {
                self.session.txns.count_unrecorded();
                Err(reply(e))
            }
        }
    }

    /// Abandon `tx` after its call failed with `cause`, as [`Database::abandon`] does. Returns
    /// the call's reply, `cause` or the abort record's failure.
    pub fn abandoned(&self, tx: Txn, cause: Error) -> Error {
        either(self.abandon(tx, || cause, |r| r))
    }

    /// Abort `tx` and return `done`, abandoning `tx` when `done` is a failure.
    pub(crate) fn abort_returning<T>(&self, tx: Txn, done: Result<T, Error>) -> Result<T, Error> {
        match done {
            Ok(v) => self.abort(tx).map(|()| v),
            Err(e) => Err(self.abandoned(tx, e)),
        }
    }
}

/// Returns the value either arm of `r` holds.
pub fn either<E>(r: Result<E, E>) -> E {
    r.unwrap_or_else(|e| e)
}
