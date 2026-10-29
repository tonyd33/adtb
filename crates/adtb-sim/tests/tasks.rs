//! The calls submitted as owned tasks: results observed in batches under supplied capacity,
//! production that stops at the handoff, snapshots held until drained, and the outcome of
//! cancel, idle expiry and drop against commit.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Wake, Waker};
use std::time::Duration;

use adtb::batch::{self, Batch, BatchBuilder, Type};
use adtb::key::Scalar;
use adtb::runtime::{Clock, Io, Op, Runtime, ThreadId, Threads};
use adtb::{
    CallOptions, CancelToken, Completion, CreateOptions, Database, Determinism, Error, Format,
    HostFns, Log, Observation, OpenOptions, Registration, RejectReason, RetryReason, RoleSession,
    Task, TaskOptions, TxnCallOptions, TxnOptions, Value,
};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::seeds::sweep;
use adtb_sys::os::{Backend, OsRuntime};

const SCHEMA: &str = r#"module shop
add module shop
add type Item = { id: Key items, n: I64 }
add relation items : Item key id generate sequence
add type Counter = { id: Key counters, value: I64 }
add relation counters : Counter key id generate sequence
define fn seen (n: I64) -> I64 = host "seen" version "1.0" deterministic
define fn gate (n: I64) -> I64 = host "gate" version "1.0" deterministic
"#;

const CALLS: &str = "import shop
query all () -> Set I64 = from i in items select seen(i.n)
query count () -> I64 = count(i for i in items)
txn bump () -> Key items =
  transact
    update c in counters set { c with value: c.value + 1 }
    let u = insert items { n: 0 - 1 }
    return key u
query hold () -> Set I64 = from n in [1, 2] select gate(n)
txn tick () -> Key items =
  transact
    update c in counters set { c with value: seen(c.value) + 1 }
    let u = insert items { n: 0 - 1 }
    return key u
query residues () -> Set I64 = from i in items select i.n % 7 + i.n / 3999 * 7
query pairs () -> Set I64 =
  (from i in items select i.n % 100 + 100)
    union (from i in items, j in items where j.n == i.n + 1 select j.n % 100)
";

const UPDATE: &str = "update c in counters set { c with value: c.value + 1 }";

const SEEDS: Range<u64> = 0..2;

/// The rows `fill` inserts.
const ROWS: usize = 4_000;

/// The rows the host function `seen` is called on, and the threads it runs on.
#[derive(Default)]
struct Seen {
    rows: AtomicUsize,
    threads: std::sync::Mutex<BTreeSet<ThreadId>>,
}

/// Holds every call of the host function `gate` until released.
#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    open: AtomicBool,
    waiters: std::sync::Mutex<Vec<ThreadId>>,
}

impl Gate {
    fn pass<R: Runtime>(&self, rt: &R) {
        self.entered.store(true, Ordering::Release);
        let me = rt.current();
        loop {
            self.waiters.lock().unwrap().push(me);
            if self.open.load(Ordering::Acquire) {
                return;
            }
            rt.park();
        }
    }

    fn release<R: Runtime>(&self, rt: &R) {
        self.open.store(true, Ordering::Release);
        let waiters = std::mem::take(&mut *self.waiters.lock().unwrap());
        for t in waiters {
            rt.unpark(t);
        }
    }
}

fn host_fns<R: Runtime>(rt: &R, seen: &Arc<Seen>, gate: &Arc<Gate>) -> HostFns {
    let mut fns = HostFns::new();
    let (grt, gate) = (rt.clone(), gate.clone());
    fns.register(
        "gate",
        "1.0",
        Determinism::Deterministic,
        move |b: &Batch| {
            gate.pass(&grt);
            let mut out = BatchBuilder::new(batch::Schema::new(vec![Type::Scalar(Scalar::I64)]));
            for i in 0..b.len() {
                out.push_row(&[b.value(0, i)]);
            }
            Ok(out.finish())
        },
    );
    let (rt, seen) = (rt.clone(), seen.clone());
    fns.register(
        "seen",
        "1.0",
        Determinism::Deterministic,
        move |b: &Batch| {
            seen.rows.fetch_add(b.len(), Ordering::AcqRel);
            seen.threads.lock().unwrap().insert(rt.current());
            let mut out = BatchBuilder::new(batch::Schema::new(vec![Type::Scalar(Scalar::I64)]));
            for i in 0..b.len() {
                out.push_row(&[b.value(0, i)]);
            }
            Ok(out.finish())
        },
    );
    fns
}

fn options<R: Runtime>(rt: &R, seen: &Arc<Seen>, gate: &Arc<Gate>, workers: usize) -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(workers),
        registrations: vec![Registration::new("shop", CALLS)],
        host_fns: host_fns(rt, seen, gate),
        ..OpenOptions::default()
    }
}

struct Fixture<R: Runtime> {
    db: Database<R>,
    owner: RoleSession<R>,
    seen: Arc<Seen>,
    gate: Arc<Gate>,
}

fn open<R: Runtime>(rt: &R, rows: usize) -> Fixture<R> {
    open_with(rt, rows, 2)
}

/// Returns a database with `rows` items whose tasks run on `workers` workers.
fn open_with<R: Runtime>(rt: &R, rows: usize, workers: usize) -> Fixture<R> {
    open_under(rt, rows, workers, None)
}

/// Returns a database with `rows` items whose tasks run on `workers` workers, each call's
/// memory bounded by `memory`.
fn open_under<R: Runtime>(rt: &R, rows: usize, workers: usize, memory: Option<u64>) -> Fixture<R> {
    let (seen, gate) = (Arc::new(Seen::default()), Arc::new(Gate::default()));
    let db = Database::open_in(
        rt.clone(),
        "shop.adt",
        &Log::new([SCHEMA]),
        &OpenOptions {
            query_memory_bytes: memory,
            ..options(rt, &seen, &gate, workers)
        },
    )
    .unwrap();
    text(&db, "insert counters { value: 0 }");
    if rows > 0 {
        text(&db, &fill(rows));
    }
    let owner = db.role_authority().session("owner").unwrap();
    seen.rows.store(0, Ordering::Release);
    Fixture {
        db,
        owner,
        seen,
        gate,
    }
}

