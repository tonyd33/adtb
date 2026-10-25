use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use adtb::btree::{BTree, Index};
use adtb::heap::{FieldType, Flags, Heap, LargeStore, walk};
use adtb::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use adtb::mvcc::{Scope, Snapshot, SnapshotId, Status, Transactions, Xid};
use adtb::runtime::Entropy;
use adtb::storage::{Options, Pager, Pid};
use adtb::vacuum::{self, Report, Sum};
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};

const PAGE: usize = 4096;
const NAME: &str = "app.adt";
const FATAL: &str = "fatal I/O error";

const EMAIL: usize = 0;
const APPLE: usize = 2;
const HEAP_IDS: [u32; 3] = [10, 11, 12];
const LARGE_ID: u32 = 9;
const INDEX_IDS: [u32; 2] = [20, 21];

fn options() -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames: 16,
        atomic_page_writes: false,
        ..Options::new(NAME)
    }
}

fn quiet() -> Faults {
    Faults {
        lose_unsynced: true,
        delay_completions: true,
        ..Faults::default()
    }
}

fn finished(sim: &mut Sim, body: impl FnOnce(SimRuntime)) {
    assert_eq!(Outcome::Finished, sim.run(body));
}

fn row_type(heap: usize) -> Vec<FieldType> {
    match heap {
        APPLE => vec![
            FieldType::Scalar(Scalar::I64),
            FieldType::Scalar(Scalar::I32),
        ],
        _ => vec![
            FieldType::Scalar(Scalar::I64),
            FieldType::Scalar(Scalar::Text),
            FieldType::Scalar(Scalar::Bytes),
        ],
    }
}

/// The fields of each heap that some index keys.
fn indexed(heap: usize) -> &'static [usize] {
    match heap {
        APPLE => &[0],
        _ => &[0, 1],
    }
}

fn blob(r: u64) -> Vec<u8> {
    let len = if r.is_multiple_of(12) {
        2000 + (r % 6000) as usize
    } else {
        (r % 120) as usize
    };
    (0..len).map(|i| (r as usize + i * 31) as u8).collect()
}

fn make_row(heap: usize, key: i64, r: u64) -> Vec<Value> {
    match heap {
        APPLE => vec![Value::I64(key), Value::I32(r as i32)],
        _ => vec![
            Value::I64(key),
            Value::Text(format!("{key}-{}", r % 4)),
            Value::Bytes(blob(r)),
        ],
    }
}

fn key_of(row: &[Value]) -> i64 {
    match row[0] {
        Value::I64(k) => k,
        ref v => panic!("key field holds {v:?}"),
    }
}

fn schemas() -> [KeySchema; 2] {
    [
        KeySchema::new(vec![(KeyType::Scalar(Scalar::I64), Dir::Asc)]),
        KeySchema::new(vec![(KeyType::Scalar(Scalar::Text), Dir::Asc)]),
    ]
}

fn index_ctors() -> [BTreeSet<CtorId>; 2] {
    [
        BTreeSet::from([CtorId(0), CtorId(1), CtorId(2)]),
        BTreeSet::from([CtorId(0), CtorId(1)]),
    ]
}

/// Returns the index entries of the version at `tid` of heap `heap` holding `row`.
fn entries(heap: usize, row: &[Value], tid: Tid) -> Vec<(usize, Entry)> {
    let ctor = CtorId(heap as u32);
    let mut out = vec![(
        0,
        Entry {
            key: vec![row[0].clone()],
            ctor,
            tid,
        },
    )];
    if heap != APPLE {
        out.push((
            1,
            Entry {
                key: vec![row[1].clone()],
                ctor,
                tid,
            },
        ));
    }
    out
}

#[derive(Clone, Copy, Debug)]
struct Roots {
    clog: Pid,
    large: Pid,
    heaps: [Pid; 3],
    indexes: [Pid; 2],
}

struct Db {
    pager: Pager<SimRuntime>,
    txns: Transactions,
    heaps: Vec<Heap>,
    indexes: Vec<Index>,
}

impl Db {
    fn create(rt: SimRuntime, fanout: u16) -> (Db, Roots) {
        let mut pager = Pager::create(rt, &options()).expect("create");
        let txns = Transactions::create(&mut pager).expect("commit log");
        let large = LargeStore::create(&mut pager, LARGE_ID).expect("large store");
        let heaps: Vec<Heap> = (0..3)
            .map(|h| {
                Heap::create_with_fanout(&mut pager, HEAP_IDS[h], row_type(h), large, fanout)
                    .expect("heap")
            })
            .collect();
        let [s0, s1] = schemas();
        let [c0, c1] = index_ctors();
        let indexes = vec![
            Index::create(&mut pager, INDEX_IDS[0], s0, c0).expect("index"),
            Index::create(&mut pager, INDEX_IDS[1], s1, c1).expect("index"),
        ];
        let roots = Roots {
            clog: txns.commit_log().root(),
            large: large.root(),
            heaps: [heaps[0].root(), heaps[1].root(), heaps[2].root()],
            indexes: [indexes[0].tree().root(), indexes[1].tree().root()],
        };
        pager.flush_log();
        (
            Db {
                pager,
                txns,
                heaps,
                indexes,
            },
            roots,
        )
    }

