use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use adtb::btree::{BTree, Index};
use adtb::heap::{FieldType, Heap, LargeStore};
use adtb::history::{self, History};
use adtb::key::{CtorId, Dir, Entry, KeySchema, KeyType, Scalar, Tid, Value};
use adtb::mvcc::{Marker, Past, PastError, Scope, Transactions};
use adtb::runtime::Threads;
use adtb::storage::{Options, Pager, Pid, Position};
use adtb::vacuum::{self, Report, Sum};
use adtb::verify::{self, Inventory};
use adtb_sim::{Config, Faults, Sim, SimRuntime};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/recovery.rs"]
mod recovery;

use harness::{finished, options};
use recovery::verify_after_crash;

const FRAMES: usize = 20;
const CTOR: CtorId = CtorId(0);
const HEAP: u32 = 10;
const PAST_HEAP: u32 = 11;
const LARGE: u32 = 9;
const PAST_LARGE: u32 = 8;
const INDEX: u32 = 20;
const PAST_INDEX: u32 = 21;
const LEDGER: u32 = 30;

/// Returns the options of a pool small enough that vacuum writes pages back as it runs, shared
/// by four threads.
fn opts() -> Options {
    Options {
        threads: 4,
        ..options(FRAMES)
    }
}

const LONG: Duration = Duration::from_secs(3600);

/// Each key's value.
type State = BTreeMap<i64, i64>;

fn row_type() -> Vec<FieldType> {
    vec![
        FieldType::Scalar(Scalar::I64),
        FieldType::Scalar(Scalar::I64),
    ]
}

fn key_schema() -> KeySchema {
    KeySchema::new(vec![(KeyType::Scalar(Scalar::I64), Dir::Asc)])
}

fn ctors() -> BTreeSet<CtorId> {
    BTreeSet::from([CTOR])
}

fn key(_: CtorId, row: &[Value]) -> Vec<Value> {
    vec![row[0].clone()]
}

fn pair(row: &[Value]) -> (i64, i64) {
    match row {
        [Value::I64(k), Value::I64(v)] => (*k, *v),
        _ => panic!("a row is a key and a value, not {row:?}"),
    }
}

#[derive(Clone, Copy, Debug)]
struct Roots {
    clog: Pid,
    large: Pid,
    past_large: Pid,
    heap: Pid,
    past_heap: Pid,
    index: Pid,
    past_index: Pid,
    ledger: Pid,
}

struct Db {
    pager: Pager<SimRuntime>,
    txns: Transactions<SimRuntime>,
    heap: Option<Heap>,
    past_heap: Heap,
    index: Index,
    past_index: Index,
    /// The TID of each key's live version.
    live: BTreeMap<i64, Tid>,
    /// Each key's committed value.
    state: State,
}

impl Db {
    fn create(rt: SimRuntime, window: Duration) -> (Db, Roots) {
        let pager = Pager::create(rt.clone(), &opts()).expect("create");
        let txns = Transactions::create(rt, &pager).expect("commit log");
        let large = LargeStore::create(&pager, LARGE).expect("large store");
        let past_large = LargeStore::create(&pager, PAST_LARGE).expect("large store");
        let heap = Heap::create(&pager, HEAP, row_type(), large).expect("heap");
        let past_heap = Heap::create(&pager, PAST_HEAP, row_type(), past_large).expect("heap");
        let index = Index::create(&pager, INDEX, key_schema(), ctors()).expect("index");
        let past_index =
            Index::create(&pager, PAST_INDEX, key_schema(), ctors()).expect("history index");
        let past = Past::create(&pager, LEDGER).expect("ledger");
        txns.keep_history(&pager, past, window).expect("history");
        let roots = Roots {
            clog: txns.commit_log().root(),
            large: large.root(),
            past_large: past_large.root(),
            heap: heap.root(),
            past_heap: past_heap.root(),
            index: index.tree().root(),
            past_index: past_index.tree().root(),
            ledger: past.tree().root(),
        };
        pager.flush_log();
        let db = Db {
            pager,
            txns,
            heap: Some(heap),
            past_heap,
            index,
            past_index,
            live: BTreeMap::new(),
            state: State::new(),
        };
        (db, roots)
    }

    /// Open the database `roots` names after a crash, with the heap of the constructor when its
    /// root is still allocated.
    fn open(rt: SimRuntime, roots: Roots, window: Duration) -> Db {
        let (pager, _) = Pager::open(rt.clone(), &opts()).expect("reopen");
        let txns = Transactions::open(rt, &pager, roots.clog).expect("commit log");
        txns.keep_history(
            &pager,
            Past::open(BTree::open(roots.ledger, LEDGER)),
            window,
        )
        .expect("history");
        let large = LargeStore::open(LARGE, roots.large);
        let past_large = LargeStore::open(PAST_LARGE, roots.past_large);
        let allocated = pager.allocated().expect("bitmap");
        let heap = allocated
            .contains(&roots.heap)
            .then(|| Heap::open(&pager, HEAP, roots.heap, row_type(), large).expect("heap"));
        let past_heap =
            Heap::open(&pager, PAST_HEAP, roots.past_heap, row_type(), past_large).expect("heap");
        let index = Index::open(BTree::open(roots.index, INDEX), key_schema(), ctors());
        let past_index = Index::open(
            BTree::open(roots.past_index, PAST_INDEX),
            key_schema(),
            ctors(),
        );
        let mut db = Db {
            pager,
            txns,
            heap,
            past_heap,
            index,
            past_index,
            live: BTreeMap::new(),
            state: State::new(),
        };
        db.live = db.find_live();
        db
    }