/// Returns the write that inserts the items numbered `0..rows`, as products of digits.
fn fill(rows: usize) -> String {
    let digits = "[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]";
    let places = rows.to_string().len();
    let vars: Vec<String> = (0..places).map(|p| format!("d{p}")).collect();
    let from = vars
        .iter()
        .map(|v| format!("{v} in {digits}"))
        .collect::<Vec<_>>()
        .join(", ");
    let n = vars
        .iter()
        .enumerate()
        .map(|(p, v)| format!("{v} * {}", 10usize.pow(p as u32)))
        .collect::<Vec<_>>()
        .join(" + ");
    format!("insert items from (from {from} where {n} < {rows} select {{ n: {n} }})")
}

fn text<R: Runtime>(db: &Database<R>, t: &str) -> Vec<Vec<Value>> {
    db.query_text(t, &[], &CallOptions::default())
        .unwrap_or_else(|e| panic!("`{t}` runs: {e:?}"))
        .into_iter()
        .map(|rows| rows.collect::<Result<_, _>>().unwrap())
        .collect()
}

impl<R: Runtime> Fixture<R> {
    fn submit(&self, ordinal: u32, opts: TaskOptions) -> Task<R> {
        self.db
            .registered("shop")
            .unwrap()
            .submit(&self.owner, ordinal, Vec::new(), opts)
            .unwrap()
    }

    fn items(&self) -> usize {
        text(&self.db, "count(i for i in items)")[0][0]
            .int()
            .unwrap() as usize
    }

    fn vacuumed(&self) -> i64 {
        text(&self.db, "vacuum items")[0]
            .iter()
            .map(|r| r.fields().unwrap()[2].int().unwrap())
            .sum()
    }
}

/// Unparks the thread that made it on each wake, counting them.
struct Wakes<R: Runtime> {
    rt: R,
    main: ThreadId,
    count: AtomicUsize,
}

fn wakes<R: Runtime>(rt: &R) -> (Arc<Wakes<R>>, Waker) {
    let w = Arc::new(Wakes {
        rt: rt.clone(),
        main: rt.current(),
        count: AtomicUsize::new(0),
    });
    (w.clone(), Waker::from(w))
}

impl<R: Runtime> Wake for Wakes<R> {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.count.fetch_add(1, Ordering::AcqRel);
        self.rt.unpark(self.main);
    }
}

/// Returns the next observation of `task` that is not pending, parking between them.
fn next<R: Runtime>(rt: &R, task: &Task<R>, waker: &Waker) -> Observation {
    loop {
        match task.observe(waker) {
            Observation::Pending => rt.park(),
            o => return o,
        }
    }
}

/// Supply unbounded capacity, take every batch, acknowledge the drain and return the elements
/// with the completion.
fn drive<R: Runtime>(rt: &R, task: &Task<R>) -> (Vec<Value>, Completion) {
    let (_, waker) = wakes(rt);
    task.supply(usize::MAX);
    let mut rows = Vec::new();
    loop {
        match next(rt, task, &waker) {
            Observation::Layout(_) => {}
            Observation::Batch(b) => rows.extend(b.rows().expect("a rows batch").iter().cloned()),
            Observation::End => task.drained(),
            Observation::Done(c) => return (rows, c),
            Observation::Pending => unreachable!("next skips pending"),
        }
    }
}

fn sleep<R: Runtime>(rt: &R, d: Duration) {
    let mut io = rt.io();
    let ticket = io.submit(Op::Timer { at: rt.now() + d });
    loop {
        if let Some(c) = io.poll() {
            assert_eq!(ticket, c.ticket);
            return;
        }
        rt.park();
    }
}

/// Return once `done` holds, checking it each millisecond.
fn wait_for<R: Runtime>(rt: &R, done: impl Fn() -> bool) {
    while !done() {
        sleep(rt, Duration::from_millis(1));
    }
}

fn sim_seeds(name: &str, body: impl Fn(&SimRuntime)) {
    for seed in sweep(SEEDS) {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.3,
                ..Config::default()
            },
        );
        let outcome = sim.run(|rt| body(&rt));
        assert_eq!(Outcome::Finished, outcome, "{name}, seed {seed}");
    }
}

fn sim_once(body: impl Fn(&SimRuntime)) {
    sim_seeds_in(0..1, body);
}

fn sim_seeds_in(seeds: Range<u64>, body: impl Fn(&SimRuntime)) {
    for seed in seeds {
        let mut sim = Sim::new(seed, Config::default());
        let outcome = sim.run(|rt| body(&rt));
        assert_eq!(Outcome::Finished, outcome, "seed {seed}");
    }
}

/// Returns a runtime over a new directory of its own, `name`.
fn real(name: &str) -> OsRuntime {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("tasks")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    OsRuntime::new(dir, Backend::Pool { threads: 2 })
}

fn ints(vs: &[Value]) -> BTreeSet<i64> {
    vs.iter().map(|v| v.int().unwrap()).collect()
}

fn read_opts() -> TaskOptions {
    TaskOptions::default()
}

#[test]
fn a_task_returns_what_the_call_returns() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f.submit(1, read_opts());
        let (rows, done) = drive(rt, &task);
        assert_eq!(rows.len(), ints(&rows).len(), "each element once");
        assert_eq!(ints(&called(&f.db, 1)), ints(&rows));
        assert!(matches!(done, Completion::Read { .. }), "{done:?}");
        let task = f.submit(2, read_opts());
        let (rows, _) = drive(rt, &task);
        assert_eq!(vec![Value::Int(3000)], rows);
        f.db.close().unwrap();
    });
}

