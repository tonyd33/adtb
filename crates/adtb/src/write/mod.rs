//! Run the write path: change sets, constraint checks, schema changes and reindexing.

mod admit;
mod apply;
mod broken;
mod check;
mod class;
mod delta;
mod eval;
mod explain;
mod fresh;
mod grant;
mod input;
mod keyed;
mod mend;
mod past;
mod polarity;
mod quarantine;
mod reindex;
mod restore;
mod rot;
mod schema;
mod sequence;
mod session;
mod stream;
mod vacuums;
mod verify;
mod writing;

#[cfg(test)]
mod admit_tests;
#[cfg(test)]
mod calls_tests;
#[cfg(test)]
mod mend_tests;
#[cfg(test)]
mod past_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod write_states_tests;
#[cfg(test)]
mod write_tests;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use adtb_file::Disk;
use adtb_lang::ids::CatIndexId;
use adtb_lang::ids::held_id;
use adtb_lang::kernel::{Terms, Tm, ValidTerms, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::Committed;
use adtb_lang::types::RelId;
use adtb_sys::lock::{Lock, Ranked};

use crate::btree::build::{IndexLatch, IndexTrees};
use crate::catalog::{IndexSpec, Structures};
use crate::exec::{self, Budget, Scheduler, Val};
use crate::key::CtorId;
use crate::mvcc::{
    Begun, Conflict, Hold, Limits, Safety, SerialId, Serializer, Snapshot, Taken, Transactions, Xid,
};
use crate::plan::explain::Node;
use crate::plan::{Catalog, HeapId, IndexId};
use crate::ranks;
use crate::runtime::{Instant, Runtime, ThreadId};
#[cfg(any(test, feature = "testing"))]
use crate::storage::Pid;
use crate::storage::{MarkFailure, PageError, Pager, Position, Quarantine};
use crate::vacuum;

pub use adtb_lang::schema::Stage;

pub use broken::{Broken, TermKind, detail};
#[cfg(any(test, feature = "testing"))]
pub use check::idle_deltas_checked;
pub use check::refusals;
pub use delta::{CheckPlan, Part, unchanged};
pub use eval::{Violated, Violator};
pub use input::Local;
pub use past::Retained;
pub(crate) use past::position;
pub use quarantine::{By, Mapped, Mapping};
pub use reindex::{Failure, HoldWait, Rebuild, VacuumHold};
pub use rot::Rot;
pub use stream::{Produced, Production};
pub use vacuums::Due;
pub use verify::{Found, VerifyScope};
pub(crate) use writing::{Writing, outside_writer};

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
        violator: Violator,
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
    /// An argument of a command is outside its parameter's domain, as `detail` states.
    Invalid {
        detail: String,
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
    /// The call reads, or writes under, `objects`, which are quarantined, each with why its mark
    /// holds only in memory if it does. `by` states what made the call fail.
    Quarantined {
        objects: BTreeMap<Quarantine, Option<MarkFailure>>,
        by: By,
    },
    /// The transactions layer refuses the call.
    Transactions(crate::mvcc::Error),
    /// The open could not arm the read-in gate, so it refuses the call.
    Unarmed(crate::storage::Unarmed),
    Corrupt(Rot),
    /// The role the statement runs for lacks a privilege, as this text states.
    Denied(String),
    SequenceExhausted,
    Internal(Box<Broken>),
    Exec(exec::Error),
    /// The schema has a constraint without an index-bounded incremental check; each constraint
    /// refused, with why.
    Refused(Vec<(String, String)>),
    /// The files a path names differ from those an earlier scan of it in the transaction read.
    FileChanged(String),
    /// The engine closed while the statement waited for a transaction.
    Closed,
    /// A statement run with [`Bounds::suspend`] must wait for transaction `on` to end, to write
    /// a version of `heap`, or at a gate for `None`. A rerun of the statements passes `rerun` as
    /// [`Bounds::rerun`].
    Blocked {
        on: Xid,
        heap: Option<u32>,
        rerun: Rerun,
    },
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Exec(exec::Error::Page(e))
    }
}

impl From<crate::heap::OrphanEntry> for Error {
    fn from(e: crate::heap::OrphanEntry) -> Error {
        Error::Exec(e.into())
    }
}

impl From<Conflict> for Error {
    fn from(f: Conflict) -> Error {
        Error::Exec(exec::Error::Serialization(f))
    }
}

impl From<crate::heap::Error> for Error {
    fn from(e: crate::heap::Error) -> Error {
        match e {
            crate::heap::Error::Page(p) => p.into(),
            crate::heap::Error::Changed(c) => c.into(),
            crate::heap::Error::TooLarge { len, limit } => {
                Error::Internal(Box::new(Broken::Unadmitted { len, limit }))
            }
            crate::heap::Error::NoDirectory => Broken::NoDirectory.into(),
        }
    }
}

