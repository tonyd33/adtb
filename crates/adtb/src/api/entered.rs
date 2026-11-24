//! The engine held open for one call.

use std::sync::Arc;

use crate::catalog::{self, VersionHash};
use crate::runtime::Runtime;
use crate::write::{self, Bounds, Local, Txn};

use super::admitted::Admitted;
use super::stated::Hashes;
use super::statements::Failure;
use super::text::{DynRows, Prepared};
use super::{CallOptions, Committed, Engine, Error, Position, RejectReason, TxnId};

/// An engine held open for one call, the call's mapping point. Dropping it ends the call: it
/// maps the damage recorded and not yet mapped, whatever the call returned.
///
/// Precondition: the dropping thread holds no page latch and not the serialization lock.
pub(super) struct Entered<'e, R: Runtime> {
    engine: &'e Engine<R>,
    mapping: write::Mapping<'e, R>,
    _open: std::sync::RwLockReadGuard<'e, bool>,
}

impl<R: Runtime> std::ops::Deref for Entered<'_, R> {
    type Target = Engine<R>;

    fn deref(&self) -> &Engine<R> {
        self.engine
    }
}

/// Returns the stored hashes `hash` of the database `db`, a call whose read of them fails
/// failing as a call that meets the failure writing catalog rows.
pub(super) fn call_hashes<R: Runtime>(
    hash: VersionHash,
    db: Arc<write::Database<R>>,
    schema: Arc<adtb_lang::schema::Committed>,
) -> Hashes {
    Hashes::new(hash, move |e| {
        Error::of_write(&db, &schema, TxnId(0), db.mapping().fail(e.into()))
    })
}

impl<R: Runtime> Engine<R> {
    /// Returns the engine held open for one call, or refuses a call of a closed database.
    pub(super) fn enter(&self) -> Result<Entered<'_, R>, Error> {
        let open = self.open.read(&self.rt);
        if *open {
            Ok(Entered {
                engine: self,
                mapping: self.db.mapping(),
                _open: open,
            })
        } else {
            Err(Error::closed())
        }
    }

    /// Returns the engine held open for one call, `None` when it is closed or closing.
    pub(super) fn try_enter(&self) -> Option<Entered<'_, R>> {
        let open = self.open.try_read()?;
        (*open).then(|| Entered {
            engine: self,
            mapping: self.db.mapping(),
            _open: open,
        })
    }
}