    fn open(rt: SimRuntime, roots: Roots) -> Option<Db> {
        let (mut pager, _) = Pager::open(rt, &options()).ok()?;
        let txns = Transactions::open(&mut pager, roots.clog).ok()?;
        let large = LargeStore::open(LARGE_ID, roots.large);
        let heaps = (0..3)
            .map(|h| Heap::open(&pager, HEAP_IDS[h], roots.heaps[h], row_type(h), large).unwrap())
            .collect();
        let [s0, s1] = schemas();
        let [c0, c1] = index_ctors();
        let indexes = vec![
            Index::open(BTree::open(roots.indexes[0], INDEX_IDS[0]), s0, c0),
            Index::open(BTree::open(roots.indexes[1], INDEX_IDS[1]), s1, c1),
        ];
        Some(Db {
            pager,
            txns,
            heaps,
            indexes,
        })
    }

    fn vacuum(&mut self, budget: usize) -> Report {
        let Db {
            pager,
            txns,
            heaps,
            indexes,
        } = self;
        let mut sum = Sum {
            heaps: heaps
                .iter_mut()
                .enumerate()
                .map(|(i, h)| (CtorId(i as u32), h))
                .collect(),
            indexes: indexes.iter().collect(),
        };
        vacuum::vacuum(pager, txns, &mut sum, budget).expect("vacuum")
    }

    fn prune(&mut self, heap: usize, pid: Pid) {
        vacuum::prune(&mut self.pager, &self.txns, &mut self.heaps[heap], pid).expect("prune");
    }

    /// Returns what the heaps, the store, the indexes and the routing between them report wrong.
    fn verify(&mut self) -> Vec<String> {
        let mut problems = Vec::new();
        for heap in &self.heaps {
            problems.extend(heap.verify(&mut self.pager).expect("heap verify"));
        }
        problems.extend(
            self.heaps[0]
                .large()
                .verify(&mut self.pager)
                .expect("store verify"),
        );
        for (i, index) in self.indexes.iter().enumerate() {
            if let Err(e) = index.tree().verify(&mut self.pager) {
                problems.push(format!("index {i}: {e:?}"));
            }
        }
        let Db {
            pager,
            heaps,
            indexes,
            ..
        } = self;
        let sum = Sum {
            heaps: heaps
                .iter_mut()
                .enumerate()
                .map(|(i, h)| (CtorId(i as u32), h))
                .collect(),
            indexes: indexes.iter().collect(),
        };
        problems.extend(vacuum::verify(pager, &sum).expect("vacuum verify"));
        problems
    }

    fn index(&mut self, heap: usize, row: &[Value], tid: Tid) {
        for (i, e) in entries(heap, row, tid) {
            assert!(self.indexes[i].insert(&mut self.pager, &e).expect("index"));
        }
    }

    /// Returns the rows `snapshot` sees in each heap of `heaps` by scanning them, by key.
    fn scan_visible(&mut self, snapshot: &Snapshot, heaps: &[usize]) -> Visible {
        let mut versions = Vec::new();
        for &h in heaps {
            self.heaps[h]
                .scan(&mut self.pager, |v| versions.push((h, v)))
                .expect("scan");
        }
        let mut out = Visible::default();
        for (h, v) in versions {
            let seen = self
                .txns
                .sees(
                    &mut self.pager,
                    snapshot,
                    None,
                    v.header.xmin,
                    v.header.xmax,
                )
                .expect("sees");
            if seen {
                out.add(key_of(&v.row), h, v.row, v.tid);
            }
        }
        out
    }

    /// Returns the rows `snapshot` sees in each heap of `heaps` through the key index: each entry's
    /// version or the heap-only versions after it on its page.
    fn index_visible(&mut self, snapshot: &Snapshot, heaps: &[usize]) -> Visible {
        let all = self.indexes[0]
            .range(&mut self.pager, Bound::Unbounded, Bound::Unbounded)
            .expect("index scan");
        let mut out = Visible::default();
        for e in all {
            let h = e.ctor.0 as usize;
            if !heaps.contains(&h) {
                continue;
            }
            let found = self.chain_visible(snapshot, e.tid);
            if let Some(tid) = found {
                let v = self.heaps[h]
                    .read(&mut self.pager, tid)
                    .expect("read")
                    .expect("a visible version");
                out.add(key_of(&v.row), h, v.row, tid);
            }
        }
        out
    }

    /// Returns the version `snapshot` sees among the one at `tid` and its heap-only successors on
    /// its page.
    fn chain_visible(&mut self, snapshot: &Snapshot, tid: Tid) -> Option<Tid> {
        let Db { pager, txns, .. } = self;
        let mut seen = None;
        let mut first = true;
        let mut checks = Vec::new();
        walk(pager, tid, |l| {
            if l.tid.page() != tid.page() || !first && !l.flags.contains(Flags::HEAP_ONLY) {
                return true;
            }
            first = false;
            checks.push((l.tid, l.header.xmin, l.header.xmax));
            false
        })
        .expect("walk");
        for (t, xmin, xmax) in checks {
            if txns.sees(pager, snapshot, None, xmin, xmax).expect("sees") {
                seen = Some(t);
                break;
            }
        }
        seen
    }

    /// Returns what is wrong with the all-visible bits: a page marked all-visible holding a
    /// version some snapshot of `held`, or a fresh one, does not see.
    fn check_visibility_map(&mut self, held: &[Held]) -> Vec<String> {
        let (fresh_id, fresh) = self.txns.take(Scope::Every);
        let mut problems = Vec::new();
        for h in 0..3 {
            let mut versions = Vec::new();
            self.heaps[h]
                .scan(&mut self.pager, |v| versions.push(v))
                .expect("scan");
            for v in versions {
                let pid = Pid(v.tid.page());
                if !self.heaps[h].all_visible(&mut self.pager, pid).unwrap() {
                    continue;
                }
                let readers = held
                    .iter()
                    .filter(|s| s.heaps.contains(&h))
                    .map(|s| &s.snapshot)
                    .chain([&fresh]);
                for s in readers {
                    let sees = self
                        .txns
                        .sees(&mut self.pager, s, None, v.header.xmin, v.header.xmax)
                        .unwrap();
                    if !sees {
                        problems.push(format!(
                            "{pid:?} of heap {h} is all-visible, but {:?} does not see {:?}",
                            s, v.tid
                        ));
                    }
                }
            }
        }
        self.txns.release(fresh_id);
        problems
    }