impl From<vacuum::Error> for Error {
    fn from(e: vacuum::Error) -> Error {
        match e {
            vacuum::Error::Page(p) | vacuum::Error::Heap(crate::heap::Error::Page(p)) => p.into(),
            vacuum::Error::Index(i) => Error::from(i),
            vacuum::Error::Heap(e @ crate::heap::Error::Changed { .. }) => Error::from(e),
            vacuum::Error::Heap(crate::heap::Error::NoDirectory) => Broken::NoDirectory.into(),
            vacuum::Error::Heap(crate::heap::Error::TooLarge { len, limit }) => panic!(
                "a version vacuum moves fits its history heap, as it fit its current heap: \
                 {len} bytes over the limit of {limit}"
            ),
            vacuum::Error::Stopped => Error::Exec(exec::Error::Cancelled),
            vacuum::Error::Unkeyed(c) => Error::Exec(exec::Broken::UnkeyedRow(c.0).into()),
        }
    }
}

impl From<crate::btree::Error> for Error {
    fn from(e: crate::btree::Error) -> Error {
        match e {
            crate::btree::Error::Page(p) => p.into(),
            crate::btree::Error::Key(e) => e.into(),
            other => Rot::Index(other).into(),
        }
    }
}

impl From<crate::key::EncodeError> for Error {
    fn from(e: crate::key::EncodeError) -> Error {
        Error::Exec(e.into())
    }
}

impl From<crate::mvcc::Error> for Error {
    fn from(e: crate::mvcc::Error) -> Error {
        Error::Transactions(e)
    }
}

impl From<crate::catalog::Error> for Error {
    fn from(e: crate::catalog::Error) -> Error {
        use crate::catalog::Error as C;
        match e {
            C::Page(p)
            | C::Heap(crate::heap::Error::Page(p))
            | C::Index(crate::btree::Error::Page(p)) => p.into(),
            C::Index(e @ (crate::btree::Error::Malformed(_) | crate::btree::Error::Entry(..))) => {
                Rot::Index(e).into()
            }
            C::Corrupt(r) => Rot::Catalog(r).into(),
            C::Missing(what) => Rot::NoRoot(what).into(),
            C::Txn(e) => Error::Transactions(e),
            C::Spill(io) => Error::Exec(exec::Error::Spill(io)),
            C::Conflict(c) | C::Heap(crate::heap::Error::Changed(c)) => c.into(),
            other => Broken::CatalogError(other).into(),
        }
    }
}

impl From<exec::Error> for Error {
    fn from(e: exec::Error) -> Error {
        Error::Exec(e)
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
    ends: bool,
}

impl Failed {
    fn new(item: usize, error: Error) -> Failed {
        let ends = error.class().ends();
        Failed { item, error, ends }
    }

    /// Returns the failure, ending the transaction, of taking back the statements of a call:
    /// `item` is the item that failed, or the number of items when every one succeeded and a
    /// trial takes them back.
    fn undone(item: usize, error: Error) -> Failed {
        Failed {
            item,
            error,
            ends: true,
        }
    }

    /// Returns whether the failure ends the transaction: the caller must abort it.
    pub fn ends(&self) -> bool {
        self.ends
    }
}

/// What a deferrable read-only transaction waits for before it reads.
#[derive(Debug)]
pub enum Deferral {
    /// Its snapshot is safe.
    Safe(Txn),
    /// Its snapshot is safe or unsafe once transaction `on` ends.
    Wait { tx: Txn, on: Xid },
}

/// A transaction, read-write or read-only. It ends once: by [`Database::commit`], by
/// [`Database::abort`] or [`Database::abandon`], or when dropped, which aborts it on every path.
/// The abort of a drop is recorded best effort, and not at all while the thread panics or once
/// the engine is closed.
///
/// Dropping it takes the serializer's lock and, inside it, the lock of the transactions, the
/// commit log and the status page of its id as its `Begun` aborts, then the waits' lock, waking
/// the waiters of its id outside it, then the readers' lock, then the lock of the transactions
/// again as its hold is released. Once the serializer's lock is released it takes the lock of
/// the dead counts.
pub struct Txn {
    /// `None` for a read-only transaction.
    xid: Option<Xid>,
    sid: SerialId,
    /// The instant of the runtime's clock at which it began.
    began: Instant,
    taken: Taken,
    /// What its statements that succeeded wrote into the file.
    written: RefCell<quarantine::Touched>,
    held: Held,
}

/// What a transaction holds: its running id, its entry in the serializer, its reader's snapshot
/// in the readers, its hold, its registration with the waits and the versions it made dead.
/// Dropping it releases what is left of each, once, on every path.
struct Held {
    /// `None` for a read-only transaction, or once its commit or abort takes it.
    begun: Option<Begun>,
    /// `None` once its commit or its release ends it in the serializer.
    serial: Option<SerialId>,
    /// `None` for a read-write transaction.
    reader: Option<SerialId>,
    hold: Option<Hold>,
    changed: RefCell<vacuums::Changed>,
    /// Set once one of its operations has failed other than by suspending on a wait; dropping it
    /// then counts no end by drop.
    failed: Cell<bool>,
    /// Set once a commit, abort or abandon takes it; dropping it then counts no end by drop.
    ending: bool,
    session: Arc<dyn session::Release>,
}

/// The transaction resources of one database: the transactions, the serializer, the snapshots
/// of the read-only transactions, the waits on running ones and the versions they made dead.
/// The database and every transaction it begins share it.
///
/// `Transactions` locks itself and takes no lock of the session.
struct Session<R: Runtime> {
    rt: R,
    pager: Pager<R>,
    txns: Arc<Transactions<R>>,
    ser: Arc<Ranked<Serializer>>,
    /// The snapshot of each running read-only transaction, for its safe snapshot marker.
    readers: Ranked<BTreeMap<SerialId, (Taken, Snapshot)>>,
    dead: Ranked<vacuums::Dead>,
    heaps: vacuums::RelHeaps,
    steps: Arc<crate::steps::Steps>,
    /// How many transactions ended by being dropped.
    #[cfg(any(test, feature = "testing"))]
    dropped: std::sync::atomic::AtomicUsize,
}

impl std::fmt::Debug for Txn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Txn")
            .field("xid", &self.xid)
            .field("sid", &self.sid)
            .finish_non_exhaustive()
    }
}

