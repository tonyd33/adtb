mod apply;
mod check;
mod delta;
mod eval;
mod explain;
mod fresh;
mod input;
mod past;
mod polarity;
mod quarantine;
mod reindex;
mod schema;
mod sequence;
mod stream;
mod verify;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::task::Waker;
use std::time::Duration;

use adtb_file::Disk;
use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;
use adtb_sys::lock::Lock;

use crate::btree::Index;
use crate::exec::{self, Budget, Scheduler, StoredHeap, StoredIndex, Val};
use crate::heap::Heap;
use crate::key::CtorId;
use crate::mvcc::{
    Access, Limits, Safety, Scope, SerialId, SerializationFailure, Serializer, Snapshot,
    SnapshotId, Transactions, Waits, Watch, Xid,
};
use crate::plan::explain::Node;
use crate::plan::{Catalog, HeapId, IndexId};
use crate::runtime::{Instant, IoError, Runtime};
use crate::storage::{PageError, Pager, Pid, Position};
use crate::vacuum;

pub use adtb_lang::schema::Stage;

pub use delta::{CheckPlan, Part};
pub use input::Local;
pub use past::Retained;
pub use reindex::{Failure, Rebuild, panic_text};
pub use stream::{Produced, Production};
pub use verify::{Found, VerifyScope, VerifySnapshot};

use schema::Rels;
pub(crate) use sequence::Sequences;
pub use sequence::counter_page;

#[cfg(test)]
pub(crate) use sequence::full_counter_page;

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
    /// A new row of constructor `ctor` of `relation` stores in `size` bytes, over the heap's
    /// `limit`.
    RowTooLarge {
        relation: RelId,
        ctor: u32,
        size: usize,
        limit: usize,
    },
    KeyTooLarge {
        relation: RelId,
        ctor: u32,
        index: String,
        size: usize,
        limit: usize,
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
    /// The statement's spill file could not be made, written or read.
    Spill(IoError),
    /// The call reads, or writes under, what this text names, which is quarantined.
    Quarantined(String),
    /// The commit log is corrupt or its ids are spent.
    Transactions(String),
    Corrupt(String),
    /// The statement needs a form the engine does not run yet.
    Unsupported(String),
    /// The role the statement runs for lacks a privilege, as this text states.
    Denied(String),
    SequenceExhausted,
    /// The plan or a value breaks the invariant this text names, one the checker guarantees.
    Internal(String),
    /// The schema has a constraint without an index-bounded incremental check; each constraint
    /// refused, with why.
    Refused(Vec<(String, String)>),
    /// The files a path names differ from those an earlier scan of it in the transaction read.
    FileChanged(String),
    Cancelled,
    DeadlineExceeded,
    /// A statement run with [`Bounds::suspend`] must wait for transaction `on` to end. A rerun
    /// of the statements passes `rerun` as [`Bounds::rerun`].
    Blocked {
        on: Xid,
        rerun: Rerun,
    },
    /// A statement reached a host function, by its name, with no body bound.
    MissingFunction(String),
    /// A read of the past names a state the engine keeps no history of; `oldest` is the oldest
    /// position a read may name.
    HistoryUnavailable {
        oldest: Position,
    },
    /// A read of the past names a state at schema version `version`, which defines what it reads
    /// otherwise than the version it is typed at.
    WrongVersion {
        version: u64,
    },
}

impl Error {
    /// Returns whether the failure ends the transaction: the caller must abort it.
    pub fn ends_transaction(&self) -> bool {
        matches!(
            self,
            Error::Serialization(_)
                | Error::Page(_)
                | Error::Transactions(_)
                | Error::Corrupt(_)
                | Error::FileChanged(_)
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
            crate::heap::Error::Changed { current } => {
                Error::Serialization(SerializationFailure::new(
                    crate::mvcc::Cause::ConcurrentWrite,
                    current
                        .into_iter()
                        .map(|h| Xid(h.xmax))
                        .filter(|x| *x != Xid::NONE)
                        .collect(),
                ))
            }
            crate::heap::Error::TooLarge { len, limit } => Error::Unsupported(format!(
                "a row version of {len} bytes, over the limit of {limit}"
            )),
        }
    }
}