    fn fresh_visible(&mut self) -> (Visible, Visible) {
        let (id, s) = self.txns.take(Scope::Every);
        let scan = self.scan_visible(&s, &[0, 1, 2]);
        let index = self.index_visible(&s, &[0, 1, 2]);
        self.txns.release(id);
        (scan, index)
    }
}

/// The rows one snapshot sees, by key, and keys seen twice.
#[derive(Debug, Default, PartialEq, Eq)]
struct Visible {
    rows: BTreeMap<i64, (usize, Vec<Value>)>,
    tids: BTreeMap<i64, Tid>,
    twice: Vec<i64>,
}

impl Visible {
    fn add(&mut self, key: i64, heap: usize, row: Vec<Value>, tid: Tid) {
        if self.rows.insert(key, (heap, row)).is_some() {
            self.twice.push(key);
        }
        self.tids.insert(key, tid);
    }

    fn rows(&self) -> &BTreeMap<i64, (usize, Vec<Value>)> {
        &self.rows
    }
}

struct Held {
    id: SnapshotId,
    snapshot: Snapshot,
    heaps: Vec<usize>,
    expected: BTreeMap<i64, (usize, Vec<Value>)>,
}

#[derive(Clone, Debug)]
enum Op {
    Insert {
        key: i64,
        heap: usize,
        row: Vec<Value>,
    },
    Update {
        key: i64,
        row: Vec<Value>,
    },
    Delete {
        key: i64,
    },
    Retag {
        key: i64,
        to: usize,
        row: Vec<Value>,
    },
}

impl Op {
    fn key(&self) -> i64 {
        match self {
            Op::Insert { key, .. }
            | Op::Update { key, .. }
            | Op::Delete { key }
            | Op::Retag { key, .. } => *key,
        }
    }
}

/// Each key's heap, the TID of its newest version and its row.
type Live = BTreeMap<i64, (usize, Tid, Vec<Value>)>;

fn generate(
    next: &mut impl FnMut() -> u64,
    live: &Live,
    locked: &BTreeSet<i64>,
    next_key: &mut i64,
) -> Op {
    let r = next();
    let free: Vec<i64> = live
        .keys()
        .filter(|k| !locked.contains(k))
        .copied()
        .collect();
    if free.is_empty() || r % 10 < 3 {
        let key = *next_key;
        *next_key += 1;
        let heap = (next() % 3) as usize;
        return Op::Insert {
            key,
            heap,
            row: make_row(heap, key, next()),
        };
    }
    let key = free[(next() % free.len() as u64) as usize];
    let (heap, _, old) = &live[&key];
    match r % 10 {
        3..=6 => {
            let mut row = make_row(*heap, key, next());
            if *heap != APPLE && !next().is_multiple_of(3) {
                row[1] = old[1].clone();
            }
            Op::Update { key, row }
        }
        7 => Op::Delete { key },
        _ => {
            let to = (heap + 1 + (next() % 2) as usize) % 3;
            let mut row = make_row(to, key, next());
            if to != APPLE && *heap != APPLE && next().is_multiple_of(2) {
                row[2] = old[2].clone();
            }
            Op::Retag { key, to, row }
        }
    }
}

fn perform(db: &mut Db, live: &mut Live, op: &Op, xid: Xid) {
    let x = xid.0;
    match op {
        Op::Insert { key, heap, row } => {
            let tid = db.heaps[*heap]
                .insert(&mut db.pager, x, row)
                .expect("insert");
            db.index(*heap, row, tid);
            live.insert(*key, (*heap, tid, row.clone()));
        }
        Op::Update { key, row } => {
            let (heap, tid, _) = live[key].clone();
            let u = db.heaps[heap]
                .update(&mut db.pager, tid, x, row, indexed(heap))
                .expect("update");
            if !u.heap_only {
                db.index(heap, row, u.tid);
            }
            live.insert(*key, (heap, u.tid, row.clone()));
        }
        Op::Delete { key } => {
            let (heap, tid, _) = live.remove(key).unwrap();
            db.heaps[heap]
                .delete(&mut db.pager, tid, x)
                .expect("delete");
        }
        Op::Retag { key, to, row } => {
            let (heap, tid, _) = live[key].clone();
            let (from, into) = pair(&mut db.heaps, heap, *to);
            let t = from.retag(&mut db.pager, tid, x, into, row).expect("retag");
            db.index(*to, row, t);
            live.insert(*key, (*to, t, row.clone()));
        }
    }
}

fn pair(heaps: &mut [Heap], a: usize, b: usize) -> (&mut Heap, &mut Heap) {
    assert_ne!(a, b);
    if a < b {
        let (l, r) = heaps.split_at_mut(b);
        (&mut l[a], &mut r[0])
    } else {
        let (l, r) = heaps.split_at_mut(a);
        (&mut r[0], &mut l[b])
    }
}

fn replay(state: &mut BTreeMap<i64, (usize, Vec<Value>)>, op: &Op) {
    match op {
        Op::Insert { key, heap, row } => {
            state.insert(*key, (*heap, row.clone()));
        }
        Op::Update { key, row } => state.get_mut(key).unwrap().1 = row.clone(),
        Op::Delete { key } => {
            state.remove(key);
        }
        Op::Retag { key, to, row } => {
            state.insert(*key, (*to, row.clone()));
        }
    }
}