impl Txn {
    /// Returns whether its statements that succeeded changed a relation of the file.
    pub fn wrote(&self) -> bool {
        self.written.borrow().wrote()
    }

    /// Returns the transaction's id, `None` for a read-only one.
    pub fn xid(&self) -> Option<Xid> {
        self.xid
    }

    pub fn began(&self) -> Instant {
        self.began
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn snapshot(&self) -> &Snapshot {
        self.hold().snapshot()
    }

    fn writer(&self) -> Result<Xid, Error> {
        self.xid.ok_or_else(|| Broken::ReadOnly.into())
    }
}

/// The statements of one call: terms of `terms` over `schema`, each variable of `params` bound
/// to its value.
#[derive(Clone, Copy, Debug)]
pub struct Text<'a> {
    /// The database's schema, or one that extends it with the transaction's temp relations.
    pub schema: &'a Committed,
    pub terms: &'a Terms,
    pub items: &'a [Tm],
    pub params: &'a [(Var, Val)],
    /// The instant, in microseconds, every `generate clock` of the statements takes.
    pub clock: i64,
    /// The principal `caller` denotes, `None` where nothing may read it.
    pub caller: Option<&'a Val>,
}

/// Leave to run statements: that of a call the admission gate admitted, or the engine's own,
/// for no caller.
#[derive(Debug)]
pub struct Leave(());

impl Leave {
    /// Returns the leave of a call the admission gate admitted.
    pub(crate) fn admitted() -> &'static Leave {
        &Leave(())
    }

    /// Returns the leave of statements the engine runs itself, for no caller.
    pub(crate) fn system() -> &'static Leave {
        &Leave(())
    }
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
    /// Fail with `Blocked` at a wait for another transaction, instead of holding the thread.
    pub suspend: bool,
    /// The first item that holds its worker at a wait although `suspend` is set.
    pub polls_from: Option<usize>,
    /// The `rerun` of the `Blocked` failure of an earlier run of the same statements.
    pub rerun: Option<Rerun>,
    /// The grant every execution of the statements takes its memory from, else one admitted
    /// for the run.
    pub grant: Option<Arc<exec::Grant>>,
    /// Read files only within the file roots.
    pub confined: bool,
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

#[cfg(test)]
impl Rerun {
    pub(crate) fn at_start() -> Rerun {
        Rerun {
            through: 0,
            since: Instant::from_nanos(0),
        }
    }
}

impl Rerun {
    /// Returns when a statement limit of `limit` runs out for the statement that suspended, if
    /// no rerun time is added to it.
    pub fn limit_at(&self, limit: Duration) -> Instant {
        self.since + limit
    }
}

/// What one vacuum of a relation did.
#[derive(Debug)]
pub struct Vacuumed {
    /// What it did to the heap of each constructor, by constructor id.
    pub ctors: Vec<(CtorId, vacuum::Freed)>,
    pub report: vacuum::Report,
}

/// What stops one statement's executions early.
#[derive(Clone, Debug, Default)]
pub struct Stop {
    /// Set to stop them with `Cancelled`.
    pub cancel: Arc<AtomicBool>,
    pub deadline: Option<exec::Deadline>,
    /// Fail with `Blocked`, carrying this, at a wait for another transaction, instead of
    /// holding the thread.
    pub suspend: Option<Rerun>,
    /// Read files only within the file roots.
    pub confined: bool,
}

impl Stop {
    /// Fail with what stops the statement at `now`, if it is stopped.
    pub(crate) fn check(&self, now: Instant) -> Result<(), exec::Error> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(exec::Error::Cancelled);
        }
        match &self.deadline {
            Some(d) if now >= d.at => Err(d.error.clone()),
            _ => Ok(()),
        }
    }

    /// Returns what stops a statement under `bounds`, its statement limit counted from `since`
    /// and unlimited without it, failing with `Blocked` carrying `suspend` at a wait.
    pub(crate) fn of(bounds: &Bounds, since: Option<Instant>, suspend: Option<Rerun>) -> Stop {
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
            confined: bounds.confined,
        }
    }
}

