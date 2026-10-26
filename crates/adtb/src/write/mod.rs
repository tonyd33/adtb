mod apply;
mod check;
mod eval;
mod fresh;
mod schema;

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
    Access, Limits, Scope, SerialId, SerializationFailure, Serializer, Snapshot, SnapshotId,
    Transactions, Waits, Xid,
};
use crate::plan::{Catalog, HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager};

pub use adtb_lang::schema::Stage;

use schema::Rels;

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
}

/// Why a statement or a commit failed. Every failure of a statement leaves its transaction to be
/// aborted.
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
            exec::Error::NoCaller => Error::Unsupported("`caller` in a statement".into()),
            exec::Error::Cancelled => Error::Unsupported("a cancelled statement".into()),
        }
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

/// A read-write transaction.
#[derive(Debug)]
pub struct Txn {
    xid: Xid,
    sid: SerialId,
    snapshot: Snapshot,
    hold: SnapshotId,
}

impl Txn {
    pub fn xid(&self) -> Xid {
        self.xid
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
/// Lock order: one index latch, then the serializer, then the sequences. `Transactions`, the
/// pager and the heaps lock themselves and take none of these.
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
    pager: Arc<Pager<R>>,
    txns: Arc<Transactions<R>>,
    ser: Arc<Lock<Serializer>>,
    waits: Waits<R>,
    heaps: BTreeMap<HeapId, StoredHeap>,
    indexes: BTreeMap<IndexId, StoredIndex>,
    latches: BTreeMap<IndexId, Lock<()>>,
    rels: Rels,
    denials: Vec<check::Denial>,
    /// The next value of each relation's `sequence`, once read.
    sequences: Lock<BTreeMap<RelId, i64>>,
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
#[allow(missing_debug_implementations)]
pub struct Setup<R: Runtime> {
    pub schema: Arc<Schema>,
    pub normal: Arc<NormalProgram>,
    pub catalog: Arc<Catalog>,
    pub pager: Arc<Pager<R>>,
    pub txns: Arc<Transactions<R>>,
    pub bindings: Bindings,
    pub budget: Arc<Budget>,
    /// The memory each statement's queries are granted.
    pub grant: usize,
    pub pool: Option<Arc<Scheduler<R>>>,
    pub options: exec::Options,
    pub limits: Limits,
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
        let denials = check::denials(&setup.schema, &setup.normal, &mut program);
        let rels = schema::rels(&setup.schema, &setup.normal, &setup.catalog)?;
        let mut sequences = BTreeMap::new();
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
            sequences.insert(r.id, held + 1);
        }
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
            uuids: AtomicU64::new(seed >> 2),
            budget: setup.budget,
            pool: setup.pool,
            options: setup.options,
            grant: setup.grant,
            rt,
        })
    }

    pub fn pager(&self) -> &Arc<Pager<R>> {
        &self.pager
    }

    pub fn begin(&self) -> Result<Txn, Error> {
        let xid = self.txns.begin(&self.pager)?;
        let (hold, snapshot, sid) = {
            let mut ser = self.ser.lock(&self.rt);
            let (hold, snapshot) = self.txns.take(Scope::Every)?;
            (hold, snapshot, ser.begin(Access::ReadWrite(xid)))
        };
        self.waits.started(xid);
        Ok(Txn {
            xid,
            sid,
            snapshot,
            hold,
        })
    }

    /// Run term `t` of `terms`, a `Transact`, a write or a query, as one statement of `tx`, each
    /// variable of `params` bound to its value. `clock` is the instant, in microseconds, every
    /// `generate clock` of the statement takes.
    ///
    /// On `Err`, the statement's writes may be partly in its heaps; abort `tx`.
    pub fn statement(
        &self,
        tx: &Txn,
        terms: &Terms,
        t: Tm,
        params: &[(Var, Val)],
        clock: i64,
    ) -> Result<Outcome, Error> {
        let s = eval::Stmt::new(self, tx, terms, t, clock);
        s.run(params)
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

    /// Commit `tx`. Returns once the log is durable past its commit and every commit it read.
    pub fn commit(&self, tx: Txn) -> Result<(), Error> {
        let done = {
            let mut ser = self.ser.lock(&self.rt);
            match ser.precommit(tx.sid) {
                Err(f) => Err(Error::Serialization(f)),
                Ok(()) => match self.txns.commit(&self.pager, tx.xid) {
                    Err(e) => Err(e.into()),
                    Ok(own) => {
                        ser.committed(tx.sid);
                        let read = self.txns.read_upto(tx.hold);
                        self.txns.release(tx.hold);
                        Ok(read.map_or(own, |r| r.max(own)))
                    }
                },
            }
        };
        match done {
            Ok(lsn) => {
                self.pager.wait_durable(lsn);
                self.waits.ended(tx.xid);
                Ok(())
            }
            Err(e) => {
                self.abort(tx);
                Err(e)
            }
        }
    }

    pub fn abort(&self, tx: Txn) {
        {
            let mut ser = self.ser.lock(&self.rt);
            let _ = self.txns.abort(&self.pager, tx.xid);
            ser.aborted(tx.sid);
            self.txns.release(tx.hold);
        }
        self.waits.ended(tx.xid);
    }

    /// Returns the next value of `rel`'s sequence.
    fn next_sequence(&self, rel: RelId) -> i64 {
        let mut s = self.sequences.lock(&self.rt);
        let n = s.entry(rel).or_insert(1);
        let v = *n;
        *n += 1;
        v
    }

    /// Advance `rel`'s sequence past `held`, a key a row of it holds.
    fn advance_sequence(&self, rel: RelId, held: i64) {
        if let Some(n) = self.sequences.lock(&self.rt).get_mut(&rel) {
            *n = (*n).max(held.saturating_add(1));
        }
    }

    fn store(&self, tx: &Txn) -> Arc<exec::Store<R>> {
        let reads = exec::AtSnapshot {
            rt: self.rt.clone(),
            pager: self.pager.clone(),
            txns: self.txns.clone(),
            snapshot: tx.snapshot.clone(),
            reader: Some(tx.xid),
            serial: Some((self.ser.clone(), tx.sid)),
        };
        Arc::new(exec::Store {
            pager: self.pager.clone(),
            reads: Arc::new(reads),
            heaps: self.heaps.clone(),
            indexes: self.indexes.clone(),
        })
    }

    fn call(&self, store: &Arc<exec::Store<R>>) -> Result<exec::Call<R>, Error> {
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
            options: self.options,
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }
}