/// A transaction left running across other work.
struct Running {
    xid: Xid,
    live: Live,
    keys: BTreeSet<i64>,
    ops: Vec<Op>,
}

/// The client's side of a workload: the committed newest versions and every transaction's
/// operations.
struct Client {
    live: Live,
    next_key: i64,
    ops: BTreeMap<Xid, Vec<Op>>,
    long: Option<Running>,
    /// Transactions committed and not yet acknowledged.
    done: Vec<Xid>,
}

impl Client {
    fn new(live: Live, next_key: i64) -> Client {
        Client {
            live,
            next_key,
            ops: BTreeMap::new(),
            long: None,
            done: Vec::new(),
        }
    }
}

impl Client {
    fn locked(&self) -> BTreeSet<i64> {
        self.long
            .as_ref()
            .map(|l| l.keys.clone())
            .unwrap_or_default()
    }

    /// Begin a transaction and perform up to `n` operations in it.
    fn start(&mut self, db: &mut Db, next: &mut impl FnMut() -> u64, n: u64) -> Running {
        let xid = db.txns.begin(&mut db.pager).expect("begin");
        let mut run = Running {
            xid,
            live: self.live.clone(),
            keys: BTreeSet::new(),
            ops: Vec::new(),
        };
        let locked = self.locked();
        for _ in 0..1 + next() % n {
            let op = generate(next, &run.live, &locked, &mut self.next_key);
            perform(db, &mut run.live, &op, xid);
            run.keys.insert(op.key());
            run.ops.push(op);
        }
        self.ops.insert(xid, run.ops.clone());
        run
    }

    fn finish(&mut self, db: &mut Db, run: Running, commit: bool) {
        if commit {
            db.txns.commit(&mut db.pager, run.xid).expect("commit");
            self.done.push(run.xid);
            for k in &run.keys {
                match run.live.get(k) {
                    Some(v) => self.live.insert(*k, v.clone()),
                    None => self.live.remove(k),
                };
            }
        } else {
            db.txns.abort(&mut db.pager, run.xid).expect("abort");
            self.ops.remove(&run.xid);
        }
    }

    /// Run one step of the workload.
    fn step(&mut self, db: &mut Db, next: &mut impl FnMut() -> u64) {
        match next() % 16 {
            0..=10 => {
                let run = self.start(db, next, 3);
                let commit = !next().is_multiple_of(7);
                self.finish(db, run, commit);
            }
            11 if self.long.is_none() => {
                let run = self.start(db, next, 4);
                self.long = Some(run);
            }
            12 => {
                if let Some(run) = self.long.take() {
                    let commit = !next().is_multiple_of(3);
                    self.finish(db, run, commit);
                }
            }
            _ => {}
        }
    }

    /// Returns the rows of the committed transactions, replayed in id order.
    fn expected(&self, committed: impl Fn(Xid) -> bool) -> BTreeMap<i64, (usize, Vec<Value>)> {
        let mut state = BTreeMap::new();
        for (x, ops) in &self.ops {
            if committed(*x) {
                for op in ops {
                    replay(&mut state, op);
                }
            }
        }
        state
    }
}

fn take(db: &mut Db, next: &mut impl FnMut() -> u64) -> Held {
    let heaps: Vec<usize> = if next().is_multiple_of(2) {
        vec![0, 1, 2]
    } else {
        let one = (next() % 3) as usize;
        vec![one]
    };
    let scope = if heaps.len() == 3 {
        Scope::Every
    } else {
        Scope::Heaps(heaps.iter().map(|&h| HEAP_IDS[h]).collect())
    };
    let (id, snapshot) = db.txns.take(scope);
    let expected = db.scan_visible(&snapshot, &heaps).rows;
    Held {
        id,
        snapshot,
        heaps,
        expected,
    }
}

/// Returns what is wrong after a vacuum: a held snapshot that no longer sees what it saw, by
/// scan or by index, a wrong all-visible bit, a structural problem, or committed rows lost.
fn check(db: &mut Db, held: &[Held], client: &Client) -> Vec<String> {
    let mut problems = db.verify();
    for h in held {
        let scan = db.scan_visible(&h.snapshot, &h.heaps);
        let index = db.index_visible(&h.snapshot, &h.heaps);
        if scan.rows() != &h.expected {
            problems.push(format!(
                "{:?} over {:?} lost rows by scan",
                h.snapshot, h.heaps
            ));
        }
        if index.rows() != &h.expected {
            problems.push(format!(
                "{:?} over {:?} sees other rows by index",
                h.snapshot, h.heaps
            ));
        }
    }
    problems.extend(db.check_visibility_map(held));
    let running: Option<Xid> = client.long.as_ref().map(|l| l.xid);
    let expected = client.expected(|x| Some(x) != running);
    let (scan, index) = db.fresh_visible();
    if scan.rows() != &expected || !scan.twice.is_empty() {
        problems.push("a fresh snapshot's scan disagrees with the model".into());
    }
    if index.rows() != &expected || !index.twice.is_empty() {
        problems.push("a fresh snapshot's index read disagrees with the model".into());
    }
    problems
}

