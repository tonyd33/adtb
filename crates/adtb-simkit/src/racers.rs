//! Step functions that race one another in an interleaving: a transaction of the list-append
//! workload, and a call run on a thread of its own.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use adtb::runtime::{ThreadId, Threads};
use adtb::testing::Step;
use adtb_sim::SimRuntime;
use adtb_testkit::isolation::{self as iso, Outcome};

use crate::elle::{Elems, Kind, Reply, Req, Worker};
use crate::steps::{Boundaries, Stepping};

/// What the history records of a request once it succeeds.
#[derive(Debug)]
enum Logged {
    Append { key: i64, elem: i64 },
    Read { key: i64 },
    Nothing,
}

/// A transaction of the workload on a thread of its own, sending its requests in turn and
/// ending at the first that fails.
#[derive(Debug)]
pub struct Txn {
    rt: SimRuntime,
    id: u64,
    worker: Worker,
    boundaries: fn(Step) -> bool,
    to_send: VecDeque<Req>,
    sent: Option<Logged>,
    ops: Vec<iso::Op>,
    replies: Vec<Reply>,
    outcome: Option<Outcome>,
}

impl Txn {
    /// Begin a transaction of `kind` that the history names `id`, whose boundaries are the steps
    /// `boundaries` accepts, to send `reqs` once started.
    pub fn start(
        rt: &SimRuntime,
        e: &Arc<Elems>,
        (id, kind): (u64, Kind),
        boundaries: fn(Step) -> bool,
        reqs: Vec<Req>,
    ) -> Txn {
        Txn {
            rt: rt.clone(),
            id,
            worker: Worker::start(rt, e, kind),
            boundaries,
            to_send: reqs.into(),
            sent: None,
            ops: Vec::new(),
            replies: Vec::new(),
            outcome: None,
        }
    }

    fn send_next(&mut self) {
        let req = self.to_send.pop_front().expect("a request to send");
        self.sent = Some(match req {
            Req::Append(key, elem) => Logged::Append { key, elem },
            Req::Read(key) => Logged::Read { key },
            Req::Text(_) | Req::Commit | Req::Abort => Logged::Nothing,
        });
        self.worker.send(&self.rt, req);
    }

    /// Take `reply` to the request last sent, and send the next while it succeeded.
    fn replied(&mut self, reply: Reply) {
        let sent = self.sent.take().expect("a reply answers a request");
        match (sent, &reply) {
            (Logged::Append { key, elem }, Ok(_)) => self.ops.push(iso::Op::Append { key, elem }),
            (Logged::Read { key }, Ok(list)) => self.ops.push(iso::Op::Read {
                key,
                list: list.clone(),
            }),
            _ => {}
        }
        let failed = reply.is_err();
        self.replies.push(reply);
        if failed {
            self.outcome = Some(Outcome::Aborted);
        } else if self.to_send.is_empty() {
            self.outcome = Some(Outcome::Committed);
        } else {
            self.send_next();
        }
    }

    /// Returns the reply of each request it sent, in order.
    pub fn replies(&self) -> &[Reply] {
        &self.replies
    }

    /// Returns how it ended, once it has.
    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }

    /// Returns the transaction as the history records it.
    ///
    /// Precondition: it has ended.
    pub fn history(&self) -> iso::Txn {
        let outcome = self.outcome.expect("the transaction ended");
        iso::Txn::new(self.id, outcome, self.ops.clone())
    }
}

impl Stepping for Txn {
    fn boundaries(&self) -> Boundaries {
        (Some(self.worker.thread()), self.boundaries)
    }

    fn start(&mut self, _: &SimRuntime) {
        self.send_next();
    }

    fn ended(&mut self) -> bool {
        while self.outcome.is_none() {
            let Some(reply) = self.worker.try_reply() else {
                return false;
            };
            self.replied(reply);
        }
        self.rt.join(self.worker.thread());
        true
    }
}

/// A call run on a thread of its own once started, whose boundaries are the steps it reaches on
/// that thread or on one it starts.
#[derive(Debug)]
pub struct Called<T> {
    thread: ThreadId,
    on: Option<ThreadId>,
    counts: fn(Step) -> bool,
    go: Arc<AtomicBool>,
    out: Arc<Mutex<Option<T>>>,
    unwinds: bool,
}

impl<T: Send + 'static> Called<T> {
    /// Spawn a thread named `name` to run `body` once started, its boundaries the steps it
    /// reaches that `counts` accepts.
    #[inline]
    pub fn spawn(
        rt: &SimRuntime,
        on: (&str, fn(Step) -> bool),
        body: impl FnOnce() -> T + Send + 'static,
    ) -> Called<T> {
        Called::spawned(rt, on, Box::new(body))
    }

    fn spawned(
        rt: &SimRuntime,
        (name, counts): (&str, fn(Step) -> bool),
        body: Box<dyn FnOnce() -> T + Send>,
    ) -> Called<T> {
        let go = Arc::new(AtomicBool::new(false));
        let out = Arc::new(Mutex::new(None));
        let (go2, out2, rt2, driver) = (go.clone(), out.clone(), rt.clone(), rt.current());
        let thread = rt.spawn(name, move || {
            while !go2.load(Ordering::Acquire) {
                rt2.park();
            }
            let got = body();
            *out2.lock().unwrap_or_else(PoisonError::into_inner) = Some(got);
            rt2.unpark(driver);
        });
        Called {
            thread,
            on: Some(thread),
            counts,
            go,
            out,
            unwinds: false,
        }
    }

    /// Returns the call going on by unwinding from its stop in an
    /// [`interleave`](crate::steps::interleave), which its body must catch.
    pub fn unwinding(self) -> Called<T> {
        Called {
            unwinds: true,
            ..self
        }
    }

    /// Returns the call with its boundaries reached on a thread it starts.
    pub fn on_its_job(self) -> Called<T> {
        Called { on: None, ..self }
    }

    /// Returns what the call returned.
    ///
    /// Precondition: it has ended.
    pub fn take(&self) -> T {
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        out.take().expect("the call returned")
    }
}

impl<T> Stepping for Called<T> {
    fn boundaries(&self) -> Boundaries {
        (self.on, self.counts)
    }

    fn start(&mut self, rt: &SimRuntime) {
        self.go.store(true, Ordering::Release);
        rt.unpark(self.thread);
    }

    fn ended(&mut self) -> bool {
        let out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        out.is_some()
    }

    fn unwinds(&self) -> bool {
        self.unwinds
    }
}