/// The relation variables of a schema over their heaps and indexes, and the transactions that
/// write them.
///
/// The session, the pager and the heaps lock themselves and take no lock of the database.
pub struct Database<R: Runtime> {
    rt: R,
    schema: Arc<Committed>,
    normal: Arc<NormalProgram>,
    /// The normal program's arena with every `Fresh` replaced by its placeholder.
    program: ValidTerms,
    program_fresh: fresh::Occurrences,
    /// Each `on delete retag` new value over the referrer's row, in `program`.
    retags: BTreeMap<RelId, (Var, Tm)>,
    catalog: Arc<Catalog>,
    pager: Pager<R>,
    session: Arc<Session<R>>,
    bindings: exec::Bindings,
    latches: BTreeMap<IndexId, IndexLatch>,
    rels: Rels,
    /// Every structure of the file, each with the holder of its pages.
    structures: Structures,
    denials: Vec<check::Denial>,
    sequences: Ranked<Sequences>,
    uuids: AtomicU64,
    budget: Arc<Budget>,
    pool: Option<Arc<Scheduler<R>>>,
    options: exec::Options,
    grant: usize,
    files: Disk,
    vacuums: vacuums::Vacuums,
    pacer: Arc<vacuum::Pacer>,
    /// How many pauses calls have made at a rebuild's gate or for the vacuum lock.
    rebuild_waits: AtomicU64,
    charged: grant::Charged,
    /// How many times a call asked a background vacuum to give up the vacuum lock.
    vacuum_yields: AtomicU64,
    /// The relations an offline rebuild holds writes to, with the rebuild's thread.
    offline: Lock<BTreeMap<RelId, ThreadId>>,
    /// The holder's thread each thread waiting for the vacuum lock or a rebuild's gate waits on.
    waiting: Lock<BTreeMap<ThreadId, ThreadId>>,
    /// Set once the engine closes: rebuilds stop and their gates refuse.
    rebuilds_stopped: AtomicBool,
    /// The structures history is kept in, once [`Database::with_history`] gives them.
    retained: Option<Arc<Retained>>,
    hosts: Arc<exec::Hosts>,
    /// The rows the engine holds for each volatile relation.
    volatile: Lock<BTreeMap<RelId, BTreeSet<Val>>>,
}

