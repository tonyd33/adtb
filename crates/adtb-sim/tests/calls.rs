use adtb::plan::fixture as planfix;

use adtb_simkit::support;

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Wake, Waker};
use std::time::Duration;

use adtb::exec::{
    self, Admission, Budget, Entry, Exceeded, Priority, Refusal, Refused, Resumable, Scheduler,
    Step, Stopped, Task, Val,
};
use adtb::mvcc::{Cause, Limits, Waits, Watch, Xid};
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
    Failed(&'static str, write::Error),
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
        rank: 0,
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
        Refusal::Overloaded,
        pool.admit(bytes(30), note("c", &log)).unwrap_err().reason
    );
    pool.admit(bytes(20), note("c", &log)).unwrap();
    assert_eq!(
        Refusal::Overloaded,
        pool.admit(bytes(0), note("d", &log)).unwrap_err().reason
    );
    a.cancel();
    pool.admit(bytes(40), note("e", &log)).unwrap();
    assert_eq!(
        Refusal::Overloaded,
        pool.admit(bytes(0), note("f", &log)).unwrap_err().reason
    );
    assert!(pool.help(), "the cancelled call's stop is a task");
    assert_eq!(vec![Event::Stopped("a", Stopped::Cancelled)], log.events());
    a.cancel();
    assert!(!pool.help());
    pool.shutdown();
}

/// A refused call is handed back; one larger than the byte bound is refused as too large with
/// nothing queued, and every call after shutdown is refused.
fn a_refused_call_is_handed_back<R: Runtime>(rt: &R) {
    let pool = Scheduler::start_with(
        rt.clone(),
        0,
        Admission {
            entries: 1,
            bytes: 100,
        },
    );
    let log = Log::new(rt);
    let big = pool
        .admit(entry(Priority::Regular, 0, 101, None), note("big", &log))
        .unwrap_err();
    assert_eq!(Refusal::TooLarge, big.reason);
    big.call.stop(Stopped::Cancelled);
    pool.admit(plain(0), note("a", &log)).unwrap();
    let full = pool.admit(plain(0), note("b", &log)).unwrap_err();
    assert_eq!(Refusal::Overloaded, full.reason);
    pool.shutdown();
    let closed = pool.admit(plain(0), full.call).unwrap_err();
    assert_eq!(Refusal::ShutDown, closed.reason);
    closed.call.stop(Stopped::Cancelled);
    assert_eq!(
        vec![
            Event::Stopped("big", Stopped::Cancelled),
            Event::Stopped("a", Stopped::ShutDown),
            Event::Stopped("b", Stopped::Cancelled),
        ],
        log.events()
    );
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
        Refusal::Overloaded,
        pool.admit(plain(0), note("f", &log)).unwrap_err().reason
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
    log.until(|es| {
        es.contains(&Event::Done("end")) && names.iter().all(|n| es.contains(&Event::Done(n)))
    });
    pool.shutdown();
}

/// A call whose deadline has passed when a worker would take it stops instead of running.
fn a_call_past_its_deadline_never_runs<R: Runtime>(rt: &R) {
    let pool = Scheduler::start(rt.clone(), 2);
    let log = Log::new(rt);
    let names = ["p0", "p1", "p2", "p3"];
    for name in names {
        pool.admit(
            entry(Priority::Regular, 0, 0, Some(rt.now())),
            note(name, &log),
        )
        .unwrap();
    }
    log.until(|es| {
        es.iter()
            .filter(|e| matches!(e, Event::Done(_) | Event::Stopped(..)))
            .count()
            == names.len()
    });
    for name in names {
        assert!(
            log.has(&Event::Stopped(name, Stopped::DeadlineExceeded)),
            "{name}: {:?}",
            log.events()
        );
    }
    pool.shutdown();
}