    fn heap(&self) -> &Heap {
        self.heap.as_ref().expect("the constructor's heap")
    }

    /// Returns the TID of each live version of the current heap, by key.
    fn find_live(&self) -> BTreeMap<i64, Tid> {
        let mut out = BTreeMap::new();
        if let Some(heap) = &self.heap {
            heap.scan(&self.pager, |v| {
                if v.header.xmax == 0 {
                    out.insert(pair(&v.row).0, v.tid);
                }
            })
            .expect("scan");
        }
        out
    }

    /// Commit one transaction setting each key of `ops` to its value, or deleting it for `None`.
    /// Returns its commit position.
    fn write(&mut self, ops: &[(i64, Option<i64>)]) -> Position {
        let x = self.txns.begin(&self.pager).expect("begin");
        for &(k, v) in ops {
            if let Some(tid) = self.live.remove(&k) {
                self.heap().delete(&self.pager, tid, x.0).expect("delete");
            }
            if let Some(v) = v {
                let row = vec![Value::I64(k), Value::I64(v)];
                let tid =
                    vacuum::insert(&self.pager, &self.txns, self.heap(), x, &row).expect("insert");
                let entry = Entry {
                    key: vec![Value::I64(k)],
                    ctor: CTOR,
                    tid,
                };
                assert!(self.index.insert(&self.pager, &entry).expect("index"));
                self.live.insert(k, tid);
            }
            match v {
                Some(v) => self.state.insert(k, v),
                None => self.state.remove(&k),
            };
        }
        let lsn = self.txns.commit(&self.pager, x).expect("commit");
        self.pager.wait_durable(lsn)
    }

    /// Take a read-only snapshot, keep its safe snapshot marker and release it. Returns the
    /// position the marker covers.
    fn mark(&self) -> Position {
        let (id, snapshot) = self.txns.take(Scope::Every).expect("snapshot");
        let at = self
            .txns
            .read_upto(id)
            .map(|l| self.pager.position(l))
            .expect("a commit to cover");
        self.txns.log_safe_snapshot(&self.pager, id, &snapshot);
        self.txns.release(id);
        at
    }

    /// Returns the latest commit time the record of the window holds.
    fn latest_commit(&self) -> u64 {
        self.txns
            .history()
            .expect("history is kept")
            .latest_commit(&self.pager)
            .expect("the record of the window")
    }

    /// Commit empty transactions until the wall clock passes `d` from now.
    fn idle(&mut self, d: Duration) {
        let until = self.pager.wall() + d;
        while self.pager.wall() <= until {
            self.write(&[]);
        }
    }