impl From<vacuum::Error> for Error {
    fn from(e: vacuum::Error) -> Error {
        match e {
            vacuum::Error::Page(p) | vacuum::Error::Heap(crate::heap::Error::Page(p)) => {
                Error::Page(p)
            }
            vacuum::Error::Index(i) => Error::from(i),
            vacuum::Error::Heap(e @ crate::heap::Error::Changed { .. }) => Error::from(e),
            vacuum::Error::Heap(crate::heap::Error::TooLarge { len, limit }) => {
                Error::Internal(format!(
                    "a version vacuum moves fits its history heap, as it fit its current heap: \
                     {len} bytes over the limit of {limit}"
                ))
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
            exec::Error::Spill(io) => Error::Spill(io),
            exec::Error::Corrupt(c) => Error::Corrupt(c),
            exec::Error::Unsupported(u) => Error::Unsupported(u),
            exec::Error::Rejected(detail) => Error::Rejected(Rejection::BadInput {
                source: String::new(),
                line: 0,
                detail,
            }),
            exec::Error::NoCaller => Error::Unsupported("`caller` in a statement".into()),
            exec::Error::Cancelled => Error::Cancelled,
            exec::Error::DeadlineExceeded => Error::DeadlineExceeded,
            exec::Error::MissingFunction(name) => Error::MissingFunction(name),
            exec::Error::OtherVersion(name) => Error::Internal(format!(
                "host function `{name}` is bound at the version its declaration names"
            )),
            exec::Error::Internal(what) => Error::Internal(what),
            exec::Error::HistoryUnavailable { oldest } => Error::HistoryUnavailable { oldest },
            exec::Error::WrongVersion { version } => Error::WrongVersion { version },
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

/// What a deferrable read-only transaction waits for before it reads.
#[derive(Debug)]
pub enum Deferral {
    /// Its snapshot is safe.
    Safe(Txn),
    /// Its snapshot is safe or unsafe once transaction `on` ends.
    Wait { tx: Txn, on: Xid },
}

/// A transaction, read-write or read-only.
#[derive(Debug)]
pub struct Txn {
    /// `None` for a read-only transaction.
    xid: Option<Xid>,
    sid: SerialId,
    snapshot: Snapshot,
    hold: SnapshotId,
    /// What its statements that succeeded wrote into the file.
    written: RefCell<quarantine::Touched>,
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

/// The statements of one call: terms of `terms` over `schema`, each variable of `params` bound
/// to its value.
#[derive(Clone, Copy, Debug)]
pub struct Text<'a> {
    /// The database's schema, or one that extends it with the transaction's temp relations.
    pub schema: &'a Schema,
    pub terms: &'a Terms,
    pub items: &'a [Tm],
    pub params: &'a [(Var, Val)],
    /// The instant, in microseconds, every `generate clock` of the statements takes.
    pub clock: i64,
    /// The principal `caller` denotes, `None` where nothing may read it.
    pub caller: Option<&'a Val>,
}

/// What runs between the statements of a call: given the number of statements run so far,
/// before the first and after each.
pub type Between<'h> = dyn FnMut(usize, &mut Local) -> Result<(), Error> + 'h;

/// What bounds the statements of one call, past the database's own bounds.
#[derive(Clone, Debug, Default)]
pub struct Bounds {
    /// The rounds a fixpoint may take, else the database's limit.
    pub fix_rounds: Option<usize>,
    /// How long each statement may run, from its start, or from [`Bounds::rerun`].
    pub limit: Option<Duration>,
    /// Set to stop the statements with `Cancelled`.
    pub cancel: Option<Arc<AtomicBool>>,
    /// The instant of the runtime's clock at which the statements fail with `DeadlineExceeded`.
    pub deadline: Option<Instant>,
    /// Fail with `Blocked` at a wait for another transaction, instead of parking the thread.
    pub suspend: bool,
    /// The `rerun` of the `Blocked` failure of an earlier run of the same statements.
    pub rerun: Option<Rerun>,
    /// The grant every execution of the statements takes its memory from, else one admitted
    /// for each.
    pub grant: Option<Arc<exec::Grant>>,
}

/// Where the statement limit of a rerun of suspended statements starts.
///
/// A rerun runs the statements before the latest one an earlier run reached without the limit,
/// and limits that latest one from its first start, later by the time the rerun took to reach
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rerun {
    through: usize,
    since: Instant,
}

impl Rerun {
    /// Returns when a statement limit of `limit` runs out for the statement that suspended, if
    /// no rerun time is added to it.
    pub fn limit_at(&self, limit: Duration) -> Instant {
        self.since + limit
    }
}

/// What stops one statement's executions early.
#[derive(Clone, Debug, Default)]
pub struct Stop {
    /// Set to stop them with `Cancelled`.
    pub cancel: Arc<AtomicBool>,
    pub deadline: Option<exec::Deadline>,
    /// Fail with `Blocked`, carrying this, at a wait for another transaction, instead of
    /// parking the thread.
    pub suspend: Option<Rerun>,
    /// The grant every execution takes its memory from, else one admitted for each.
    pub grant: Option<Arc<exec::Grant>>,
}

impl Stop {
    /// Returns what stops a statement under `bounds`, its statement limit counted from `since`
    /// and unlimited without it, failing with `Blocked` carrying `suspend` at a wait.
    fn of(bounds: &Bounds, since: Option<Instant>, suspend: Option<Rerun>) -> Stop {
        let limit = since
            .zip(bounds.limit)
            .map(|(since, after)| exec::Deadline {
                at: since + after,
                error: exec::Error::StatementLimit(exec::Exceeded::Time { after }),
            });
        let deadline = bounds.deadline.map(|at| exec::Deadline {
            at,
            error: exec::Error::DeadlineExceeded,
        });
        Stop {
            cancel: bounds.cancel.clone().unwrap_or_default(),
            deadline: [limit, deadline].into_iter().flatten().min_by_key(|d| d.at),
            suspend,
            grant: bounds.grant.clone(),
        }
    }
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
    files: Disk,
    /// Held while a vacuum runs or the retention window changes, so that one runs at a time.
    vacuuming: Lock<()>,
    /// The relations an offline rebuild holds writes to.
    offline: Lock<BTreeSet<RelId>>,
    /// Set once the engine closes: rebuilds stop and their gates refuse.
    rebuilds_stopped: AtomicBool,
    /// The structures history is kept in, once [`Database::with_history`] gives them.
    retained: Option<Arc<Retained>>,
    /// The heap of the catalog's kept rows, once [`Database::with_kept_store`] gives it. Its
    /// damage is a finding and marks nothing.
    kept_store: Option<Arc<Heap>>,
    hosts: Arc<exec::Hosts>,
    /// The rows the engine holds for each volatile relation.
    volatile: Lock<BTreeMap<RelId, BTreeSet<Val>>>,
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
    /// The page of the database's generated counters.
    pub sequences: Pid,
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
        let sequences = if setup.sequences == Pid(0) {
            Sequences::none()
        } else {
            Sequences::open(&setup.pager, setup.sequences)?
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
            files: Disk::default(),
            vacuuming: Lock::new(()),
            offline: Lock::default(),
            rebuilds_stopped: AtomicBool::new(false),
            retained: None,
            kept_store: None,
            hosts: Arc::default(),
            volatile: Lock::default(),
            rt,
        })
    }

    /// Returns the database calling the host functions of `hosts`.
    ///
    /// Panic if a statement has run.
    pub fn with_hosts(mut self, hosts: Arc<exec::Hosts>) -> Self {
        for d in &mut self.denials {
            for (_, p) in &mut d.deltas {
                Arc::get_mut(p)
                    .expect("no statement holds a check's program before the first runs")
                    .shared
                    .hosts = hosts.clone();
            }
        }
        for rel in self.rels.values_mut() {
            for (_, indexed) in &mut rel.indexes {
                indexed.keys.set_hosts(&hosts);
            }
        }
        self.hosts = hosts;
        self
    }

    /// Returns the database keeping history in `retained`, for a retention window of `window`:
    /// keeping it from now on when `window` is not zero.
    ///
    /// Fail if the record of the window is damaged.
    pub fn with_history(mut self, retained: Retained, window: Duration) -> Result<Self, Error> {
        self.retained = Some(Arc::new(retained));
        self.set_retention(window)?;
        Ok(self)
    }

    /// Returns the database reading its kept rows from `heap`.
    pub fn with_kept_store(mut self, heap: Arc<Heap>) -> Self {
        self.kept_store = Some(heap);
        self
    }

    /// Returns the database reading `rows` for each volatile relation of `rows`.
    pub fn with_volatile(mut self, rows: BTreeMap<RelId, BTreeSet<Val>>) -> Self {
        self.volatile = Lock::new(rows);
        self
    }

    /// Hold `rows` as the rows of volatile relation `rel` from now on.
    pub fn set_volatile(&self, rel: RelId, rows: BTreeSet<Val>) {
        self.volatile.lock(&self.rt).insert(rel, rows);
    }

    /// Returns the transactions layer.
    pub fn txns(&self) -> &Transactions<R> {
        &self.txns
    }

    /// Returns the database reading the files of file scans through `files`.
    pub fn with_files(mut self, files: Disk) -> Self {
        self.files = files;
        self
    }

    pub fn pager(&self) -> &Pager<R> {
        &self.pager
    }

    /// Begin a read-write transaction. Fail if the database is quarantined.
    pub fn begin(&self) -> Result<Txn, Error> {
        self.admit_database()?;
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
            written: RefCell::default(),
        })
    }