/// Shutdown stops every queued and waiting call, and a cancel after it does nothing.
fn shutdown_stops_queued_and_waiting_calls<R: Runtime>(rt: &R) {
    let log = Log::new(rt);
    let idle = Scheduler::start(rt.clone(), 0);
    let queued = idle.admit(plain(0), note("queued", &log)).unwrap();
    idle.shutdown();
    assert_eq!(
        vec![Event::Stopped("queued", Stopped::ShutDown)],
        log.events()
    );
    queued.cancel();
    assert!(!idle.help(), "a cancel after shutdown queues nothing");

    let pool = Scheduler::start(rt.clone(), 1);
    let waits = Arc::new(Waits::new(rt.clone()));
    waits.started(X);
    let late = rt.now() + Duration::from_secs(3600);
    let waiting = pool
        .admit(
            entry(Priority::Regular, 0, 0, Some(late)),
            gate("waiting", X, &waits, &log),
        )
        .unwrap();
    log.until(|es| es.contains(&Event::Waiting("waiting")));
    pool.shutdown();
    assert!(log.has(&Event::Stopped("waiting", Stopped::ShutDown)));
    waiting.cancel();
    assert!(!pool.help(), "a cancel after shutdown queues nothing");
    waits.ended(X);
    assert!(!log.has(&Event::Done("waiting")));
}

/// Admit `call`, admitting it again while the admission queue is full.
fn admit_retrying<R: Runtime>(pool: &Scheduler<R>, entry: Entry, mut call: Box<dyn Resumable>) {
    while let Err(Refused { reason, call: back }) = pool.admit(entry, call) {
        assert_eq!(Refusal::Overloaded, reason);
        call = back;
        pool.runtime().yield_now();
    }
}

/// More calls wait than the admission bound admits.
fn waiting_calls_leave_the_admission_bound<R: Runtime>(rt: &R) {
    let pool = Scheduler::start_with(
        rt.clone(),
        2,
        Admission {
            entries: 2,
            bytes: 1 << 20,
        },
    );
    let log = Log::new(rt);
    let waits = Arc::new(Waits::new(rt.clone()));
    waits.started(X);
    let names = ["w0", "w1", "w2", "w3", "w4", "w5"];
    for (i, name) in names.into_iter().enumerate() {
        admit_retrying(&pool, plain(i as u64), gate(name, X, &waits, &log));
    }
    log.until(|es| names.iter().all(|n| es.contains(&Event::Waiting(n))));
    let end = End {
        name: "end",
        on: X,
        waits,
        log: log.clone(),
    };
    admit_retrying(&pool, entry(Priority::AdHoc, 9, 0, None), Box::new(end));
    log.until(|es| names.iter().all(|n| es.contains(&Event::Done(n))));
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

/// Returns a runtime over a new directory of its own, `name`.
fn real_in(name: &str) -> OsRuntime {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("calls-db")
        .join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
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
fn a_refused_call_is_handed_back_sim() {
    sim_seeds("refused", a_refused_call_is_handed_back);
}

#[test]
fn a_refused_call_is_handed_back_real() {
    a_refused_call_is_handed_back(&real());
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

#[test]
fn a_call_past_its_deadline_never_runs_sim() {
    sim_seeds("past deadline", a_call_past_its_deadline_never_runs);
}

#[test]
fn a_call_past_its_deadline_never_runs_real() {
    let rt = real();
    for _ in 0..20 {
        a_call_past_its_deadline_never_runs(&rt);
    }
}

#[test]
fn shutdown_stops_queued_and_waiting_calls_sim() {
    sim_seeds("shutdown", shutdown_stops_queued_and_waiting_calls);
}

#[test]
fn shutdown_stops_queued_and_waiting_calls_real() {
    shutdown_stops_queued_and_waiting_calls(&real());
}

#[test]
fn waiting_calls_leave_the_admission_bound_sim() {
    sim_seeds("past the bound", waiting_calls_leave_the_admission_bound);
}

#[test]
fn waiting_calls_leave_the_admission_bound_real() {
    waiting_calls_leave_the_admission_bound(&real());
}

const ITEMS: &str =
    "type Item = { id: Key items, n: I64 }\nrelation items : Item key id generate sequence\n";
const ONE: &str = "[Item { id: items.key(1), n: 0 }]";
const TWO: &str = "[Item { id: items.key(1), n: 0 }, Item { id: items.key(2), n: 10 }]";
const BUMP: &str = "update i in items set { i with n: i.n + 1 }";
const BUMP_FIRST: &str = "update i in items where i.id == items.key(1) set { i with n: i.n + 1 }";
const BUMP_SECOND: &str = "update i in items where i.id == items.key(2) set { i with n: i.n + 1 }";
const INSERT_THEN_BUMP: &str =
    "insert items { id: items.key(9), n: 5 }\nupdate i in items set { i with n: i.n + 1 }";
const READ: &str = "from i in items select i.n";
const IDS: &str = "from i in items select i.id";
const TEXTS: [&str; 6] = [BUMP, BUMP_FIRST, BUMP_SECOND, INSERT_THEN_BUMP, READ, IDS];

/// Statements ready for the engine.
struct Prepared {
    terms: Terms,
    items: Vec<Tm>,
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
        items: normal.items,
        terms: normal.terms,
    }
}

/// A database of the items `rows`, with each of [`TEXTS`] prepared over it.
struct Items<R: Runtime> {
    schema: Arc<Schema>,
    db: Database<R>,
    texts: BTreeMap<&'static str, Prepared>,
}

fn items<R: Runtime>(rt: &R, rows: &str) -> Arc<Items<R>> {
    let mut f = Fixture::new(ITEMS);
    f.rows("items", rows);
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
    let sequences = support::counters(&pager, &schema, &catalog, &indexes);
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
            sequences,
        },
    )
    .expect("a database over the fixture");
    Arc::new(Items {
        texts: TEXTS.into_iter().map(|t| (t, prepare(&f, t))).collect(),
        schema,
        db,
    })
}