    fn sum(&self) -> Sum<'_> {
        Sum {
            heaps: vec![(CTOR, self.heap())],
            indexes: vec![&self.index],
        }
    }

    fn history(&self) -> History<'_> {
        History {
            heaps: vec![(CTOR, &self.past_heap)],
            index: &self.past_index,
            key: &key,
        }
    }

    fn vacuum_keeping(&self) -> Report {
        self.vacuum_keeping_by(usize::MAX)
    }

    /// Vacuum, moving versions to history, cleaning each time the dead TIDs found take `budget`
    /// bytes.
    fn vacuum_keeping_by(&self, budget: usize) -> Report {
        let history = self.history();
        vacuum::vacuum_keeping(
            &self.pager,
            &self.txns,
            &self.sum(),
            &history,
            budget,
            &vacuum::Progress::new(),
        )
        .expect("vacuum")
    }

    fn vacuum_in_place(&self) -> Report {
        vacuum::vacuum(
            &self.pager,
            &self.txns,
            &self.sum(),
            usize::MAX,
            &vacuum::Progress::new(),
        )
        .expect("vacuum")
    }

    fn expire(&self) -> Report {
        let past = Sum {
            heaps: vec![(CTOR, &self.past_heap)],
            indexes: vec![&self.past_index],
        };
        vacuum::expire(
            &self.pager,
            &self.txns,
            &past,
            usize::MAX,
            &vacuum::Progress::new(),
        )
        .expect("expire")
    }

    /// Returns the latest position the database has committed.
    fn upto(&self) -> Position {
        let (id, _) = self.txns.take(Scope::Every).expect("snapshot");
        let at = self.txns.read_upto(id);
        self.txns.release(id);
        self.pager
            .position(at.unwrap_or_else(|| self.pager.durable()))
    }

    /// Returns the rows a read of the past at `at` sees, scanning.
    fn read_at(&self, at: Position) -> Result<State, PastError> {
        let marker = self.txns.pin(&self.pager, at, self.upto())?;
        let out = self.scan_at(&marker);
        self.txns.unpin(&marker);
        Ok(out)
    }

    fn scan_at(&self, marker: &Marker) -> State {
        let mut heaps = vec![&self.past_heap];
        heaps.extend(&self.heap);
        let rows = history::rows(&self.pager, &self.txns, marker, &heaps).expect("rows");
        let mut out = State::new();
        for row in rows {
            let (k, v) = pair(&row);
            assert!(
                out.insert(k, v).is_none(),
                "key {k} is read twice at {marker:?}"
            );
        }
        out
    }

    /// Returns the value a read of the past at `at` finds for `k`, probing by key.
    fn probe_at(&self, at: Position, k: i64) -> Option<i64> {
        let marker = self.txns.pin(&self.pager, at, self.upto()).expect("pin");
        let found = history::probe(
            &self.pager,
            &self.txns,
            &marker,
            CTOR,
            (self.heap(), &self.index),
            &self.history(),
            &[Value::I64(k)],
        )
        .expect("probe");
        self.txns.unpin(&marker);
        found.map(|row| pair(&row).1)
    }

    /// Returns the value a read of the past at `at` finds for each key of `keys`, probing by key
    /// under one marker.
    fn probes_at(
        &self,
        at: Position,
        keys: impl IntoIterator<Item = i64>,
    ) -> Result<State, PastError> {
        let marker = self.txns.pin(&self.pager, at, self.upto())?;
        let history = self.history();
        let mut out = State::new();
        for k in keys {
            let found = history::probe(
                &self.pager,
                &self.txns,
                &marker,
                CTOR,
                (self.heap(), &self.index),
                &history,
                &[Value::I64(k)],
            )
            .expect("probe");
            if let Some(row) = found {
                out.insert(k, pair(&row).1);
            }
        }
        self.txns.unpin(&marker);
        Ok(out)
    }

    /// Returns the number of versions of the current heap and of the history heap.
    fn versions(&self) -> (usize, usize) {
        let count = |h: &Heap| h.headers(&self.pager).expect("headers").len();
        (self.heap.as_ref().map_or(0, count), count(&self.past_heap))
    }

    /// Returns what the whole-file `verify` and the history's own checks find wrong.
    fn verify_file(&self) -> Vec<String> {
        let clog = self.txns.commit_log();
        let mut sums = vec![verify::Sum {
            heaps: vec![(CTOR, &self.past_heap)],
            indexes: vec![&self.past_index],
        }];
        let mut indexes = Vec::new();
        if let Some(heap) = &self.heap {
            sums.push(verify::Sum {
                heaps: vec![(CTOR, heap)],
                indexes: vec![&self.index],
            });
        } else {
            indexes.push(self.index.tree());
        }
        let ledger = self.txns.history().expect("history").tree();
        let mut trees = vec![ledger];
        trees.extend(indexes);
        let inventory = Inventory {
            commit_log: Some(&clog),
            sums,
            trees,
            ..Inventory::default()
        };
        let mut problems = verify_after_crash(&self.pager, &inventory);
        problems.extend(self.duplicated());
        problems
    }

    /// Returns each version of the history heap named twice by the history key index, by key and
    /// writers, and each version no entry names.
    fn duplicated(&self) -> Vec<String> {
        let entries = self
            .past_index
            .scan_prefix(&self.pager, &[])
            .expect("history entries");
        let named: BTreeSet<Tid> = entries.iter().map(|e| e.tid).collect();
        let mut out = Vec::new();
        self.past_heap
            .scan(&self.pager, |v| {
                if !named.contains(&v.tid) {
                    let id = (pair(&v.row).0, v.header.xmin, v.header.xmax);
                    out.push(format!("history holds {id:?} at {:?} unindexed", v.tid));
                }
            })
            .expect("history scan");
        let mut seen = BTreeSet::new();
        for e in entries {
            let v = self
                .past_heap
                .read(&self.pager, e.tid)
                .expect("read")
                .unwrap_or_else(|| panic!("history entry {e:?} names no version"));
            let id = (pair(&v.row).0, v.header.xmin, v.header.xmax);
            if !seen.insert(id) {
                out.push(format!("history holds {id:?} twice"));
            }
        }
        out
    }
}

/// Run `rounds` transactions of random writes over `keys` keys, each followed by a marker.
/// Returns each marker's position with the state it covers.
fn history_of(db: &mut Db, seed: u64, rounds: usize, keys: i64) -> Vec<(Position, State)> {
    let mut r = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        r
    };
    let mut out = Vec::new();
    for _ in 0..rounds {
        let mut ops = Vec::new();
        for _ in 0..=next() % 3 {
            let k = (next() % keys as u64) as i64;
            let v = (next() % 4 != 0).then(|| (next() % 1000) as i64);
            ops.push((k, v));
        }
        db.write(&ops);
        out.push((db.mark(), db.state.clone()));
    }
    out
}