    /// Begin a read-only transaction. A `deferrable` one waits until its snapshot is safe,
    /// taking a new snapshot each time one is found unsafe, and then never fails with a
    /// serialization failure.
    pub fn begin_read(&self, deferrable: bool) -> Result<Txn, Error> {
        let mut tx = self.begin_reader(deferrable)?;
        if !deferrable {
            return Ok(tx);
        }
        loop {
            match self.defer(tx)? {
                Deferral::Safe(t) => return Ok(t),
                Deferral::Wait { tx: t, on } => {
                    self.waits.wait(None, on)?;
                    tx = t;
                }
            }
        }
    }

    /// Begin a deferrable read-only transaction, its snapshot not yet known to be safe.
    pub fn begin_deferrable(&self) -> Result<Txn, Error> {
        self.begin_reader(true)
    }

    /// Decide the snapshot of deferrable transaction `tx`. Returns it once its snapshot is
    /// safe, or with the writer its snapshot waits for. Aborts it when its snapshot is unsafe
    /// and decides a new one in its place.
    pub fn defer(&self, mut tx: Txn) -> Result<Deferral, Error> {
        loop {
            let safety = self.ser.lock(&self.rt).safety(tx.sid);
            match safety {
                Safety::Safe => return Ok(Deferral::Safe(tx)),
                Safety::Pending(writers) => {
                    return Ok(Deferral::Wait { tx, on: writers[0] });
                }
                Safety::Unsafe => {
                    self.abort(tx)?;
                    tx = self.begin_reader(true)?;
                }
            }
        }
    }