/// Returns the elements the registered call `ordinal` returns.
fn called<R: Runtime>(db: &Database<R>, ordinal: u32) -> Vec<Value> {
    let adtb::Called::Rows(rows) = db
        .registered("shop")
        .unwrap()
        .call(ordinal, &[], &CallOptions::default())
        .unwrap()
    else {
        panic!("a query returns rows")
    };
    rows.collect::<Result<_, _>>().unwrap()
}

/// Supply `step` bytes of capacity each time the task waits for it, releasing every batch once
/// taken, and return the elements with, after each batch, the bytes of the batches so far and
/// the bytes the task holds.
fn drive_released<R: Runtime>(
    rt: &R,
    task: &Task<R>,
    step: usize,
) -> (Vec<Value>, Vec<(usize, usize)>) {
    let (_, waker) = wakes(rt);
    task.supply(step);
    let (mut rows, mut bytes, mut held) = (Vec::new(), 0, Vec::new());
    loop {
        match task.observe(&waker) {
            Observation::Pending => {
                task.supply(step);
                rt.park();
            }
            Observation::Layout(_) => {}
            Observation::Batch(b) => {
                rows.extend(b.rows().expect("a rows batch").iter().cloned());
                bytes += b.bytes();
                b.release();
                held.push((bytes, task.held()));
            }
            Observation::End => {
                task.drained();
                return (rows, held);
            }
            Observation::Done(c) => panic!("the task ends before its batches: {c:?}"),
        }
    }
}

/// Elements repeating across units of pages are handed out, and charged, once.
#[test]
fn a_streamed_set_repeats_no_element_across_units() {
    sim_once(|rt| {
        let f = open(rt, ROWS);
        let task = f.submit(6, read_opts());
        let (rows, held) = drive_released(rt, &task, 1);
        assert_eq!(rows.len(), ints(&rows).len(), "each element once");
        assert_eq!(BTreeSet::from([0, 1, 2, 3, 4, 5, 6, 9]), ints(&rows));
        for (bytes, held) in held {
            assert_eq!(bytes, held, "the sent elements are charged once each");
        }
        f.db.close().unwrap();
    });
}

/// A join whose build is partitioned defers its probes to the production's last unit, which
/// runs after batches were handed out.
#[test]
fn a_streamed_partitioned_join_returns_what_the_call_returns() {
    sim_once(|rt| {
        let f = open_under(rt, 1000, 2, Some(256 << 10));
        let want = ints(&called(&f.db, 7));
        let task = f.submit(7, read_opts());
        let (rows, held) = drive_released(rt, &task, 1);
        assert!(held.len() > 1, "{} batches", held.len());
        assert_eq!(rows.len(), ints(&rows).len(), "each element once");
        assert_eq!(want, ints(&rows));
        assert_eq!((0..200).collect::<BTreeSet<i64>>(), ints(&rows));
        assert!(
            rows[..100].iter().all(|v| v.int().unwrap() >= 100),
            "the scanned elements come before the deferred ones"
        );
        f.db.close().unwrap();
    });
}

#[test]
fn an_arrow_task_returns_record_batches() {
    sim_once(|rt| {
        let f = open(rt, 100);
        let task = f.submit(
            1,
            TaskOptions {
                format: Format::Arrow,
                ..TaskOptions::default()
            },
        );
        let (_, waker) = wakes(rt);
        task.supply(usize::MAX);
        let Observation::Layout(layout) = next(rt, &task, &waker) else {
            panic!("the layout comes first")
        };
        assert!(layout.arrow().is_some());
        let mut n = 0;
        loop {
            match next(rt, &task, &waker) {
                Observation::Batch(b) => n += b.record_batch().expect("an Arrow batch").num_rows(),
                Observation::End => task.drained(),
                Observation::Done(_) => break,
                o => panic!("unexpected {o:?}"),
            }
        }
        assert_eq!(100, n);
    });
}

/// A reader that supplies little capacity and takes nothing stops production short of the
/// result: rows evaluated, not rows sent, stay within one unit of work past the capacity.
fn a_slow_reader_stops_production<R: Runtime>(rt: &R) {
    let f = open(rt, ROWS);
    let task = f.submit(1, read_opts());
    let (_, waker) = wakes(rt);
    task.supply(1);
    assert!(matches!(next(rt, &task, &waker), Observation::Layout(_)));
    assert!(matches!(next(rt, &task, &waker), Observation::Batch(_)));
    sleep(rt, Duration::from_millis(50));
    let produced = f.seen.rows.load(Ordering::Acquire);
    assert!(produced > 0);
    assert!(
        produced < ROWS / 2,
        "{produced} of {ROWS} rows produced for one byte of capacity"
    );
    sleep(rt, Duration::from_millis(50));
    assert_eq!(
        produced,
        f.seen.rows.load(Ordering::Acquire),
        "production stopped"
    );
    let (rows, done) = drive(rt, &task);
    assert!(matches!(done, Completion::Read { .. }), "{done:?}");
    assert!(rows.len() + 1024 >= ROWS - produced);
    drop(task);
    f.db.close().unwrap();
}

#[test]
fn a_slow_reader_stops_production_sim() {
    sim_seeds("slow reader", a_slow_reader_stops_production);
}

#[test]
fn a_slow_reader_stops_production_real() {
    a_slow_reader_stops_production(&real("slow-reader"));
}

#[test]
fn observe_never_runs_a_statement() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        assert!(matches!(
            task.observe(&waker),
            Observation::Layout(_) | Observation::Pending
        ));
        for _ in 0..20 {
            assert!(matches!(task.observe(&waker), Observation::Pending));
            sleep(rt, Duration::from_millis(1));
        }
        assert_eq!(
            0,
            f.seen.rows.load(Ordering::Acquire),
            "no capacity, no production"
        );
        drive(rt, &task);
        assert!(
            !f.seen.threads.lock().unwrap().contains(&rt.current()),
            "the observer runs no statement"
        );
    });
}