impl<R: Runtime> Entered<'_, R> {
    /// Returns the call's mapping point.
    pub(super) fn mapping(&self) -> &write::Mapping<'_, R> {
        &self.mapping
    }

    /// Returns the failure of every call but `verify` and `dump` on a database quarantined whole.
    pub(super) fn refusal(&self) -> Option<Error> {
        let refused = self.db.admit_database().err()?;
        Some(self.write_error(TxnId(0), refused))
    }

    /// Returns the client error of `e`, met writing catalog rows.
    pub(super) fn catalog_error(&self, e: catalog::Error) -> Error {
        self.write_error(TxnId(0), e.into())
    }

    pub(super) fn begin(&self, id: TxnId) -> Result<Txn, Error> {
        self.db.begin().map_err(|e| self.write_error(id, e))
    }

    pub(super) fn begin_read(&self, id: TxnId, deferrable: bool) -> Result<Txn, Error> {
        self.db
            .begin_read(deferrable)
            .map_err(|e| self.write_error(id, e))
    }

    pub(super) fn abort(&self, id: TxnId, tx: Txn) -> Result<(), Error> {
        self.db.abort(tx).map_err(|e| self.write_error(id, e))
    }

    /// Returns the run of `verify` that `p` is, when `p` is nothing else and the commit log's
    /// damage refuses every transaction.
    pub(super) fn verify_without_transaction(
        &self,
        p: &Prepared,
    ) -> Option<Result<DynRows, Error>> {
        self.pager.whole_cause()?;
        let ([], [c]) = (p.items(), p.commands()) else {
            return None;
        };
        let adtb_lang::check::Command::Verify(rel) = c.command else {
            return None;
        };
        Some(
            self.verified(p.schema(), rel, write::HoldWait::unbounded())
                .map_err(|e| self.write_error(TxnId(0), e)),
        )
    }

    /// Run `admitted` in `tx`, the transaction `id`, bounded by `bounds`, and commit it, or abort it on
    /// any failure. Returns the rows of each item that yields a result and the commit position.
    pub(super) fn run(
        &self,
        id: TxnId,
        tx: Txn,
        admitted: &Admitted,
        bounds: Bounds,
    ) -> Result<(Vec<DynRows>, Position), Failure> {
        let p = admitted.prepared();
        let (cancel, deadline) = (bounds.cancel.clone(), bounds.deadline);
        let past_commands = |error| Failure::new(error, true, !p.commands().is_empty());
        let rows = self
            .statements(id, &tx, admitted, &mut Local::default(), bounds)
            .and_then(|rows| {
                if cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Acquire)) {
                    Err(past_commands(Error::rejected(RejectReason::Cancelled)))
                } else if deadline.is_some_and(|d| self.rt.now() >= d) {
                    Err(past_commands(Error::rejected(
                        RejectReason::DeadlineExceeded,
                    )))
                } else {
                    Ok(rows)
                }
            });
        let failed = |e| past_commands(self.write_error(id, e));
        match rows {
            Ok(rows) => {
                let at = self.db.commit(tx).map_err(failed)?;
                Ok((rows, Position::from(at)))
            }
            Err(f) => Err(write::either(self.db.abandon(tx, || f, failed))),
        }
    }

    /// Returns `p`, the checked text of a call. On a database quarantined whole, fail with
    /// `Quarantined` whether or not it checked, unless it is a run of `verify` or `dump` alone.
    pub(super) fn admit_text(&self, p: Result<Prepared, Error>) -> Result<Prepared, Error> {
        let Some(refused) = self.refusal() else {
            return p;
        };
        match p {
            Ok(p) if self.lone_verify(&p) || Engine::<R>::lone_dump(&p) => Ok(p),
            _ => Err(refused),
        }
    }

    /// Run `admitted` as one transaction of its own under `opts`: read-only when it only reads, else
    /// rerun on a serialization failure before any command ran, up to `opts.retry`, else its
    /// `retry n`, else the file setting `RetryLimit`. Returns the rows of each item that yields a
    /// result, with the commit position and the attempts made.
    ///
    /// A call whose token is cancelled, or whose deadline passes, before it commits fails with
    /// `Cancelled` or `DeadlineExceeded` and has no effect.
    /// Invoke `on_ready` once after checking the call options, before beginning execution.
    pub(super) fn call(
        &self,
        admitted: &Admitted,
        opts: &CallOptions,
        on_ready: impl FnOnce(),
    ) -> Result<Committed<Vec<DynRows>>, Error> {
        let p = admitted.prepared();
        let stops = opts.stops();
        stops.check(&self.rt)?;
        self.reads_after(opts.after)?;
        let bounds = self.granted(admitted, None, stops.bounds(&self.rt, opts.fix_rounds))?;
        on_ready();
        if let Some(rows) = self.verify_without_transaction(p) {
            return rows.map(|rows| Committed {
                value: vec![rows],
                position: Position::from(self.pager.position(self.pager.durable())),
                attempts: 1,
            });
        }
        if !p.writes {
            let id = self.txn_id();
            let tx = self.begin_read(id, p.deferrable)?;
            return self
                .run(id, tx, admitted, bounds)
                .map(|(value, position)| Committed {
                    value,
                    position,
                    attempts: 1,
                })
                .map_err(|f| f.error);
        }
        let mut attempts = 1;
        loop {
            stops.check(&self.rt)?;
            let id = self.txn_id();
            let tx = self.begin(id)?;
            match self.run(id, tx, admitted, bounds.clone()) {
                Err(f) if self.reruns_after(p, opts.retry, attempts, &f.error, f.commanded) => {
                    attempts += 1;
                }
                done => {
                    return done
                        .map(|(value, position)| Committed {
                            value,
                            position,
                            attempts,
                        })
                        .map_err(|f| f.error);
                }
            }
        }
    }

    /// Returns the client error of write-path failure `e` of transaction `txn`, the damage
    /// recorded mapped first.
    pub(super) fn write_error(&self, txn: TxnId, e: write::Error) -> Error {
        Error::of_write(&self.db, self.schema(), txn, self.mapping.fail(e))
    }
}