#[test]
fn vacuum_keeps_what_live_snapshots_see_and_cleans_the_rest() {
    for seed in 0..4 {
        let mut sim = Sim::new(seed, Config::default());
        finished(&mut sim, |rt| {
            let (mut db, _) = Db::create(rt.clone(), 4);
            let mut client = Client::new(Live::new(), 0);
            let mut held: Vec<Held> = Vec::new();
            let mut next = || rt.next_u64();
            let mut total = db.vacuum(usize::MAX);
            let mut most_rounds = 0;
            for step in 0..1200 {
                match next() % 20 {
                    0..=13 => client.step(&mut db, &mut next),
                    14 if held.len() < 4 => held.push(take(&mut db, &mut next)),
                    15 if !held.is_empty() => {
                        let h = held.swap_remove((next() % held.len() as u64) as usize);
                        db.txns.release(h.id);
                    }
                    16 => {
                        let budget = 64 + (next() % 2048) as usize;
                        let r = db.vacuum(budget);
                        total.freed += r.freed;
                        total.entries += r.entries;
                        total.cleared += r.cleared;
                        total.all_visible += r.all_visible;
                        total.removed += r.removed;
                        total.chunks += r.chunks;
                        total.skipped += r.skipped;
                        total.rounds += r.rounds;
                        most_rounds = most_rounds.max(r.rounds);
                        let problems = check(&mut db, &held, &client);
                        assert!(
                            problems.is_empty(),
                            "seed {seed} step {step}: {problems:#?}"
                        );
                    }
                    17 => {
                        let h = (next() % 3) as usize;
                        let pages = db.heaps[h].pages(&mut db.pager).unwrap();
                        if !pages.is_empty() {
                            let pid = pages[(next() % pages.len() as u64) as usize];
                            db.prune(h, pid);
                        }
                    }
                    _ => {}
                }
            }
            for h in held.drain(..) {
                db.txns.release(h.id);
            }
            if let Some(run) = client.long.take() {
                client.finish(&mut db, run, true);
            }
            let last = db.vacuum(usize::MAX);
            let problems = check(&mut db, &[], &client);
            assert!(problems.is_empty(), "seed {seed} at the end: {problems:#?}");
            let mut writers = Vec::new();
            for heap in &db.heaps {
                heap.scan(&mut db.pager, |v| {
                    writers.extend([v.header.xmin, v.header.xmax]);
                })
                .unwrap();
            }
            for x in writers.into_iter().filter(|&x| x != 0) {
                let aborted = db.txns.status(&mut db.pager, Xid(x)).unwrap() == Status::Aborted;
                assert!(
                    !aborted || Xid(x) >= last.clean_below,
                    "a version names aborted {x}"
                );
            }
            eprintln!("seed {seed}: {total:?}");
            assert!(total.freed > 100, "{total:?}");
            assert!(total.entries > 100, "{total:?}");
            assert!(total.cleared > 0, "{total:?}");
            assert!(total.all_visible > 0 && total.skipped > 0, "{total:?}");
            assert!(most_rounds > 1, "no vacuum filled its budget: {total:?}");
            assert!(total.chunks > 0, "{total:?}");
        });
    }
}

#[test]
fn a_heap_only_chain_keeps_its_first_slot_as_a_redirect() {
    let mut sim = Sim::new(31, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let pager = &mut db.pager;
        let row = |n: i32| {
            vec![
                Value::I64(1),
                Value::Text("a".into()),
                Value::Bytes(vec![n as u8; 8]),
            ]
        };
        let x = db.txns.begin(pager).unwrap();
        let first = db.heaps[EMAIL].insert(pager, x.0, &row(0)).unwrap();
        db.index(EMAIL, &row(0), first);
        let pager = &mut db.pager;
        db.txns.commit(pager, x).unwrap();
        let mut at = first;
        for n in 1..=3 {
            let x = db.txns.begin(pager).unwrap();
            let u = db.heaps[EMAIL]
                .update(pager, at, x.0, &row(n), &[0, 1])
                .unwrap();
            assert!(u.heap_only);
            at = u.tid;
            db.txns.commit(pager, x).unwrap();
        }
        let report = db.vacuum(usize::MAX);
        assert_eq!(0, report.entries, "{report:?}");
        assert_eq!(2, report.freed, "{report:?}");
        assert_eq!(1, report.all_visible, "{report:?}");
        let pager = &mut db.pager;
        let found = walk(pager, first, |_| true).unwrap().unwrap();
        assert_eq!(at, found.tid);
        assert_eq!(None, db.heaps[EMAIL].read(pager, first).unwrap());
        assert!(db.heaps[EMAIL].all_visible(pager, Pid(at.page())).unwrap());
        assert_eq!(Vec::<String>::new(), db.verify());
        let (scan, index) = db.fresh_visible();
        assert_eq!(scan, index);
        assert_eq!(Some(&(EMAIL, row(3))), scan.rows().get(&1));
    });
}

#[test]
fn a_write_clears_the_all_visible_bit() {
    let mut sim = Sim::new(32, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let x = db.txns.begin(&mut db.pager).unwrap();
        let mut tids = Vec::new();
        for key in 0..40 {
            let row = make_row(APPLE, key, key as u64);
            let tid = db.heaps[APPLE].insert(&mut db.pager, x.0, &row).unwrap();
            db.index(APPLE, &row, tid);
            tids.push(tid);
        }
        db.txns.commit(&mut db.pager, x).unwrap();
        db.vacuum(usize::MAX);
        let pid = Pid(tids[7].page());
        assert!(db.heaps[APPLE].all_visible(&mut db.pager, pid).unwrap());
        assert_eq!(1, db.vacuum(usize::MAX).skipped);
        let x = db.txns.begin(&mut db.pager).unwrap();
        db.heaps[APPLE].delete(&mut db.pager, tids[7], x.0).unwrap();
        assert!(!db.heaps[APPLE].all_visible(&mut db.pager, pid).unwrap());
        assert_eq!(Vec::<String>::new(), db.verify());
        db.txns.abort(&mut db.pager, x).unwrap();
        let report = db.vacuum(usize::MAX);
        assert_eq!((1, 1), (report.cleared, report.all_visible), "{report:?}");
        assert!(db.heaps[APPLE].all_visible(&mut db.pager, pid).unwrap());
        assert_eq!(Vec::<String>::new(), db.verify());
    });
}