/// A pending observation registered before a batch is formed is woken.
fn a_pending_observe_wakes<R: Runtime>(rt: &R) {
    let f = open(rt, 10);
    let task = f.submit(1, read_opts());
    let (count, waker) = wakes(rt);
    while !matches!(task.observe(&waker), Observation::Pending) {}
    task.supply(usize::MAX);
    while matches!(task.observe(&waker), Observation::Pending) {
        rt.park();
    }
    assert!(count.count.load(Ordering::Acquire) > 0);
    drive(rt, &task);
}

#[test]
fn a_pending_observe_wakes_sim() {
    sim_seeds("pending observe", a_pending_observe_wakes);
}

#[test]
fn a_pending_observe_wakes_real() {
    a_pending_observe_wakes(&real("pending-observe"));
}

/// Capacity supplied while production waits for it wakes production.
fn a_supply_wakes_a_waiting_producer<R: Runtime>(rt: &R) {
    let f = open(rt, 1000);
    let task = f.submit(1, read_opts());
    let (_, waker) = wakes(rt);
    let mut n = 0;
    loop {
        task.supply(4096);
        match next(rt, &task, &waker) {
            Observation::Layout(_) => {}
            Observation::Batch(b) => n += b.len(),
            Observation::End => task.drained(),
            Observation::Done(_) => break,
            Observation::Pending => unreachable!(),
        }
    }
    assert_eq!(1000, n);
}

#[test]
fn a_supply_wakes_a_waiting_producer_sim() {
    sim_seeds("supply", a_supply_wakes_a_waiting_producer);
}

#[test]
fn a_supply_wakes_a_waiting_producer_real() {
    a_supply_wakes_a_waiting_producer(&real("supply"));
}

/// The read's snapshot stays held after its last batch is formed, until the drain is
/// acknowledged: vacuum removes no version the read could see.
#[test]
fn the_snapshot_is_held_until_drained() {
    sim_once(|rt| {
        let f = open(rt, 100);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        task.supply(usize::MAX);
        while !matches!(next(rt, &task, &waker), Observation::End) {}
        text(&f.db, "delete i in items");
        assert_eq!(
            0,
            f.vacuumed(),
            "the encoded but unsent result holds its snapshot"
        );
        task.drained();
        assert!(matches!(
            next(rt, &task, &waker),
            Observation::Done(Completion::Read { .. })
        ));
        assert_eq!(100, f.vacuumed());
    });
}

#[test]
fn a_dropped_task_releases_its_snapshot() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        task.supply(1);
        while !matches!(next(rt, &task, &waker), Observation::Batch(_)) {}
        text(&f.db, "delete i in items");
        assert_eq!(0, f.vacuumed());
        drop(task);
        sleep(rt, Duration::from_millis(5));
        assert_eq!(3000, f.vacuumed());
    });
}

#[test]
fn a_transferred_batch_stays_charged_until_released() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        task.supply(1);
        let batch = loop {
            if let Observation::Batch(b) = next(rt, &task, &waker) {
                break b;
            }
        };
        let before = task.held();
        assert!(batch.bytes() > 0);
        assert!(before >= batch.bytes());
        let bytes = batch.bytes();
        batch.release();
        assert_eq!(before - bytes, task.held());
    });
}

/// A write that waits for a transaction which then commits fails its attempt with a
/// serialization failure and reruns; only the committed attempt's result is handed out.
fn a_retried_write_publishes_only_the_committed_attempt<R: Runtime>(rt: &R) {
    let f = open(rt, 0);
    let mut txn = f.db.begin(&TxnOptions::default()).unwrap();
    txn.query_text(UPDATE, &[], &TxnCallOptions::default())
        .unwrap();
    let task = f.submit(5, read_opts());
    task.supply(usize::MAX);
    let (_, waker) = wakes(rt);
    wait_for(rt, || f.seen.rows.load(Ordering::Acquire) > 0);
    assert!(
        matches!(task.observe(&waker), Observation::Pending),
        "nothing before commit"
    );
    txn.commit(None).unwrap();
    let (rows, done) = drive(rt, &task);
    let Completion::Committed { attempts, .. } = done else {
        panic!("the write commits: {done:?}")
    };
    assert_eq!(2, attempts);
    assert_eq!(1, rows.len());
    assert_eq!(1, f.items());
    let ids = text(&f.db, "from i in items select i.id");
    assert_eq!(ids[0], rows);
}

#[test]
fn a_retried_write_publishes_only_the_committed_attempt_sim() {
    sim_seeds(
        "retried write",
        a_retried_write_publishes_only_the_committed_attempt,
    );
}

#[test]
fn a_retried_write_publishes_only_the_committed_attempt_real() {
    a_retried_write_publishes_only_the_committed_attempt(&real("retried-write"));
}

#[test]
fn cancel_before_commit_has_no_effect() {
    sim_once(|rt| {
        let f = open(rt, 0);
        let mut txn = f.db.begin(&TxnOptions::default()).unwrap();
        txn.query_text(UPDATE, &[], &TxnCallOptions::default())
            .unwrap();
        let task = f.submit(3, read_opts());
        sleep(rt, Duration::from_millis(10));
        task.cancel();
        let (_, done) = drive(rt, &task);
        assert_eq!(
            Completion::Failed(Error::Rejected {
                reason: RejectReason::Cancelled
            }),
            done
        );
        txn.commit(None).unwrap();
        assert_eq!(0, f.items());
    });
}

#[test]
fn cancel_after_commit_keeps_the_commit() {
    sim_once(|rt| {
        let f = open(rt, 0);
        let task = f.submit(3, read_opts());
        while f.items() == 0 {
            sleep(rt, Duration::from_millis(1));
        }
        task.cancel();
        let (rows, done) = drive(rt, &task);
        assert!(
            matches!(
                done,
                Completion::Committed {
                    incomplete: false,
                    ..
                }
            ),
            "{done:?}"
        );
        assert_eq!(1, rows.len());
    });
}