fn check_reads(db: &Db, marks: &[(Position, State)], why: &str) {
    for (at, want) in marks {
        let got = db
            .read_at(*at)
            .unwrap_or_else(|e| panic!("{why}: read at {at:?}: {e:?}"));
        assert_eq!(want, &got, "{why}: rows at {at:?}");
    }
}

fn check_probes(db: &Db, marks: &[(Position, State)], keys: i64, why: &str) {
    for (at, want) in marks {
        let got = db
            .probes_at(*at, 0..keys)
            .unwrap_or_else(|e| panic!("{why}: probes at {at:?}: {e:?}"));
        assert_eq!(want, &got, "{why}: rows probed at {at:?}");
    }
}

#[test]
fn vacuum_moves_versions_the_window_keeps_and_every_marker_reads_what_it_covered() {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, LONG);
        let marks = history_of(&mut db, 1, 40, 12);
        check_reads(&db, &marks, "before vacuum");
        let report = db.vacuum_keeping();
        assert!(report.moved > 0, "{report:?}");
        let (current, past) = db.versions();
        assert_eq!(
            db.live.len(),
            current,
            "the current heap holds only live versions"
        );
        assert_eq!(report.moved, past);
        check_reads(&db, &marks, "after vacuum");
        assert_eq!(
            0,
            db.vacuum_keeping().moved,
            "a second vacuum moves nothing"
        );
        assert_eq!(past, db.versions().1);
        for (at, want) in &marks {
            for k in 0..12 {
                assert_eq!(
                    want.get(&k).copied(),
                    db.probe_at(*at, k),
                    "key {k} at {at:?}"
                );
            }
        }
        assert!(db.verify_file().is_empty(), "{:?}", db.verify_file());
    });
}

#[test]
fn a_position_between_markers_reads_the_marker_before_it() {
    let mut sim = Sim::new(2, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, LONG);
        db.write(&[(1, Some(10))]);
        let first = db.mark();
        let unmarked = db.write(&[(1, Some(11))]);
        db.write(&[(1, Some(12)), (2, Some(20))]);
        let last = db.mark();
        db.vacuum_keeping();
        assert_eq!(State::from([(1, 10)]), db.read_at(unmarked).expect("read"));
        assert_eq!(State::from([(1, 10)]), db.read_at(first).expect("read"));
        assert_eq!(
            State::from([(1, 12), (2, 20)]),
            db.read_at(last).expect("read")
        );
    });
}

#[test]
fn vacuum_in_place_and_pruning_on_access_keep_what_the_window_keeps() {
    let mut sim = Sim::new(3, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, LONG);
        let marks = history_of(&mut db, 3, 30, 6);
        let headers = |db: &Db| -> BTreeSet<(Tid, u64, u64)> {
            let hs = db.heap().headers(&db.pager).expect("headers");
            hs.into_iter()
                .map(|(t, h, _)| (t, h.xmin, h.xmax))
                .collect()
        };
        let before = headers(&db);
        let report = db.vacuum_in_place();
        for pid in db.heap().pages(&db.pager).expect("pages") {
            vacuum::prune(&db.pager, &db.txns, db.heap(), pid).expect("prune");
        }
        let after = headers(&db);
        assert_eq!(0, report.moved);
        assert_eq!(0, db.versions().1);
        assert_eq!(before.len() - report.freed, after.len(), "{report:?}");
        for (tid, xmin, xmax) in before.difference(&after) {
            assert_eq!(
                xmin, xmax,
                "{tid:?} is freed though another transaction deleted it"
            );
        }
        check_reads(&db, &marks, "after vacuum in place");
    });
}

#[test]
fn versions_past_the_window_are_freed_and_reads_there_are_unavailable() {
    let mut sim = Sim::new(4, Config::default());
    finished(&mut sim, |rt| {
        let window = Duration::from_millis(200);
        let (mut db, _) = Db::create(rt, window);
        let old = history_of(&mut db, 4, 10, 5);
        db.vacuum_keeping();
        let moved = db.versions().1;
        assert!(moved > 0);
        db.idle(window);
        let recent = history_of(&mut db, 5, 10, 5);
        db.vacuum_keeping();
        db.expire();
        let oldest = recent[0].0;
        for (at, _) in &old {
            assert_eq!(
                Err(PastError::Unavailable {
                    oldest: Some(oldest)
                }),
                db.read_at(*at),
                "a read at {at:?}, before the window"
            );
        }
        check_reads(&db, &recent, "within the window");
        let after = Position {
            epoch: db.upto().epoch,
            lsn: adtb::storage::Lsn(db.upto().lsn.0 + 1),
        };
        assert_eq!(
            Err(PastError::Unavailable {
                oldest: Some(oldest)
            }),
            db.read_at(after),
            "a read after the latest commit"
        );
        let history = db.past_heap.headers(&db.pager).expect("headers");
        let record = db.txns.history().expect("history");
        let cutoff = db.txns.cutoff().expect("a cutoff");
        for (tid, h, _) in history {
            let t = record
                .committed_at(&db.pager, adtb::mvcc::Xid(h.xmax))
                .expect("ledger");
            assert!(
                t.is_some_and(|t| t >= cutoff),
                "history keeps {tid:?}, deleted at {t:?}, before {cutoff}"
            );
        }
        assert!(db.verify_file().is_empty(), "{:?}", db.verify_file());
    });
}

