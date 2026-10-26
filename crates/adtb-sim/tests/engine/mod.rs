#![allow(dead_code)]

use std::convert::Infallible;
use std::ops::Bound;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::btree::{BTree, Index, prefix_end};
use adtb::heap::{FieldType, Heap, LargeStore, walk};
use adtb::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use adtb::mvcc::{
    Access, Limits, Probe, Safety, Scope, SerialId, SerializationFailure, Serializer, Snapshot,
    SnapshotId, Status, Transactions, Waits, Write, Xid, check_write, probe_unique, read_before,
};
use adtb::runtime::{Clock, Io, Op, Runtime, ThreadId, Threads};
use adtb::storage::{Options, Pager, Pid};
use adtb::verify::{Inventory, Sum};
use adtb_isolation::{Aborted, Database};
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};
use adtb_sys::lock::Lock;

pub const PAGE: usize = 4096;
const ELEMS: u32 = 10;
const COUNTERS: u32 = 11;
const LARGE: u32 = 9;
const ELEM: CtorId = CtorId(1);
const COUNTER: CtorId = CtorId(2);

pub fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames: 64,
        atomic_page_writes: false,
        ..Options::new("ssi.adt")
    }
}

fn i64s(n: usize) -> Vec<FieldType> {
    vec![FieldType::Scalar(Scalar::I64); n]
}

fn schema(n: usize) -> KeySchema {
    KeySchema::new(vec![(KeyType::Scalar(Scalar::I64), Dir::Asc); n])
}

