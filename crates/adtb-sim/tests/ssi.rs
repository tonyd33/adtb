use std::ops::Bound;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use adtb::btree::{Index, prefix_end};
use adtb::heap::{FieldType, Heap, LargeStore, walk};
use adtb::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use adtb::mvcc::{
    Access, Limits, Probe, Safety, Scope, SerialId, SerializationFailure, Serializer, Snapshot,
    SnapshotId, Transactions, Waits, Write, Xid, check_write, probe_unique, read_before,
};
use adtb::runtime::{Entropy, ThreadId, Threads};
use adtb::storage::{Options, Pager, Pid};
use adtb_isolation::{self as iso, Aborted, Database, Outcome as Ended, SCENARIOS, Step};
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};

const PAGE: usize = 4096;
const ELEMS: u32 = 10;
const COUNTERS: u32 = 11;
const LARGE: u32 = 9;
const ELEM: CtorId = CtorId(1);
const COUNTER: CtorId = CtorId(2);

fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames: 64,
        atomic_page_writes: false,
        ..Options::new("ssi.adt")
    }
}

/// A mutex whose waiters spin through `yield_now`, so the simulator can run its holder.
#[derive(Debug)]
struct Latch<T>(Mutex<T>);

impl<T> Latch<T> {
    fn lock(&self, rt: &SimRuntime) -> MutexGuard<'_, T> {
        loop {
            match self.0.try_lock() {
                Ok(g) => return g,
                Err(TryLockError::Poisoned(e)) => return e.into_inner(),
                Err(TryLockError::WouldBlock) => rt.yield_now(),
            }
        }
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

/// The pages of the workload: `elems { id, k, pos, e }` with `unique (k, pos)` and
/// `unique (k, e)`, and `counters { id, n }` keyed by `id`.
struct Store {
    pager: Pager<SimRuntime>,
    txns: Transactions,
    elems: Pid,
    counters: Pid,
    by_pos: Index,
    by_elem: Index,
    by_id: Index,
    next_id: i64,
}

impl Store {
    fn heap(&self, id: u32) -> Heap {
        let (root, fields) = match id {
            ELEMS => (self.elems, 4),
            _ => (self.counters, 2),
        };
        Heap::open(
            &self.pager,
            id,
            root,
            i64s(fields),
            LargeStore::open(LARGE, Pid(0)),
        )
        .unwrap()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Serialization,
    KeyViolation,
}

impl From<SerializationFailure> for Failure {
    fn from(_: SerializationFailure) -> Failure {
        Failure::Serialization
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    ReadWrite,
    ReadOnly,
    Deferrable,
}

struct Tx {
    xid: Option<Xid>,
    sid: SerialId,
    snapshot: Snapshot,
    hold: SnapshotId,
}

/// The transactions layer over the workload: one latch over the pages, taken before the
/// serializer's.
struct Engine {
    rt: SimRuntime,
    store: Latch<Store>,
    ser: Latch<Serializer>,
    waits: Waits<SimRuntime>,
}

impl Engine {
    fn create(rt: SimRuntime, limits: Limits) -> Engine {
        let mut pager = Pager::create(rt.clone(), &options()).unwrap();
        let large = LargeStore::create(&mut pager, LARGE).unwrap();
        let elems = Heap::create(&mut pager, ELEMS, i64s(4), large)
            .unwrap()
            .root();
        let counters = Heap::create(&mut pager, COUNTERS, i64s(2), large)
            .unwrap()
            .root();
        let by_pos = Index::create(&mut pager, ELEMS, schema(2), [ELEM].into()).unwrap();
        let by_elem = Index::create(&mut pager, ELEMS, schema(2), [ELEM].into()).unwrap();
        let by_id = Index::create(&mut pager, COUNTERS, schema(1), [COUNTER].into()).unwrap();
        let txns = Transactions::create(&mut pager).unwrap();
        pager.flush_log();
        let store = Store {
            pager,
            txns,
            elems,
            counters,
            by_pos,
            by_elem,
            by_id,
            next_id: 1,
        };
        Engine {
            waits: Waits::new(rt.clone()),
            rt,
            store: Latch(Mutex::new(store)),
            ser: Latch(Mutex::new(Serializer::new(limits))),
        }
    }

