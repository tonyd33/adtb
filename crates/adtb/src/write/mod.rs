mod apply;
mod check;
mod delta;
mod eval;
mod fresh;
mod polarity;
mod schema;
mod sequence;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;
use adtb_sys::lock::Lock;

use crate::exec::{self, Budget, Scheduler, StoredHeap, StoredIndex, Val};
use crate::mvcc::{
    Access, Limits, Safety, Scope, SerialId, SerializationFailure, Serializer, Snapshot,
    SnapshotId, Transactions, Waits, Xid,
};
use crate::plan::{Catalog, HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid, Position};

pub use adtb_lang::schema::Stage;

use schema::Rels;
use sequence::Sequences;

/// Why a statement was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// `constraint` is the violated constraint's name; `report` the rows its query returns.
    Violated {
        constraint: String,
        report: BTreeSet<Val>,
    },
    /// Two rows of `relation` hold `key`, the least such key.
    KeyViolation {
        relation: RelId,
        key: Val,
    },
    KeyChanged {
        relation: RelId,
    },
    /// `ctor` is qualified by its relation (`identities.Apple`) or by its sum (`Provider.Okta`).
    NotOpen {
        ctor: String,
        stage: Stage,
    },
    UserError {
        message: String,
    },
    /// A record of the statement's source failed: record `line` of relation `source`, or `""`
    /// and 0 when the source is not one relation.
    BadInput {
        source: String,
        line: i64,
        detail: String,
    },
}

/// Why a statement or a commit failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Rejected(Rejection),
    Fault(exec::Fault),
    StatementLimit(exec::Exceeded),
    Serialization(SerializationFailure),
    Page(PageError),
    /// The commit log is corrupt or its ids are spent.
    Transactions(String),
    Corrupt(String),
    /// The statement needs a form the engine does not run yet.
    Unsupported(String),
    /// The plan or a value breaks the invariant this text names, one the checker guarantees.
    Internal(String),
    /// The schema has a constraint without an index-bounded incremental check; each constraint
    /// refused, with why.
    Refused(Vec<(String, String)>),
}

impl Error {
    /// Returns whether the failure ends the transaction: the caller must abort it.
    pub fn ends_transaction(&self) -> bool {
        matches!(
            self,
            Error::Serialization(_) | Error::Page(_) | Error::Transactions(_) | Error::Corrupt(_)
        )
    }
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

impl From<SerializationFailure> for Error {
    fn from(f: SerializationFailure) -> Error {
        Error::Serialization(f)
    }
}

impl From<crate::heap::Error> for Error {
    fn from(e: crate::heap::Error) -> Error {
        match e {
            crate::heap::Error::Page(p) => Error::Page(p),
            crate::heap::Error::TooLarge { len } => {
                Error::Unsupported(format!("a row version of {len} bytes"))
            }
        }
    }
}

impl From<crate::btree::Error> for Error {
    fn from(e: crate::btree::Error) -> Error {
        match e {
            crate::btree::Error::Page(p) => Error::Page(p),
            other => Error::Corrupt(format!("{other:?}")),
        }
    }
}

impl From<crate::mvcc::Error> for Error {
    fn from(e: crate::mvcc::Error) -> Error {
        Error::Transactions(format!("{e:?}"))
    }
}

impl From<exec::Error> for Error {
    fn from(e: exec::Error) -> Error {
        match e {
            exec::Error::Fault(f) => Error::Fault(f),
            exec::Error::StatementLimit(x) => Error::StatementLimit(x),
            exec::Error::Serialization(f) => Error::Serialization(f),
            exec::Error::Page(p) => Error::Page(p),
            exec::Error::Corrupt(c) => Error::Corrupt(c),
            exec::Error::Unsupported(u) => Error::Unsupported(u),
            exec::Error::Rejected(detail) => Error::Rejected(Rejection::BadInput {
                source: String::new(),
                line: 0,
                detail,
            }),
            exec::Error::NoCaller => Error::Unsupported("`caller` in a statement".into()),
            exec::Error::Cancelled => Error::Unsupported("a cancelled statement".into()),
            exec::Error::Internal(what) => Error::Internal(what),
        }
    }
}

/// Returns each constraint and check of `schema` without an index-bounded incremental check
/// over the indexes `catalog` describes, by qualified name, with why; empty when the schema
/// admits every one.
///
/// Fails with the guarantee of the checker that compiling a check breaks.
pub fn refusals(
    schema: &Schema,
    normal: &NormalProgram,
    catalog: &Catalog,
) -> Result<Vec<(String, String)>, String> {
    let mut terms = normal.terms.clone();
    match delta::compile(schema, normal, &mut terms, catalog) {
        Ok(_) => Ok(Vec::new()),
        Err(delta::Unadmitted::Refused(why)) => {
            Ok(why.into_iter().map(|r| (r.constraint, r.detail)).collect())
        }
        Err(delta::Unadmitted::Internal(what)) => Err(what),
    }
}

/// The rows of each constructor relation, by relation and constructor.
pub type Rows = BTreeMap<(RelId, u32), BTreeSet<Val>>;

/// What a statement that succeeds returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub result: Val,
    /// The normalised change set.
    pub inserted: Rows,
    pub deleted: Rows,
}