#[test]
fn a_held_marker_keeps_what_it_reads_past_the_window() {
    let mut sim = Sim::new(5, Config::default());
    finished(&mut sim, |rt| {
        let window = Duration::from_millis(200);
        let (mut db, _) = Db::create(rt, window);
        let marks = history_of(&mut db, 6, 12, 4);
        let (at, want) = marks[3].clone();
        let held = db.txns.pin(&db.pager, at, db.upto()).expect("pin");
        db.idle(window * 2);
        db.write(&[(0, None), (1, None), (2, None), (3, None)]);
        db.vacuum_keeping();
        db.expire();
        assert_eq!(
            want,
            db.scan_at(&held),
            "a held marker reads what it covered"
        );
        db.txns.unpin(&held);
        db.idle(window * 2);
        db.vacuum_keeping();
        db.expire();
        assert!(matches!(db.read_at(at), Err(PastError::Unavailable { .. })));
        assert_eq!(0, db.versions().1, "history keeps nothing past the window");
    });
}

#[test]
fn a_read_of_the_past_without_history_is_unavailable() {
    let mut sim = Sim::new(6, Config::default());
    finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &opts()).expect("create");
        let txns = Transactions::create(rt, &pager).expect("commit log");
        let x = txns.begin(&pager).expect("begin");
        let lsn = txns.commit(&pager, x).expect("commit");
        let at = pager.wait_durable(lsn);
        assert_eq!(
            Err(PastError::Unavailable { oldest: None }),
            txns.pin(&pager, at, at)
        );
        assert_eq!(None, txns.cutoff());
    });
}

#[test]
fn a_read_by_time_reads_the_latest_marker_committed_by_then() {
    let mut sim = Sim::new(7, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, LONG);
        db.write(&[(1, Some(1))]);
        db.mark();
        let between = db.pager.wall();
        db.idle(Duration::from_millis(5));
        db.write(&[(1, Some(2))]);
        db.mark();
        db.vacuum_keeping();
        let read = |t: SystemTime| {
            let m = db.txns.pin_time(&db.pager, t, db.upto()).expect("pin");
            let out = db.scan_at(&m);
            db.txns.unpin(&m);
            out
        };
        assert_eq!(State::from([(1, 1)]), read(between));
        assert_eq!(State::from([(1, 2)]), read(db.pager.wall()));
    });
}

#[test]
fn a_clock_that_steps_back_across_a_reopen_keeps_the_order_of_commit_times() {
    let mut sim = Sim::new(3, Config::default());
    let mut out = None;
    finished(&mut sim, |rt| {
        let (mut db, roots) = Db::create(rt, LONG);
        db.idle(Duration::from_millis(50));
        db.write(&[(1, Some(1))]);
        db.mark();
        let between = db.pager.wall();
        db.idle(Duration::from_millis(5));
        db.write(&[(1, Some(2))]);
        db.mark();
        db.pager.flush_log();
        db.pager.checkpoint();
        let latest = db.latest_commit();
        out = Some((roots, between, latest));
    });
    let (roots, between, latest) = out.expect("the run finished");
    let mut sim = Sim::with_files(4, Config::default(), sim.files());
    finished(&mut sim, |rt| {
        let mut db = Db::open(rt, roots, LONG);
        let reopened = db.pager.wall();
        assert!(
            reopened < between,
            "the clock reads {reopened:?} at the reopen"
        );
        db.write(&[(1, Some(3))]);
        db.mark();
        let times: Vec<u64> = db
            .txns
            .history()
            .expect("history is kept")
            .markers(&db.pager)
            .expect("the markers")
            .iter()
            .map(|m| m.time)
            .collect();
        assert!(
            times.is_sorted_by(|later, earlier| later >= earlier),
            "markers from the latest back record times {times:?}"
        );
        let read = |t: SystemTime| {
            let m = db.txns.pin_time(&db.pager, t, db.upto()).expect("pin");
            let out = db.scan_at(&m);
            db.txns.unpin(&m);
            out
        };
        assert_eq!(State::from([(1, 1)]), read(between));
        assert_eq!(
            State::from([(1, 3)]),
            read(SystemTime::UNIX_EPOCH + Duration::from_nanos(latest))
        );
    });
}