impl<R: Runtime> Items<R> {
    fn text<'a>(&'a self, src: &str) -> Text<'a> {
        let p = &self.texts[src];
        Text {
            schema: &self.schema,
            terms: &p.terms,
            items: &p.items,
            params: &[],
            clock: 0,
            caller: None,
        }
    }

    fn run(&self, tx: &Txn, src: &str, bounds: Bounds) -> Result<Vec<write::Outcome>, Failed> {
        self.db
            .statements(tx, self.text(src), &mut Local::default(), bounds)
    }

    fn bump(&self, tx: &Txn) {
        self.run(tx, BUMP, Bounds::default()).unwrap();
    }

    /// Returns the set `src` reads in a transaction of its own.
    fn read(&self, src: &str) -> Val {
        let tx = self.db.begin_read(false).unwrap();
        let mut out = self.run(&tx, src, Bounds::default()).unwrap();
        self.db.commit(tx).unwrap();
        out.remove(0).result
    }

    fn values(&self) -> Val {
        self.read(READ)
    }
}

fn ints(ns: &[i64]) -> Val {
    Val::set(ns.iter().map(|n| Val::Int(*n)).collect())
}

/// A call that runs `src` as the statements of one transaction, suspending at transaction
/// waits, then commits.
struct Write<R: Runtime> {
    name: &'static str,
    items: Arc<Items<R>>,
    src: &'static str,
    bounds: Bounds,
    /// The transaction, begun at the first step when `None`.
    tx: Option<Txn>,
    /// The transaction the waker is registered on, with the waker.
    watched: Option<(Xid, Waker)>,
    log: Arc<Log<R>>,
}

fn write<R: Runtime>(
    name: &'static str,
    src: &'static str,
    items: &Arc<Items<R>>,
    log: &Arc<Log<R>>,
) -> Write<R> {
    Write {
        name,
        items: items.clone(),
        src,
        bounds: Bounds::default(),
        tx: None,
        watched: None,
        log: log.clone(),
    }
}

impl<R: Runtime> Write<R> {
    fn failed(&mut self, error: write::Error) -> Step {
        let tx = self.tx.take().expect("a failing write has its transaction");
        self.items.db.abort(tx).unwrap();
        self.log.push(Event::Failed(self.name, error));
        Step::Done
    }
}