#[test]
fn empty_pages_leave_the_heap_and_their_positions_are_reused() {
    let mut sim = Sim::new(33, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 3);
        let row = |key: i64| {
            vec![
                Value::I64(key),
                Value::Text(String::new()),
                Value::Bytes(vec![key as u8; 900]),
            ]
        };
        let x = db.txns.begin(&mut db.pager).unwrap();
        let mut tids = Vec::new();
        for key in 0..60 {
            let tid = db.heaps[EMAIL]
                .insert(&mut db.pager, x.0, &row(key))
                .unwrap();
            db.index(EMAIL, &row(key), tid);
            tids.push(tid);
        }
        db.txns.commit(&mut db.pager, x).unwrap();
        let before = db.heaps[EMAIL].pages(&mut db.pager).unwrap();
        assert!(before.len() > 10, "{} pages", before.len());
        let x = db.txns.begin(&mut db.pager).unwrap();
        for (key, tid) in tids.iter().enumerate() {
            if key % 20 != 0 {
                db.heaps[EMAIL].delete(&mut db.pager, *tid, x.0).unwrap();
            }
        }
        db.txns.commit(&mut db.pager, x).unwrap();
        let report = db.vacuum(usize::MAX);
        let after = db.heaps[EMAIL].pages(&mut db.pager).unwrap();
        assert_eq!(before.len() - after.len(), report.removed, "{report:?}");
        assert!(after.len() <= 3, "{} pages left", after.len());
        assert_eq!(Vec::<String>::new(), db.verify());
        let x = db.txns.begin(&mut db.pager).unwrap();
        for key in 100..160 {
            let tid = db.heaps[EMAIL]
                .insert(&mut db.pager, x.0, &row(key))
                .unwrap();
            db.index(EMAIL, &row(key), tid);
        }
        db.txns.commit(&mut db.pager, x).unwrap();
        let refilled = db.heaps[EMAIL].pages(&mut db.pager).unwrap();
        assert!(
            refilled.len() >= before.len() - 3,
            "{} pages",
            refilled.len()
        );
        assert_eq!(Vec::<String>::new(), db.verify());
        let (scan, index) = db.fresh_visible();
        assert_eq!(scan, index);
        assert_eq!(63, scan.rows().len());
    });
}

#[test]
fn an_aborted_large_value_leaves_no_chunk_behind() {
    let mut sim = Sim::new(34, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 64);
        let large = db.heaps[EMAIL].large();
        let big: Vec<u8> = (0..50_000u32).map(|i| (i * 7 + i / 300) as u8).collect();
        let row = vec![Value::I64(1), Value::Text("a".into()), Value::Bytes(big)];
        let x = db.txns.begin(&mut db.pager).unwrap();
        let tid = db.heaps[EMAIL].insert(&mut db.pager, x.0, &row).unwrap();
        db.index(EMAIL, &row, tid);
        assert!(large.pages(&mut db.pager).unwrap().len() >= 50_000 / PAGE);
        db.txns.abort(&mut db.pager, x).unwrap();
        let report = db.vacuum(usize::MAX);
        assert!(report.chunks >= 50_000 / PAGE, "{report:?}");
        assert_eq!(2, report.entries, "{report:?}");
        assert!(large.pages(&mut db.pager).unwrap().is_empty());
        assert!(db.heaps[EMAIL].pages(&mut db.pager).unwrap().is_empty());
        assert_eq!(Vec::<String>::new(), db.verify());
    });
}

fn counter_type(note: bool) -> Vec<FieldType> {
    let mut t = vec![
        FieldType::Scalar(Scalar::I64),
        FieldType::Scalar(Scalar::I32),
    ];
    if note {
        t.push(FieldType::Scalar(Scalar::Text));
    }
    t
}

#[test]
fn vacuum_reopens_a_page_conversion_left_full() {
    let mut sim = Sim::new(35, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let pager = &mut db.pager;
        let large = db.heaps[EMAIL].large();
        let mut heap = Heap::create(pager, 31, counter_type(false), large).unwrap();
        let x = db.txns.begin(pager).unwrap();
        let mut old: Vec<Tid> = Vec::new();
        for k in 0.. {
            let t = heap
                .insert(pager, x.0, &[Value::I64(k), Value::I32(0)])
                .unwrap();
            if !old.is_empty() && t.page() != old[0].page() {
                break;
            }
            old.push(t);
        }
        db.txns.commit(pager, x).unwrap();
        let first = old[0].page();
        heap.add_format(
            counter_type(true),
            Box::new(|mut row| {
                row.push(Value::Text("z".repeat(60)));
                row
            }),
        )
        .unwrap();
        let x = db.txns.begin(pager).unwrap();
        let fresh = |k: i64| [Value::I64(k), Value::I32(1), Value::Text(String::new())];
        let t = heap.insert(pager, x.0, &fresh(1000)).unwrap();
        assert_ne!(first, t.page());
        for t in &old[..old.len() * 3 / 4] {
            heap.delete(pager, *t, x.0).unwrap();
        }
        db.txns.commit(pager, x).unwrap();
        let w = db.txns.watermark(31);
        let mut freed = 0;
        for pid in heap.pages(pager).unwrap() {
            let slots = heap
                .dead_slots(pager, pid, |p, h| vacuum::judge(&db.txns, p, w, h))
                .unwrap();
            freed += heap
                .prune(
                    pager,
                    pid,
                    |p, h| vacuum::judge(&db.txns, p, w, h),
                    &slots,
                    true,
                )
                .unwrap()
                .freed;
        }
        assert!(freed >= old.len() * 3 / 4, "{freed} freed");
        assert_eq!(Vec::<String>::new(), heap.verify(pager).unwrap());
        let mut heap = Heap::open(pager, 31, heap.root(), counter_type(false), large).unwrap();
        heap.add_format(
            counter_type(true),
            Box::new(|mut row| {
                row.push(Value::Text("z".repeat(60)));
                row
            }),
        )
        .unwrap();
        let x = db.txns.begin(pager).unwrap();
        let t = heap.insert(pager, x.0, &fresh(1001)).unwrap();
        assert_eq!(first, t.page());
        db.txns.commit(pager, x).unwrap();
        let v = heap.read(pager, old[old.len() - 1]).unwrap().unwrap();
        assert_eq!(Value::Text("z".repeat(60)), v.row[2]);
        assert_eq!(Vec::<String>::new(), heap.verify(pager).unwrap());
    });
}