/// Returns the files of a database with history after `body` ran on it, the recording index
/// where `body` started, and the database's roots.
fn recorded(
    seed: u64,
    setup: impl FnOnce(&mut Db) -> Vec<(Position, State)>,
    body: impl FnOnce(&mut Db),
) -> (Sim, usize, Roots, Vec<(Position, State)>) {
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut out = None;
    finished(&mut sim, |rt| {
        let (mut db, roots) = Db::create(rt.clone(), LONG);
        let marks = setup(&mut db);
        db.pager.flush_log();
        db.pager.checkpoint();
        let start = rt.recorded();
        body(&mut db);
        db.pager.flush_log();
        out = Some((start, roots, marks));
    });
    let (start, roots, marks) = out.unwrap();
    (sim, start, roots, marks)
}

/// Returns every flush of `recording` from `start` on, and about 40 writes between them.
fn crash_points(recording: &adtb_sim::Recording, start: usize) -> BTreeSet<usize> {
    let n = recording.len();
    let mut out: BTreeSet<usize> = recording
        .flushes()
        .into_iter()
        .filter(|&f| f >= start)
        .collect();
    out.extend((start..n).step_by((n - start).div_ceil(40).max(1)));
    out
}

fn crash_config() -> Config {
    Config {
        faults: Faults {
            torn_p: 0.5,
            ..Faults::default()
        },
        ..Config::default()
    }
}

#[test]
fn a_crash_at_every_flush_while_vacuum_moves_versions_loses_no_past_row() {
    let seed = 11;
    let (sim, start, roots, marks) = recorded(
        seed,
        |db| history_of(db, seed, 400, 600),
        |db| {
            let report = db.vacuum_keeping_by(1);
            assert!(report.moved > 0 && report.rounds > 3, "{report:?}");
        },
    );
    let recording = sim.recording();
    let points = crash_points(&recording, start);
    assert!(points.len() > 20, "{} crash points", points.len());
    let mut moved = BTreeSet::new();
    let mut unindexed = 0;
    for at in points {
        let s = seed + at as u64;
        let files = recording.images(at, 1, s).swap_remove(0);
        let mut sim = Sim::with_files(s, crash_config(), files);
        finished(&mut sim, |rt| {
            let db = Db::open(rt, roots, LONG);
            moved.insert(db.versions().1);
            let some: Vec<(Position, State)> = marks.iter().step_by(16).cloned().collect();
            let fewer: Vec<(Position, State)> = some.iter().step_by(5).cloned().collect();
            check_reads(&db, &some, &format!("crash at {at}"));
            check_probes(&db, &fewer, 600, &format!("crash at {at}"));
            unindexed += db
                .duplicated()
                .iter()
                .filter(|p| p.contains("unindexed"))
                .count();
            db.vacuum_keeping();
            check_reads(&db, &some, &format!("vacuum again after a crash at {at}"));
            check_probes(
                &db,
                &fewer,
                600,
                &format!("vacuum again after a crash at {at}"),
            );
            assert_eq!(db.live.len(), db.versions().0, "crash at {at}");
            let problems = db.verify_file();
            assert!(problems.is_empty(), "crash at {at}: {problems:?}");
        });
    }
    assert!(
        moved.len() > 3,
        "crashes leave history at only {moved:?} versions"
    );
    assert!(unindexed > 0, "no crash left a copy without its entry");
}

#[test]
fn a_crash_at_every_flush_while_a_removed_constructors_heap_drops_keeps_its_past() {
    let seed = 12;
    let (sim, start, roots, marks) = recorded(
        seed,
        |db| {
            let mut marks = history_of(db, seed, 20, 6);
            let all: Vec<(i64, Option<i64>)> = (0..6).map(|k| (k, None)).collect();
            db.write(&all);
            marks.push((db.mark(), State::new()));
            db.vacuum_keeping();
            assert!(db.heap().is_empty(&db.pager).expect("pages"));
            marks
        },
        |db| {
            db.heap.take().unwrap().drop_empty(&db.pager).expect("drop");
        },
    );
    let recording = sim.recording();
    let points = crash_points(&recording, start);
    assert!(!points.is_empty());
    let mut dropped = BTreeSet::new();
    for at in points.into_iter().chain([recording.len()]) {
        let s = seed + at as u64;
        let files = recording.images(at, 1, s).swap_remove(0);
        let mut sim = Sim::with_files(s, crash_config(), files);
        finished(&mut sim, |rt| {
            let db = Db::open(rt, roots, LONG);
            dropped.insert(db.heap.is_none());
            check_reads(&db, &marks, &format!("crash at {at}"));
            let problems = db.verify_file();
            assert!(problems.is_empty(), "crash at {at}: {problems:?}");
        });
    }
    assert_eq!(BTreeSet::from([false, true]), dropped);
}

