use adtb::plan::fixture as planfix;

use adtb_simkit::support;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Wake, Waker};
use std::time::Duration;

use adtb::exec::{
    self, Admission, Budget, Entry, Overloaded, Priority, Resumable, Scheduler, Step, Stopped,
    Task, Val,
};
use adtb::mvcc::{Limits, Waits, Watch, Xid};
use adtb::runtime::{Instant, Io, Op, Runtime, ThreadId, Threads};
use adtb::write::{self, Bindings, Bounds, Database, Deferral, Failed, Local, Setup, Text, Txn};
use adtb::{CallOptions, CreateOptions, Log as SchemaLog, OpenOptions, Registration};
use adtb::{TxnCallOptions, TxnOptions, Value};
use adtb_lang::check::check_adhoc;
use adtb_lang::kernel::{Terms, Tm};
use adtb_lang::norm::{self, NormalProgram};
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::schema::Schema;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_sys::os::{Backend, OsRuntime};
use adtb_testkit::fixture::Fixture;

use adtb_simkit::seeds::sweep;
use support::{Db, World};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Ran(&'static str),
    Waiting(&'static str),
    Done(&'static str),
    Stopped(&'static str, Stopped),
    Failed(&'static str, String),
}

/// What the calls of one test did, waking the thread that made it on each event.
struct Log<R: Runtime> {
    rt: R,
    main: ThreadId,
    events: Mutex<Vec<Event>>,
}

impl<R: Runtime> Log<R> {
    fn new(rt: &R) -> Arc<Log<R>> {
        Arc::new(Log {
            rt: rt.clone(),
            main: rt.current(),
            events: Mutex::new(Vec::new()),
        })
    }

    fn push(&self, e: Event) {
        self.events.lock().unwrap().push(e);
        self.rt.unpark(self.main);
    }

    fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    fn count(&self, f: impl Fn(&Event) -> bool) -> usize {
        self.events.lock().unwrap().iter().filter(|e| f(e)).count()
    }

    /// Park the calling thread, the one that made the log, until `done` holds of the events.
    fn until(&self, done: impl Fn(&[Event]) -> bool) {
        while !done(&self.events.lock().unwrap()) {
            self.rt.park();
        }
    }

    fn has(&self, e: &Event) -> bool {
        self.events.lock().unwrap().contains(e)
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

fn entry(priority: Priority, start: u64, bytes: usize, deadline: Option<Instant>) -> Entry {
    Entry {
        priority,
        start: Instant::from_nanos(start),
        bytes,
        deadline,
    }
}

fn plain(start: u64) -> Entry {
    entry(Priority::Regular, start, 0, None)
}

/// A call that records that it ran, and finishes.
struct Note<R: Runtime> {
    name: &'static str,
    log: Arc<Log<R>>,
}

impl<R: Runtime> Resumable for Note<R> {
    fn run(&mut self, _: &Waker) -> Step {
        self.log.push(Event::Ran(self.name));
        self.log.push(Event::Done(self.name));
        Step::Done
    }

    fn stop(self: Box<Self>, why: Stopped) {
        self.log.push(Event::Stopped(self.name, why));
    }
}

fn note<R: Runtime>(name: &'static str, log: &Arc<Log<R>>) -> Box<Note<R>> {
    Box::new(Note {
        name,
        log: log.clone(),
    })
}

/// A call that waits for transaction `on` to end, and finishes. With `end_first`, it ends
/// `on` itself during its first step, after registering.
struct Gate<R: Runtime> {
    name: &'static str,
    on: Xid,
    waits: Arc<Waits<R>>,
    log: Arc<Log<R>>,
    registered: Option<Waker>,
    end_first: bool,
}

impl<R: Runtime> Resumable for Gate<R> {
    fn run(&mut self, waker: &Waker) -> Step {
        self.log.push(Event::Ran(self.name));
        match self.waits.watch(None, self.on, waker).unwrap() {
            Watch::Ended => {
                self.log.push(Event::Done(self.name));
                Step::Done
            }
            Watch::Pending => {
                self.registered = Some(waker.clone());
                if std::mem::take(&mut self.end_first) {
                    self.waits.ended(self.on);
                }
                self.log.push(Event::Waiting(self.name));
                Step::Wait
            }
        }
    }

    fn stop(self: Box<Self>, why: Stopped) {
        if let Some(w) = &self.registered {
            self.waits.withdraw(None, self.on, w);
        }
        self.log.push(Event::Stopped(self.name, why));
    }
}

fn gate<R: Runtime>(
    name: &'static str,
    on: Xid,
    waits: &Arc<Waits<R>>,
    log: &Arc<Log<R>>,
) -> Box<Gate<R>> {
    Box::new(Gate {
        name,
        on,
        waits: waits.clone(),
        log: log.clone(),
        registered: None,
        end_first: false,
    })
}

/// A call that ends transaction `on`, as its commit does.
struct End<R: Runtime> {
    name: &'static str,
    on: Xid,
    waits: Arc<Waits<R>>,
    log: Arc<Log<R>>,
}

impl<R: Runtime> Resumable for End<R> {
    fn run(&mut self, _: &Waker) -> Step {
        self.log.push(Event::Ran(self.name));
        self.waits.ended(self.on);
        self.log.push(Event::Done(self.name));
        Step::Done
    }

    fn stop(self: Box<Self>, why: Stopped) {
        self.log.push(Event::Stopped(self.name, why));
    }
}

/// A task that keeps its worker until released.
struct Hold<R: Runtime> {
    rt: R,
    state: Mutex<(bool, Option<ThreadId>)>,
}

impl<R: Runtime> Hold<R> {
    fn new(rt: &R) -> Arc<Hold<R>> {
        Arc::new(Hold {
            rt: rt.clone(),
            state: Mutex::new((false, None)),
        })
    }

    fn task(self: &Arc<Self>) -> Task {
        let hold = self.clone();
        Box::new(move || {
            hold.state.lock().unwrap().1 = Some(hold.rt.current());
            while !hold.state.lock().unwrap().0 {
                hold.rt.park();
            }
        })
    }

    fn release(&self) {
        let held = {
            let mut st = self.state.lock().unwrap();
            st.0 = true;
            st.1
        };
        if let Some(t) = held {
            self.rt.unpark(t);
        }
    }
}

const X: Xid = Xid(7);
const SEEDS: Range<u64> = 0..8;

fn admission_bounds_entries_and_bytes<R: Runtime>(rt: &R) {
    let pool = Scheduler::start_with(
        rt.clone(),
        0,
        Admission {
            entries: 3,
            bytes: 100,
        },
    );
    let log = Log::new(rt);
    let bytes = |n| entry(Priority::Regular, 0, n, None);
    let a = pool.admit(bytes(40), note("a", &log)).unwrap();
    pool.admit(bytes(40), note("b", &log)).unwrap();
    assert_eq!(
        Overloaded,
        pool.admit(bytes(30), note("c", &log)).unwrap_err()
    );
    pool.admit(bytes(20), note("c", &log)).unwrap();
    assert_eq!(
        Overloaded,
        pool.admit(bytes(0), note("d", &log)).unwrap_err()
    );
    a.cancel();
    pool.admit(bytes(40), note("e", &log)).unwrap();
    assert_eq!(
        Overloaded,
        pool.admit(bytes(0), note("f", &log)).unwrap_err()
    );
    assert!(pool.help(), "the cancelled call's stop is a task");
    assert_eq!(vec![Event::Stopped("a", Stopped::Cancelled)], log.events());
    a.cancel();
    assert!(!pool.help());
    pool.shutdown();
}

fn calls_run_by_priority_then_start<R: Runtime>(rt: &R) {
    let pool = Scheduler::start(rt.clone(), 1);
    let log = Log::new(rt);
    let hold = Hold::new(rt);
    pool.submit(hold.task());
    let calls = [
        ("adhoc-1", Priority::AdHoc, 1),
        ("regular-5", Priority::Regular, 5),
        ("background-0", Priority::Background, 0),
        ("regular-2", Priority::Regular, 2),
        ("regular-2b", Priority::Regular, 2),
        ("regular-1", Priority::Regular, 1),
    ];
    for (name, p, start) in calls {
        pool.admit(entry(p, start, 0, None), note(name, &log))
            .unwrap();
    }
    hold.release();
    log.until(|es| es.iter().filter(|e| matches!(e, Event::Done(_))).count() == calls.len());
    let done: Vec<&str> = log
        .events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Done(n) => Some(n),
            _ => None,
        })
        .collect();
    assert_eq!(
        vec![
            "regular-1",
            "regular-2",
            "regular-2b",
            "regular-5",
            "background-0",
            "adhoc-1"
        ],
        done
    );
    pool.shutdown();
}

/// Run queued tasks on the calling thread until `done` holds of `log`.
fn help_until<R: Runtime>(pool: &Scheduler<R>, log: &Log<R>, done: impl Fn(&[Event]) -> bool) {
    while !done(&log.events()) {
        if !pool.help() {
            pool.runtime().yield_now();
        }
    }
}

fn queued_calls_stop_on_cancel_and_deadline<R: Runtime>(rt: &R) {
    let pool = Scheduler::start_with(
        rt.clone(),
        0,
        Admission {
            entries: 3,
            bytes: 100,
        },
    );
    let log = Log::new(rt);
    let soon = rt.now() + Duration::from_millis(10);
    let late = rt.now() + Duration::from_secs(3600);
    let a = pool.admit(plain(0), note("a", &log)).unwrap();
    pool.admit(entry(Priority::Regular, 0, 0, Some(soon)), note("b", &log))
        .unwrap();
    pool.admit(entry(Priority::Regular, 0, 0, Some(late)), note("c", &log))
        .unwrap();
    a.cancel();
    sleep(rt, Duration::from_millis(20));
    help_until(&pool, &log, |es| es.len() == 2);
    let mut events = log.events();
    events.sort_by_key(|e| format!("{e:?}"));
    assert_eq!(
        vec![
            Event::Stopped("a", Stopped::Cancelled),
            Event::Stopped("b", Stopped::DeadlineExceeded),
        ],
        events
    );
    pool.admit(plain(0), note("d", &log)).unwrap();
    pool.admit(plain(0), note("e", &log)).unwrap();
    assert_eq!(
        Overloaded,
        pool.admit(plain(0), note("f", &log)).unwrap_err()
    );
    pool.shutdown();
}

fn waiting_calls_stop_on_cancel_and_deadline<R: Runtime>(rt: &R) {
    let pool = Scheduler::start(rt.clone(), 1);
    let log = Log::new(rt);
    let waits = Arc::new(Waits::new(rt.clone()));
    waits.started(X);
    let soon = rt.now() + Duration::from_millis(10);
    let g1 = pool.admit(plain(0), gate("g1", X, &waits, &log)).unwrap();
    pool.admit(
        entry(Priority::Regular, 0, 0, Some(soon)),
        gate("g2", X, &waits, &log),
    )
    .unwrap();
    log.until(|es| {
        es.contains(&Event::Waiting("g1"))
            && (es.contains(&Event::Waiting("g2"))
                || es.contains(&Event::Stopped("g2", Stopped::DeadlineExceeded)))
    });
    g1.cancel();
    log.until(|es| es.contains(&Event::Stopped("g1", Stopped::Cancelled)));
    log.until(|es| es.contains(&Event::Stopped("g2", Stopped::DeadlineExceeded)));
    waits.ended(X);
    pool.admit(plain(1), note("after", &log)).unwrap();
    log.until(|es| es.contains(&Event::Done("after")));
    assert!(!log.has(&Event::Done("g1")));
    assert!(!log.has(&Event::Done("g2")));
    pool.shutdown();
}

struct Flag<R: Runtime> {
    rt: R,
    main: ThreadId,
    set: AtomicBool,
}

impl<R: Runtime> Wake for Flag<R> {
    fn wake(self: Arc<Self>) {
        self.set.store(true, Ordering::Release);
        self.rt.unpark(self.main);
    }
}

/// Race a registration with the end of the transaction it waits for, `rounds` times.
fn a_registration_racing_the_end_is_woken<R: Runtime>(rt: &R, rounds: usize) {
    for round in 0..rounds {
        let waits = Arc::new(Waits::new(rt.clone()));
        waits.started(X);
        let flag = Arc::new(Flag {
            rt: rt.clone(),
            main: rt.current(),
            set: AtomicBool::new(false),
        });
        let waker = Waker::from(flag.clone());
        let ender = {
            let waits = waits.clone();
            rt.spawn("ender", move || waits.ended(X))
        };
        if waits.watch(None, X, &waker).unwrap() == Watch::Pending {
            while !flag.set.load(Ordering::Acquire) {
                rt.park();
            }
        }
        rt.join(ender);
        assert_eq!(
            Watch::Ended,
            waits.watch(None, X, &waker).unwrap(),
            "round {round}"
        );
    }
}

/// A wake that comes while the call's step still runs runs it again.
fn a_wake_during_a_step_runs_the_call_again<R: Runtime>(rt: &R, rounds: usize) {
    let pool = Scheduler::start(rt.clone(), 2);
    let log = Log::new(rt);
    let waits = Arc::new(Waits::new(rt.clone()));
    waits.started(X);
    let mut g = gate("self", X, &waits, &log);
    g.end_first = true;
    pool.admit(plain(0), g).unwrap();
    log.until(|es| es.contains(&Event::Done("self")));
    for round in 0..rounds {
        let on = Xid(100 + round as u64);
        waits.started(on);
        pool.admit(plain(0), gate("gate", on, &waits, &log))
            .unwrap();
        pool.admit(
            plain(0),
            Box::new(End {
                name: "end",
                on,
                waits: waits.clone(),
                log: log.clone(),
            }),
        )
        .unwrap();
        let want = round + 1;
        log.until(|es| {
            es.iter().filter(|e| **e == Event::Done("gate")).count() == want
                && es.iter().filter(|e| **e == Event::Done("end")).count() == want
        });
    }
    pool.shutdown();
}

/// Every worker's call waits for a transaction; the call that ends it, queued last at the lowest
/// priority, still runs.
fn a_queued_end_runs_while_every_call_waits<R: Runtime>(rt: &R) {
    let workers = 2;
    let pool = Scheduler::start(rt.clone(), workers);
    let log = Log::new(rt);
    let waits = Arc::new(Waits::new(rt.clone()));
    waits.started(X);
    let names = ["w0", "w1", "w2", "w3"];
    for (i, name) in names.into_iter().enumerate() {
        pool.admit(plain(10 + i as u64), gate(name, X, &waits, &log))
            .unwrap();
    }
    log.until(|es| {
        names
            .iter()
            .all(|n| es.contains(&Event::Waiting(n)) || es.contains(&Event::Done(n)))
    });
    pool.admit(
        entry(Priority::AdHoc, 20, 0, None),
        Box::new(End {
            name: "end",
            on: X,
            waits,
            log: log.clone(),
        }),
    )
    .unwrap();
    log.until(|es| names.iter().all(|n| es.contains(&Event::Done(n))));
    assert!(log.has(&Event::Done("end")));
    pool.shutdown();
}

fn sim_seeds(name: &str, body: impl Fn(&SimRuntime)) {
    sim_seeds_in(name, SEEDS, body);
}

fn sim_seeds_in(name: &str, seeds: Range<u64>, body: impl Fn(&SimRuntime)) {
    for seed in sweep(seeds) {
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

fn real() -> OsRuntime {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("calls");
    std::fs::create_dir_all(&dir).unwrap();
    OsRuntime::new(dir, Backend::Pool { threads: 2 })
}

#[test]
fn admission_bounds_entries_and_bytes_sim() {
    sim_seeds("admission", admission_bounds_entries_and_bytes);
}

#[test]
fn admission_bounds_entries_and_bytes_real() {
    admission_bounds_entries_and_bytes(&real());
}

#[test]
fn calls_run_by_priority_then_start_sim() {
    sim_seeds("priority", calls_run_by_priority_then_start);
}

#[test]
fn calls_run_by_priority_then_start_real() {
    calls_run_by_priority_then_start(&real());
}

#[test]
fn queued_calls_stop_on_cancel_and_deadline_sim() {
    sim_seeds("queued stop", queued_calls_stop_on_cancel_and_deadline);
}

#[test]
fn queued_calls_stop_on_cancel_and_deadline_real() {
    queued_calls_stop_on_cancel_and_deadline(&real());
}

#[test]
fn waiting_calls_stop_on_cancel_and_deadline_sim() {
    sim_seeds("waiting stop", waiting_calls_stop_on_cancel_and_deadline);
}

#[test]
fn waiting_calls_stop_on_cancel_and_deadline_real() {
    waiting_calls_stop_on_cancel_and_deadline(&real());
}

#[test]
fn a_registration_racing_the_end_is_woken_sim() {
    sim_seeds("registration race", |rt| {
        a_registration_racing_the_end_is_woken(rt, 4);
    });
}

#[test]
fn a_registration_racing_the_end_is_woken_real() {
    a_registration_racing_the_end_is_woken(&real(), 200);
}

#[test]
fn a_wake_during_a_step_runs_the_call_again_sim() {
    sim_seeds("wake during step", |rt| {
        a_wake_during_a_step_runs_the_call_again(rt, 4);
    });
}

#[test]
fn a_wake_during_a_step_runs_the_call_again_real() {
    a_wake_during_a_step_runs_the_call_again(&real(), 200);
}

#[test]
fn a_queued_end_runs_while_every_call_waits_sim() {
    sim_seeds("queued end", a_queued_end_runs_while_every_call_waits);
}

#[test]
fn a_queued_end_runs_while_every_call_waits_real() {
    let rt = real();
    for _ in 0..20 {
        a_queued_end_runs_while_every_call_waits(&rt);
    }
}

const ITEMS: &str =
    "type Item = { id: Key items, n: I64 }\nrelation items : Item key id generate sequence\n";
const BUMP: &str = "update i in items set { i with n: i.n + 1 }";
const READ: &str = "from i in items select i.n";

/// A statement ready for the engine.
struct Prepared {
    terms: Terms,
    item: Tm,
}

fn prepare(f: &Fixture, text: &str) -> Prepared {
    let items = parse_adhoc(text, Options::default())
        .ok()
        .unwrap_or_else(|_| panic!("`{text}` does not parse"));
    let adhoc = check_adhoc(&f.schema, &f.program, &items, &BTreeMap::new())
        .unwrap_or_else(|ds| panic!("`{text}` does not check: {ds:?}"));
    let normal = norm::adhoc(&f.schema, &f.program, &adhoc)
        .unwrap_or_else(|e| panic!("`{text}` has no normal form: {e:?}"));
    Prepared {
        item: normal.items[0],
        terms: normal.terms,
    }
}

/// A database of one item, `n` 0, with the bump and read statements over it.
struct Items {
    schema: Arc<Schema>,
    db: Database<SimRuntime>,
    bump: Prepared,
    read: Prepared,
}

fn items(rt: &SimRuntime) -> Arc<Items> {
    let mut f = Fixture::new(ITEMS);
    f.rows("items", "[Item { id: items.key(1), n: 0 }]");
    let catalog = Arc::new(planfix::catalog(&f.schema));
    let normal: Arc<NormalProgram> =
        Arc::new(norm::program(&f.schema, &f.program).expect("the schema normalises"));
    let schema = Arc::new(f.schema.clone());
    let world = World {
        schema: &f.schema,
        normal: &normal,
        catalog: &catalog,
        db: &f.db,
    };
    let (pager, txns, heaps, indexes) = Db::load_with(rt, &world, false).bindings();
    let db = Database::new(
        rt.clone(),
        Setup {
            schema: schema.clone(),
            normal,
            catalog,
            pager,
            txns,
            bindings: Bindings { heaps, indexes },
            budget: Budget::new(1 << 30),
            grant: 64 << 20,
            pool: None,
            options: exec::Options::default(),
            limits: Limits::default(),
            sequences: None,
        },
    )
    .expect("a database over the fixture");
    Arc::new(Items {
        bump: prepare(&f, BUMP),
        read: prepare(&f, READ),
        schema,
        db,
    })
}

impl Items {
    fn text<'a>(&'a self, p: &'a Prepared) -> Text<'a> {
        Text {
            schema: &self.schema,
            terms: &p.terms,
            items: std::slice::from_ref(&p.item),
            params: &[],
            clock: 0,
            caller: None,
        }
    }

    fn bump(&self, tx: &Txn) {
        self.db
            .statements(
                tx,
                self.text(&self.bump),
                &mut Local::default(),
                Bounds::default(),
            )
            .unwrap();
    }

    fn values(&self) -> Val {
        let tx = self.db.begin_read(false).unwrap();
        let mut out = self
            .db
            .statements(
                &tx,
                self.text(&self.read),
                &mut Local::default(),
                Bounds::default(),
            )
            .unwrap();
        self.db.commit(tx).unwrap();
        out.remove(0).result
    }
}

/// Bumps the item in a transaction of its own, suspending at transaction waits, then commits.
struct Bump {
    name: &'static str,
    items: Arc<Items>,
    tx: Option<Txn>,
    log: Arc<Log<SimRuntime>>,
}

impl Bump {
    fn failed(&mut self, error: &write::Error) -> Step {
        let tx = self.tx.take().expect("a failing bump has its transaction");
        self.items.db.abort(tx).unwrap();
        self.log.push(Event::Failed(self.name, kind(error)));
        Step::Done
    }
}

fn kind(e: &write::Error) -> String {
    let debug = format!("{e:?}");
    debug
        .split(['(', ' ', '{'])
        .next()
        .unwrap_or_default()
        .to_string()
}

impl Resumable for Bump {
    fn run(&mut self, waker: &Waker) -> Step {
        self.log.push(Event::Ran(self.name));
        let items = self.items.clone();
        let tx = self
            .tx
            .get_or_insert_with(|| items.db.begin().expect("a transaction"));
        let bounds = Bounds {
            suspend: true,
            ..Bounds::default()
        };
        loop {
            let ran = items.db.statements(
                tx,
                items.text(&items.bump),
                &mut Local::default(),
                bounds.clone(),
            );
            match ran {
                Ok(_) => {
                    let tx = self.tx.take().expect("a bump has its transaction");
                    items.db.commit(tx).unwrap();
                    self.log.push(Event::Done(self.name));
                    return Step::Done;
                }
                Err(Failed {
                    error: write::Error::Blocked { on },
                    ..
                }) => match items.db.waits().watch(tx.xid(), on, waker) {
                    Ok(Watch::Ended) => {}
                    Ok(Watch::Pending) => {
                        self.log.push(Event::Waiting(self.name));
                        return Step::Wait;
                    }
                    Err(f) => return self.failed(&f.into()),
                },
                Err(Failed { error, .. }) => return self.failed(&error),
            }
        }
    }

    fn stop(mut self: Box<Self>, why: Stopped) {
        if let Some(tx) = self.tx.take() {
            self.items.db.abort(tx).unwrap();
        }
        self.log.push(Event::Stopped(self.name, why));
    }
}

/// A call that commits or aborts a transaction another thread began.
struct Finish {
    items: Arc<Items>,
    tx: Option<Txn>,
    commit: bool,
    log: Arc<Log<SimRuntime>>,
}

impl Resumable for Finish {
    fn run(&mut self, _: &Waker) -> Step {
        let tx = self.tx.take().expect("a finish runs once");
        if self.commit {
            self.items.db.commit(tx).unwrap();
        } else {
            self.items.db.abort(tx).unwrap();
        }
        self.log.push(Event::Done("finish"));
        Step::Done
    }

    fn stop(self: Box<Self>, why: Stopped) {
        self.log.push(Event::Stopped("finish", why));
    }
}

const BUMPS: [&str; 4] = ["b0", "b1", "b2", "b3"];

/// Four bumps wait on a transaction holding the item, on two workers; a queued call then ends
/// that transaction. Returns the log.
fn bumps_behind(rt: &SimRuntime, commit: bool) -> (Arc<Items>, Arc<Log<SimRuntime>>) {
    let items = items(rt);
    let pool = Scheduler::start(rt.clone(), 2);
    let log = Log::new(rt);
    let t0 = items.db.begin().unwrap();
    items.bump(&t0);
    for (i, name) in BUMPS.into_iter().enumerate() {
        let bump = Bump {
            name,
            items: items.clone(),
            tx: None,
            log: log.clone(),
        };
        pool.admit(plain(10 + i as u64), Box::new(bump)).unwrap();
    }
    log.until(|es| BUMPS.iter().all(|n| es.contains(&Event::Waiting(n))));
    let finish = Finish {
        items: items.clone(),
        tx: Some(t0),
        commit,
        log: log.clone(),
    };
    pool.admit(entry(Priority::AdHoc, 20, 0, None), Box::new(finish))
        .unwrap();
    log.until(|es| {
        es.contains(&Event::Done("finish"))
            && BUMPS.iter().all(|n| {
                es.iter()
                    .any(|e| matches!(e, Event::Done(m) | Event::Failed(m, _) if m == n))
            })
    });
    pool.shutdown();
    (items, log)
}

#[test]
fn a_queued_commit_runs_while_every_write_waits() {
    sim_seeds("queued commit", |rt| {
        let (items, log) = bumps_behind(rt, true);
        for n in BUMPS {
            assert!(
                log.has(&Event::Failed(n, "Serialization".into())),
                "{n}: {:?}",
                log.events()
            );
        }
        assert_eq!(Val::set(BTreeSet::from([Val::Int(1)])), items.values());
    });
}

#[test]
fn waiting_writes_resume_after_an_abort() {
    sim_seeds("queued abort", |rt| {
        let (items, log) = bumps_behind(rt, false);
        let done = log.count(|e| matches!(e, Event::Done(n) if n.starts_with('b')));
        assert_eq!(1, done, "{:?}", log.events());
        assert_eq!(Val::set(BTreeSet::from([Val::Int(1)])), items.values());
    });
}

#[test]
fn a_cancelled_waiting_write_leaves_the_others_waiting() {
    sim_seeds("cancel write", |rt| {
        let items = items(rt);
        let pool = Scheduler::start(rt.clone(), 1);
        let log = Log::new(rt);
        let t0 = items.db.begin().unwrap();
        items.bump(&t0);
        let bump = |name| {
            Box::new(Bump {
                name,
                items: items.clone(),
                tx: None,
                log: log.clone(),
            })
        };
        let b0 = pool.admit(plain(0), bump("b0")).unwrap();
        pool.admit(plain(1), bump("b1")).unwrap();
        log.until(|es| es.contains(&Event::Waiting("b0")) && es.contains(&Event::Waiting("b1")));
        b0.cancel();
        log.until(|es| es.contains(&Event::Stopped("b0", Stopped::Cancelled)));
        items.db.abort(t0).unwrap();
        log.until(|es| es.contains(&Event::Done("b1")));
        assert!(!log.has(&Event::Done("b0")));
        assert_eq!(Val::set(BTreeSet::from([Val::Int(1)])), items.values());
        pool.shutdown();
    });
}

/// Waits, suspended, for a deferrable read-only transaction's snapshot to be safe.
struct Defer {
    items: Arc<Items>,
    tx: Option<Txn>,
    log: Arc<Log<SimRuntime>>,
}

impl Resumable for Defer {
    fn run(&mut self, waker: &Waker) -> Step {
        let db = &self.items.db;
        loop {
            let tx = self.tx.take().expect("a deferral holds its transaction");
            match db.defer(tx).unwrap() {
                Deferral::Safe(tx) => {
                    db.commit(tx).unwrap();
                    self.log.push(Event::Done("defer"));
                    return Step::Done;
                }
                Deferral::Wait { tx, on } => {
                    self.tx = Some(tx);
                    if db.waits().watch(None, on, waker).unwrap() == Watch::Pending {
                        self.log.push(Event::Waiting("defer"));
                        return Step::Wait;
                    }
                }
            }
        }
    }

    fn stop(mut self: Box<Self>, why: Stopped) {
        if let Some(tx) = self.tx.take() {
            self.items.db.abort(tx).unwrap();
        }
        self.log.push(Event::Stopped("defer", why));
    }
}

#[test]
fn a_deferrable_read_waits_suspended_for_a_safe_snapshot() {
    sim_seeds("deferrable", |rt| {
        let items = items(rt);
        let pool = Scheduler::start(rt.clone(), 1);
        let log = Log::new(rt);
        let writer = items.db.begin().unwrap();
        items.bump(&writer);
        let defer = Defer {
            items: items.clone(),
            tx: Some(items.db.begin_deferrable().unwrap()),
            log: log.clone(),
        };
        pool.admit(plain(0), Box::new(defer)).unwrap();
        log.until(|es| es.contains(&Event::Waiting("defer")));
        pool.admit(plain(1), note("other", &log)).unwrap();
        log.until(|es| es.contains(&Event::Done("other")));
        assert!(!log.has(&Event::Done("defer")));
        items.db.commit(writer).unwrap();
        log.until(|es| es.contains(&Event::Done("defer")));
        pool.shutdown();
    });
}

const COUNTERS: &str = "module counters\nadd module counters\nadd type Counter = { id: Key counters, value: I64 }\nadd relation counters : Counter key id generate sequence\n";
const INCREMENT: &str = "import counters\nquery value () -> Set I64 = from c in counters select c.value\ntxn increment () -> () = transact update c in counters set { c with value: c.value + 1 }\n";
const UPDATE: &str = "update c in counters set { c with value: c.value + 1 }";

fn counter_options() -> OpenOptions {
    OpenOptions {
        create: Some(CreateOptions::default()),
        workers: Some(2),
        registrations: vec![Registration::new("counter", INCREMENT)],
        ..OpenOptions::default()
    }
}

fn counter_value<R: Runtime>(db: &adtb::Database<R>) -> Vec<Value> {
    let calls = db.registered("counter").unwrap();
    let adtb::Called::Rows(rows) = calls.call(1, &[], &CallOptions::default()).unwrap() else {
        panic!("the query returns rows")
    };
    rows.collect::<Result<_, _>>().unwrap()
}

#[test]
fn a_synchronous_call_still_blocks_on_a_transaction_sim() {
    sim_seeds_in("synchronous", 0..2, |rt| {
        let db = adtb::Database::open_in(
            rt.clone(),
            "counter.adt",
            &SchemaLog::new([COUNTERS]),
            &counter_options(),
        )
        .unwrap();
        db.query_text("insert counters { value: 0 }", &[], &CallOptions::default())
            .unwrap();
        let mut txn = db.begin(&TxnOptions::default()).unwrap();
        txn.query_text(UPDATE, &[], &TxnCallOptions::default())
            .unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let calls = db.registered("counter").unwrap();
        let caller = {
            let done = done.clone();
            rt.spawn("increment", move || {
                calls.call(2, &[], &CallOptions::default()).unwrap();
                done.store(true, Ordering::Release);
            })
        };
        sleep(rt, Duration::from_millis(20));
        assert!(
            !done.load(Ordering::Acquire),
            "the call waits for the transaction"
        );
        txn.commit(None).unwrap();
        rt.join(caller);
        assert_eq!(vec![Value::Int(2)], counter_value(&db));
        db.close().unwrap();
    });
}

#[test]
fn a_synchronous_call_still_blocks_on_a_transaction_real() {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("calls-synchronous");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    let db = adtb::open(
        dir.join("counter.adt"),
        &SchemaLog::new([COUNTERS]),
        &counter_options(),
    )
    .unwrap();
    db.query_text("insert counters { value: 0 }", &[], &CallOptions::default())
        .unwrap();
    let calls = db.registered("counter").unwrap();
    let mut txn = db.begin(&TxnOptions::default()).unwrap();
    txn.query_text(UPDATE, &[], &TxnCallOptions::default())
        .unwrap();
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let caller = scope.spawn(|| {
            calls.call(2, &[], &CallOptions::default()).unwrap();
            done.store(true, Ordering::Release);
        });
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !done.load(Ordering::Acquire),
            "the call waits for the transaction"
        );
        txn.commit(None).unwrap();
        caller.join().unwrap();
    });
    assert_eq!(vec![Value::Int(2)], counter_value(&db));
    db.close().unwrap();
}