/// The item of a [`Database::statements`] call that failed, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failed {
    pub item: usize,
    pub error: Error,
}

/// A transaction, read-write or read-only.
#[derive(Debug)]
pub struct Txn {
    /// `None` for a read-only transaction.
    xid: Option<Xid>,
    sid: SerialId,
    snapshot: Snapshot,
    hold: SnapshotId,
}

impl Txn {
    /// Returns the transaction's id, `None` for a read-only one.
    pub fn xid(&self) -> Option<Xid> {
        self.xid
    }

    fn writer(&self) -> Result<Xid, Error> {
        self.xid
            .ok_or_else(|| Error::Internal("a read-only transaction runs no write".into()))
    }
}

/// What bounds the statements of one call, past the database's own bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
    /// The rounds a fixpoint may take, else the database's limit.
    pub fix_rounds: Option<usize>,
}

/// The heaps and indexes of the relation variables a catalog describes.
#[derive(Debug)]
pub struct Bindings {
    pub heaps: BTreeMap<HeapId, StoredHeap>,
    pub indexes: BTreeMap<IndexId, StoredIndex>,
}

/// The relation variables of a schema over their heaps and indexes, and the transactions that
/// write them.
///
/// Lock order: one index latch, then the serializer, then the readers, then the sequences.
/// `Transactions`, the pager and the heaps lock themselves and take none of these.
pub struct Database<R: Runtime> {
    rt: R,
    schema: Arc<Schema>,
    normal: Arc<NormalProgram>,
    /// The normal program's arena with every `Fresh` replaced by its placeholder.
    program: Terms,
    program_fresh: fresh::Occurrences,
    /// Each `on delete retag` new value over the referrer's row, in `program`.
    retags: BTreeMap<RelId, (Var, Tm)>,
    catalog: Arc<Catalog>,
    pager: Pager<R>,
    txns: Arc<Transactions<R>>,
    ser: Arc<Lock<Serializer>>,
    waits: Waits<R>,
    heaps: BTreeMap<HeapId, StoredHeap>,
    indexes: BTreeMap<IndexId, StoredIndex>,
    latches: BTreeMap<IndexId, Lock<()>>,
    rels: Rels,
    denials: Vec<check::Denial>,
    sequences: Lock<Sequences>,
    /// The snapshot of each running read-only transaction, for its safe snapshot marker.
    readers: Lock<BTreeMap<SerialId, (SnapshotId, Snapshot)>>,
    uuids: AtomicU64,
    budget: Arc<Budget>,
    pool: Option<Arc<Scheduler<R>>>,
    options: exec::Options,
    grant: usize,
}