    fn begin_reader(&self, deferrable: bool) -> Result<Txn, Error> {
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
        Ok(Txn {
            xid: None,
            sid,
            snapshot,
            hold,
            written: RefCell::default(),
        })
    }

    /// Register `waker` to be woken once transaction `on` has ended, as [`Waits::watch`] does.
    /// `me` is the waiting transaction, if it has an xid.
    pub fn watch(
        &self,
        me: Option<Xid>,
        on: Xid,
        waker: &Waker,
    ) -> Result<Watch, SerializationFailure> {
        self.waits.watch(me, on, waker)
    }

    /// Withdraw the registration of `waker` on `on` made as `me`, if `on` has not ended.
    pub fn withdraw(&self, me: Option<Xid>, on: Xid, waker: &Waker) {
        self.waits.withdraw(me, on, waker);
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
        let text = Text {
            schema: &self.schema,
            terms,
            items: &[t],
            params,
            clock,
            caller: None,
        };
        match self.statements(tx, text, &mut Local::default(), Bounds::default()) {
            Ok(mut done) => Ok(done.remove(0)),
            Err(f) => Err(f.error),
        }
    }

    /// Run the items of `text` in order as statements of `tx`, each reading the change sets of
    /// the ones before it and bounded by `bounds`, as [`statement`](Database::statement) runs
    /// one, over `local`, what `tx` holds outside the file. Returns each one's outcome.
    ///
    /// On an `Err` that [ends the transaction](Error::ends_transaction), abort `tx`. On any other
    /// `Err` none of the items has an effect on the file or on the temp relations of `local`, and
    /// `tx` goes on. Every file an item read stays pinned in `local` either way.
    pub fn statements(
        &self,
        tx: &Txn,
        text: Text<'_>,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<Vec<Outcome>, Failed> {
        self.statements_with(tx, text, local, bounds, &mut |_, _| Ok(()))
    }