/// A cancel racing the commit settles once: cancelled exactly when nothing was written.
fn cancel_racing_commit_settles_once<R: Runtime>(rt: &R) {
    let f = open(rt, 0);
    for round in 0..4 {
        let before = f.items();
        let task = f.submit(3, read_opts());
        if round % 2 == 1 {
            sleep(rt, Duration::from_micros(200));
        }
        task.cancel();
        let (_, done) = drive(rt, &task);
        let after = f.items();
        match done {
            Completion::Committed { .. } => assert_eq!(before + 1, after, "round {round}"),
            Completion::Failed(Error::Rejected {
                reason: RejectReason::Cancelled,
            }) => assert_eq!(before, after, "round {round}"),
            other => panic!("round {round}: {other:?}"),
        }
    }
}

#[test]
fn cancel_racing_commit_settles_once_sim() {
    sim_seeds("cancel racing commit", cancel_racing_commit_settles_once);
}

#[test]
fn cancel_racing_commit_settles_once_real() {
    cancel_racing_commit_settles_once(&real("cancel-commit"));
}

#[test]
fn idle_before_commit_fails_stream_idle() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        task.supply(1);
        while !matches!(next(rt, &task, &waker), Observation::Batch(_)) {}
        let limit = Duration::from_secs(30);
        task.expire_idle(limit);
        let Observation::Done(done) = next(rt, &task, &waker) else {
            panic!("the task ends")
        };
        assert_eq!(
            Completion::Failed(Error::Retry {
                reason: RetryReason::StreamIdle { limit }
            }),
            done
        );
    });
}

#[test]
fn idle_after_commit_marks_output_incomplete() {
    sim_once(|rt| {
        let f = open(rt, 0);
        let task = f.submit(3, read_opts());
        while f.items() == 0 {
            sleep(rt, Duration::from_millis(1));
        }
        task.expire_idle(Duration::from_secs(30));
        let (_, waker) = wakes(rt);
        let done = loop {
            if let Observation::Done(c) = next(rt, &task, &waker) {
                break c;
            }
        };
        assert!(
            matches!(
                done,
                Completion::Committed {
                    incomplete: true,
                    ..
                }
            ),
            "{done:?}"
        );
        assert_eq!(1, f.items());
    });
}

#[test]
fn a_drained_write_keeps_its_outcome() {
    sim_once(|rt| {
        let f = open(rt, 0);
        let task = f.submit(3, read_opts());
        let (rows, done) = drive(rt, &task);
        assert_eq!(1, rows.len());
        assert!(
            matches!(
                done,
                Completion::Committed {
                    incomplete: false,
                    ..
                }
            ),
            "{done:?}"
        );
        task.expire_idle(Duration::from_secs(1));
        task.cancel();
        let (_, waker) = wakes(rt);
        let Observation::Done(again) = next(rt, &task, &waker) else {
            panic!("the task stays done")
        };
        assert_eq!(done, again);
    });
}

/// A request after the drain acknowledgement leaves a read to commit: its whole result was
/// sent.
#[test]
fn a_drained_read_keeps_its_outcome() {
    sim_seeds("drained read", |rt| {
        let f = open(rt, 100);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        task.supply(usize::MAX);
        let mut rows = Vec::new();
        loop {
            match next(rt, &task, &waker) {
                Observation::Layout(_) => {}
                Observation::Batch(b) => {
                    rows.extend(b.rows().expect("a rows batch").iter().cloned());
                }
                Observation::End => break,
                o => panic!("unexpected {o:?}"),
            }
        }
        task.drained();
        task.expire_idle(Duration::from_secs(1));
        task.cancel();
        let Observation::Done(done) = next(rt, &task, &waker) else {
            panic!("the task ends")
        };
        assert!(matches!(done, Completion::Read { .. }), "{done:?}");
        assert_eq!(100, ints(&rows).len());
    });
}

/// A transport that acknowledges the drain before the end of the batches can still cancel.
#[test]
fn an_early_drained_then_cancelled_read_settles() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        task.supply(1);
        while !matches!(next(rt, &task, &waker), Observation::Batch(_)) {}
        task.drained();
        task.cancel();
        let mut done = None;
        for _ in 0..200 {
            match task.observe(&waker) {
                Observation::Done(c) => {
                    done = Some(c);
                    break;
                }
                _ => sleep(rt, Duration::from_millis(1)),
            }
        }
        assert_eq!(
            Some(Completion::Failed(Error::Rejected {
                reason: RejectReason::Cancelled
            })),
            done
        );
    });
}

/// A task dropped after a drain acknowledged before the end of the batches releases its
/// snapshot.
#[test]
fn an_early_drained_then_dropped_read_releases_its_snapshot() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        task.supply(1);
        while !matches!(next(rt, &task, &waker), Observation::Batch(_)) {}
        text(&f.db, "delete i in items");
        task.drained();
        drop(task);
        sleep(rt, Duration::from_millis(5));
        assert_eq!(3000, f.vacuumed());
    });
}

#[test]
fn closing_the_database_stops_its_tasks() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        task.supply(1);
        while !matches!(next(rt, &task, &waker), Observation::Batch(_)) {}
        f.db.close().unwrap();
        let done = loop {
            if let Observation::Done(c) = next(rt, &task, &waker) {
                break c;
            }
        };
        assert_eq!(
            Completion::Failed(Error::Rejected {
                reason: RejectReason::Closed
            }),
            done
        );
    });
}

#[test]
fn a_role_other_than_the_owner_is_refused() {
    sim_once(|rt| {
        let f = open(rt, 0);
        let authority = f.db.role_authority();
        authority.create_role("web").unwrap();
        let web = authority.session("web").unwrap();
        let err =
            f.db.registered("shop")
                .unwrap()
                .submit(&web, 1, Vec::new(), read_opts())
                .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Rejected {
                    reason: RejectReason::Denied { .. }
                }
            ),
            "{err:?}"
        );
        let err = web
            .submit_text("from i in items select i.n", &[], read_opts())
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::Rejected {
                    reason: RejectReason::Denied { .. }
                }
            ),
            "{err:?}"
        );
    });
}