impl<R: Runtime> Resumable for Write<R> {
    fn run(&mut self, waker: &Waker) -> Step {
        self.log.push(Event::Ran(self.name));
        self.watched = None;
        let items = self.items.clone();
        let tx = self
            .tx
            .get_or_insert_with(|| items.db.begin().expect("a transaction"));
        loop {
            let bounds = Bounds {
                suspend: true,
                ..self.bounds.clone()
            };
            match items.run(tx, self.src, bounds) {
                Ok(_) => {
                    let tx = self.tx.take().expect("a write has its transaction");
                    items.db.commit(tx).unwrap();
                    self.log.push(Event::Done(self.name));
                    return Step::Done;
                }
                Err(Failed {
                    error: write::Error::Blocked { on, rerun },
                    ..
                }) => {
                    self.bounds.rerun = Some(rerun);
                    match items.db.watch(tx.xid(), on, waker) {
                        Ok(Watch::Ended) => {}
                        Ok(Watch::Pending) => {
                            self.watched = Some((on, waker.clone()));
                            self.log.push(Event::Waiting(self.name));
                            return Step::Wait;
                        }
                        Err(f) => return self.failed(f.into()),
                    }
                }
                Err(Failed { error, .. }) => return self.failed(error),
            }
        }
    }

    fn stop(mut self: Box<Self>, why: Stopped) {
        if let Some(tx) = self.tx.take() {
            if let Some((on, w)) = &self.watched {
                self.items.db.withdraw(tx.xid(), *on, w);
            }
            self.items.db.abort(tx).unwrap();
        }
        self.log.push(Event::Stopped(self.name, why));
    }
}

/// A call that commits or aborts a transaction another thread began.
struct Finish<R: Runtime> {
    items: Arc<Items<R>>,
    tx: Option<Txn>,
    commit: bool,
    log: Arc<Log<R>>,
}