impl<R: Runtime> std::fmt::Debug for Database<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database")
            .field("heaps", &self.bindings.heaps().keys().collect::<Vec<_>>())
            .field(
                "indexes",
                &self.bindings.indexes().keys().collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// What a database runs its statements with.
#[derive(Debug)]
pub struct Setup<R: Runtime> {
    pub schema: Arc<Committed>,
    pub normal: Arc<NormalProgram>,
    pub catalog: Arc<Catalog>,
    pub pager: Pager<R>,
    pub txns: Arc<Transactions<R>>,
    pub bindings: exec::Bindings,
    pub budget: Arc<Budget>,
    /// The memory each statement's queries are granted.
    pub grant: usize,
    pub pool: Option<Arc<Scheduler<R>>>,
    pub options: exec::Options,
    pub limits: Limits,
    /// Every structure of the file, the page of the database's generated counters among them.
    pub structures: Structures,
}

impl<R: Runtime> Database<R> {
    /// Returns the database of `setup`, whose catalog's schema declares the indexes `declared`, by
    /// index id.
    ///
    /// Preconditions: `setup.bindings` holds a heap for every constructor and every index the
    /// catalog names, and the catalog's key index of each relation is unique.
    pub fn new(
        rt: R,
        setup: Setup<R>,
        declared: &BTreeMap<CatIndexId, IndexSpec>,
    ) -> Result<Database<R>, Error> {
        let mut program = setup.normal.terms().clone();
        let mut program_fresh = fresh::Occurrences::new();
        let retags: BTreeMap<RelId, (Var, Tm)> = setup
            .normal
            .retags()
            .iter()
            .map(|(r, (var, body))| {
                let body = fresh::rewrite(
                    &setup.schema,
                    &mut program,
                    *body,
                    &[*var],
                    &mut program_fresh,
                );
                (*r, (*var, body))
            })
            .collect();
        let denials = check::denials(&setup.schema, &setup.normal, &setup.catalog, &mut program)?;
        let roots = setup
            .normal
            .roots()
            .into_iter()
            .chain(retags.values().map(|(var, body)| (*body, vec![*var])));
        let program = ValidTerms::new(&setup.schema, program, &roots.collect::<Vec<_>>())
            .map_err(Broken::IllTyped)?;
        let rels = schema::rels(&setup.schema, &setup.normal, &setup.catalog, declared)?;
        let sequences = match setup.structures.counters() {
            None => Sequences::none(),
            Some(page) => Sequences::open(&setup.pager, page)?,
        };
        let latches = latches(&setup.bindings, None);
        let seed = rt.next_u64();
        let heaps = (rels.values())
            .map(|r| {
                let heaps = (r.ctors.iter())
                    .map(|c| (c.heap, setup.bindings.heaps()[&c.heap].heap.id().0))
                    .collect();
                (r.id, heaps)
            })
            .collect();
        let session = Session::new(
            rt.clone(),
            setup.pager.clone(),
            setup.txns,
            setup.limits,
            heaps,
        );
        Ok(Database {
            schema: setup.schema,
            normal: setup.normal,
            program,
            program_fresh,
            retags,
            catalog: setup.catalog,
            pager: setup.pager,
            session,
            bindings: setup.bindings,
            latches,
            rels,
            structures: setup.structures,
            denials,
            sequences: Ranked::new(ranks::SEQUENCES, sequences),
            uuids: AtomicU64::new(seed >> 2),
            budget: setup.budget,
            pool: setup.pool,
            options: setup.options,
            grant: setup.grant,
            files: Disk::default(),
            vacuums: vacuums::Vacuums::default(),
            pacer: Arc::new(vacuum::Pacer::new(0.1)),
            rebuild_waits: AtomicU64::new(0),
            charged: grant::Charged::default(),
            vacuum_yields: AtomicU64::new(0),
            offline: Lock::default(),
            waiting: Lock::default(),
            rebuilds_stopped: AtomicBool::new(false),
            retained: None,
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
        self.latches = latches(&self.bindings, Some(&retained));
        self.retained = Some(Arc::new(retained));
        self.hold_vacuum(HoldWait::unbounded())?
            .set_retention(window)?;
        Ok(self)
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
        &self.session.txns
    }

    pub(crate) fn structures(&self) -> &Structures {
        &self.structures
    }

    /// Returns the planner's description of the relations the database runs.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn planned(&self) -> &Catalog {
        &self.catalog
    }

    /// Returns the normal program of the schema the database runs.
    pub fn normal(&self) -> &NormalProgram {
        &self.normal
    }

    /// Returns the files file scans read and exports write.
    pub fn files(&self) -> &Disk {
        &self.files
    }

    /// Returns the database reading the files of file scans through `files`.
    pub fn with_files(mut self, files: Disk) -> Self {
        self.files = files;
        self
    }

    #[cfg(test)]
    pub fn pager(&self) -> &Pager<R> {
        &self.pager
    }

    /// Begin a read-write transaction. Fail if the database is quarantined.
    pub fn begin(&self) -> Result<Txn, Error> {
        self.admit_database()?;
        self.session.begin()
    }

    /// Begin a read-only transaction. A `deferrable` one waits until its snapshot is safe,
    /// taking a new snapshot each time one is found unsafe, and then never fails with a
    /// serialization failure.
    pub fn begin_read(&self, deferrable: bool) -> Result<Txn, Error> {
        if deferrable {
            outside_writer()?;
        }
        let mut tx = self.session.begin_reader(deferrable)?;
        if !deferrable {
            return Ok(tx);
        }
        loop {
            match self.defer(tx)? {
                Deferral::Safe(t) => return Ok(t),
                Deferral::Wait { tx: t, on } => {
                    self.session.waits().wait(None, on, None)?;
                    tx = t;
                }
            }
        }
    }

    /// Begin a deferrable read-only transaction, its snapshot not yet known to be safe.
    pub fn begin_deferrable(&self) -> Result<Txn, Error> {
        self.session.begin_reader(true)
    }

    /// Decide the snapshot of deferrable transaction `tx`. Returns it once its snapshot is
    /// safe, or with the writer its snapshot waits for. Aborts it when its snapshot is unsafe
    /// and decides a new one in its place.
    pub fn defer(&self, mut tx: Txn) -> Result<Deferral, Error> {
        loop {
            let safety = self.session.ser.lock(&self.rt).safety(tx.sid);
            match safety {
                Safety::Safe => return Ok(Deferral::Safe(tx)),
                Safety::Pending(writers) => {
                    return Ok(Deferral::Wait { tx, on: writers[0] });
                }
                Safety::Unsafe => {
                    self.abort(tx)?;
                    tx = self.session.begin_reader(true)?;
                }
            }
        }
    }

    /// Returns the relation and constructor position whose heap, or whose history heap, is
    /// numbered `heap`, `None` when no relation of the database has it.
    pub fn relation_of(&self, heap: u32) -> Option<(RelId, u32)> {
        let history = self.retained.as_ref().and_then(|r| {
            r.heaps
                .iter()
                .find(|(_, h)| h.heap.id().0 == heap)
                .map(|(current, _)| *current)
        });
        let id = history.unwrap_or(HeapId(u64::from(heap)));
        self.rels.iter().find_map(|(r, rel)| {
            let c = rel.ctors.iter().position(|c| c.heap == id)?;
            Some((*r, u32::try_from(c).ok()?))
        })
    }

    /// Run term `t` of `terms`, a `Transact`, a write or a query, as one statement of `tx`, each
    /// variable of `params` bound to its value. `clock` is the instant, in microseconds, every
    /// `generate clock` of the statement takes.
    ///
    /// On an `Err` that [ends the transaction](Failed::ends), abort `tx`. On any other
    /// `Err` the statement has no effect and `tx` goes on.
    #[cfg(test)]
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
        let local = &mut Local::default();
        match self.statements(Leave::system(), tx, text, local, Bounds::default()) {
            Ok(mut done) => Ok(done.remove(0)),
            Err(f) => Err(f.error),
        }
    }

    /// Run the items of `text` in order as statements of `tx`, each reading the change sets of
    /// the ones before it and bounded by `bounds`, as [`statement`](Database::statement) runs
    /// one, over `local`, what `tx` holds outside the file. Returns each one's outcome.
    ///
    /// On an `Err` that [ends the transaction](Failed::ends), abort `tx`. On any other
    /// `Err` none of the items has an effect on the file or on the temp relations of `local`, and
    /// `tx` goes on. Every file an item read stays pinned in `local` either way.
    pub fn statements(
        &self,
        leave: &Leave,
        tx: &Txn,
        text: Text<'_>,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<Vec<Outcome>, Failed> {
        self.statements_with(leave, tx, text, local, bounds, &mut |_, _| Ok(()))
    }

    /// Run the items of `text` as [`statements`](Database::statements) does, running `between`
    /// before the first and after each. A failure of `between` fails the call as a failure of
    /// the next item does.
    pub fn statements_with(
        &self,
        _leave: &Leave,
        tx: &Txn,
        text: Text<'_>,
        local: &mut Local,
        bounds: Bounds,
        between: &mut Between<'_>,
    ) -> Result<Vec<Outcome>, Failed> {
        match self.run_items(tx, text, local, bounds, between) {
            Err(
                f @ Failed {
                    error: Error::Blocked { .. },
                    ..
                },
            ) => Err(f),
            ran => tx.noted(ran),
        }
    }

    fn run_items(
        &self,
        tx: &Txn,
        text: Text<'_>,
        local: &mut Local,
        bounds: Bounds,
        between: &mut Between<'_>,
    ) -> Result<Vec<Outcome>, Failed> {
        let _writing = tx.xid.map(|_| Writing::enter());
        self.admit(&text).map_err(|error| Failed::new(0, error))?;
        let (bounds, _peak) = self
            .granted(tx, bounds)
            .map_err(|error| Failed::new(0, error))?;
        let mut ran = Vec::with_capacity(text.items.len());
        let mut out = Vec::with_capacity(text.items.len());
        let before = local.clone();
        let started = self.rt.now();
        for item in 0..=text.items.len() {
            let done = between(item, local).and_then(|()| match text.items.get(item) {
                Some(t) => {
                    let s = self.stmt(tx, text, *t, item, &bounds, started)?;
                    let o = s.run(text.params, local);
                    ran.push(s);
                    o.map(|o| out.push(o))
                }
                None => Ok(()),
            });
            if let Err(error) = done {
                local.restore_temps(before);
                return Err(self.taken_back(tx, &ran, Failed::new(item, error)));
            }
        }
        for o in &out {
            self.record_written(&mut tx.written.borrow_mut(), o);
            self.record_changed(&mut tx.held.changed.borrow_mut(), o);
        }
        Ok(out)
    }

    /// Run the items of `text` as [`statements`](Database::statements) does, then undo every
    /// one, leaving `tx` and the temp relations of `local` as they were and every file an item
    /// read pinned. Returns each one's outcome. A failure to undo them ends the transaction.
    pub fn trial(
        &self,
        tx: &Txn,
        text: Text<'_>,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<Vec<Outcome>, Failed> {
        tx.noted(self.try_items(tx, text, local, bounds))
    }

    fn try_items(
        &self,
        tx: &Txn,
        text: Text<'_>,
        local: &mut Local,
        bounds: Bounds,
    ) -> Result<Vec<Outcome>, Failed> {
        self.admit(&text).map_err(|error| Failed::new(0, error))?;
        let (bounds, _peak) = self
            .granted(tx, bounds)
            .map_err(|error| Failed::new(0, error))?;
        let before = local.clone();
        let mut ran = Vec::with_capacity(text.items.len());
        let mut out = Vec::with_capacity(text.items.len());
        let started = self.rt.now();
        for (item, t) in text.items.iter().enumerate() {
            let o = self
                .stmt(tx, text, *t, item, &bounds, started)
                .and_then(|s| {
                    let o = s.run(text.params, local);
                    ran.push(s);
                    o
                });
            match o {
                Ok(o) => out.push(o),
                Err(error) => {
                    local.restore_temps(before);
                    return Err(self.taken_back(tx, &ran, Failed::new(item, error)));
                }
            }
        }
        local.restore_temps(before);
        let item = text.items.len();
        self.take_back(tx, &ran, false)
            .map_err(|error| Failed::undone(item, error))?;
        Ok(out)
    }

    /// Returns the operators of term `t` of `text` as a statement of `tx` would run it.
    pub fn explain(&self, tx: &Txn, text: Text<'_>, t: Tm) -> Result<Node, Error> {
        let explained = self
            .stmt(tx, text, t, 0, &Bounds::default(), self.rt.now())?
            .explain();
        tx.noted(explained)
    }

    /// Vacuum the constructor relations of `rel` together, within the budget of
    /// [`Database::pacer`]. Returns the id of each constructor with what the vacuum did to its
    /// heap, and what it did in all.
    ///
    /// Stop as [`Database::hold_vacuum`] says while another holds off vacuum, and as the same
    /// `waiting`, timed from the call, says while the vacuum runs.
    pub fn vacuum(&self, rel: RelId, waiting: HoldWait<'_>) -> Result<Vacuumed, Error> {
        let stop = waiting.stop(self.rt.now());
        let one = self.hold_vacuum_until(&stop)?;
        one.vacuum(rel, &stop)
    }

    /// Vacuum `rel` as the background worker, if no one holds the vacuum lock, stopping with
    /// `Cancelled` once another caller waits for the lock. Returns `None` if one holds it.
    pub fn vacuum_background(&self, rel: RelId) -> Result<Option<Vacuumed>, Error> {
        let stop = Stop::default();
        let Some(one) = self.try_hold_yielding(stop.cancel.clone()) else {
            return Ok(None);
        };
        one.vacuum(rel, &stop).map(Some)
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
    ) -> Result<eval::Stmt<'a, R>, Error> {
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
            (bounds.suspend && bounds.polls_from.is_none_or(|from| item < from)).then_some(rerun),
        );
        let grant = self.grant_of(tx, bounds)?;
        eval::Stmt::new(self, tx, text, t, options, (stop, grant))
    }

    /// Take back the statements of `ran` when the class of `failed` takes them back. Returns
    /// `failed`, or the failure of taking them back.
    fn taken_back(&self, tx: &Txn, ran: &[eval::Stmt<'_, R>], failed: Failed) -> Failed {
        match self.take_back(tx, ran, !failed.error.class().takes_back()) {
            Ok(()) => failed,
            Err(error) => Failed::undone(failed.item, error),
        }
    }

    /// Count each version `ran` inserted as dead in `tx`, and undo the effects of `ran` on the
    /// file, the last first, unless the transaction `ends`.
    fn take_back(&self, tx: &Txn, ran: &[eval::Stmt<'_, R>], ends: bool) -> Result<(), Error> {
        let mut changed = tx.held.changed.borrow_mut();
        for s in ran {
            for u in s.undo.borrow().iter() {
                if let (heap, crate::heap::Undo::Kill(..)) = u {
                    changed.entry(*heap).or_default().undone += 1;
                }
            }
        }
        if ends {
            return Ok(());
        }
        ran.iter().rev().try_for_each(eval::Stmt::undo)
    }

    /// Returns every row of every relation variable the catalog describes, as `tx` reads them.
    #[cfg(test)]
    pub fn rows(&self, tx: &Txn) -> Result<Rows, Error> {
        let grant =
            self.admit_grant(exec::Priority::AdHoc, (tx.began(), None), &Stop::default())?;
        let s = eval::Stmt::reader(self, tx, grant);
        let mut out = Rows::new();
        for rel in self.rels.keys() {
            for (c, v) in s.rows(*rel, None)? {
                out.entry((*rel, c)).or_default().insert(v);
            }
        }
        Ok(out)
    }

    /// Returns the rows of `rel` visible to `tx`, read under a grant of no budget.
    pub(crate) fn relation_rows(&self, tx: &Txn, rel: RelId) -> Result<Vec<Val>, Error> {
        let grant = Budget::new(usize::MAX).admit(usize::MAX);
        let grant = grant.expect("a budget admits its whole limit");
        let rows = eval::Stmt::reader(self, tx, grant).rows(rel, None);
        tx.noted(rows.map(|rows| rows.into_iter().map(|(_, row)| row).collect()))
    }

    /// Returns the deployment facts of `d` that `tx` reads.
    pub(crate) fn deployed(
        &self,
        tx: &Txn,
        d: &crate::catalog::Deployment,
    ) -> Result<crate::catalog::Deployed, crate::catalog::Error> {
        tx.noted(d.at(&self.pager, &self.session.txns, tx.hold()))
    }

    /// Returns the position of the latest commit `tx` reads, else of the log's durable end.
    pub(crate) fn read_position(&self, tx: &Txn) -> Position {
        let upto = tx.taken.upto();
        self.pager
            .position(upto.unwrap_or_else(|| self.pager.durable()))
    }

    /// Returns the value `rel`'s sequence draws next.
    pub(crate) fn sequence_next(&self, rel: RelId) -> i64 {
        self.sequences.lock(&self.rt).peek(rel.0)
    }

    /// Make `rel`'s sequence draw no value below `next`, durably.
    pub(crate) fn resume_sequence(&self, rel: RelId, next: i64) -> Result<(), Error> {
        self.advance_sequence(rel, next.saturating_sub(1))
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
    /// Fail with `Quarantined` if a mark set before its commit record stops what `tx` wrote, by
    /// the rule that stops a statement. On any failure, abandon `tx` as [`Database::abandon`]
    /// does: fail with the commit's failure, or with the abort record's when it fails.
    pub fn commit(&self, tx: Txn) -> Result<Position, Error> {
        let (sid, upto) = (tx.sid, tx.taken.upto());
        let written = tx.written.take();
        let mut held = tx.end();
        let begun = held.begun.take();
        let (serial, reader, hold) = (&mut held.serial, &mut held.reader, &mut held.hold);
        let committed = self.session.txns.commit_with(
            &self.pager,
            begun,
            upto,
            |ser| {
                self.map_committing(ser.ordered());
                self.admit_commit(ser, written)?;
                ser.precommit(sid).map_err(Error::from)
            },
            |ser| {
                if let Some(sid) = serial.take() {
                    ser.committed(sid);
                }
                self.session.log_safe(ser);
                if let Some(sid) = reader.take() {
                    self.session.readers.lock(&self.rt).remove(&sid);
                }
                drop(hold.take());
            },
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

    /// Returns how many transactions the serializer tracks.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn tracked(&self) -> usize {
        self.session.ser.lock(&self.rt).tracked()
    }

    /// Returns the SIREAD locks `tx` holds.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn locks(&self, tx: &Txn) -> Vec<crate::mvcc::Lock> {
        self.session.ser.lock(&self.rt).held_by(tx.sid)
    }

    /// Returns how many read-only transactions keep a snapshot for their safe snapshot marker.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn readers_held(&self) -> usize {
        self.session.readers.lock(&self.rt).len()
    }

    /// Returns the transactions a waiter may wait on, started and not yet ended.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn waits_running(&self) -> BTreeSet<Xid> {
        self.session.waits().running()
    }

    /// Returns the transactions some waiter waits on to end.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn waited_on(&self) -> BTreeSet<Xid> {
        self.session.waits().watched()
    }

    /// Returns how many transactions have ended by being dropped, none of their operations
    /// having failed, since the count was last taken, and starts the count again.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn take_ends_by_drop(&self) -> usize {
        self.session.dropped.swap(0, Ordering::Relaxed)
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn set_step_hook(&self, hook: crate::steps::StepHook) {
        self.session.steps.set(hook);
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

    /// Returns the page of the database's generated counters, for [`Setup::sequences`] when
    /// it is opened again.
    #[cfg(any(test, feature = "testing"))]
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

    /// Returns the store the statements of `tx` read.
    pub(crate) fn store(&self, tx: &Txn) -> Arc<exec::Store<R>> {
        let upto = tx.taken.upto().unwrap_or_else(|| self.pager.durable());
        Arc::new(exec::Store::serializable(
            self.session.txns.reader(&self.pager, tx.hold(), tx.xid),
            (self.session.ser.clone(), tx.sid),
            self.session.steps.clone(),
            self.bindings.clone(),
            self.pasts(self.pager.position(upto)),
        ))
    }

    fn call(
        &self,
        store: &Arc<exec::Store<R>>,
        options: exec::Options,
        stop: &Stop,
        grant: &Arc<exec::Grant>,
    ) -> Result<exec::Call<R>, Error> {
        let call = exec::Call::new(
            store.clone(),
            grant.clone(),
            options,
            stop.cancel.clone(),
            stop.deadline.clone(),
        );
        Ok(call.on(self.pool.clone()))
    }
}