#[test]
fn ad_hoc_text_runs_as_a_task() {
    sim_once(|rt| {
        let f = open(rt, 50);
        let task = f
            .owner
            .submit_text(
                "from i in items where i.n < 10 select i.n",
                &[],
                read_opts(),
            )
            .unwrap();
        let (rows, done) = drive(rt, &task);
        assert_eq!((0..10).collect::<BTreeSet<i64>>(), ints(&rows));
        assert!(matches!(done, Completion::Read { .. }), "{done:?}");
        let task = f
            .owner
            .submit_text("insert items { n: 99 }", &[], read_opts())
            .unwrap();
        let (_, done) = drive(rt, &task);
        assert!(matches!(done, Completion::Committed { .. }), "{done:?}");
        assert_eq!(51, f.items());
    });
}

#[test]
fn a_waiting_write_fails_at_its_statement_limit() {
    sim_once(|rt| {
        let f = open(rt, 0);
        let mut txn = f.db.begin(&TxnOptions::default()).unwrap();
        txn.query_text(UPDATE, &[], &TxnCallOptions::default())
            .unwrap();
        let after = Duration::from_millis(30);
        let task = f.submit(
            3,
            TaskOptions {
                call: CallOptions {
                    limit: Some(after),
                    ..CallOptions::default()
                },
                ..TaskOptions::default()
            },
        );
        let (_, done) = drive(rt, &task);
        assert_eq!(
            Completion::Failed(Error::Rejected {
                reason: RejectReason::StatementLimit {
                    limit: adtb::Limit::Time { after }
                }
            }),
            done
        );
        txn.rollback().unwrap();
        assert_eq!(0, f.items());
    });
}

#[test]
fn a_finished_task_holds_no_memory_grant() {
    sim_once(|rt| {
        let f = open(rt, 10);
        let finished: Vec<Task<SimRuntime>> = (0..4)
            .map(|_| {
                let task = f.submit(2, read_opts());
                let (_, done) = drive(rt, &task);
                assert!(matches!(done, Completion::Read { .. }), "{done:?}");
                task
            })
            .collect();
        assert_eq!(10, f.items(), "a call runs beside four finished tasks");
        let task = f.submit(2, read_opts());
        assert_eq!(vec![Value::Int(10)], drive(rt, &task).0);
        drop(finished);
    });
}

/// With one worker, a deferrable read waiting for a writer that itself waits leaves the worker
/// to that writer.
#[test]
fn a_deferrable_read_task_waits_without_its_worker() {
    sim_seeds("deferrable task", |rt| {
        let f = open_with(rt, 1, 1);
        let mut txn = f.db.begin(&TxnOptions::default()).unwrap();
        txn.query_text(UPDATE, &[], &TxnCallOptions::default())
            .unwrap();
        let writer = f.submit(5, read_opts());
        wait_for(rt, || f.seen.rows.load(Ordering::Acquire) > 0);
        let reader = f
            .owner
            .submit_text(
                "transact read_only deferrable\n  return count(i for i in items)",
                &[],
                read_opts(),
            )
            .unwrap();
        sleep(rt, Duration::from_millis(10));
        txn.commit(None).unwrap();
        let (_, done) = drive(rt, &writer);
        assert!(matches!(done, Completion::Committed { .. }), "{done:?}");
        let (rows, done) = drive(rt, &reader);
        assert!(matches!(done, Completion::Read { .. }), "{done:?}");
        assert!(
            rows == vec![Value::Int(1)] || rows == vec![Value::Int(2)],
            "{rows:?}"
        );
        f.db.close().unwrap();
    });
}

/// Time a streamed read waits for capacity is not time its statement executes.
#[test]
fn a_slow_reader_is_no_statement_time() {
    sim_once(|rt| {
        let f = open(rt, ROWS);
        let task = f.submit(
            1,
            TaskOptions {
                call: CallOptions {
                    limit: Some(Duration::from_millis(100)),
                    ..CallOptions::default()
                },
                ..TaskOptions::default()
            },
        );
        let (_, waker) = wakes(rt);
        task.supply(1);
        assert!(matches!(next(rt, &task, &waker), Observation::Layout(_)));
        let Observation::Batch(first) = next(rt, &task, &waker) else {
            panic!("a batch follows the layout")
        };
        sleep(rt, Duration::from_millis(150));
        let (rows, done) = drive(rt, &task);
        assert!(matches!(done, Completion::Read { .. }), "{done:?}");
        assert_eq!(ROWS, first.len() + rows.len());
    });
}

#[test]
fn a_call_stating_no_priority_runs_first_in_its_class() {
    sim_once(|rt| {
        let f = open_with(rt, 1, 1);
        let hold = f.submit(4, read_opts());
        hold.supply(usize::MAX);
        wait_for(rt, || f.gate.entered.load(Ordering::Acquire));
        let lowered = f.submit(
            3,
            TaskOptions {
                call: CallOptions {
                    priority: Some(1),
                    ..CallOptions::default()
                },
                ..TaskOptions::default()
            },
        );
        let unstated = f.submit(3, read_opts());
        f.gate.release(rt);
        drive(rt, &hold);
        let (first, _) = drive(rt, &unstated);
        let (second, _) = drive(rt, &lowered);
        assert!(first < second, "{first:?} ran after {second:?}");
    });
}

/// Once a committed write's output is abandoned, it forms no more batches.
#[test]
fn an_abandoned_output_forms_no_more_batches() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f
            .owner
            .submit_text(
                "insert counters { value: 1 }\nfrom i in items select i.n",
                &[],
                read_opts(),
            )
            .unwrap();
        let (_, waker) = wakes(rt);
        task.supply(1);
        while !matches!(next(rt, &task, &waker), Observation::Batch(_)) {}
        task.expire_idle(Duration::from_secs(30));
        sleep(rt, Duration::from_millis(20));
        let Observation::Done(done) = next(rt, &task, &waker) else {
            panic!("a batch was formed after the output was abandoned")
        };
        assert!(
            matches!(
                done,
                Completion::Committed {
                    incomplete: true,
                    ..
                }
            ),
            "{done:?}"
        );
    });
}

