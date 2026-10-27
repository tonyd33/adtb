use std::sync::Arc;
use std::time::{Duration, SystemTime};

use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use crate::exec::Val;
use crate::runtime::Runtime;
use crate::write::{Bounds, Local, Txn};

use super::error::FatalReason;
use super::text::{DynRows, Prepared};
use super::{Database, Durability, Engine, Error, Position, TxnId, unsupported};

/// The options of an interactive transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TxnOptions {
    /// Refuse every call that writes.
    pub read_only: bool,
    /// Wait for a safe snapshot before the first read; only with `read_only`.
    pub deferrable: bool,
    pub deadline: Option<SystemTime>,
    pub limit: Option<Duration>,
    pub priority: Option<u8>,
    pub durability: Durability,
    /// Read no snapshot older than this commit position.
    pub after: Option<Position>,
    /// An end-user or anonymous principal every call of the transaction runs for, a value of
    /// the schema's `Principal`.
    pub assert: Option<Val>,
}

/// The options of one call inside an interactive transaction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TxnCallOptions {
    pub deadline: Option<SystemTime>,
    pub limit: Option<Duration>,
    pub priority: Option<u8>,
    /// The rounds a fixpoint of the call may take, else the limit the database was opened with.
    pub fix_rounds: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Committed<T> {
    pub value: T,
    pub position: Position,
    pub attempts: u32,
}

#[derive(Debug)]
enum State {
    Open(Txn),
    /// Aborted by the engine with this error.
    Ended(Error),
    Closed,
}

/// An interactive transaction of ad hoc text. Dropping it without a commit rolls it back.
#[derive(Debug)]
pub struct AdHocTransaction<'a, R: Runtime = adtb_sys::os::OsRuntime> {
    pub(super) engine: &'a Engine<R>,
    id: TxnId,
    pub(super) read_only: bool,
    /// The principal every call of the transaction runs for.
    pub(super) caller: Val,
    state: State,
    /// The schema extended with the temp relations the transaction's texts declared, once one
    /// has.
    temps: Option<Arc<Schema>>,
    local: Local,
}

impl<R: Runtime> Database<R> {
    /// Begin an interactive transaction.
    ///
    /// A database without replicas commits at `local` whatever level the options name.
    pub fn begin(&self, opts: &TxnOptions) -> Result<AdHocTransaction<'_, R>, Error> {
        if opts.deferrable && !opts.read_only {
            return Err(Error::invalid("only a read-only transaction is deferrable"));
        }
        if opts.deadline.is_some() || opts.limit.is_some() {
            return Err(unsupported(
                "a deadline or statement limit on a transaction",
            ));
        }
        let e = &*self.engine;
        let caller = e.principal(opts.assert.as_ref())?;
        e.reads_after(opts.after)?;
        let id = e.txn_id();
        let tx = if opts.read_only {
            e.begin_read(id, opts.deferrable)?
        } else {
            e.begin(id)?
        };
        Ok(AdHocTransaction {
            engine: e,
            id,
            read_only: opts.read_only,
            caller,
            state: State::Open(tx),
            temps: None,
            local: Local::default(),
        })
    }

    /// Run `f` in a new interactive transaction and commit it, running it again in a fresh
    /// transaction while it fails with a serialization failure, at most `attempts` times in all.
    ///
    /// Preconditions: `f` has no effect outside the database.
    pub fn transact<T>(
        &self,
        opts: &TxnOptions,
        attempts: u32,
        mut f: impl FnMut(&mut AdHocTransaction<'_, R>) -> Result<T, Error>,
    ) -> Result<Committed<T>, Error> {
        let mut attempt = 1;
        loop {
            let mut t = self.begin(opts)?;
            let done = f(&mut t).and_then(|value| t.commit(None).map(|position| (value, position)));
            match done {
                Ok((value, position)) => {
                    return Ok(Committed {
                        value,
                        position,
                        attempts: attempt,
                    });
                }
                Err(e) if e.is_serialization() && attempt < attempts => attempt += 1,
                Err(e) => return Err(e),
            }
        }
    }
}