#[test]
fn a_chain_ends_where_vacuum_freed_the_successor() {
    let mut sim = Sim::new(36, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, 4);
        let row = vec![Value::I64(1), Value::I32(0)];
        let x = db.txns.begin(&mut db.pager).unwrap();
        let t = db.heaps[APPLE].insert(&mut db.pager, x.0, &row).unwrap();
        db.index(APPLE, &row, t);
        db.txns.commit(&mut db.pager, x).unwrap();
        let (old_id, _) = db
            .txns
            .take(Scope::Heaps(BTreeSet::from([HEAP_IDS[APPLE]])));
        let x = db.txns.begin(&mut db.pager).unwrap();
        let (apple, email) = pair(&mut db.heaps, APPLE, EMAIL);
        let moved_row = vec![Value::I64(1), Value::Text("m".into()), Value::Bytes(vec![])];
        let moved = apple
            .retag(&mut db.pager, t, x.0, email, &moved_row)
            .unwrap();
        db.index(EMAIL, &moved_row, moved);
        db.txns.commit(&mut db.pager, x).unwrap();
        let x = db.txns.begin(&mut db.pager).unwrap();
        db.heaps[EMAIL].delete(&mut db.pager, moved, x.0).unwrap();
        db.txns.commit(&mut db.pager, x).unwrap();
        let report = db.vacuum(usize::MAX);
        assert!(report.freed >= 1, "{report:?}");
        assert!(db.heaps[APPLE].read(&mut db.pager, t).unwrap().is_some());
        assert!(!db.heaps[EMAIL].holds(&mut db.pager, moved).unwrap());
        let x = db.txns.begin(&mut db.pager).unwrap();
        let other = vec![Value::I64(2), Value::Text("o".into()), Value::Bytes(vec![])];
        let reused = db.heaps[EMAIL].insert(&mut db.pager, x.0, &other).unwrap();
        db.index(EMAIL, &other, reused);
        db.txns.commit(&mut db.pager, x).unwrap();
        assert_eq!(moved, reused, "the freed slot is taken again");
        assert_eq!(None, walk(&mut db.pager, t, |l| l.tid == reused).unwrap());
        db.txns.release(old_id);
        assert_eq!(Vec::<String>::new(), db.verify());
    });
}

fn silence_fatal_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let msg = info
                .payload()
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| info.payload().downcast_ref::<&str>().copied())
                .unwrap_or("");
            if !msg.starts_with(FATAL) {
                default(info);
            }
        }));
    });
}

fn run_process<F: FnOnce(SimRuntime)>(sim: &mut Sim, body: F) -> Outcome {
    match panic::catch_unwind(AssertUnwindSafe(|| sim.run(body))) {
        Ok(o) => o,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_default();
            if msg.starts_with(FATAL) {
                Outcome::Crashed
            } else {
                panic::resume_unwind(payload)
            }
        }
    }
}

/// What the client knows, kept outside the simulated process so it survives crashes.
struct World {
    roots: Roots,
    next_key: i64,
    ops: BTreeMap<Xid, Vec<Op>>,
    acked: BTreeSet<Xid>,
    vacuums: usize,
    vacuuming: bool,
    crashed_vacuuming: usize,
}

fn workload(rt: SimRuntime, world: &Mutex<World>, steps: usize) {
    let roots = world.lock().unwrap().roots;
    let Some(mut db) = Db::open(rt.clone(), roots) else {
        return;
    };
    let (scan, _) = db.fresh_visible();
    let live = scan
        .rows
        .iter()
        .map(|(k, (h, row))| (*k, (*h, scan.tids[k], row.clone())))
        .collect();
    let mut client = Client::new(live, world.lock().unwrap().next_key);
    let mut held: Vec<Held> = Vec::new();
    let mut next = || rt.next_u64();
    let mut unacked: Vec<Xid> = Vec::new();
    for _ in 0..steps {
        match next() % 24 {
            0..=14 => {
                client.step(&mut db, &mut next);
                unacked.append(&mut client.done);
                let mut w = world.lock().unwrap();
                w.next_key = client.next_key;
                for (x, ops) in &client.ops {
                    w.ops.insert(*x, ops.clone());
                }
            }
            15 if held.len() < 3 => held.push(take(&mut db, &mut next)),
            16 if !held.is_empty() => {
                let h = held.swap_remove((next() % held.len() as u64) as usize);
                db.txns.release(h.id);
            }
            17..=19 => {
                {
                    let mut w = world.lock().unwrap();
                    w.vacuums += 1;
                    w.vacuuming = true;
                }
                db.vacuum(64 + (next() % 1024) as usize);
                world.lock().unwrap().vacuuming = false;
            }
            20 => {
                db.pager.flush_pages(1 + (next() % 6) as usize);
            }
            21 => db.pager.checkpoint(),
            _ => {
                db.pager.flush_log();
                let mut w = world.lock().unwrap();
                w.acked.extend(unacked.drain(..));
            }
        }
    }
    db.pager.flush_log();
    world.lock().unwrap().acked.extend(unacked);
}