impl<R: Runtime> std::fmt::Debug for Database<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("heaps", &self.heaps.keys().collect::<Vec<_>>())
            .field("indexes", &self.indexes.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// What a database runs its statements with.
#[derive(Debug)]
pub struct Setup<R: Runtime> {
    pub schema: Arc<Schema>,
    pub normal: Arc<NormalProgram>,
    pub catalog: Arc<Catalog>,
    pub pager: Pager<R>,
    pub txns: Arc<Transactions<R>>,
    pub bindings: Bindings,
    pub budget: Arc<Budget>,
    /// The memory each statement's queries are granted.
    pub grant: usize,
    pub pool: Option<Arc<Scheduler<R>>>,
    pub options: exec::Options,
    pub limits: Limits,
    /// The page of the database's generated counters, or `None` to allocate one and start each
    /// `sequence` past the keys its relation holds.
    pub sequences: Option<Pid>,
}

impl<R: Runtime> Database<R> {
    /// Returns the database of `setup`.
    ///
    /// Preconditions: `setup.bindings` holds a heap for every constructor and every index the
    /// catalog names, and the catalog's key index of each relation is unique.
    pub fn new(rt: R, setup: Setup<R>) -> Result<Database<R>, Error> {
        let mut program = setup.normal.terms.clone();
        let mut program_fresh = fresh::Occurrences::new();
        let retags = setup
            .normal
            .retags
            .iter()
            .map(|(r, (var, body))| {
                let body = fresh::rewrite(&mut program, *body, &[*var], &mut program_fresh);
                (*r, (*var, body))
            })
            .collect();
        let denials = check::denials(&setup.schema, &setup.normal, &setup.catalog, &mut program)?;
        let rels = schema::rels(&setup.schema, &setup.normal, &setup.catalog)?;
        let sequences = match setup.sequences {
            Some(pid) => Sequences::open(&setup.pager, pid)?,
            None => {
                let pid = Sequences::create(&setup.pager)?;
                let mut seqs = Sequences::open(&setup.pager, pid)?;
                for r in rels.values() {
                    if r.generate != Some(adtb_lang::schema::Generator::Sequence) || !r.single_key {
                        continue;
                    }
                    let index = &setup.bindings.indexes[&r.key_index].index;
                    let held = index
                        .scan_prefix(&setup.pager, &[])?
                        .into_iter()
                        .filter_map(|e| match e.key.first() {
                            Some(crate::key::Value::I64(k)) => Some(*k),
                            _ => None,
                        })
                        .max()
                        .unwrap_or(0);
                    seqs.advance(&setup.pager, r.id.0, held)?;
                }
                seqs
            }
        };
        let latches = setup
            .bindings
            .indexes
            .keys()
            .map(|i| (*i, Lock::new(())))
            .collect();
        let seed = rt.next_u64();
        Ok(Database {
            schema: setup.schema,
            normal: setup.normal,
            program,
            program_fresh,
            retags,
            catalog: setup.catalog,
            pager: setup.pager,
            txns: setup.txns,
            ser: Arc::new(Lock::new(Serializer::new(setup.limits))),
            waits: Waits::new(rt.clone()),
            heaps: setup.bindings.heaps,
            indexes: setup.bindings.indexes,
            latches,
            rels,
            denials,
            sequences: Lock::new(sequences),
            readers: Lock::default(),
            uuids: AtomicU64::new(seed >> 2),
            budget: setup.budget,
            pool: setup.pool,
            options: setup.options,
            grant: setup.grant,
            rt,
        })
    }

    pub fn pager(&self) -> &Pager<R> {
        &self.pager
    }

    /// Begin a read-write transaction.
    pub fn begin(&self) -> Result<Txn, Error> {
        let xid = self.txns.begin(&self.pager)?;
        let (hold, snapshot, sid) = {
            let mut ser = self.ser.lock(&self.rt);
            let (hold, snapshot) = self.txns.take(Scope::Every)?;
            (hold, snapshot, ser.begin(Access::ReadWrite(xid)))
        };
        self.waits.started(xid);
        Ok(Txn {
            xid: Some(xid),
            sid,
            snapshot,
            hold,
        })
    }

    /// Begin a read-only transaction. A `deferrable` one waits until its snapshot is safe,
    /// taking a new snapshot each time one is found unsafe, and then never fails with a
    /// serialization failure.
    pub fn begin_read(&self, deferrable: bool) -> Result<Txn, Error> {
        loop {
            let tx = {
                let mut ser = self.ser.lock(&self.rt);
                let (hold, snapshot) = self.txns.take(Scope::Every)?;
                let sid = ser.begin(if deferrable {
                    Access::Deferrable
                } else {
                    Access::ReadOnly
                });
                self.readers
                    .lock(&self.rt)
                    .insert(sid, (hold, snapshot.clone()));
                self.log_safe(&mut ser);
                Txn {
                    xid: None,
                    sid,
                    snapshot,
                    hold,
                }
            };
            if !deferrable {
                return Ok(tx);
            }
            loop {
                let safety = self.ser.lock(&self.rt).safety(tx.sid);
                match safety {
                    Safety::Safe => return Ok(tx),
                    Safety::Unsafe => break,
                    Safety::Pending(writers) => self.waits.wait(None, writers[0])?,
                }
            }
            self.abort(tx)?;
        }
    }

    /// Log a safe snapshot marker for every read-only transaction `ser` has found safe.
    fn log_safe(&self, ser: &mut Serializer) {
        let readers = self.readers.lock(&self.rt);
        for sid in ser.take_safe() {
            let (hold, snapshot) = &readers[&sid];
            self.txns.log_safe_snapshot(&self.pager, *hold, snapshot);
        }
    }

    /// Run term `t` of `terms`, a `Transact`, a write or a query, as one statement of `tx`, each
    /// variable of `params` bound to its value. `clock` is the instant, in microseconds, every
    /// `generate clock` of the statement takes.
    ///
    /// On an `Err` that [ends the transaction](Error::ends_transaction), abort `tx`. On any other
    /// `Err` the statement has no effect and `tx` goes on.
    pub fn statement(
        &self,
        tx: &Txn,
        terms: &Terms,
        t: Tm,
        params: &[(Var, Val)],
        clock: i64,
    ) -> Result<Outcome, Error> {
        match self.statements(tx, terms, &[t], params, clock, Bounds::default()) {
            Ok(mut done) => Ok(done.remove(0)),
            Err(f) => Err(f.error),
        }
    }

    /// Run terms `items` of `terms` in order as statements of `tx`, each reading the change sets
    /// of the ones before it and bounded by `bounds`, as [`statement`](Database::statement) runs
    /// one. Returns each one's outcome.
    ///
    /// On an `Err` that [ends the transaction](Error::ends_transaction), abort `tx`. On any other
    /// `Err` none of the items has an effect and `tx` goes on.
    pub fn statements(
        &self,
        tx: &Txn,
        terms: &Terms,
        items: &[Tm],
        params: &[(Var, Val)],
        clock: i64,
        bounds: Bounds,
    ) -> Result<Vec<Outcome>, Failed> {
        let options = exec::Options {
            fix_rounds: bounds.fix_rounds.unwrap_or(self.options.fix_rounds),
            ..self.options
        };
        let mut ran = Vec::with_capacity(items.len());
        let mut out = Vec::with_capacity(items.len());
        for (item, t) in items.iter().enumerate() {
            let s = eval::Stmt::new(self, tx, terms, *t, clock, options);
            match s.run(params) {
                Ok(o) => {
                    out.push(o);
                    ran.push(s);
                }
                Err(error) => {
                    if !error.ends_transaction() {
                        for s in std::iter::once(&s).chain(ran.iter().rev()) {
                            s.undo().map_err(|error| Failed { item, error })?;
                        }
                    }
                    return Err(Failed { item, error });
                }
            }
        }
        Ok(out)
    }

    /// Returns every row of every relation variable the catalog describes, as `tx` reads them.
    pub fn rows(&self, tx: &Txn) -> Result<Rows, Error> {
        let s = eval::Stmt::reader(self, tx);
        let mut out = Rows::new();
        for rel in self.rels.keys() {
            for (c, v) in s.rows(*rel, None)? {
                out.entry((*rel, c)).or_default().insert(v);
            }
        }
        Ok(out)
    }

    /// Commit `tx`. Returns its commit position once the log is durable past its commit and
    /// every commit it read; for a read-only transaction, the position of the latest commit it
    /// read, else of the log's durable end.
    pub fn commit(&self, tx: Txn) -> Result<Position, Error> {
        let done = {
            let mut ser = self.ser.lock(&self.rt);
            match ser.precommit(tx.sid) {
                Err(f) => Err(Error::Serialization(f)),
                Ok(()) => match tx.xid.map(|x| self.txns.commit(&self.pager, x)).transpose() {
                    Err(e) => Err(e.into()),
                    Ok(own) => {
                        ser.committed(tx.sid);
                        self.log_safe(&mut ser);
                        self.readers.lock(&self.rt).remove(&tx.sid);
                        let read = self.txns.read_upto(tx.hold);
                        self.txns.release(tx.hold);
                        Ok(own.max(read))
                    }
                },
            }
        };
        match done {
            Ok(wait) => {
                let wait = wait.unwrap_or_else(|| self.pager.durable());
                self.pager.wait_durable(wait);
                if let Some(x) = tx.xid {
                    self.waits.ended(x);
                }
                Ok(self.pager.position(wait))
            }
            Err(e) => {
                self.abort(tx)?;
                Err(e)
            }
        }
    }

    /// Abort `tx` and release what it holds.
    ///
    /// Fails if the database is quarantined, leaving `tx`'s id running in the commit log.
    pub fn abort(&self, tx: Txn) -> Result<(), Error> {
        let aborted = {
            let mut ser = self.ser.lock(&self.rt);
            let aborted = tx.xid.map(|x| self.txns.abort(&self.pager, x)).transpose();
            ser.aborted(tx.sid);
            self.log_safe(&mut ser);
            self.readers.lock(&self.rt).remove(&tx.sid);
            self.txns.release(tx.hold);
            aborted
        };
        if let Some(x) = tx.xid {
            self.waits.ended(x);
        }
        aborted?;
        Ok(())
    }

    /// Returns the page of the database's generated counters, for [`Setup::sequences`] when
    /// it is opened again.
    pub fn sequences_root(&self) -> Pid {
        self.sequences.lock(&self.rt).root()
    }

    /// Returns the next value of `rel`'s sequence.
    fn next_sequence(&self, rel: RelId) -> Result<i64, Error> {
        self.sequences.lock(&self.rt).draw(&self.pager, rel.0, 1)
    }

    /// Advance `rel`'s sequence past `held`, a key a row of it holds.
    fn advance_sequence(&self, rel: RelId, held: i64) -> Result<(), Error> {
        self.sequences
            .lock(&self.rt)
            .advance(&self.pager, rel.0, held)
    }

    /// Returns a `load` value never issued before and greater than `held`.
    fn next_load(&self, held: i64) -> Result<i64, Error> {
        self.sequences
            .lock(&self.rt)
            .draw(&self.pager, sequence::LOADS, held.saturating_add(1))
    }

    fn store(&self, tx: &Txn) -> Arc<exec::Store<R>> {
        let reads = exec::AtSnapshot {
            rt: self.rt.clone(),
            pager: self.pager.clone(),
            txns: self.txns.clone(),
            snapshot: tx.snapshot.clone(),
            reader: tx.xid,
            serial: Some((self.ser.clone(), tx.sid)),
        };
        Arc::new(exec::Store {
            pager: self.pager.clone(),
            reads: Arc::new(reads),
            heaps: self.heaps.clone(),
            indexes: self.indexes.clone(),
        })
    }

    fn call(
        &self,
        store: &Arc<exec::Store<R>>,
        options: exec::Options,
    ) -> Result<exec::Call<R>, Error> {
        let grant =
            self.budget
                .admit(self.grant)
                .ok_or(Error::StatementLimit(exec::Exceeded::Memory {
                    limit: self.budget.limit(),
                }))?;
        Ok(exec::Call {
            store: store.clone(),
            grant,
            pool: self.pool.clone(),
            options,
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }
}