    /// Run the items of `text` as [`statements`](Database::statements) does, running `between`
    /// before the first and after each. A failure of `between` fails the call as a failure of
    /// the next item does.
    pub fn statements_with(
        &self,
        tx: &Txn,
        text: Text<'_>,
        local: &mut Local,
        bounds: Bounds,
        between: &mut Between<'_>,
    ) -> Result<Vec<Outcome>, Failed> {
        self.admit(&text)
            .map_err(|error| Failed { item: 0, error })?;
        let mut ran = Vec::with_capacity(text.items.len());
        let mut out = Vec::with_capacity(text.items.len());
        let before = local.clone();
        let started = self.rt.now();
        for item in 0..=text.items.len() {
            let done = between(item, local).and_then(|()| match text.items.get(item) {
                Some(t) => {
                    let s = self.stmt(tx, text, *t, item, &bounds, started);
                    let o = s.run(text.params, local);
                    ran.push(s);
                    o.map(|o| out.push(o))
                }
                None => Ok(()),
            });
            if let Err(error) = done {
                local.restore_temps(before);
                if !error.ends_transaction() {
                    self.undo(&ran).map_err(|error| Failed { item, error })?;
                }
                return Err(Failed { item, error });
            }
        }
        for o in &out {
            self.record_written(&mut tx.written.borrow_mut(), o);
        }
        Ok(out)
    }

    /// Run the items of `text` as [`statements`](Database::statements) does, then undo every
    /// one, leaving `tx` and the temp relations of `local` as they were and every file an item
    /// read pinned. Returns each one's outcome.
    pub fn trial(
        &self,
        tx: &Txn,
        text: Text<'_>,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<Vec<Outcome>, Failed> {
        self.admit(&text)
            .map_err(|error| Failed { item: 0, error })?;
        let before = local.clone();
        let mut ran = Vec::with_capacity(text.items.len());
        let mut out = Vec::with_capacity(text.items.len());
        let started = self.rt.now();
        for (item, t) in text.items.iter().enumerate() {
            let s = self.stmt(tx, text, *t, item, &bounds, started);
            let o = s.run(text.params, local);
            ran.push(s);
            match o {
                Ok(o) => out.push(o),
                Err(error) => {
                    local.restore_temps(before);
                    if !error.ends_transaction() {
                        self.undo(&ran).map_err(|error| Failed { item, error })?;
                    }
                    return Err(Failed { item, error });
                }
            }
        }
        local.restore_temps(before);
        let item = text.items.len();
        self.undo(&ran).map_err(|error| Failed { item, error })?;
        Ok(out)
    }

    /// Returns the operators of term `t` of `text` as a statement of `tx` would run it.
    pub fn explain(&self, tx: &Txn, text: Text<'_>, t: Tm) -> Result<Node, Error> {
        let params: Vec<Var> = text.params.iter().map(|(v, _)| *v).collect();
        self.stmt(tx, text, t, 0, &Bounds::default(), self.rt.now())
            .explain(&params)
    }

    /// Vacuums each constructor relation of `rel` in turn. Returns the id of each constructor
    /// with what its vacuum did.
    pub fn vacuum(&self, rel: RelId) -> Result<Vec<(CtorId, vacuum::Report)>, Error> {
        let _one = self.vacuuming.lock(&self.rt);
        let r = self
            .rels
            .get(&rel)
            .ok_or_else(|| Error::Internal(format!("vacuum of {rel:?}, which has no heaps")))?;
        self.admit_vacuum(r)?;
        let ids: BTreeMap<u32, CtorId> = self.indexes[&r.key_index]
            .ctors
            .iter()
            .map(|(id, c)| (*c, *id))
            .collect();
        let indexes: Vec<&Index> = r
            .indexes
            .iter()
            .map(|(i, _)| &self.indexes[i].index)
            .collect();
        let mut out = Vec::with_capacity(r.ctors.len());
        for (c, ctor) in r.ctors.iter().enumerate() {
            let id = ids[&(c as u32)];
            let sum = vacuum::Sum {
                heaps: vec![(id, &*self.heaps[&ctor.heap].heap)],
                indexes: indexes.clone(),
            };
            let report = self.vacuum_sum(r, &sum)?;
            out.push((id, report));
        }
        Ok(out)
    }