impl<R: Runtime> Resumable for Finish<R> {
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
fn bumps_behind<R: Runtime>(rt: &R, commit: bool) -> (Arc<Items<R>>, Arc<Log<R>>) {
    let items = items(rt, ONE);
    let pool = Scheduler::start(rt.clone(), 2);
    let log = Log::new(rt);
    let t0 = items.db.begin().unwrap();
    items.bump(&t0);
    for (i, name) in BUMPS.into_iter().enumerate() {
        pool.admit(
            plain(10 + i as u64),
            Box::new(write(name, BUMP, &items, &log)),
        )
        .unwrap();
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

fn a_queued_commit_runs_while_every_write_waits<R: Runtime>(rt: &R) {
    let (items, log) = bumps_behind(rt, true);
    for n in BUMPS {
        assert!(
            log.events()
                .iter()
                .any(|e| matches!(e, Event::Failed(m, write::Error::Serialization(_)) if *m == n)),
            "{n}: {:?}",
            log.events()
        );
    }
    assert_eq!(ints(&[1]), items.values());
}

#[test]
fn a_queued_commit_runs_while_every_write_waits_sim() {
    sim_seeds(
        "queued commit",
        a_queued_commit_runs_while_every_write_waits,
    );
}

#[test]
fn a_queued_commit_runs_while_every_write_waits_real() {
    a_queued_commit_runs_while_every_write_waits(&real_in("queued-commit"));
}

fn waiting_writes_resume_after_an_abort<R: Runtime>(rt: &R) {
    let (items, log) = bumps_behind(rt, false);
    let done = log.count(|e| matches!(e, Event::Done(n) if n.starts_with('b')));
    assert_eq!(1, done, "{:?}", log.events());
    assert_eq!(ints(&[1]), items.values());
}

#[test]
fn waiting_writes_resume_after_an_abort_sim() {
    sim_seeds("queued abort", waiting_writes_resume_after_an_abort);
}

#[test]
fn waiting_writes_resume_after_an_abort_real() {
    waiting_writes_resume_after_an_abort(&real_in("queued-abort"));
}

#[test]
fn a_cancelled_waiting_write_leaves_the_others_waiting() {
    sim_seeds("cancel write", |rt| {
        let items = items(rt, ONE);
        let pool = Scheduler::start(rt.clone(), 1);
        let log = Log::new(rt);
        let t0 = items.db.begin().unwrap();
        items.bump(&t0);
        let b0 = pool
            .admit(plain(0), Box::new(write("b0", BUMP, &items, &log)))
            .unwrap();
        pool.admit(plain(1), Box::new(write("b1", BUMP, &items, &log)))
            .unwrap();
        log.until(|es| es.contains(&Event::Waiting("b0")) && es.contains(&Event::Waiting("b1")));
        b0.cancel();
        log.until(|es| es.contains(&Event::Stopped("b0", Stopped::Cancelled)));
        items.db.abort(t0).unwrap();
        log.until(|es| es.contains(&Event::Done("b1")));
        assert!(!log.has(&Event::Done("b0")));
        assert_eq!(ints(&[1]), items.values());
        pool.shutdown();
    });
}

/// A call of two statements suspends at its second, and once woken reruns both, the first's
/// insert undone in between.
fn a_two_statement_call_suspends_and_resumes<R: Runtime>(rt: &R) {
    let items = items(rt, ONE);
    let pool = Scheduler::start(rt.clone(), 1);
    let log = Log::new(rt);
    let t0 = items.db.begin().unwrap();
    items.bump(&t0);
    pool.admit(
        plain(0),
        Box::new(write("two", INSERT_THEN_BUMP, &items, &log)),
    )
    .unwrap();
    log.until(|es| es.contains(&Event::Waiting("two")));
    items.db.abort(t0).unwrap();
    log.until(|es| {
        es.iter()
            .any(|e| matches!(e, Event::Done(_) | Event::Failed(..)))
    });
    assert_eq!(
        vec![
            Event::Ran("two"),
            Event::Waiting("two"),
            Event::Ran("two"),
            Event::Done("two"),
        ],
        log.events()
    );
    assert_eq!(ints(&[1, 6]), items.values());
    assert_eq!(2, items.read(IDS).as_set().unwrap().len());
    pool.shutdown();
}

#[test]
fn a_two_statement_call_suspends_and_resumes_sim() {
    sim_seeds("two statements", a_two_statement_call_suspends_and_resumes);
}

#[test]
fn a_two_statement_call_suspends_and_resumes_real() {
    a_two_statement_call_suspends_and_resumes(&real_in("two-statements"));
}

/// A write waiting for a transaction stops at its deadline, aborting its own.
fn a_waiting_write_stops_at_its_deadline<R: Runtime>(rt: &R) {
    let items = items(rt, ONE);
    let pool = Scheduler::start(rt.clone(), 1);
    let log = Log::new(rt);
    let t0 = items.db.begin().unwrap();
    items.bump(&t0);
    let deadline = rt.now() + Duration::from_millis(100);
    pool.admit(
        entry(Priority::Regular, 0, 0, Some(deadline)),
        Box::new(write("late", BUMP, &items, &log)),
    )
    .unwrap();
    log.until(|es| es.iter().any(|e| matches!(e, Event::Stopped(..))));
    let stopped = Event::Stopped("late", Stopped::DeadlineExceeded);
    let events = log.events();
    assert!(
        events == [Event::Ran("late"), Event::Waiting("late"), stopped.clone()]
            || events == [stopped],
        "a waiting or a queued call stops at its deadline: {events:?}"
    );
    items.db.commit(t0).unwrap();
    pool.admit(plain(1), Box::new(write("next", BUMP, &items, &log)))
        .unwrap();
    log.until(|es| es.contains(&Event::Done("next")));
    assert_eq!(ints(&[2]), items.values());
    pool.shutdown();
}

#[test]
fn a_waiting_write_stops_at_its_deadline_sim() {
    sim_seeds("waiting deadline", a_waiting_write_stops_at_its_deadline);
}

#[test]
fn a_waiting_write_stops_at_its_deadline_real() {
    a_waiting_write_stops_at_its_deadline(&real_in("waiting-deadline"));
}

/// A suspended wait counts toward the statement's time limit: the rerun fails once the limit
/// from the first run has passed.
#[test]
fn a_rerun_keeps_the_statement_limit_of_its_first_run() {
    sim_seeds("rerun limit", |rt| {
        let items = items(rt, ONE);
        let pool = Scheduler::start(rt.clone(), 1);
        let log = Log::new(rt);
        let t0 = items.db.begin().unwrap();
        items.bump(&t0);
        let after = Duration::from_millis(30);
        let mut w = write("limited", BUMP, &items, &log);
        w.bounds.limit = Some(after);
        pool.admit(plain(0), Box::new(w)).unwrap();
        log.until(|es| es.contains(&Event::Waiting("limited")));
        sleep(rt, after * 2);
        items.db.abort(t0).unwrap();
        log.until(|es| {
            es.iter()
                .any(|e| matches!(e, Event::Done(_) | Event::Failed(..)))
        });
        assert_eq!(
            Some(&Event::Failed(
                "limited",
                write::Error::StatementLimit(Exceeded::Time { after })
            )),
            log.events().last()
        );
        assert_eq!(ints(&[0]), items.values());
        pool.shutdown();
    });
}

/// A rerun runs the statements before the one that suspended without the statement limit: a
/// wait past the limit fails the suspended statement, not an earlier one.
#[test]
fn a_rerun_limits_only_the_statement_that_suspended() {
    sim_seeds("rerun limit item", |rt| {
        let items = items(rt, ONE);
        let t0 = items.db.begin().unwrap();
        items.bump(&t0);
        let after = Duration::from_millis(30);
        let tx = items.db.begin().unwrap();
        let bounds = Bounds {
            limit: Some(after),
            suspend: true,
            ..Bounds::default()
        };
        let Err(Failed {
            item: 1,
            error: write::Error::Blocked { rerun, .. },
        }) = items.run(&tx, INSERT_THEN_BUMP, bounds.clone())
        else {
            panic!("the second statement suspends")
        };
        sleep(rt, after * 2);
        items.db.abort(t0).unwrap();
        let rerun = Bounds {
            rerun: Some(rerun),
            ..bounds
        };
        assert_eq!(
            Err(Failed {
                item: 1,
                error: write::Error::StatementLimit(Exceeded::Time { after }),
            }),
            items.run(&tx, INSERT_THEN_BUMP, rerun)
        );
        items.db.abort(tx).unwrap();
    });
}

/// A wait shorter than the limit leaves the rerun of the earlier statements out of the
/// suspended statement's limit.
#[test]
fn a_rerun_within_the_limit_finishes() {
    sim_seeds("rerun within limit", |rt| {
        let items = items(rt, ONE);
        let t0 = items.db.begin().unwrap();
        items.bump(&t0);
        let after = Duration::from_millis(30);
        let tx = items.db.begin().unwrap();
        let bounds = Bounds {
            limit: Some(after),
            suspend: true,
            ..Bounds::default()
        };
        let Err(Failed {
            error: write::Error::Blocked { rerun, .. },
            ..
        }) = items.run(&tx, INSERT_THEN_BUMP, bounds.clone())
        else {
            panic!("the second statement suspends")
        };
        sleep(rt, after / 3);
        items.db.abort(t0).unwrap();
        let rerun = Bounds {
            rerun: Some(rerun),
            ..bounds
        };
        assert!(items.run(&tx, INSERT_THEN_BUMP, rerun).is_ok());
        items.db.commit(tx).unwrap();
    });
}

/// A cancel racing shutdown stops the call once, with one of the two.
fn a_cancel_racing_shutdown_stops_once<R: Runtime>(rt: &R) {
    let pool = Scheduler::start(rt.clone(), 1);
    let log = Log::new(rt);
    let waits = Arc::new(Waits::new(rt.clone()));
    waits.started(X);
    let call = pool.admit(plain(0), gate("g", X, &waits, &log)).unwrap();
    log.until(|es| es.contains(&Event::Waiting("g")));
    let canceller = {
        let rt2 = rt.clone();
        rt.spawn("cancel", move || {
            rt2.yield_now();
            call.cancel();
        })
    };
    pool.shutdown();
    rt.join(canceller);
    let stops = log.count(|e| matches!(e, Event::Stopped("g", _)));
    assert_eq!(1, stops, "{:?}", log.events());
}

#[test]
fn a_cancel_racing_shutdown_stops_once_sim() {
    sim_seeds("cancel shutdown", a_cancel_racing_shutdown_stops_once);
}

#[test]
fn a_cancel_racing_shutdown_stops_once_real() {
    a_cancel_racing_shutdown_stops_once(&real());
}

/// Dropping a scheduler without shutting it down stops its waiting calls.
fn a_dropped_scheduler_stops_its_calls<R: Runtime>(rt: &R) {
    let pool = Scheduler::start(rt.clone(), 1);
    let log = Log::new(rt);
    let waits = Arc::new(Waits::new(rt.clone()));
    waits.started(X);
    pool.admit(plain(0), gate("g", X, &waits, &log)).unwrap();
    log.until(|es| es.contains(&Event::Waiting("g")));
    drop(pool);
    assert!(log.has(&Event::Stopped("g", Stopped::ShutDown)));
}

#[test]
fn a_dropped_scheduler_stops_its_calls_sim() {
    sim_seeds("dropped scheduler", a_dropped_scheduler_stops_its_calls);
}

#[test]
fn a_dropped_scheduler_stops_its_calls_real() {
    a_dropped_scheduler_stops_its_calls(&real());
}

/// A call waiting with an alarm is woken at it, though its dependency is not ready.
fn an_alarm_wakes_a_waiting_call<R: Runtime>(rt: &R) {
    let pool = Scheduler::start(rt.clone(), 1);
    let log = Log::new(rt);
    let at = rt.now() + Duration::from_millis(20);
    pool.admit(
        plain(0),
        Box::new(Alarm {
            at,
            runs: 0,
            log: log.clone(),
        }),
    )
    .unwrap();
    log.until(|es| es.contains(&Event::Done("alarm")));
    assert!(rt.now() >= at);
    assert_eq!(
        vec![
            Event::Ran("alarm"),
            Event::Ran("alarm"),
            Event::Done("alarm")
        ],
        log.events()
    );
    pool.shutdown();
}

/// A call that waits once with an alarm and no dependency, then finishes.
struct Alarm<R: Runtime> {
    at: Instant,
    runs: usize,
    log: Arc<Log<R>>,
}

impl<R: Runtime> Resumable for Alarm<R> {
    fn run(&mut self, _: &Waker) -> Step {
        self.log.push(Event::Ran("alarm"));
        self.runs += 1;
        if self.runs == 1 {
            return Step::WaitUntil(self.at);
        }
        self.log.push(Event::Done("alarm"));
        Step::Done
    }

    fn stop(self: Box<Self>, why: Stopped) {
        self.log.push(Event::Stopped("alarm", why));
    }
}

#[test]
fn an_alarm_wakes_a_waiting_call_sim() {
    sim_seeds("alarm", an_alarm_wakes_a_waiting_call);
}

#[test]
fn an_alarm_wakes_a_waiting_call_real() {
    an_alarm_wakes_a_waiting_call(&real());
}

/// Of two transactions each holding the item the other next writes, the second to wait fails
/// with a deadlock and the first goes on, whether the waits suspend or block.
fn a_deadlock_fails_the_second_waiter<R: Runtime>(rt: &R, suspend: bool, settle: Duration) {
    let items = items(rt, TWO);
    let log = Log::new(rt);
    let t1 = items.db.begin().unwrap();
    items.run(&t1, BUMP_FIRST, Bounds::default()).unwrap();
    let t2 = items.db.begin().unwrap();
    items.run(&t2, BUMP_SECOND, Bounds::default()).unwrap();
    let mut first = write("first", BUMP_SECOND, &items, &log);
    first.tx = Some(t1);
    let mut second = write("second", BUMP_FIRST, &items, &log);
    second.tx = Some(t2);
    let finished = |es: &[Event]| {
        es.iter()
            .filter(|e| matches!(e, Event::Done(_) | Event::Failed(..)))
            .count()
            == 2
    };
    if suspend {
        let pool = Scheduler::start(rt.clone(), 1);
        pool.admit(plain(0), Box::new(first)).unwrap();
        log.until(|es| es.contains(&Event::Waiting("first")));
        pool.admit(plain(1), Box::new(second)).unwrap();
        log.until(finished);
        pool.shutdown();
    } else {
        let a = rt.spawn("first", move || blocking(first));
        sleep(rt, settle);
        let b = rt.spawn("second", move || blocking(second));
        log.until(finished);
        rt.join(a);
        rt.join(b);
    }
    let ends: Vec<Event> = log
        .events()
        .into_iter()
        .filter(|e| matches!(e, Event::Done(_) | Event::Failed(..)))
        .collect();
    assert!(
        matches!(
            &ends[..],
            [
                Event::Failed("second", write::Error::Serialization(f)),
                Event::Done("first"),
            ] if f.cause == Cause::Deadlock
        ),
        "{ends:?}"
    );
    assert_eq!(ints(&[1, 11]), items.values());
}

/// Run `w`'s statements once, parking at transaction waits, then commit or abort.
fn blocking<R: Runtime>(mut w: Write<R>) {
    let tx =
        w.tx.take()
            .expect("a blocking write is given its transaction");
    match w.items.run(&tx, w.src, w.bounds.clone()) {
        Ok(_) => {
            w.items.db.commit(tx).unwrap();
            w.log.push(Event::Done(w.name));
        }
        Err(Failed { error, .. }) => {
            w.items.db.abort(tx).unwrap();
            w.log.push(Event::Failed(w.name, error));
        }
    }
}

#[test]
fn a_deadlock_between_suspended_calls_fails_the_second_waiter_sim() {
    sim_seeds("suspended deadlock", |rt| {
        a_deadlock_fails_the_second_waiter(rt, true, Duration::ZERO);
    });
}

#[test]
fn a_deadlock_between_suspended_calls_fails_the_second_waiter_real() {
    a_deadlock_fails_the_second_waiter(&real_in("suspended-deadlock"), true, Duration::ZERO);
}

#[test]
fn a_deadlock_between_blocked_writes_fails_the_second_waiter_sim() {
    sim_seeds("blocking deadlock", |rt| {
        a_deadlock_fails_the_second_waiter(rt, false, Duration::from_millis(20));
    });
}

/// Waits, suspended, for a deferrable read-only transaction's snapshot to be safe.
struct Defer {
    items: Arc<Items<SimRuntime>>,
    tx: Option<Txn>,
    /// The transaction the waker is registered on, with the waker.
    watched: Option<(Xid, Waker)>,
    log: Arc<Log<SimRuntime>>,
}

impl Resumable for Defer {
    fn run(&mut self, waker: &Waker) -> Step {
        let db = &self.items.db;
        self.watched = None;
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
                    if db.watch(None, on, waker).unwrap() == Watch::Pending {
                        self.watched = Some((on, waker.clone()));
                        self.log.push(Event::Waiting("defer"));
                        return Step::Wait;
                    }
                }
            }
        }
    }

    fn stop(mut self: Box<Self>, why: Stopped) {
        if let Some((on, w)) = &self.watched {
            self.items.db.withdraw(None, *on, w);
        }
        if let Some(tx) = self.tx.take() {
            self.items.db.abort(tx).unwrap();
        }
        self.log.push(Event::Stopped("defer", why));
    }
}

#[test]
fn a_deferrable_read_waits_suspended_for_a_safe_snapshot() {
    sim_seeds("deferrable", |rt| {
        let items = items(rt, ONE);
        let pool = Scheduler::start(rt.clone(), 1);
        let log = Log::new(rt);
        let writer = items.db.begin().unwrap();
        items.bump(&writer);
        let defer = Defer {
            items: items.clone(),
            tx: Some(items.db.begin_deferrable().unwrap()),
            watched: None,
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