#[test]
fn a_heap_emptied_from_its_end_leaves_no_hole_behind_its_last_page() {
    let mut sim = Sim::new(13, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt, Duration::ZERO);
        let ops: Vec<(i64, Option<i64>)> = (0..2000).map(|k| (k, Some(k))).collect();
        for chunk in ops.chunks(200) {
            db.write(chunk);
        }
        let pages = db.heap().pages(&db.pager).expect("pages").len();
        assert!(pages > 4, "{pages} pages");
        let tail: Vec<(i64, Option<i64>)> = (1000..2000).map(|k| (k, None)).collect();
        db.write(&tail);
        db.vacuum_keeping();
        let check = db.heap().check(&db.pager).expect("check");
        assert!(check.is_empty(), "{check:?}");
        let left = db.heap().pages(&db.pager).expect("pages").len();
        assert!(left < pages, "{left} of {pages} pages left");
        assert_eq!(0, db.heap().holes(&db.pager).expect("holes"));
        db.write(&(0..1000).map(|k| (k, None)).collect::<Vec<_>>());
        db.vacuum_keeping();
        assert!(db.heap().is_empty(&db.pager).expect("pages"));
        assert_eq!(0, db.heap().holes(&db.pager).expect("holes"));
        let tid = vacuum::insert(
            &db.pager,
            &db.txns,
            db.heap(),
            db.txns.begin(&db.pager).expect("begin"),
            &[Value::I64(1), Value::I64(1)],
        )
        .expect("insert");
        let pages = db.heap().pages(&db.pager).expect("pages");
        assert_eq!(vec![Pid(tid.page())], pages);
    });
}

#[test]
fn a_moved_version_keeps_its_out_of_line_values_until_the_window_passes() {
    let mut sim = Sim::new(14, Config::default());
    finished(&mut sim, |rt| {
        let window = Duration::from_millis(200);
        let pager = Pager::create(rt.clone(), &opts()).expect("create");
        let txns = Transactions::create(rt, &pager).expect("commit log");
        let past = Past::create(&pager, LEDGER).expect("ledger");
        txns.keep_history(&pager, past, window).expect("history");
        let fields = vec![
            FieldType::Scalar(Scalar::I64),
            FieldType::Scalar(Scalar::Bytes),
        ];
        let large = LargeStore::create(&pager, LARGE).expect("large store");
        let past_large = LargeStore::create(&pager, PAST_LARGE).expect("large store");
        let heap = Heap::create(&pager, HEAP, fields.clone(), large).expect("heap");
        let past_heap = Heap::create(&pager, PAST_HEAP, fields, past_large).expect("heap");
        let index = Index::create(&pager, INDEX, key_schema(), ctors()).expect("index");
        let past_index = Index::create(&pager, PAST_INDEX, key_schema(), ctors()).expect("index");
        let blob: Vec<u8> = (0..9000u32).map(|i| (i * 7) as u8).collect();
        let row = vec![Value::I64(1), Value::Bytes(blob)];
        let commit = |x| pager.wait_durable(txns.commit(&pager, x).expect("commit"));
        let x = txns.begin(&pager).expect("begin");
        let tid = heap.insert(&pager, x.0, &row).expect("insert");
        let entry = Entry {
            key: vec![Value::I64(1)],
            ctor: CTOR,
            tid,
        };
        index.insert(&pager, &entry).expect("index");
        let at = commit(x);
        let (id, snapshot) = txns.take(Scope::Every).expect("snapshot");
        txns.log_safe_snapshot(&pager, id, &snapshot);
        txns.release(id);
        let x = txns.begin(&pager).expect("begin");
        heap.delete(&pager, tid, x.0).expect("delete");
        commit(x);
        let sum = Sum {
            heaps: vec![(CTOR, &heap)],
            indexes: vec![&index],
        };
        let history = History {
            heaps: vec![(CTOR, &past_heap)],
            index: &past_index,
            key: &key,
        };
        let progress = vacuum::Progress::new();
        let report = vacuum::vacuum_keeping(&pager, &txns, &sum, &history, usize::MAX, &progress)
            .expect("vacuum");
        assert_eq!(1, report.moved);
        assert!(
            report.chunks > 0,
            "the current heap's copy is swept: {report:?}"
        );
        let marker = txns.pin(&pager, at, at).expect("pin");
        let rows = history::rows(&pager, &txns, &marker, &[&heap, &past_heap]).expect("rows");
        txns.unpin(&marker);
        assert_eq!(vec![row], rows);
        let until = pager.wall() + window * 2;
        while pager.wall() <= until {
            commit(txns.begin(&pager).expect("begin"));
        }
        let past = Sum {
            heaps: vec![(CTOR, &past_heap)],
            indexes: vec![&past_index],
        };
        let report = vacuum::expire(&pager, &txns, &past, usize::MAX, &progress).expect("expire");
        assert_eq!(1, report.freed, "{report:?}");
        assert!(
            report.chunks > 0,
            "the history heap's copy is swept: {report:?}"
        );
        let clog = txns.commit_log();
        let inventory = Inventory {
            commit_log: Some(&clog),
            sums: vec![
                verify::Sum {
                    heaps: vec![(CTOR, &heap)],
                    indexes: vec![&index],
                },
                verify::Sum {
                    heaps: vec![(CTOR, &past_heap)],
                    indexes: vec![&past_index],
                },
            ],
            trees: vec![txns.history().expect("history").tree()],
            ..Inventory::default()
        };
        let problems = verify_after_crash(&pager, &inventory);
        assert!(problems.is_empty(), "{problems:?}");
    });
}