/// Returns the value either arm of `r` holds.
pub fn either<E>(r: Result<E, E>) -> E {
    r.unwrap_or_else(|e| e)
}

impl<R: Runtime> VacuumHold<'_, R> {
    /// Vacuum the constructor relations of `rel` together under this lock, stopping as `stop`
    /// says. Returns what [`Database::vacuum`] returns.
    fn vacuum(&self, rel: RelId, stop: &Stop) -> Result<Vacuumed, Error> {
        let db = self.db();
        let r = (db.rels.get(&rel))
            .unwrap_or_else(|| panic!("a relation a vacuum names has heaps: {rel:?} has none"));
        db.admit_vacuum(r)?;
        let horizon = db.horizon(r);
        let started = db.rt.now();
        let ids: BTreeMap<u32, CtorId> = db.bindings.indexes()[&r.key_index]
            .ctors
            .iter()
            .map(|(id, c)| (*c, *id))
            .collect();
        let sum = vacuum::Sum {
            heaps: (r.ctors.iter().enumerate())
                .map(|(c, ctor)| (ids[&(c as u32)], &*db.bindings.heaps()[&ctor.heap].heap))
                .collect(),
            indexes: (r.indexes.iter())
                .map(|(i, _)| &db.bindings.indexes()[i].index)
                .collect(),
        };
        let shown: Vec<(CtorId, u32)> = sum.heaps.iter().map(|(id, h)| (*id, h.id().0)).collect();
        let _shown = self.show_vacuum(held_id(db.schema.ids().relation_id(rel)).raw(), &shown);
        let mut began = db.rt.now();
        let mut io = None;
        let halted = Cell::new(None);
        let ran = self.vacuum_sum(r, &sum, &mut || {
            let paced = db.pace(stop, &mut began, &mut io);
            paced.map_err(|e| halted.set(Some(e))).is_ok()
        });
        db.pacer.charge(&db.rt, began);
        let (report, kept) = ran.map_err(|e| match e {
            vacuum::Error::Stopped => halted.take().unwrap_or(Error::Exec(exec::Error::Cancelled)),
            e => e.into(),
        })?;
        let freed: BTreeMap<HeapId, u64> = (r.ctors.iter())
            .map(|c| {
                let heap = db.bindings.heaps()[&c.heap].heap.id().0;
                let freed = report.heaps.get(&heap).map_or(0, |f| f.freed);
                (c.heap, freed as u64)
            })
            .collect();
        let pages = (report.scanned - report.removed) as u64;
        db.vacuumed(rel, started, horizon, &freed, pages, kept);
        let ctors = (shown.iter())
            .map(|(id, heap)| (*id, report.heaps.get(heap).copied().unwrap_or_default()))
            .collect();
        Ok(Vacuumed { ctors, report })
    }
}

/// Returns the latch of each index of `bindings`, over its tree and its history tree in
/// `retained`.
fn latches(
    bindings: &exec::Bindings,
    retained: Option<&Retained>,
) -> BTreeMap<IndexId, IndexLatch> {
    (bindings.indexes().iter())
        .map(|(id, b)| {
            let history = retained.and_then(|r| Some(r.indexes.get(id)?.tree()));
            (
                *id,
                IndexLatch::new(IndexTrees::new(b.index.tree(), history)),
            )
        })
        .collect()
}