    /// Returns term `t`, item `item` of `text`, as a statement of `tx` bounded by `bounds`, in
    /// a run of the statements that started at `started`.
    fn stmt<'a>(
        &'a self,
        tx: &'a Txn,
        text: Text<'a>,
        t: Tm,
        item: usize,
        bounds: &Bounds,
        started: Instant,
    ) -> eval::Stmt<'a, R> {
        let options = exec::Options {
            fix_rounds: bounds.fix_rounds.unwrap_or(self.options.fix_rounds),
            ..self.options
        };
        let now = self.rt.now();
        let (rerun, limited) = match bounds.rerun {
            Some(r) if item < r.through => (r, false),
            Some(r) if item == r.through => (
                Rerun {
                    through: item,
                    since: r.since + now.saturating_since(started),
                },
                true,
            ),
            _ => (
                Rerun {
                    through: item,
                    since: now,
                },
                true,
            ),
        };
        let stop = Stop::of(
            bounds,
            limited.then_some(rerun.since),
            bounds.suspend.then_some(rerun),
        );
        eval::Stmt::new(self, tx, text, t, options, stop)
    }

    /// Undoes the effects of `ran` on the file, the last first.
    fn undo(&self, ran: &[eval::Stmt<'_, R>]) -> Result<(), Error> {
        ran.iter().rev().try_for_each(eval::Stmt::undo)
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

    /// Returns the rows of `rel` visible to `tx`.
    pub(crate) fn relation_rows(&self, tx: &Txn, rel: RelId) -> Result<Vec<Val>, Error> {
        eval::Stmt::reader(self, tx)
            .rows(rel, None)
            .map(|rows| rows.into_iter().map(|(_, row)| row).collect())
    }

    /// Returns the next deployment row id of `rel`, at least `floor`.
    pub(crate) fn deployment_id(&self, rel: RelId, floor: i64) -> Result<i64, Error> {
        self.sequences
            .lock(&self.rt)
            .draw(&self.pager, rel.0, floor)
    }

    /// Commit `tx`. Returns its commit position once the log is durable past its commit and
    /// every commit it read; for a read-only transaction, the position of the latest commit it
    /// read, else of the log's durable end.
    ///
    /// Abort `tx` and fail with `Quarantined` if a mark set before its commit record stops what
    /// `tx` wrote, by the rule that stops a statement.
    pub fn commit(&self, tx: Txn) -> Result<Position, Error> {
        let done = {
            let mut ser = self.ser.lock(&self.rt);
            let admitted = self.admit_commit(&tx);
            match admitted.and_then(|()| ser.precommit(tx.sid).map_err(Error::Serialization)) {
                Err(e) => Err(e),
                Ok(()) => match tx.xid.map(|x| self.txns.commit(&self.pager, x)).transpose() {
                    Err(e) => Err(e.into()),
                    Ok(own) => {
                        ser.committed(tx.sid);
                        if let (Some(xid), Some(lsn)) = (tx.xid, own) {
                            self.txns.publish_commit(&self.pager, xid, lsn);
                        }
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
                if tx.xid.is_some() && self.txns.history().is_some() {
                    self.pager.flush_below(self.pager.appended());
                }
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

    /// Returns a grant of the database's per-call memory from its budget.
    pub fn admit_grant(&self) -> Result<Arc<exec::Grant>, Error> {
        self.budget
            .admit(self.grant)
            .ok_or(Error::StatementLimit(exec::Exceeded::Memory {
                limit: self.budget.limit(),
            }))
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
        let upto = self
            .txns
            .read_upto(tx.hold)
            .unwrap_or_else(|| self.pager.durable());
        Arc::new(exec::Store {
            pager: self.pager.clone(),
            reads: Arc::new(reads),
            heaps: self.heaps.clone(),
            indexes: self.indexes.clone(),
            history: exec::Histories::default(),
            past: self.pasts(self.pager.position(upto)),
        })
    }

    fn call(
        &self,
        store: &Arc<exec::Store<R>>,
        options: exec::Options,
        stop: &Stop,
    ) -> Result<exec::Call<R>, Error> {
        let grant = match &stop.grant {
            Some(g) => g.clone(),
            None => self.admit_grant()?,
        };
        Ok(exec::Call {
            store: store.clone(),
            grant,
            pool: self.pool.clone(),
            options,
            cancel: stop.cancel.clone(),
            deadline: stop.deadline.clone(),
        })
    }
}