    fn begin(&self, kind: Kind) -> Tx {
        loop {
            let tx = {
                let mut st = self.store.lock(&self.rt);
                let st = &mut *st;
                let xid = match kind {
                    Kind::ReadWrite => Some(st.txns.begin(&mut st.pager).unwrap()),
                    Kind::ReadOnly | Kind::Deferrable => None,
                };
                let (hold, snapshot) = st.txns.take(Scope::Every);
                let sid = self.ser.lock(&self.rt).begin(match (kind, xid) {
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
            self.end(tx, false);
        }
    }

    /// Returns the elements of `k` at `tx`'s snapshot, ordered by position.
    fn read(&self, tx: &Tx, k: i64) -> Result<Vec<i64>, Failure> {
        let mut st = self.store.lock(&self.rt);
        let mut ser = self.ser.lock(&self.rt);
        Ok(Engine::scan(&mut st, &mut ser, tx, k)?
            .into_iter()
            .map(|(_, e)| e)
            .collect())
    }

    fn scan(
        st: &mut Store,
        ser: &mut Serializer,
        tx: &Tx,
        k: i64,
    ) -> Result<Vec<(i64, i64)>, Failure> {
        ser.check(tx.sid)?;
        let prefix = schema(1).encode(&[Value::I64(k)]);
        ser.read_range(
            tx.sid,
            st.by_pos.tree().root(),
            Bound::Included(prefix.clone()),
            prefix_end(&prefix),
            [ELEM].into(),
        )?;
        let heap = st.heap(ELEMS);
        let Store {
            pager,
            txns,
            by_pos,
            ..
        } = st;
        let mut out = Vec::new();
        for entry in by_pos.scan_prefix(pager, &prefix).unwrap() {
            let v = heap
                .read(pager, entry.tid)
                .unwrap()
                .expect("an entry names a version");
            let (xmin, xmax) = (v.header.xmin, v.header.xmax);
            let status = |x| txns.status(pager, x);
            if let Some(w) = read_before(tx.xid, &tx.snapshot, xmin, xmax, status).unwrap() {
                ser.read_from(tx.sid, w)?;
            }
            if txns.sees(pager, &tx.snapshot, tx.xid, xmin, xmax).unwrap() {
                ser.read_tuple(tx.sid, ELEMS, entry.tid)?;
                out.push((int(&v.row[2]), int(&v.row[3])));
            }
        }
        out.sort();
        Ok(out)
    }

    /// Append `e` to `k`: insert it at the position after every element `tx` sees.
    fn append(&self, tx: &Tx, k: i64, e: i64) -> Result<(), Failure> {
        let xid = tx.xid.expect("a read-only transaction appends");
        loop {
            let mut st = self.store.lock(&self.rt);
            let mut ser = self.ser.lock(&self.rt);
            let n = Engine::scan(&mut st, &mut ser, tx, k)?.len() as i64;
            let keys = [
                (st.by_pos.clone(), vec![Value::I64(k), Value::I64(n)]),
                (st.by_elem.clone(), vec![Value::I64(k), Value::I64(e)]),
            ];
            if let Some(x) = Engine::probe(&mut st, tx, xid, ELEMS, &keys)? {
                drop((st, ser));
                self.waits.wait(Some(xid), x)?;
                continue;
            }
            let id = st.next_id;
            st.next_id += 1;
            let row = [Value::I64(id), Value::I64(k), Value::I64(n), Value::I64(e)];
            let mut heap = st.heap(ELEMS);
            let tid = heap.insert(&mut st.pager, xid.0, &row).unwrap();
            ser.insert_heap(tx.sid, ELEMS)?;
            for (index, key) in keys {
                Engine::insert_entry(&mut st, &mut ser, tx, &index, key, ELEM, tid)?;
            }
            return Ok(());
        }
    }

    fn insert_entry(
        st: &mut Store,
        ser: &mut Serializer,
        tx: &Tx,
        index: &Index,
        key: Vec<Value>,
        ctor: CtorId,
        tid: Tid,
    ) -> Result<(), Failure> {
        let encoded = index.schema().encode(&key);
        index
            .insert(&mut st.pager, &Entry { key, ctor, tid })
            .unwrap();
        ser.insert_entry(tx.sid, index.tree().root(), &encoded, ctor)?;
        Ok(())
    }

    /// Probe each unique index for its key. Returns the transaction to wait for, if any.
    fn probe(
        st: &mut Store,
        tx: &Tx,
        xid: Xid,
        heap: u32,
        keys: &[(Index, Vec<Value>)],
    ) -> Result<Option<Xid>, Failure> {
        let heap = st.heap(heap);
        let Store { pager, txns, .. } = st;
        for (index, key) in keys {
            for entry in index.probe(pager, key).unwrap() {
                let v = heap
                    .read(pager, entry.tid)
                    .unwrap()
                    .expect("an entry names a version");
                let status = |x| txns.status(pager, x);
                match probe_unique(xid, &tx.snapshot, v.header.xmin, v.header.xmax, status).unwrap()
                {
                    Probe::Skip => {}
                    Probe::Conflict => return Err(Failure::KeyViolation),
                    Probe::Wait(x) => return Ok(Some(x)),
                    Probe::Fail(f) => return Err(f.into()),
                }
            }
        }
        Ok(None)
    }

    fn create_counter(&self, tx: &Tx, id: i64) -> Result<(), Failure> {
        let xid = tx.xid.expect("a read-only transaction writes");
        loop {
            let mut st = self.store.lock(&self.rt);
            let mut ser = self.ser.lock(&self.rt);
            ser.check(tx.sid)?;
            let keys = [(st.by_id.clone(), vec![Value::I64(id)])];
            if let Some(x) = Engine::probe(&mut st, tx, xid, COUNTERS, &keys)? {
                drop((st, ser));
                self.waits.wait(Some(xid), x)?;
                continue;
            }
            let mut heap = st.heap(COUNTERS);
            let row = [Value::I64(id), Value::I64(0)];
            let tid = heap.insert(&mut st.pager, xid.0, &row).unwrap();
            ser.insert_heap(tx.sid, COUNTERS)?;
            let [(index, key)] = keys;
            return Engine::insert_entry(&mut st, &mut ser, tx, &index, key, COUNTER, tid);
        }
    }

    /// Returns the version of counter `id` that `tx` sees, with its TID and current `xmax`.
    fn find_counter(
        st: &mut Store,
        ser: &mut Serializer,
        tx: &Tx,
        id: i64,
    ) -> Result<Option<(Tid, u64, i64)>, Failure> {
        ser.check(tx.sid)?;
        let key = schema(1).encode(&[Value::I64(id)]);
        ser.read_range(
            tx.sid,
            st.by_id.tree().root(),
            Bound::Included(key.clone()),
            Bound::Included(key.clone()),
            [COUNTER].into(),
        )?;
        let heap = st.heap(COUNTERS);
        let Store {
            pager, txns, by_id, ..
        } = st;
        for entry in by_id.probe(pager, &[Value::I64(id)]).unwrap() {
            let mut links = Vec::new();
            walk(pager, entry.tid, |l| {
                links.push(*l);
                false
            })
            .unwrap();
            for l in links {
                let (xmin, xmax) = (l.header.xmin, l.header.xmax);
                let status = |x| txns.status(pager, x);
                if let Some(w) = read_before(tx.xid, &tx.snapshot, xmin, xmax, status).unwrap() {
                    ser.read_from(tx.sid, w)?;
                }
                if txns.sees(pager, &tx.snapshot, tx.xid, xmin, xmax).unwrap() {
                    ser.read_tuple(tx.sid, COUNTERS, l.tid)?;
                    let v = heap
                        .read(pager, l.tid)
                        .unwrap()
                        .expect("a link names a version");
                    return Ok(Some((l.tid, xmax, int(&v.row[1]))));
                }
            }
        }
        Ok(None)
    }

    fn get_counter(&self, tx: &Tx, id: i64) -> Result<Option<i64>, Failure> {
        let mut st = self.store.lock(&self.rt);
        let mut ser = self.ser.lock(&self.rt);
        Ok(Engine::find_counter(&mut st, &mut ser, tx, id)?.map(|(_, _, n)| n))
    }

    /// Add one to counter `id`. Returns the new value.
    fn increment(&self, tx: &Tx, id: i64) -> Result<i64, Failure> {
        let xid = tx.xid.expect("a read-only transaction writes");
        loop {
            let mut st = self.store.lock(&self.rt);
            let mut ser = self.ser.lock(&self.rt);
            let (tid, xmax, n) =
                Engine::find_counter(&mut st, &mut ser, tx, id)?.expect("the counter exists");
            let Store { pager, txns, .. } = &mut *st;
            match check_write(xid, xmax, |x| txns.status(pager, x)).unwrap() {
                Write::Proceed => {}
                Write::Wait(x) => {
                    drop((st, ser));
                    self.waits.wait(Some(xid), x)?;
                    continue;
                }
                Write::Fail(f) => return Err(f.into()),
            }
            let mut heap = st.heap(COUNTERS);
            let row = [Value::I64(id), Value::I64(n + 1)];
            let updated = heap.update(&mut st.pager, tid, xid.0, &row, &[0]).unwrap();
            ser.write_tuple(tx.sid, COUNTERS, tid)?;
            ser.insert_heap(tx.sid, COUNTERS)?;
            if !updated.heap_only {
                let index = st.by_id.clone();
                let key = vec![Value::I64(id)];
                Engine::insert_entry(&mut st, &mut ser, tx, &index, key, COUNTER, updated.tid)?;
            }
            return Ok(n + 1);
        }
    }

    fn commit(&self, tx: Tx) -> Result<(), Failure> {
        let ok = {
            let mut st = self.store.lock(&self.rt);
            let mut ser = self.ser.lock(&self.rt);
            let st = &mut *st;
            let ok = ser.precommit(tx.sid).is_ok();
            if ok {
                if let Some(x) = tx.xid {
                    st.txns.commit(&mut st.pager, x).unwrap();
                }
                ser.committed(tx.sid);
                st.pager.flush_log();
            }
            ok
        };
        if !ok {
            self.end(tx, false);
            return Err(Failure::Serialization);
        }
        self.end(tx, true);
        Ok(())
    }

    fn abort(&self, tx: Tx) {
        self.end(tx, false);
    }

    fn end(&self, tx: Tx, committed: bool) {
        {
            let mut st = self.store.lock(&self.rt);
            let mut ser = self.ser.lock(&self.rt);
            let st = &mut *st;
            if !committed {
                if let Some(x) = tx.xid {
                    st.txns.abort(&mut st.pager, x).unwrap();
                }
                ser.aborted(tx.sid);
            }
            st.txns.release(tx.hold);
        }
        if let Some(x) = tx.xid {
            self.waits.ended(x);
        }
    }

    fn tracked(&self) -> usize {
        self.ser.lock(&self.rt).tracked()
    }
}

enum Req {
    Append(i64, i64),
    Read(i64),
    CreateCounter(i64),
    GetCounter(i64),
    Increment(i64),
    Commit,
    Abort,
}

type Reply = Result<Vec<i64>, Failure>;

/// A transaction running on its own simulated thread, driven one request at a time.
struct Mailbox {
    req: Mutex<Option<Req>>,
    reply: Mutex<Option<Reply>>,
    driver: ThreadId,
}

struct Worker {
    mailbox: Arc<Mailbox>,
    thread: ThreadId,
}

fn take<T>(rt: &SimRuntime, slot: &Mutex<Option<T>>) -> T {
    loop {
        if let Some(v) = slot.lock().unwrap().take() {
            return v;
        }
        rt.park();
    }
}

impl Worker {
    fn start(engine: &Arc<Engine>, kind: Kind) -> Worker {
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
    fn send(&self, rt: &SimRuntime, req: Req) {
        *self.mailbox.req.lock().unwrap() = Some(req);
        rt.unpark(self.thread);
    }

    fn reply(&self, rt: &SimRuntime) -> Reply {
        take(rt, &self.mailbox.reply)
    }

    /// Returns the reply if the worker has answered.
    fn try_reply(&self) -> Option<Reply> {
        self.mailbox.reply.lock().unwrap().take()
    }

    fn call(&self, rt: &SimRuntime, req: Req) -> Reply {
        self.send(rt, req);
        self.reply(rt)
    }
}

/// The isolation checker's database: each transaction a worker thread.
struct Client {
    engine: Arc<Engine>,
    threads: Vec<ThreadId>,
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

fn quiet() -> Faults {
    Faults {
        delay_completions: true,
        ..Faults::default()
    }
}

/// Run `body` in a simulated process over a fresh database.
fn simulate<T>(seed: u64, body: impl FnOnce(&Arc<Engine>) -> T) -> T {
    simulate_with(seed, Limits::default(), body)
}

fn simulate_with<T>(seed: u64, limits: Limits, body: impl FnOnce(&Arc<Engine>) -> T) -> T {
    let mut sim = Sim::new(
        seed,
        Config {
            faults: quiet(),
            ..Config::default()
        },
    );
    let mut out = None;
    let outcome = sim.run(|rt| out = Some(body(&Arc::new(Engine::create(rt, limits)))));
    assert_eq!(Outcome::Finished, outcome);
    out.unwrap()
}

fn script(engine: &Arc<Engine>, steps: &[Step]) -> Vec<iso::Txn> {
    let mut client = Client {
        engine: engine.clone(),
        threads: Vec::new(),
    };
    let history = iso::run(&mut client, steps);
    for t in client.threads {
        engine.rt.join(t);
    }
    history
}

fn outcome(history: &[iso::Txn], t: u64) -> Ended {
    history.iter().find(|x| x.id.0 == t).unwrap().outcome
}

#[test]
fn every_scenario_runs_without_anomaly() {
    for scenario in SCENARIOS {
        for seed in 0..4 {
            let history = simulate(seed, |e| script(e, scenario.steps));
            assert_eq!(
                Ok(vec![]),
                iso::check(&history),
                "{} seed {seed}",
                scenario.name
            );
        }
    }
}

#[test]
fn write_skew_aborts_one_side() {
    let history = simulate(1, |e| script(e, iso::WRITE_SKEW.steps));
    let sides = [outcome(&history, 1), outcome(&history, 2)];
    assert!(sides.contains(&Ended::Aborted), "{sides:?}");
    assert!(sides.contains(&Ended::Committed), "{sides:?}");
}

#[test]
fn lost_update_by_appends_aborts_one_side() {
    let history = simulate(2, |e| script(e, iso::LOST_UPDATE.steps));
    let sides = [outcome(&history, 1), outcome(&history, 2)];
    assert_eq!([Ended::Committed, Ended::Aborted], sides);
}

use Step::{Append, Begin, Commit, Read};

/// Fekete, O'Neil and O'Neil's read-only anomaly over keys 1 (X) and 2 (Y): 2 reads both and
/// appends to X; 1 appends to Y and commits; 3 reads both after 1 commits and before 2 does.
const FEKETE: &[Step] = &[
    Begin(2),
    Read(2, 1),
    Read(2, 2),
    Begin(1),
    Read(1, 2),
    Append(1, 2, 10),
    Commit(1),
    Append(2, 1, 20),
    Begin(3),
    Read(3, 1),
    Read(3, 2),
    Commit(3),
    Commit(2),
    Begin(4),
    Read(4, 1),
    Read(4, 2),
    Commit(4),
];

#[test]
fn feketes_read_only_anomaly_aborts_one_transaction() {
    for seed in 0..4 {
        let history = simulate(seed, |e| script(e, FEKETE));
        assert_eq!(Ok(vec![]), iso::check(&history), "seed {seed}");
        let ends = [1, 2, 3].map(|t| outcome(&history, t));
        assert!(ends.contains(&Ended::Aborted), "{ends:?}");
    }
}

#[test]
fn a_deferrable_reader_waits_for_a_safe_snapshot_and_never_fails() {
    for seed in 0..8 {
        let (ends, seen) = simulate(seed, |e| {
            let rt = &e.rt;
            let t2 = Worker::start(e, Kind::ReadWrite);
            t2.call(rt, Req::Read(1)).unwrap();
            t2.call(rt, Req::Read(2)).unwrap();
            let t1 = Worker::start(e, Kind::ReadWrite);
            t1.call(rt, Req::Read(2)).unwrap();
            t1.call(rt, Req::Append(2, 10)).unwrap();
            t1.call(rt, Req::Commit).unwrap();
            t2.call(rt, Req::Append(1, 20)).unwrap();
            let e2 = e.clone();
            let rt2 = rt.clone();
            let seen = Arc::new(Mutex::new(None));
            let out = seen.clone();
            let d = rt.spawn("deferrable", move || {
                let r = Worker::start(&e2, Kind::Deferrable);
                let x = r.call(&rt2, Req::Read(1)).unwrap();
                let y = r.call(&rt2, Req::Read(2)).unwrap();
                r.call(&rt2, Req::Commit).unwrap();
                rt2.join(r.thread);
                *out.lock().unwrap() = Some((x, y));
            });
            for _ in 0..20 {
                rt.yield_now();
            }
            assert!(
                seen.lock().unwrap().is_none(),
                "the reader ran on an unsafe snapshot"
            );
            let end2 = t2.call(rt, Req::Commit);
            rt.join(d);
            for t in [t1.thread, t2.thread] {
                rt.join(t);
            }
            let seen = seen.lock().unwrap().take().unwrap();
            (end2, seen)
        });
        assert_eq!(Ok(vec![]), ends, "seed {seed}");
        assert_eq!((vec![20], vec![10]), seen, "seed {seed}");
    }
}

#[test]
fn a_deferrable_reader_with_no_concurrent_writer_starts_at_once() {
    simulate(0, |e| {
        let rt = &e.rt;
        let w = Worker::start(e, Kind::ReadWrite);
        w.call(rt, Req::Append(1, 1)).unwrap();
        w.call(rt, Req::Commit).unwrap();
        let r = Worker::start(e, Kind::Deferrable);
        assert_eq!(Ok(vec![1]), r.call(rt, Req::Read(1)));
        r.call(rt, Req::Commit).unwrap();
        for t in [w.thread, r.thread] {
            rt.join(t);
        }
    });
}

fn appender(e: &Arc<Engine>, k: i64, elem: i64) -> Worker {
    let w = Worker::start(e, Kind::ReadWrite);
    w.call(&e.rt, Req::Append(k, elem)).unwrap();
    w
}

/// Let the other threads run until `w` answers. Returns its reply, or `None` if it is still
/// waiting after many switches.
fn settle(rt: &SimRuntime, w: &Worker) -> Option<Reply> {
    for _ in 0..200 {
        if let Some(r) = w.try_reply() {
            return Some(r);
        }
        rt.yield_now();
    }
    None
}

#[test]
fn an_insert_of_a_key_a_running_transaction_inserted_waits_and_fails_if_it_commits() {
    for seed in 0..4 {
        let r = simulate(seed, |e| {
            let rt = &e.rt;
            let t1 = appender(e, 1, 1);
            let t2 = Worker::start(e, Kind::ReadWrite);
            t2.send(rt, Req::Append(1, 2));
            assert_eq!(None, settle(rt, &t2), "the second insert did not wait");
            t1.call(rt, Req::Commit).unwrap();
            let r = t2.reply(rt);
            for t in [t1.thread, t2.thread] {
                rt.join(t);
            }
            r
        });
        assert_eq!(Err(Failure::Serialization), r, "seed {seed}");
    }
}

#[test]
fn an_insert_that_waited_proceeds_if_the_other_transaction_aborts() {
    for seed in 0..4 {
        let read = simulate(seed, |e| {
            let rt = &e.rt;
            let t1 = appender(e, 1, 1);
            let t2 = Worker::start(e, Kind::ReadWrite);
            t2.send(rt, Req::Append(1, 2));
            assert_eq!(None, settle(rt, &t2));
            t1.call(rt, Req::Abort).unwrap();
            t2.reply(rt).unwrap();
            t2.call(rt, Req::Commit).unwrap();
            let r = Worker::start(e, Kind::ReadOnly);
            let read = r.call(rt, Req::Read(1));
            r.call(rt, Req::Commit).unwrap();
            for t in [t1.thread, t2.thread, r.thread] {
                rt.join(t);
            }
            read
        });
        assert_eq!(Ok(vec![2]), read, "seed {seed}");
    }
}

#[test]
fn an_insert_of_a_key_in_the_snapshot_is_a_key_violation() {
    let r = simulate(3, |e| {
        let rt = &e.rt;
        let t1 = appender(e, 1, 5);
        t1.call(rt, Req::Commit).unwrap();
        let t2 = Worker::start(e, Kind::ReadWrite);
        let r = t2.call(rt, Req::Append(1, 5));
        for t in [t1.thread, t2.thread] {
            rt.join(t);
        }
        r
    });
    assert_eq!(Err(Failure::KeyViolation), r);
}

#[test]
fn an_insert_of_a_key_committed_after_the_snapshot_fails() {
    let r = simulate(4, |e| {
        let rt = &e.rt;
        let t2 = Worker::start(e, Kind::ReadWrite);
        let t1 = appender(e, 1, 5);
        t1.call(rt, Req::Commit).unwrap();
        let r = t2.call(rt, Req::Append(1, 5));
        for t in [t1.thread, t2.thread] {
            rt.join(t);
        }
        r
    });
    assert_eq!(Err(Failure::Serialization), r);
}

#[test]
fn two_inserts_waiting_on_each_other_abort_one() {
    for seed in 0..4 {
        let (r1, r2) = simulate(seed, |e| {
            let rt = &e.rt;
            let t1 = appender(e, 1, 1);
            let t2 = appender(e, 2, 1);
            t1.send(rt, Req::Append(2, 2));
            assert_eq!(None, settle(rt, &t1));
            let r2 = t2.call(rt, Req::Append(1, 2));
            if r2.is_ok() {
                t2.call(rt, Req::Abort).unwrap();
            }
            let r1 = t1.reply(rt);
            if r1.is_ok() {
                t1.call(rt, Req::Abort).unwrap();
            }
            for t in [t1.thread, t2.thread] {
                rt.join(t);
            }
            (r1, r2)
        });
        assert_eq!(Ok(vec![]), r1, "seed {seed}");
        assert_eq!(Err(Failure::Serialization), r2, "seed {seed}");
    }
}

fn counter(e: &Arc<Engine>, id: i64) {
    let w = Worker::start(e, Kind::ReadWrite);
    w.call(&e.rt, Req::CreateCounter(id)).unwrap();
    w.call(&e.rt, Req::Commit).unwrap();
    e.rt.join(w.thread);
}

#[test]
fn a_lost_update_fails_the_second_writer() {
    for seed in 0..4 {
        let (r, n) = simulate(seed, |e| {
            let rt = &e.rt;
            counter(e, 7);
            let t1 = Worker::start(e, Kind::ReadWrite);
            let t2 = Worker::start(e, Kind::ReadWrite);
            assert_eq!(Ok(vec![0]), t1.call(rt, Req::GetCounter(7)));
            assert_eq!(Ok(vec![0]), t2.call(rt, Req::GetCounter(7)));
            assert_eq!(Ok(vec![1]), t1.call(rt, Req::Increment(7)));
            t1.call(rt, Req::Commit).unwrap();
            let r = t2.call(rt, Req::Increment(7));
            let check = Worker::start(e, Kind::ReadOnly);
            let n = check.call(rt, Req::GetCounter(7));
            check.call(rt, Req::Commit).unwrap();
            for t in [t1.thread, t2.thread, check.thread] {
                rt.join(t);
            }
            (r, n)
        });
        assert_eq!(Err(Failure::Serialization), r, "seed {seed}");
        assert_eq!(Ok(vec![1]), n, "seed {seed}");
    }
}

#[test]
fn a_write_of_a_row_another_transaction_is_writing_waits_for_it() {
    for seed in 0..4 {
        let (committed, aborted) = simulate(seed, |e| {
            let rt = &e.rt;
            counter(e, 1);
            counter(e, 2);
            let mut out = Vec::new();
            for (id, commit) in [(1, true), (2, false)] {
                let t1 = Worker::start(e, Kind::ReadWrite);
                t1.call(rt, Req::Increment(id)).unwrap();
                let t2 = Worker::start(e, Kind::ReadWrite);
                t2.send(rt, Req::Increment(id));
                assert_eq!(None, settle(rt, &t2), "the second write did not wait");
                t1.call(rt, if commit { Req::Commit } else { Req::Abort })
                    .unwrap();
                let r = t2.reply(rt);
                if r.is_ok() {
                    t2.call(rt, Req::Commit).unwrap();
                }
                for t in [t1.thread, t2.thread] {
                    rt.join(t);
                }
                out.push(r);
            }
            (out[0].clone(), out[1].clone())
        });
        assert_eq!(Err(Failure::Serialization), committed, "seed {seed}");
        assert_eq!(Ok(vec![1]), aborted, "seed {seed}");
    }
}

/// Clients running random transactions of reads and appends over a few keys.
fn random_history(
    seed: u64,
    limits: Limits,
    clients: usize,
    per_client: usize,
) -> (Vec<iso::Txn>, usize) {
    simulate_with(seed, limits, move |e| {
        let rt = &e.rt;
        let history = Arc::new(Mutex::new(Vec::new()));
        let next = Arc::new(Mutex::new(0_u64));
        let threads: Vec<ThreadId> = (0..clients)
            .map(|_| {
                let (e, history, next) = (e.clone(), history.clone(), next.clone());
                let rt2 = rt.clone();
                rt.spawn("client", move || {
                    for _ in 0..per_client {
                        let id = {
                            let mut n = next.lock().unwrap();
                            *n += 1;
                            *n
                        };
                        let txn = random_txn(&e, &rt2, id);
                        history.lock().unwrap().push(txn);
                    }
                })
            })
            .collect();
        for t in threads {
            rt.join(t);
        }
        let reader = e.begin(Kind::ReadOnly);
        let mut ops = Vec::new();
        for key in 0..KEYS {
            let list = e.read(&reader, key).unwrap();
            ops.push(iso::Op::Read { key, list });
        }
        e.commit(reader).unwrap();
        let mut history = std::mem::take(&mut *history.lock().unwrap());
        history.push(iso::Txn::new(0, Ended::Committed, ops));
        (history, e.tracked())
    })
}

const KEYS: i64 = 3;

fn random_txn(e: &Engine, rt: &SimRuntime, id: u64) -> iso::Txn {
    let kind = match rt.next_u64() % 10 {
        0 => Kind::Deferrable,
        1 => Kind::ReadOnly,
        _ => Kind::ReadWrite,
    };
    let read_only = kind != Kind::ReadWrite;
    let tx = e.begin(kind);
    let mut ops = Vec::new();
    let steps = 1 + rt.next_u64() % 4;
    for step in 0..steps {
        rt.yield_now();
        let key = (rt.next_u64() % KEYS as u64) as i64;
        let r = if read_only || rt.next_u64().is_multiple_of(2) {
            e.read(&tx, key).map(|list| iso::Op::Read { key, list })
        } else {
            let elem = (id * 10 + step) as i64;
            e.append(&tx, key, elem)
                .map(|()| iso::Op::Append { key, elem })
        };
        match r {
            Ok(op) => ops.push(op),
            Err(f) => {
                assert_eq!(Failure::Serialization, f);
                assert_ne!(Kind::Deferrable, kind, "a deferrable transaction failed");
                e.abort(tx);
                return iso::Txn::new(id, Ended::Aborted, ops);
            }
        }
    }
    rt.yield_now();
    let outcome = match e.commit(tx) {
        Ok(()) => Ended::Committed,
        Err(_) if kind == Kind::Deferrable => panic!("a deferrable transaction failed"),
        Err(_) => Ended::Aborted,
    };
    iso::Txn::new(id, outcome, ops)
}

#[test]
fn every_committed_history_of_random_clients_is_serializable() {
    let (mut committed, mut aborted) = (0, 0);
    for seed in 0..24 {
        let (history, tracked) = random_history(seed, Limits::default(), 4, 6);
        assert_eq!(Ok(vec![]), iso::check(&history), "seed {seed}");
        assert_eq!(0, tracked, "seed {seed} left transactions tracked");
        for t in &history {
            match t.outcome {
                Ended::Committed => committed += 1,
                _ => aborted += 1,
            }
        }
    }
    assert!(committed > 200, "only {committed} transactions committed");
    assert!(aborted > 10, "only {aborted} transactions aborted");
}

#[test]
fn a_seed_replays_the_same_history() {
    let run = || random_history(5, Limits::default(), 4, 4).0;
    assert_eq!(run(), run());
}

#[test]
fn histories_stay_serializable_when_every_lock_promotes() {
    let limits = Limits {
        tuples_per_page: 1,
        pages_per_heap: 1,
        ranges_per_index: 1,
    };
    for seed in 0..12 {
        let (history, _) = random_history(seed, limits, 4, 6);
        assert_eq!(Ok(vec![]), iso::check(&history), "seed {seed}");
    }
}

#[test]
fn locks_of_finished_transactions_are_released() {
    simulate(6, |e| {
        let rt = &e.rt;
        let w = appender(e, 1, 1);
        let r = Worker::start(e, Kind::ReadWrite);
        r.call(rt, Req::Read(1)).unwrap();
        w.call(rt, Req::Commit).unwrap();
        r.call(rt, Req::Commit).unwrap();
        for t in [w.thread, r.thread] {
            rt.join(t);
        }
        assert_eq!(0, e.tracked());
    });
}