impl<R: Runtime> AdHocTransaction<'_, R> {
    /// Returns the id that names the transaction in its errors.
    pub fn id(&self) -> TxnId {
        self.id
    }

    /// Run ad hoc text as one call of the transaction, each item one statement, each free name
    /// bound by `params`. Returns the rows of each item that yields a result, in item order.
    ///
    /// A call that fails applies none of its items and leaves the transaction open, except on a
    /// serialization failure, which aborts the transaction; every later call and the commit fail
    /// with it.
    pub fn query_text(
        &mut self,
        text: &str,
        params: &[(&str, Ty, Val)],
        opts: &TxnCallOptions,
    ) -> Result<Vec<DynRows>, Error> {
        let e = self.engine;
        let schema = self.temps.as_ref().unwrap_or(&e.schema);
        let p = Prepared::new(
            schema,
            &e.program,
            &e.bundles,
            text,
            params,
            self.caller.clone(),
        )?;
        if p.deferrable {
            return Err(Error::invalid(
                "a read_only deferrable block does not run inside an interactive transaction",
            ));
        }
        if self.read_only && p.writes {
            return Err(Error::invalid("a read-only transaction does not write"));
        }
        let bounds = Bounds {
            fix_rounds: opts.fix_rounds,
        };
        let rows = self.run(&p, opts, bounds)?;
        self.temps = Some(p.schema().clone());
        Ok(rows)
    }

    /// Runs the items of `p` as one call of the transaction. Returns the rows of each item that
    /// yields a result, in item order.
    pub(super) fn run(
        &mut self,
        p: &Prepared,
        opts: &TxnCallOptions,
        bounds: Bounds,
    ) -> Result<Vec<DynRows>, Error> {
        let tx = match &self.state {
            State::Open(tx) => tx,
            State::Ended(err) => return Err(err.clone()),
            State::Closed => unreachable!("a transaction is closed only as it is consumed"),
        };
        if opts.deadline.is_some() || opts.limit.is_some() {
            return Err(unsupported("a deadline or statement limit on a call"));
        }
        let (err, ends) = match self
            .engine
            .statements(self.id, tx, p, &mut self.local, bounds)
        {
            Ok(rows) => return Ok(rows),
            Err(failed) => failed,
        };
        if ends {
            self.end(err.clone())?;
        }
        Err(err)
    }

    /// Commit the transaction. Returns its commit position.
    ///
    /// A database without replicas commits at `local` whatever level `_durability` names.
    pub fn commit(mut self, _durability: Option<Durability>) -> Result<Position, Error> {
        match std::mem::replace(&mut self.state, State::Closed) {
            State::Open(tx) => self
                .engine
                .db
                .commit(tx)
                .map(Position::from)
                .map_err(|e| self.engine.write_error(self.id, e)),
            State::Ended(err) => Err(err),
            State::Closed => unreachable!("a transaction is closed only as it is consumed"),
        }
    }

    /// Roll the transaction back.
    pub fn rollback(mut self) -> Result<(), Error> {
        match std::mem::replace(&mut self.state, State::Closed) {
            State::Open(tx) => self.engine.abort(self.id, tx),
            State::Ended(_) | State::Closed => Ok(()),
        }
    }

    /// Abort the transaction, leaving every later call and the commit to fail with `err`.
    fn end(&mut self, err: Error) -> Result<(), Error> {
        match std::mem::replace(&mut self.state, State::Ended(err)) {
            State::Open(tx) => self.engine.abort(self.id, tx),
            State::Ended(_) | State::Closed => Ok(()),
        }
    }
}

impl<R: Runtime> Drop for AdHocTransaction<'_, R> {
    fn drop(&mut self) {
        if let State::Open(tx) = std::mem::replace(&mut self.state, State::Closed)
            && let Err(e) = self.engine.abort(self.id, tx)
            && !std::thread::panicking()
        {
            assert!(
                matches!(
                    e,
                    Error::Fatal {
                        reason: FatalReason::Storage { .. }
                    }
                ),
                "an abort fails only on a quarantined database, which refuses every later call: \
                 {e:?}"
            );
        }
    }
}