/// Reopen the database, check it against every committed transaction, vacuum it and check again.
fn inspect(rt: SimRuntime, world: &Mutex<World>) -> Vec<String> {
    let roots = world.lock().unwrap().roots;
    let Some(mut db) = Db::open(rt, roots) else {
        return vec!["open failed".into()];
    };
    let v = db.pager.verify();
    let mut problems: Vec<String> = v.findings.iter().map(|f| format!("{f:?}")).collect();
    let w = world.lock().unwrap();
    let mut committed = BTreeSet::new();
    for x in w.ops.keys() {
        if db.txns.status(&mut db.pager, *x).unwrap() == Status::Committed {
            committed.insert(*x);
        }
    }
    for x in w.acked.iter().filter(|x| !committed.contains(x)) {
        problems.push(format!("acknowledged {x:?} did not commit"));
    }
    let mut client = Client::new(Live::new(), 0);
    client.ops = w.ops.clone();
    drop(w);
    let expected = client.expected(|x| committed.contains(&x));
    for round in ["recovered", "vacuumed"] {
        problems.extend(db.verify().into_iter().map(|p| format!("{round}: {p}")));
        let (scan, index) = db.fresh_visible();
        if scan.rows() != &expected || !scan.twice.is_empty() {
            problems.push(format!(
                "{round}: the scan disagrees with the committed rows"
            ));
        }
        if index.rows() != &expected || !index.twice.is_empty() {
            problems.push(format!(
                "{round}: the index disagrees with the committed rows"
            ));
        }
        problems.extend(db.check_visibility_map(&[]));
        db.vacuum(usize::MAX);
    }
    problems
}

/// Returns the number of crashes, and of those that fell during a vacuum.
fn crash_loop(seed: u64, faults: Faults, rounds: usize) -> (usize, usize) {
    silence_fatal_panics();
    let mut sim = Sim::new(
        seed,
        Config {
            faults: quiet(),
            ..Config::default()
        },
    );
    let mut roots = None;
    finished(&mut sim, |rt| {
        let (db, r) = Db::create(rt, 3);
        db.pager.checkpoint();
        roots = Some(r);
    });
    let world = Arc::new(Mutex::new(World {
        roots: roots.unwrap(),
        next_key: 0,
        ops: BTreeMap::new(),
        acked: BTreeSet::new(),
        vacuums: 0,
        vacuuming: false,
        crashed_vacuuming: 0,
    }));
    let mut crashes = 0;
    for round in 0..rounds {
        sim.set_faults(faults.clone());
        let at = sim.now()
            + Duration::from_micros(2000 + (seed * 7919 + round as u64 * 104_729) % 80_000);
        sim.crash_at(at);
        let w = world.clone();
        if run_process(&mut sim, |rt| workload(rt, &w, 300)) == Outcome::Crashed {
            crashes += 1;
            let mut w = world.lock().unwrap();
            w.crashed_vacuuming += usize::from(w.vacuuming);
            w.vacuuming = false;
        }
        sim.set_faults(quiet());
        let mut problems = Vec::new();
        let w = world.clone();
        finished(&mut sim, |rt| problems = inspect(rt, &w));
        assert!(
            problems.is_empty(),
            "seed {seed} round {round}: {problems:#?}"
        );
    }
    let w = world.lock().unwrap();
    eprintln!(
        "seed {seed}: {crashes} crashes, {} during vacuum, {} transactions, {} acknowledged, {} vacuums",
        w.crashed_vacuuming,
        w.ops.len(),
        w.acked.len(),
        w.vacuums
    );
    assert!(!w.acked.is_empty(), "seed {seed} acknowledged nothing");
    (crashes, w.crashed_vacuuming)
}

#[test]
fn crashes_during_vacuum_recover_to_a_clean_database() {
    let (crashes, during): (Vec<usize>, Vec<usize>) = (0..8)
        .map(|seed| {
            crash_loop(
                seed,
                Faults {
                    torn_p: 0.5,
                    ..quiet()
                },
                4,
            )
        })
        .unzip();
    let (crashes, during) = (crashes.iter().sum::<usize>(), during.iter().sum::<usize>());
    assert!(
        crashes > 10 && during > 3,
        "{crashes} crashes, {during} during vacuum"
    );
}

#[test]
fn failed_writes_during_vacuum_crash_and_recover_clean() {
    let (crashes, during): (Vec<usize>, Vec<usize>) = (100..106)
        .map(|seed| {
            crash_loop(
                seed,
                Faults {
                    torn_p: 0.3,
                    fail_p: 0.003,
                    ..quiet()
                },
                4,
            )
        })
        .unzip();
    let (crashes, during) = (crashes.iter().sum::<usize>(), during.iter().sum::<usize>());
    assert!(
        crashes > 5 && during > 1,
        "{crashes} crashes, {during} during vacuum"
    );
}