fn int(v: &Value) -> i64 {
    match v {
        Value::I64(i) => *i,
        v => panic!("{v:?} is not an I64"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Serialization(SerializationFailure),
    KeyViolation,
}

impl From<SerializationFailure> for Failure {
    fn from(f: SerializationFailure) -> Failure {
        Failure::Serialization(f)
    }
}

impl Failure {
    pub fn is_serialization(&self) -> bool {
        matches!(self, Failure::Serialization(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    ReadWrite,
    ReadOnly,
    Deferrable,
}

pub struct Tx {
    xid: Option<Xid>,
    sid: SerialId,
    snapshot: Snapshot,
    hold: SnapshotId,
}

/// The root pages of the workload's structures, which a recovered process reopens.
#[derive(Clone, Copy, Debug)]
pub struct Roots {
    large: Pid,
    elems: Pid,
    counters: Pid,
    by_pos: Pid,
    by_elem: Pid,
    by_id: Pid,
    clog: Pid,
}

/// The transactions layer over the workload `elems { id, k, pos, e }` with `unique (k, pos)`
/// and `unique (k, e)`, and `counters { id, n }` keyed by `id`.
///
/// Each index, heap and the serializer has its own lock, taken in that order; `Transactions`
/// locks itself and takes no other. Taking a snapshot and recording a commit both run under the
/// serializer's lock. No lock is held across a wait.
pub struct Engine {
    pub rt: SimRuntime,
    pager: Pager<SimRuntime>,
    roots: Roots,
    by_pos: Lock<Index>,
    by_elem: Lock<Index>,
    by_id: Lock<Index>,
    elems: Lock<Heap>,
    counters: Lock<Heap>,
    txns: Transactions<SimRuntime>,
    ser: Lock<Serializer>,
    waits: Waits<SimRuntime>,
    next_id: AtomicI64,
    /// The interval of group commit, when a flusher forces the log and commits wait for it
    /// instead of forcing it themselves.
    group_commit: Option<Duration>,
    stop_flusher: AtomicBool,
}

fn index(root: Pid, owner: u32, columns: usize, ctor: CtorId) -> Index {
    Index::open(BTree::open(root, owner), schema(columns), [ctor].into())
}

impl Engine {
    pub fn create(rt: SimRuntime, limits: Limits) -> Engine {
        let pager = Pager::create(rt.clone(), &options()).unwrap();
        let large = LargeStore::create(&pager, LARGE).unwrap();
        let elems = Heap::create(&pager, ELEMS, i64s(4), large).unwrap().root();
        let counters = Heap::create(&pager, COUNTERS, i64s(2), large)
            .unwrap()
            .root();
        let tree = |owner| BTree::create(&pager, owner).unwrap().root();
        let (by_pos, by_elem, by_id) = (tree(ELEMS), tree(ELEMS), tree(COUNTERS));
        let clog = Transactions::create(rt.clone(), &pager)
            .unwrap()
            .commit_log()
            .root();
        pager.flush_log();
        let roots = Roots {
            large: large.root(),
            elems,
            counters,
            by_pos,
            by_elem,
            by_id,
            clog,
        };
        drop(pager);
        Engine::open(rt, roots, limits).expect("a created database opens")
    }

    /// Open the database at `roots` after recovery. Returns `None` if the pager cannot open
    /// the file.
    pub fn open(rt: SimRuntime, roots: Roots, limits: Limits) -> Option<Engine> {
        let (pager, _) = Pager::open(rt.clone(), &options()).ok()?;
        let large = LargeStore::open(LARGE, roots.large);
        let heap = |id, root, fields| Heap::open(&pager, id, root, i64s(fields), large).unwrap();
        let txns = Transactions::open(rt.clone(), &pager, roots.clog).unwrap();
        pager.flush_log();
        Some(Engine {
            by_pos: Lock::new(index(roots.by_pos, ELEMS, 2, ELEM)),
            by_elem: Lock::new(index(roots.by_elem, ELEMS, 2, ELEM)),
            by_id: Lock::new(index(roots.by_id, COUNTERS, 1, COUNTER)),
            elems: Lock::new(heap(ELEMS, roots.elems, 4)),
            counters: Lock::new(heap(COUNTERS, roots.counters, 2)),
            txns,
            group_commit: None,
            stop_flusher: AtomicBool::new(false),
            ser: Lock::new(Serializer::new(limits)),
            waits: Waits::new(rt.clone()),
            next_id: AtomicI64::new(1),
            pager,
            roots,
            rt,
        })
    }

    /// Force the log only from a flusher every `interval`, so a commit is visible for up to
    /// that long before it is durable. [`Engine::flusher`] must run while transactions commit.
    pub fn with_group_commit(mut self, interval: Duration) -> Engine {
        self.group_commit = Some(interval);
        self
    }

    /// Start the group commit flusher, if the engine has one, until [`Engine::stop`].
    pub fn flusher(self: &Arc<Engine>) -> Option<ThreadId> {
        let interval = self.group_commit?;
        let e = self.clone();
        Some(self.rt.spawn("flusher", move || {
            while !e.stop_flusher.load(Ordering::Relaxed) {
                e.sleep(interval);
                e.pager.flush_log();
            }
        }))
    }

    /// Stop the flusher and wait for it.
    pub fn stop(&self, flusher: Option<ThreadId>) {
        self.stop_flusher.store(true, Ordering::Relaxed);
        if let Some(t) = flusher {
            self.rt.join(t);
        }
    }

    fn sleep(&self, d: Duration) {
        let mut io = self.rt.io();
        io.submit(Op::Timer {
            at: self.rt.now() + d,
        });
        while io.poll().is_none() {
            self.rt.park();
        }
    }

    pub fn roots(&self) -> Roots {
        self.roots
    }

    /// Returns what the whole-file `verify` finds, freeing leaked pages first.
    ///
    /// Precondition: no transaction is running.
    pub fn verify(&self) -> Vec<String> {
        let by_pos = self.by_pos.lock(&self.rt);
        let by_elem = self.by_elem.lock(&self.rt);
        let by_id = self.by_id.lock(&self.rt);
        let elems = self.elems.lock(&self.rt);
        let counters = self.counters.lock(&self.rt);
        let clog = self.txns.commit_log();
        let inventory = Inventory {
            commit_log: Some(&clog),
            sums: vec![
                Sum {
                    heaps: vec![(ELEM, &*elems)],
                    indexes: vec![&*by_pos, &*by_elem],
                },
                Sum {
                    heaps: vec![(COUNTER, &*counters)],
                    indexes: vec![&*by_id],
                },
            ],
            ..Inventory::default()
        };
        crate::common::verify_after_crash(&self.pager, &inventory)
    }

    fn status(&self, x: Xid) -> Result<Status, Infallible> {
        Ok(self.txns.status(&self.pager, x).unwrap())
    }

    fn sees(&self, tx: &Tx, xmin: u64, xmax: u64) -> bool {
        let Ok(seen) = tx.snapshot.sees(tx.xid, xmin, xmax, |x| self.status(x));
        seen
    }

    fn depends(&self, tx: &Tx, xmin: u64, xmax: u64) -> Result<(), Failure> {
        let Ok(writer) = read_before(tx.xid, &tx.snapshot, xmin, xmax, |x| self.status(x));
        if let Some(w) = writer {
            self.ser.lock(&self.rt).read_from(tx.sid, w)?;
        }
        Ok(())
    }

    pub fn begin(&self, kind: Kind) -> Tx {
        loop {
            let tx = {
                let xid = match kind {
                    Kind::ReadWrite => Some(self.txns.begin(&self.pager).unwrap()),
                    Kind::ReadOnly | Kind::Deferrable => None,
                };
                let mut ser = self.ser.lock(&self.rt);
                let (hold, snapshot) = self.txns.take(Scope::Every);
                let sid = ser.begin(match (kind, xid) {
                    (_, Some(x)) => Access::ReadWrite(x),
                    (Kind::Deferrable, None) => Access::Deferrable,
                    (_, None) => Access::ReadOnly,
                });
                Tx {
                    xid,
                    sid,
                    snapshot,
                    hold,
                }
            };
            if let Some(x) = tx.xid {
                self.waits.started(x);
            }
            if kind != Kind::Deferrable {
                return tx;
            }
            loop {
                let safety = self.ser.lock(&self.rt).safety(tx.sid);
                match safety {
                    Safety::Safe => return tx,
                    Safety::Unsafe => break,
                    Safety::Pending(xs) => self.waits.wait(None, xs[0]).unwrap(),
                }
            }
            self.abort(tx);
        }
    }

    /// Returns the elements of `k` at `tx`'s snapshot, ordered by position.
    pub fn read(&self, tx: &Tx, k: i64) -> Result<Vec<i64>, Failure> {
        let by_pos = self.by_pos.lock(&self.rt);
        let heap = self.elems.lock(&self.rt);
        Ok(self
            .scan(tx, k, &by_pos, &heap)?
            .into_iter()
            .map(|(_, e)| e)
            .collect())
    }

    fn scan(
        &self,
        tx: &Tx,
        k: i64,
        by_pos: &Index,
        heap: &Heap,
    ) -> Result<Vec<(i64, i64)>, Failure> {
        let prefix = schema(1).encode(&[Value::I64(k)]);
        {
            let mut ser = self.ser.lock(&self.rt);
            ser.check(tx.sid)?;
            ser.read_range(
                tx.sid,
                by_pos.tree().root(),
                Bound::Included(prefix.clone()),
                prefix_end(&prefix),
                [ELEM].into(),
            )?;
        }
        let mut out = Vec::new();
        for entry in by_pos.scan_prefix(&self.pager, &prefix).unwrap() {
            self.ser
                .lock(&self.rt)
                .read_tuple(tx.sid, ELEMS, entry.tid)?;
            let v = heap
                .read(&self.pager, entry.tid)
                .unwrap()
                .expect("an entry names a version");
            let (xmin, xmax) = (v.header.xmin, v.header.xmax);
            self.depends(tx, xmin, xmax)?;
            if self.sees(tx, xmin, xmax) {
                out.push((int(&v.row[2]), int(&v.row[3])));
            }
        }
        out.sort();
        Ok(out)
    }

    /// Append `e` to `k`: insert it at the position after every element `tx` sees.
    pub fn append(&self, tx: &Tx, k: i64, e: i64) -> Result<(), Failure> {
        let xid = tx.xid.expect("a read-only transaction appends");
        loop {
            let by_pos = self.by_pos.lock(&self.rt);
            let by_elem = self.by_elem.lock(&self.rt);
            let mut heap = self.elems.lock(&self.rt);
            let n = self.scan(tx, k, &by_pos, &heap)?.len() as i64;
            let keys = [
                (&*by_pos, vec![Value::I64(k), Value::I64(n)]),
                (&*by_elem, vec![Value::I64(k), Value::I64(e)]),
            ];
            if let Some(x) = self.probe(tx, xid, &heap, &keys)? {
                drop((heap, by_elem, by_pos));
                self.waits.wait(Some(xid), x)?;
                continue;
            }
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let row = [Value::I64(id), Value::I64(k), Value::I64(n), Value::I64(e)];
            let tid = heap.insert(&self.pager, xid.0, &row).unwrap();
            self.ser.lock(&self.rt).insert_heap(tx.sid, ELEMS)?;
            for (index, key) in keys {
                self.insert_entry(tx, index, key, ELEM, tid)?;
            }
            return Ok(());
        }
    }

    fn insert_entry(
        &self,
        tx: &Tx,
        index: &Index,
        key: Vec<Value>,
        ctor: CtorId,
        tid: Tid,
    ) -> Result<(), Failure> {
        let encoded = index.schema().encode(&key);
        index
            .insert(&self.pager, &Entry { key, ctor, tid })
            .unwrap();
        self.ser
            .lock(&self.rt)
            .insert_entry(tx.sid, index.tree().root(), &encoded, ctor)?;
        Ok(())
    }

    /// Probe each unique index for its key. Returns the transaction to wait for, if any.
    fn probe(
        &self,
        tx: &Tx,
        xid: Xid,
        heap: &Heap,
        keys: &[(&Index, Vec<Value>)],
    ) -> Result<Option<Xid>, Failure> {
        for (index, key) in keys {
            for entry in index.probe(&self.pager, key).unwrap() {
                let v = heap
                    .read(&self.pager, entry.tid)
                    .unwrap()
                    .expect("an entry names a version");
                let (xmin, xmax) = (v.header.xmin, v.header.xmax);
                let Ok(probe) = probe_unique(xid, &tx.snapshot, xmin, xmax, |x| self.status(x));
                match probe {
                    Probe::Skip => {}
                    Probe::Conflict => return Err(Failure::KeyViolation),
                    Probe::Wait(x) => return Ok(Some(x)),
                    Probe::Fail(f) => return Err(f.into()),
                }
            }
        }
        Ok(None)
    }

    pub fn create_counter(&self, tx: &Tx, id: i64) -> Result<(), Failure> {
        let xid = tx.xid.expect("a read-only transaction writes");
        loop {
            let by_id = self.by_id.lock(&self.rt);
            let mut heap = self.counters.lock(&self.rt);
            self.ser.lock(&self.rt).check(tx.sid)?;
            let keys = [(&*by_id, vec![Value::I64(id)])];
            if let Some(x) = self.probe(tx, xid, &heap, &keys)? {
                drop((heap, by_id));
                self.waits.wait(Some(xid), x)?;
                continue;
            }
            let row = [Value::I64(id), Value::I64(0)];
            let tid = heap.insert(&self.pager, xid.0, &row).unwrap();
            self.ser.lock(&self.rt).insert_heap(tx.sid, COUNTERS)?;
            let [(index, key)] = keys;
            return self.insert_entry(tx, index, key, COUNTER, tid);
        }
    }

    /// Returns the version of counter `id` that `tx` sees, with its TID and current `xmax`.
    fn find_counter(
        &self,
        tx: &Tx,
        id: i64,
        by_id: &Index,
        heap: &Heap,
    ) -> Result<Option<(Tid, u64, i64)>, Failure> {
        let key = schema(1).encode(&[Value::I64(id)]);
        {
            let mut ser = self.ser.lock(&self.rt);
            ser.check(tx.sid)?;
            ser.read_range(
                tx.sid,
                by_id.tree().root(),
                Bound::Included(key.clone()),
                Bound::Included(key),
                [COUNTER].into(),
            )?;
        }
        for entry in by_id.probe(&self.pager, &[Value::I64(id)]).unwrap() {
            let mut links = Vec::new();
            walk(&self.pager, entry.tid, |l| {
                links.push(*l);
                false
            })
            .unwrap();
            for l in links {
                if !self.sees(tx, l.header.xmin, l.header.xmax) {
                    self.depends(tx, l.header.xmin, l.header.xmax)?;
                    continue;
                }
                self.ser
                    .lock(&self.rt)
                    .read_tuple(tx.sid, COUNTERS, l.tid)?;
                let v = heap
                    .read(&self.pager, l.tid)
                    .unwrap()
                    .expect("a link names a version");
                self.depends(tx, v.header.xmin, v.header.xmax)?;
                return Ok(Some((l.tid, v.header.xmax, int(&v.row[1]))));
            }
        }
        Ok(None)
    }

    pub fn get_counter(&self, tx: &Tx, id: i64) -> Result<Option<i64>, Failure> {
        let by_id = self.by_id.lock(&self.rt);
        let heap = self.counters.lock(&self.rt);
        Ok(self.find_counter(tx, id, &by_id, &heap)?.map(|(_, _, n)| n))
    }

    /// Add one to counter `id`. Returns the new value.
    pub fn increment(&self, tx: &Tx, id: i64) -> Result<i64, Failure> {
        let xid = tx.xid.expect("a read-only transaction writes");
        loop {
            let by_id = self.by_id.lock(&self.rt);
            let mut heap = self.counters.lock(&self.rt);
            let (tid, xmax, n) = self
                .find_counter(tx, id, &by_id, &heap)?
                .expect("the counter exists");
            let Ok(write) = check_write(xid, xmax, |x| self.status(x));
            match write {
                Write::Proceed => {}
                Write::Wait(x) => {
                    drop((heap, by_id));
                    self.waits.wait(Some(xid), x)?;
                    continue;
                }
                Write::Fail(f) => return Err(f.into()),
            }
            let row = [Value::I64(id), Value::I64(n + 1)];
            let updated = heap.update(&self.pager, tid, xid.0, &row, &[0]).unwrap();
            {
                let mut ser = self.ser.lock(&self.rt);
                ser.write_tuple(tx.sid, COUNTERS, tid)?;
                ser.insert_heap(tx.sid, COUNTERS)?;
            }
            if !updated.heap_only {
                let key = vec![Value::I64(id)];
                self.insert_entry(tx, &by_id, key, COUNTER, updated.tid)?;
            }
            return Ok(n + 1);
        }
    }

    /// Commit `tx`, acknowledging once the log is durable past every commit it may have read.
    pub fn commit(&self, tx: Tx) -> Result<(), Failure> {
        let durable_past = {
            let mut ser = self.ser.lock(&self.rt);
            match ser.precommit(tx.sid) {
                Err(f) => Err(f),
                Ok(()) => {
                    let own = tx.xid.map(|x| self.txns.commit(&self.pager, x).unwrap());
                    ser.committed(tx.sid);
                    let read = self.txns.read_upto(tx.hold);
                    self.txns.release(tx.hold);
                    Ok((own, read))
                }
            }
        };
        let durable_past = match durable_past {
            Ok((own, read)) => own.max(read),
            Err(f) => {
                self.abort(tx);
                return Err(f.into());
            }
        };
        if let Some(lsn) = durable_past {
            match self.group_commit {
                None if self.pager.durable() <= lsn => self.pager.flush_log(),
                None => {}
                Some(interval) => {
                    while self.pager.durable() <= lsn {
                        self.sleep(interval / 4);
                    }
                }
            }
        }
        if let Some(x) = tx.xid {
            self.waits.ended(x);
        }
        Ok(())
    }

    pub fn abort(&self, tx: Tx) {
        {
            let mut ser = self.ser.lock(&self.rt);
            if let Some(x) = tx.xid {
                self.txns.abort(&self.pager, x).unwrap();
            }
            ser.aborted(tx.sid);
            self.txns.release(tx.hold);
        }
        if let Some(x) = tx.xid {
            self.waits.ended(x);
        }
    }

    pub fn tracked(&self) -> usize {
        self.ser.lock(&self.rt).tracked()
    }
}

pub enum Req {
    Append(i64, i64),
    Read(i64),
    CreateCounter(i64),
    GetCounter(i64),
    Increment(i64),
    Commit,
    Abort,
}

pub type Reply = Result<Vec<i64>, Failure>;

/// A transaction running on its own simulated thread, driven one request at a time.
pub struct Mailbox {
    req: Mutex<Option<Req>>,
    reply: Mutex<Option<Reply>>,
    driver: ThreadId,
}

pub struct Worker {
    mailbox: Arc<Mailbox>,
    pub thread: ThreadId,
}

pub fn take<T>(rt: &SimRuntime, slot: &Mutex<Option<T>>) -> T {
    loop {
        if let Some(v) = slot.lock().unwrap().take() {
            return v;
        }
        rt.park();
    }
}

impl Worker {
    pub fn start(engine: &Arc<Engine>, kind: Kind) -> Worker {
        let rt = engine.rt.clone();
        let mailbox = Arc::new(Mailbox {
            req: Mutex::new(None),
            reply: Mutex::new(None),
            driver: rt.current(),
        });
        let (m, e, rt2) = (mailbox.clone(), engine.clone(), rt.clone());
        let thread = rt.spawn("txn", move || Worker::serve(&e, &rt2, &m, kind));
        take(&rt, &mailbox.reply).unwrap();
        Worker { mailbox, thread }
    }

    fn serve(engine: &Engine, rt: &SimRuntime, m: &Mailbox, kind: Kind) {
        let tx = engine.begin(kind);
        let reply = |r: Reply| {
            *m.reply.lock().unwrap() = Some(r);
            rt.unpark(m.driver);
        };
        reply(Ok(vec![]));
        loop {
            let one = |r: Result<i64, Failure>| r.map(|n| vec![n]);
            let r = match take(rt, &m.req) {
                Req::Append(k, e) => engine.append(&tx, k, e).map(|()| vec![]),
                Req::Read(k) => engine.read(&tx, k),
                Req::CreateCounter(id) => engine.create_counter(&tx, id).map(|()| vec![]),
                Req::GetCounter(id) => engine.get_counter(&tx, id).map(|n| n.into_iter().collect()),
                Req::Increment(id) => one(engine.increment(&tx, id)),
                Req::Commit => {
                    reply(engine.commit(tx).map(|()| vec![]));
                    return;
                }
                Req::Abort => {
                    engine.abort(tx);
                    reply(Ok(vec![]));
                    return;
                }
            };
            let failed = r.is_err();
            if failed {
                engine.abort(tx);
                reply(r);
                return;
            }
            reply(r);
        }
    }

    /// Send `req` and return without waiting for the reply.
    pub fn send(&self, rt: &SimRuntime, req: Req) {
        *self.mailbox.req.lock().unwrap() = Some(req);
        rt.unpark(self.thread);
    }

    pub fn reply(&self, rt: &SimRuntime) -> Reply {
        take(rt, &self.mailbox.reply)
    }

    /// Returns the reply if the worker has answered.
    pub fn try_reply(&self) -> Option<Reply> {
        self.mailbox.reply.lock().unwrap().take()
    }

    pub fn call(&self, rt: &SimRuntime, req: Req) -> Reply {
        self.send(rt, req);
        self.reply(rt)
    }
}

/// The isolation checker's database: each transaction a worker thread.
pub struct Client {
    pub engine: Arc<Engine>,
    pub threads: Vec<ThreadId>,
}

impl Database for Client {
    type Txn = Worker;

    fn begin(&mut self) -> Worker {
        let w = Worker::start(&self.engine, Kind::ReadWrite);
        self.threads.push(w.thread);
        w
    }

    fn append(&mut self, txn: &mut Worker, key: i64, elem: i64) -> Result<(), Aborted> {
        txn.call(&self.engine.rt, Req::Append(key, elem))
            .map(|_| ())
            .map_err(|_| Aborted)
    }

    fn read(&mut self, txn: &mut Worker, key: i64) -> Result<Vec<i64>, Aborted> {
        txn.call(&self.engine.rt, Req::Read(key))
            .map_err(|_| Aborted)
    }

    fn commit(&mut self, txn: Worker) -> Result<(), Aborted> {
        txn.call(&self.engine.rt, Req::Commit)
            .map(|_| ())
            .map_err(|_| Aborted)
    }

    fn abort(&mut self, txn: Worker) {
        txn.call(&self.engine.rt, Req::Abort).unwrap();
    }
}

pub fn quiet() -> Faults {
    Faults {
        delay_completions: true,
        ..Faults::default()
    }
}

/// Run `body` in a simulated process over a fresh database.
pub fn simulate<T>(seed: u64, body: impl FnOnce(&Arc<Engine>) -> T) -> T {
    simulate_with(seed, Limits::default(), body)
}

pub fn simulate_with<T>(seed: u64, limits: Limits, body: impl FnOnce(&Arc<Engine>) -> T) -> T {
    let mut sim = Sim::new(
        seed,
        Config {
            faults: quiet(),
            ..Config::default()
        },
    );
    let mut out = None;
    let outcome = sim.run(|rt| {
        let engine = Arc::new(Engine::create(rt, limits));
        out = Some(body(&engine));
        let problems = engine.verify();
        assert!(problems.is_empty(), "{problems:#?}");
    });
    assert_eq!(Outcome::Finished, outcome);
    out.unwrap()
}