#[test]
fn reads_of_the_past_beside_a_vacuum_moving_their_versions_read_every_row_once() {
    let mut sim = Sim::new(15, Config::default());
    finished(&mut sim, |rt| {
        let (mut db, _) = Db::create(rt.clone(), LONG);
        let marks = Arc::new(history_of(&mut db, 15, 200, 300));
        let db = Arc::new(db);
        let stop = Arc::new(AtomicBool::new(false));
        let wrong = Arc::new(Mutex::new(Vec::new()));
        let reads = Arc::new(AtomicUsize::new(0));
        let readers: Vec<_> = (0..3)
            .map(|r| {
                let (rt2, db, marks, stop, wrong, reads) = (
                    rt.clone(),
                    db.clone(),
                    marks.clone(),
                    stop.clone(),
                    wrong.clone(),
                    reads.clone(),
                );
                rt.spawn(&format!("reader {r}"), move || {
                    for (n, (at, want)) in marks.iter().skip(r).step_by(7).cycle().enumerate() {
                        if stop.load(Ordering::Relaxed) && n > 0 {
                            break;
                        }
                        match db.read_at(*at) {
                            Ok(got) if &got == want => {}
                            other => wrong.lock().unwrap().push(format!("at {at:?}: {other:?}")),
                        }
                        let keys = (n as i64 % 10..300).step_by(10);
                        let some: State = want
                            .iter()
                            .filter(|(k, _)| keys.clone().any(|j| j == **k))
                            .map(|(k, v)| (*k, *v))
                            .collect();
                        match db.probes_at(*at, keys) {
                            Ok(got) if got == some => {}
                            other => wrong
                                .lock()
                                .unwrap()
                                .push(format!("probed at {at:?}: {other:?}")),
                        }
                        reads.fetch_add(1, Ordering::Relaxed);
                        rt2.yield_now();
                    }
                })
            })
            .collect();
        let report = db.vacuum_keeping();
        stop.store(true, Ordering::Relaxed);
        for t in readers {
            rt.join(t);
        }
        assert!(report.moved > 0);
        let reads = reads.load(Ordering::Relaxed);
        assert!(reads > 12, "{reads} reads ran beside the vacuum");
        let wrong = wrong.lock().unwrap();
        assert!(wrong.is_empty(), "{wrong:#?}");
        check_reads(&db, &marks, "after vacuum");
    });
}

#[test]
fn a_marker_a_read_of_the_past_used_survives_a_crash() {
    let seed = 16;
    let mut sim = Sim::new(seed, Config::default());
    sim.record();
    let mut out = None;
    finished(&mut sim, |rt| {
        let (mut db, roots) = Db::create(rt.clone(), LONG);
        db.write(&[(1, Some(1))]);
        db.mark();
        db.write(&[(1, Some(2))]);
        let at = db.mark();
        assert_eq!(State::from([(1, 2)]), db.read_at(at).expect("read"));
        out = Some((rt.recorded(), roots, at));
    });
    let (read, roots, at) = out.unwrap();
    let recording = sim.recording();
    for point in (read..=recording.len()).step_by(7).chain([recording.len()]) {
        let s = seed + point as u64;
        let files = recording.images(point, 1, s).swap_remove(0);
        let mut sim = Sim::with_files(s, crash_config(), files);
        finished(&mut sim, |rt| {
            let db = Db::open(rt, roots, LONG);
            assert_eq!(
                Ok(State::from([(1, 2)])),
                db.read_at(at),
                "crash at {point}"
            );
        });
    }
}

#[test]
fn a_history_heap_write_that_fails_on_a_page_is_a_page_error() {
    let damaged = adtb::storage::PageError {
        pid: Pid(7),
        damage: adtb::storage::Damage::Unreadable,
    };
    let e = adtb::write::Error::from(vacuum::Error::Heap(adtb::heap::Error::Page(damaged)));
    assert!(
        matches!(e, adtb::write::Error::Page(p) if p == damaged),
        "{e:?}"
    );
    let e = adtb::write::Error::from(vacuum::Error::Heap(adtb::heap::Error::TooLarge {
        len: 9000,
        limit: 4000,
    }));
    assert!(
        matches!(&e, adtb::write::Error::Internal(text) if text.contains("history heap")),
        "{e:?}"
    );
}