/// After idle expiry, a read forms no more batches and fails.
#[test]
fn an_expired_read_forms_no_more_batches() {
    sim_once(|rt| {
        let f = open(rt, 3000);
        let task = f.submit(1, read_opts());
        let (_, waker) = wakes(rt);
        task.supply(1);
        while !matches!(next(rt, &task, &waker), Observation::Batch(_)) {}
        let limit = Duration::from_secs(30);
        task.expire_idle(limit);
        task.supply(usize::MAX);
        sleep(rt, Duration::from_millis(20));
        let done = next(rt, &task, &waker);
        assert!(
            matches!(
                done,
                Observation::Done(Completion::Failed(Error::Retry {
                    reason: RetryReason::StreamIdle { .. }
                }))
            ),
            "{done:?}"
        );
    });
}

#[test]
fn a_caller_token_stops_a_queued_task() {
    sim_once(|rt| {
        let f = open_with(rt, 1, 1);
        let hold = f.submit(4, read_opts());
        hold.supply(usize::MAX);
        wait_for(rt, || f.gate.entered.load(Ordering::Acquire));
        let token = CancelToken::new();
        let task = f.submit(
            2,
            TaskOptions {
                call: CallOptions {
                    cancel: Some(token.clone()),
                    ..CallOptions::default()
                },
                ..TaskOptions::default()
            },
        );
        token.cancel();
        let (_, done) = drive(rt, &task);
        assert_eq!(
            Completion::Failed(Error::Rejected {
                reason: RejectReason::Cancelled
            }),
            done
        );
        f.gate.release(rt);
        drive(rt, &hold);
    });
}

#[test]
fn a_caller_token_stops_a_waiting_task() {
    sim_once(|rt| {
        let f = open(rt, 0);
        let mut txn = f.db.begin(&TxnOptions::default()).unwrap();
        txn.query_text(UPDATE, &[], &TxnCallOptions::default())
            .unwrap();
        let token = CancelToken::new();
        let task = f.submit(
            5,
            TaskOptions {
                call: CallOptions {
                    cancel: Some(token.clone()),
                    ..CallOptions::default()
                },
                ..TaskOptions::default()
            },
        );
        wait_for(rt, || f.seen.rows.load(Ordering::Acquire) > 0);
        token.cancel();
        let (_, done) = drive(rt, &task);
        assert_eq!(
            Completion::Failed(Error::Rejected {
                reason: RejectReason::Cancelled
            }),
            done
        );
        txn.rollback().unwrap();
        assert_eq!(0, f.items());
    });
}

/// A close concurrent with another returns only once the file and its lock are released.
#[test]
fn a_concurrent_close_returns_once_the_file_is_released() {
    let rt = real("concurrent-close");
    let path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("tasks")
        .join("concurrent-close")
        .join("shop.adt");
    let (seen, gate) = (Arc::new(Seen::default()), Arc::new(Gate::default()));
    let opts = options(&rt, &seen, &gate, 1);
    let log = Log::new([SCHEMA]);
    let db = Database::open(&path, &log, &opts).unwrap();
    text(&db, &fill(10));
    let owner = db.role_authority().session("owner").unwrap();
    let hold = db
        .registered("shop")
        .unwrap()
        .submit(&owner, 4, Vec::new(), read_opts())
        .unwrap();
    hold.supply(usize::MAX);
    wait_for(&rt, || gate.entered.load(Ordering::Acquire));
    let reopen = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..2 {
            let (db, reopen, path, log, opts) = (db.clone(), &reopen, &path, &log, &opts);
            s.spawn(move || {
                db.close().unwrap();
                let mut reopened = reopen.lock().unwrap();
                let again = Database::open(path, log, opts);
                reopened.push(again.as_ref().err().map(|e| format!("{e:?}")));
                if let Ok(again) = again {
                    again.close().unwrap();
                }
            });
        }
        sleep(&rt, Duration::from_millis(100));
        gate.release(&rt);
    });
    assert_eq!(vec![None, None], reopen.into_inner().unwrap());
    drop(hold);
}

/// A waker that closes the database, then unparks the thread that made it, counting the wakes.
struct Closer<R: Runtime> {
    db: Database<R>,
    wakes: Arc<Wakes<R>>,
}

fn closer<R: Runtime>(rt: &R, db: &Database<R>) -> (Arc<Wakes<R>>, Waker) {
    let (wakes, _) = wakes(rt);
    let waker = Waker::from(Arc::new(Closer {
        db: db.clone(),
        wakes: wakes.clone(),
    }));
    (wakes, waker)
}

impl<R: Runtime> Wake for Closer<R> {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.db.clone().close().unwrap();
        self.wakes.wake_by_ref();
    }
}

/// A close run on the worker by an observer's wake stops a queued task whose observer closes
/// again; neither close waits for the other.
#[test]
fn an_observer_closing_from_a_close_on_a_worker_returns() {
    sim_seeds(
        "an_observer_closing_from_a_close_on_a_worker_returns",
        |rt| {
            let f = open_with(rt, 0, 1);
            let hold = f.submit(4, read_opts());
            hold.supply(usize::MAX);
            wait_for(rt, || f.gate.entered.load(Ordering::Acquire));
            let queued = f.submit(2, read_opts());
            let (held_wakes, held) = closer(rt, &f.db);
            let (queued_wakes, waker) = closer(rt, &f.db);
            assert!(matches!(hold.observe(&held), Observation::Pending));
            assert!(matches!(queued.observe(&waker), Observation::Pending));
            f.gate.release(rt);
            wait_for(rt, || queued_wakes.count.load(Ordering::Acquire) > 0);
            assert!(held_wakes.count.load(Ordering::Acquire) > 0);
            let done = queued.observe(&waker);
            assert!(
                matches!(
                    done,
                    Observation::Done(Completion::Failed(Error::Rejected {
                        reason: RejectReason::Closed
                    }))
                ),
                "{done:?}"
            );
        },
    );
}

/// An observer woken on the worker while another thread closes the database closes it too,
/// and returns without waiting for that close.
#[test]
fn an_observer_closing_on_a_worker_during_a_close_returns() {
    sim_seeds(
        "an_observer_closing_on_a_worker_during_a_close_returns",
        |rt| {
            let f = open_with(rt, 0, 1);
            let hold = f.submit(4, read_opts());
            hold.supply(usize::MAX);
            wait_for(rt, || f.gate.entered.load(Ordering::Acquire));
            let (wakes, waker) = closer(rt, &f.db);
            assert!(matches!(hold.observe(&waker), Observation::Pending));
            let db = f.db.clone();
            let closing = rt.spawn("closer", move || db.close().unwrap());
            sleep(rt, Duration::from_millis(10));
            f.gate.release(rt);
            rt.join(closing);
            wait_for(rt, || wakes.count.load(Ordering::Acquire) > 0);
        },
    );
}

/// Returns the options of a task carrying `token`.
fn carrying(token: &CancelToken) -> TaskOptions {
    TaskOptions {
        call: CallOptions {
            cancel: Some(token.clone()),
            ..CallOptions::default()
        },
        ..TaskOptions::default()
    }
}

/// Stops a task its owner holds.
type Stop = fn(Task<SimRuntime>);

/// Cancelling, expiring or dropping a task leaves its caller's token, its sibling task and a
/// later call carrying the token unaffected.
#[test]
fn stopping_a_task_leaves_its_callers_token() {
    let stops: [(&str, Stop); 3] = [
        ("cancel", |t| t.cancel()),
        ("expire", |t| t.expire_idle(Duration::from_secs(30))),
        ("drop", drop),
    ];
    for (name, stop) in stops {
        sim_once(|rt| {
            let f = open_with(rt, 1, 1);
            let hold = f.submit(4, read_opts());
            hold.supply(usize::MAX);
            wait_for(rt, || f.gate.entered.load(Ordering::Acquire));
            let token = CancelToken::new();
            let stopped = f.submit(2, carrying(&token));
            let sibling = f.submit(2, carrying(&token));
            stop(stopped);
            f.gate.release(rt);
            drive(rt, &hold);
            let (rows, done) = drive(rt, &sibling);
            assert!(!matches!(done, Completion::Failed(_)), "{name}: {done:?}");
            assert_eq!(vec![Value::Int(1)], rows, "{name}");
            assert!(!token.is_cancelled(), "{name}");
            let later = f.db.query_text(
                "count(i for i in items)",
                &[],
                &CallOptions {
                    cancel: Some(token),
                    ..CallOptions::default()
                },
            );
            assert!(later.is_ok(), "{name}: {later:?}");
        });
    }
}

/// A queued task stopped at its deadline while no close runs wakes an observer that closes the
/// database, and that close returns.
#[test]
fn an_observer_closing_on_a_stop_outside_a_close_returns() {
    sim_seeds(
        "an_observer_closing_on_a_stop_outside_a_close_returns",
        |rt| {
            let f = open_with(rt, 0, 1);
            let hold = f.submit(4, read_opts());
            hold.supply(usize::MAX);
            wait_for(rt, || f.gate.entered.load(Ordering::Acquire));
            let queued = f.submit(
                2,
                TaskOptions {
                    call: CallOptions {
                        deadline: Some(rt.wall() + Duration::from_millis(10)),
                        ..CallOptions::default()
                    },
                    ..TaskOptions::default()
                },
            );
            let (wakes, waker) = closer(rt, &f.db);
            assert!(matches!(queued.observe(&waker), Observation::Pending));
            sleep(rt, Duration::from_millis(50));
            f.gate.release(rt);
            wait_for(rt, || wakes.count.load(Ordering::Acquire) > 0);
            let done = queued.observe(&waker);
            assert!(
                matches!(
                    done,
                    Observation::Done(Completion::Failed(Error::Rejected {
                        reason: RejectReason::DeadlineExceeded
                    }))
                ),
                "{done:?}"
            );
        },
    );
}

/// A started task waiting for capacity, stopped by a cancel while no close runs, wakes an
/// observer that closes the database, and that close returns.
#[test]
fn an_observer_closing_on_a_cancel_of_a_waiting_task_returns() {
    sim_seeds(
        "an_observer_closing_on_a_cancel_of_a_waiting_task_returns",
        |rt| {
            let f = open_with(rt, 3000, 1);
            let task = f.submit(1, read_opts());
            let (wakes, waker) = closer(rt, &f.db);
            let (_, parker) = self::wakes(rt);
            assert!(matches!(next(rt, &task, &parker), Observation::Layout(_)));
            assert!(matches!(task.observe(&waker), Observation::Pending));
            sleep(rt, Duration::from_millis(10));
            task.cancel();
            wait_for(rt, || wakes.count.load(Ordering::Acquire) > 0);
            let done = task.observe(&waker);
            assert!(
                matches!(
                    done,
                    Observation::Done(Completion::Failed(Error::Rejected {
                        reason: RejectReason::Cancelled
                    }))
                ),
                "{done:?}"
            );
        },
    );
}

/// A task carrying a token cancelled before it is submitted never runs a statement.
#[test]
fn a_task_carrying_a_cancelled_token_never_runs() {
    for seed in sweep(SEEDS) {
        let mut sim = Sim::new(
            seed,
            Config {
                switch_rate: 0.5,
                ..Config::default()
            },
        );
        let outcome = sim.run(|rt| {
            let f = open(&rt, 0);
            let token = CancelToken::new();
            token.cancel();
            let task = f.submit(5, carrying(&token));
            let (_, done) = drive(&rt, &task);
            assert_eq!(
                Completion::Failed(Error::Rejected {
                    reason: RejectReason::Cancelled
                }),
                done,
                "seed {seed}"
            );
            assert_eq!(0, f.seen.rows.load(Ordering::Acquire), "seed {seed}");
            assert_eq!(0, f.items(), "seed {seed}");
        });
        assert_eq!(Outcome::Finished, outcome, "seed {seed}");
    }
}
