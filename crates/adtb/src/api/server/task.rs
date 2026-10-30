//! The calls the server submits: each an owned task that the engine's workers run in steps, whose
//! results the transport observes in batches under the capacity it supplies.

use std::collections::VecDeque;
use std::sync::{Arc, OnceLock, Weak};
use std::task::{Wake, Waker};
use std::time::Duration;

use adtb_lang::types::Ty;
use adtb_sys::lock::Lock;

use crate::exec::{Admitted, Entry, Grant, Priority, Refusal, Refused, Stopped, Val};
use crate::runtime::Runtime;

use super::super::text::Prepared;
use super::super::{
    CallOptions, CancelToken, Engine, Error, Limit, OWNER_ROLE, Registered, RejectReason,
    RetryReason,
};
use super::call::{Call, Work};
use super::result::{Completion, Format, Layout, ResultBatch};
use super::{RoleAuthority, RoleSession, denied};

/// The bytes a queued task's metadata counts for besides its arguments.
const TASK_BYTES: usize = 256;

/// The options of a submitted call.
#[derive(Clone, Debug, Default)]
pub struct TaskOptions {
    pub call: CallOptions,
    pub format: Format,
}

/// What [`Task::observe`] found.
#[derive(Debug)]
pub enum Observation {
    /// The layout of the batches that follow, before each result's first batch.
    Layout(Arc<Layout>),
    Batch(ResultBatch),
    /// Every batch has been observed; [`Task::drained`] completes the task.
    End,
    Done(Completion),
    /// Nothing yet; the waker passed is woken once an observation may make progress.
    Pending,
}

/// Why the task's owner asked it to stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Request {
    Cancel,
    Idle(Duration),
    Drop,
}

impl Request {
    pub(super) fn error(self) -> Error {
        match self {
            Request::Cancel | Request::Drop => Error::rejected(RejectReason::Cancelled),
            Request::Idle(limit) => Error::Retry {
                reason: RetryReason::StreamIdle { limit },
            },
        }
    }
}

pub(super) enum Out {
    Layout(Arc<Layout>),
    Batch(ResultBatch),
}

/// What a task's handle and its call share.
#[derive(Default)]
pub(super) struct State {
    pub(super) out: VecDeque<Out>,
    /// The bytes of batches the call may still form.
    pub(super) room: usize,
    /// Set once the call has formed its last batch.
    pub(super) produced: bool,
    pub(super) drained: bool,
    pub(super) settled: Option<Completion>,
    pub(super) request: Option<Request>,
    /// Set once the call starts; a request before then settles the task at once.
    pub(super) started: bool,
    /// Set while the call commits; a request then waits for the commit's outcome.
    pub(super) committing: bool,
    pub(super) committed: bool,
    pub(super) observer: Option<Waker>,
    /// The call's waker while it waits for capacity or the drain acknowledgement.
    pub(super) producer: Option<Waker>,
    pub(super) grant: Weak<Grant>,
    /// Wakers a step of the call notifies once it has left the engine.
    pub(super) deferred: Vec<Waker>,
}

impl State {
    /// Settle the task with `c` unless it has settled, discarding unobserved batches of a
    /// failure.
    pub(super) fn settle(&mut self, c: Completion) {
        if self.settled.is_some() {
            return;
        }
        if matches!(c, Completion::Failed(_)) {
            self.out.clear();
            self.produced = true;
        }
        self.settled = Some(c);
    }

    /// Abandon the output of a committed call: discard unobserved batches and mark the commit
    /// incomplete. A no-op once the output has been produced and drained.
    pub(super) fn abandon(&mut self) {
        if self.drained && self.produced {
            return;
        }
        self.out.clear();
        self.produced = true;
        if let Some(Completion::Committed { incomplete, .. }) = &mut self.settled {
            *incomplete = true;
        }
    }

    pub(super) fn wakers(&mut self) -> [Option<Waker>; 2] {
        [self.observer.take(), self.producer.take()]
    }
}

pub(super) struct Shared<R: Runtime> {
    pub(super) rt: R,
    pub(super) state: Lock<State>,
}

impl<R: Runtime> Shared<R> {
    /// Run `f` on the state, then wake the wakers it returns once the lock is released.
    pub(super) fn with<T>(&self, f: impl FnOnce(&mut State) -> (T, [Option<Waker>; 2])) -> T {
        let (t, wakers) = f(&mut self.state.lock(&self.rt));
        for w in wakers.into_iter().flatten() {
            w.wake();
        }
        t
    }

    /// Run `f` on the state, keeping the wakers it returns for [`notify`](Shared::notify).
    pub(super) fn defer<T>(&self, f: impl FnOnce(&mut State) -> (T, [Option<Waker>; 2])) -> T {
        let mut s = self.state.lock(&self.rt);
        let (t, wakers) = f(&mut s);
        s.deferred.extend(wakers.into_iter().flatten());
        t
    }

    /// Wake the wakers [`defer`](Shared::defer) kept.
    pub(super) fn notify(&self) {
        for w in self.deferred() {
            w.wake();
        }
    }

    /// Returns the wakers [`defer`](Shared::defer) kept, keeping none.
    pub(super) fn deferred(&self) -> Vec<Waker> {
        std::mem::take(&mut self.state.lock(&self.rt).deferred)
    }

    /// Record the owner's request `r`, and stop the call unless it has committed, settled, or
    /// produced and drained its output: cancel its running statement through `token` and
    /// withdraw it from the scheduler.
    fn request(&self, admitted: &Admitted<R>, r: Request, token: &CancelToken) {
        let stop = self.with(|s| {
            if (s.drained && s.produced) || (s.settled.is_some() && !s.committed) {
                return (false, [None, None]);
            }
            if s.committed {
                if r != Request::Cancel {
                    s.abandon();
                    return (true, s.wakers());
                }
                return (false, [None, None]);
            }
            let r = *s.request.get_or_insert(r);
            if !s.started {
                s.settle(Completion::Failed(r.error()));
                return (true, s.wakers());
            }
            (!s.committing, [None, None])
        });
        if stop {
            if !self.state.lock(&self.rt).committed {
                token.cancel();
            }
            admitted.cancel();
        }
    }
}

/// The waker that cancels a call once its caller's token is cancelled, only the token of the
/// call's own statements until the call is admitted.
struct Stopper<R: Runtime> {
    shared: Weak<Shared<R>>,
    /// The call's admission, set once the scheduler admits the call.
    admitted: OnceLock<Admitted<R>>,
    /// The token of the call's own statements.
    cancel: CancelToken,
}

impl<R: Runtime> Wake for Stopper<R> {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        match (self.shared.upgrade(), self.admitted.get()) {
            (Some(shared), Some(admitted)) => {
                shared.request(admitted, Request::Cancel, &self.cancel);
            }
            _ => self.cancel.cancel(),
        }
    }
}

/// A submitted call, owned by its submitter. Dropping it cancels a call that has not committed
/// and abandons its output.
pub struct Task<R: Runtime = adtb_sys::os::OsRuntime> {
    shared: Arc<Shared<R>>,
    admitted: Admitted<R>,
    /// The token of the call's own statements, cancelled with the caller's.
    cancel: CancelToken,
    /// The caller's token, and the registration on it that stops the call.
    caller: Option<(CancelToken, u64)>,
}

impl<R: Runtime> std::fmt::Debug for Task<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Task")
            .field("admitted", &self.admitted)
            .finish_non_exhaustive()
    }
}

impl<R: Runtime> Task<R> {
    /// Returns the next layout or batch, the end of the batches, the completion, or pending
    /// with `waker` registered. Never runs a statement.
    pub fn observe(&self, waker: &Waker) -> Observation {
        let mut s = self.shared.state.lock(&self.shared.rt);
        if let Some(o) = s.out.pop_front() {
            return match o {
                Out::Layout(l) => Observation::Layout(l),
                Out::Batch(b) => Observation::Batch(b),
            };
        }
        match &s.settled {
            Some(
                c @ (Completion::Failed(_)
                | Completion::Committed {
                    incomplete: true, ..
                }),
            ) => return Observation::Done(c.clone()),
            Some(c) if s.drained => return Observation::Done(c.clone()),
            _ => {}
        }
        if s.produced && !s.drained {
            return Observation::End;
        }
        s.observer = Some(waker.clone());
        Observation::Pending
    }

    /// Allow the call to form `bytes` more of batches.
    pub fn supply(&self, bytes: usize) {
        self.shared.with(|s| {
            s.room = s.room.saturating_add(bytes);
            ((), [None, s.producer.take()])
        });
    }

    /// Acknowledge that the last batch's encoded bytes have been sent.
    pub fn drained(&self) {
        self.shared.with(|s| {
            s.drained = true;
            ((), [s.observer.take(), s.producer.take()])
        });
    }

    /// Cancel the call unless it has committed or finished. Does not wait for a worker.
    pub fn cancel(&self) {
        self.request(Request::Cancel);
    }

    /// Abandon the output of a stream idle past `limit`: an uncommitted call fails with
    /// `StreamIdle`, a committed one keeps its commit and marks its output incomplete.
    pub fn expire_idle(&self, limit: Duration) {
        self.request(Request::Idle(limit));
    }

    /// Returns the bytes the call's grant has out, batches handed out included.
    pub fn held(&self) -> usize {
        let grant = self.shared.state.lock(&self.shared.rt).grant.upgrade();
        grant.map_or(0, |g| g.used())
    }

    fn request(&self, r: Request) {
        self.shared.request(&self.admitted, r, &self.cancel);
    }
}

impl<R: Runtime> Drop for Task<R> {
    fn drop(&mut self) {
        if let Some((token, id)) = &self.caller {
            token.forget(*id);
        }
        self.request(Request::Drop);
        let out = std::mem::take(&mut self.shared.state.lock(&self.shared.rt).out);
        drop(out);
    }
}

impl<R: Runtime> Registered<R> {
    /// Submit the call of `ordinal` with `args` under `session` as a task.
    ///
    /// Refuse a session of any role but the embedded owner with `Denied`, and a call the
    /// admission queue has no room for, without running it.
    pub fn submit(
        &self,
        session: &RoleSession<R>,
        ordinal: u32,
        args: Vec<Val>,
        opts: TaskOptions,
    ) -> Result<Task<R>, Error> {
        let caller = owner(session, opts.call.assert.as_ref())?;
        let e = self.engine_ref().upgrade().ok_or_else(Error::closed)?;
        let _open = e.enter()?;
        let bound = self.bound(&e, ordinal, &args, caller)?;
        let bytes = TASK_BYTES + args.iter().map(Val::size).sum::<usize>();
        let class = bound.priority;
        submit(&e, Work::Registered(Box::new(bound)), opts, class, bytes)
    }
}

impl<R: Runtime> RoleSession<R> {
    /// Submit ad hoc `text` as a task, each free name typed and bound by `params`.
    ///
    /// A role but the embedded owner holds `adhoc` and issues only commands, each needing what
    /// the command requires. Refuse any other text of such a role with `Denied`, recorded in the
    /// audit, before checking it against the schema, and a call the admission queue has no room
    /// for, without running it.
    pub fn submit_text(
        &self,
        text: &str,
        params: &[(&str, Ty, Val)],
        opts: TaskOptions,
    ) -> Result<Task<R>, Error> {
        let authority = RoleAuthority {
            engine: self.engine.clone(),
        };
        let caller = authority.admit_caller(self, text, opts.call.assert.as_ref())?;
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let _open = e.enter()?;
        let items = Prepared::parsed(text).map_err(|err| e.refusal().unwrap_or(err))?;
        let issuer = authority.admit_items(self, &e, &items, &caller, text)?;
        let p = Prepared::of_items(
            &e.schema,
            &e.program,
            &e.explained(),
            &items,
            params,
            caller,
        );
        let mut p = e.admit_text(p)?;
        p.issuer = issuer;
        let bytes = TASK_BYTES + text.len() + p.bytes();
        submit(&e, Work::Text(Box::new(p)), opts, Priority::AdHoc, bytes)
    }
}

/// Returns the principal a task of `session` runs for, refusing every role but the embedded
/// owner.
fn owner<R: Runtime>(session: &RoleSession<R>, assert: Option<&Val>) -> Result<Val, Error> {
    if session.name != OWNER_ROLE {
        return Err(denied("a task runs only for the embedded owner's role"));
    }
    RoleAuthority {
        engine: session.engine.clone(),
    }
    .principal(session, assert)
}

fn submit<R: Runtime>(
    e: &Arc<Engine<R>>,
    work: Work,
    mut opts: TaskOptions,
    class: Priority,
    bytes: usize,
) -> Result<Task<R>, Error> {
    let sched = e.scheduler();
    let rt = &e.rt;
    let deadline = opts.call.deadline.map(|d| {
        let left = d.duration_since(rt.wall()).unwrap_or(Duration::ZERO);
        rt.now() + left
    });
    let entry = Entry {
        priority: class,
        rank: opts.call.priority.unwrap_or(u8::MAX),
        start: rt.now(),
        bytes,
        deadline,
    };
    let shared = Arc::new(Shared {
        rt: rt.clone(),
        state: Lock::new(State::default()),
    });
    let cancel = CancelToken::new();
    let stopper = Arc::new(Stopper {
        shared: Arc::downgrade(&shared),
        admitted: OnceLock::new(),
        cancel: cancel.clone(),
    });
    let caller = opts.call.cancel.replace(cancel.clone()).map(|t| {
        let id = t.on_cancel(Waker::from(stopper.clone()));
        (t, id)
    });
    let call = Call::new(e, shared.clone(), work, opts);
    match sched.admit(entry, Box::new(call)) {
        Ok(admitted) => {
            assert!(
                stopper.admitted.set(admitted.clone()).is_ok(),
                "a call is admitted once"
            );
            if caller.as_ref().is_some_and(|(t, _)| t.is_cancelled()) {
                stopper.wake_by_ref();
            }
            Ok(Task {
                shared,
                admitted,
                cancel,
                caller,
            })
        }
        Err(Refused { reason, .. }) => {
            if let Some((t, id)) = &caller {
                t.forget(*id);
            }
            Err(refusal(reason, sched.admission().bytes))
        }
    }
}

/// Returns the error of a call the admission queue refused, `bound` its byte bound.
fn refusal(reason: Refusal, bound: usize) -> Error {
    match reason {
        Refusal::Overloaded => Error::Retry {
            reason: RetryReason::Overloaded {
                retry_after: Duration::ZERO,
            },
        },
        Refusal::TooLarge => Error::rejected(RejectReason::StatementLimit {
            limit: Limit::Memory {
                bytes: i64::try_from(bound).unwrap_or(i64::MAX),
            },
        }),
        Refusal::ShutDown => Error::closed(),
    }
}

/// Returns the error of a call the scheduler stopped for `why`.
pub(super) fn stopped(why: Stopped) -> Error {
    match why {
        Stopped::Cancelled => Error::rejected(RejectReason::Cancelled),
        Stopped::DeadlineExceeded => Error::rejected(RejectReason::DeadlineExceeded),
        Stopped::ShutDown => Error::closed(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Wake;

    use adtb_sys::os::{Backend, OsRuntime};

    use crate::runtime::Threads;

    use super::super::super::{CreateOptions, Database, Log, OpenOptions};
    use super::*;

    /// A waker counting the wakes a worker runs while the engine is held open for a call.
    struct Probe {
        db: Database,
        main: std::thread::Thread,
        under: AtomicUsize,
    }

    impl Wake for Probe {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            if std::thread::current().id() != self.main.id()
                && self.db.engine.open.try_write().is_none()
            {
                self.under.fetch_add(1, Ordering::AcqRel);
            }
            self.main.unpark();
        }
    }

    /// Returns a new directory `name` under the target directory, a relative one taken from the
    /// workspace root, else under the temporary one, and a database in it run on one worker.
    fn opened(name: &str) -> (std::path::PathBuf, Database) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(std::env::temp_dir, |d| root.join(d))
            .join(format!("{name}-{}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
        std::fs::create_dir_all(&dir).unwrap();
        let rt = OsRuntime::new(&dir, Backend::Pool { threads: 1 });
        let schema = "module m\nadd module m\nadd type Item = { id: Key items, n: I64 }\nadd relation items : Item key id generate sequence\n";
        let db = Database::open_in(
            rt,
            "wakes.adt",
            &Log::new([schema]),
            &OpenOptions {
                create: Some(CreateOptions::default()),
                workers: Some(1),
                ..OpenOptions::default()
            },
        )
        .unwrap();
        (dir, db)
    }

    #[test]
    fn a_texts_bytes_count_its_text_literals() {
        let (dir, db) = opened("task-literals");
        let e = &*db.engine;
        let prepared = |t: &str| {
            let caller = e.principal(None).unwrap();
            Prepared::new(&e.schema, &e.program, &e.explained(), t, &[], caller)
                .unwrap()
                .bytes()
        };
        let long = format!("\"{}\"", "x".repeat(10_000));
        assert!(prepared(&long) >= prepared("\"x\"") + 9_999);
        db.close().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_close_reentered_on_the_closing_thread_returns_at_once() {
        let (dir, db) = opened("task-reentry");
        let e = &*db.engine;
        *e.close.lock(&e.rt) = Some(super::super::super::Close {
            by: e.rt.current(),
            kept: Vec::new(),
        });
        db.clone().close().unwrap();
        let open = db.query_text("count(i for i in items)", &[], &CallOptions::default());
        assert!(open.is_ok(), "{open:?}");
        *e.close.lock(&e.rt) = None;
        db.close().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_observer_wake_runs_under_the_open_guard() {
        let (dir, db) = opened("task-wakes");
        db.query_text(
            "insert items from (from a in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9], b in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] select { n: a * 10 + b })",
            &[],
            &CallOptions::default(),
        )
        .unwrap();
        let owner = db.role_authority().session(OWNER_ROLE).unwrap();
        let probe = Arc::new(Probe {
            db: db.clone(),
            main: std::thread::current(),
            under: AtomicUsize::new(0),
        });
        let waker = Waker::from(probe.clone());
        let task = owner
            .submit_text("from i in items select i.n", &[], TaskOptions::default())
            .unwrap();
        let mut rows = 0;
        loop {
            task.supply(64);
            match task.observe(&waker) {
                Observation::Pending => std::thread::park(),
                Observation::Batch(b) => rows += b.len(),
                Observation::End => task.drained(),
                Observation::Done(_) => break,
                Observation::Layout(_) => {}
            }
        }
        assert_eq!(100, rows);
        assert_eq!(0, probe.under.load(Ordering::Acquire));
        drop(task);
        db.close().unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refusals_map_to_client_errors() {
        assert_eq!(
            Error::Retry {
                reason: RetryReason::Overloaded {
                    retry_after: Duration::ZERO
                }
            },
            refusal(Refusal::Overloaded, 64)
        );
        assert_eq!(
            Error::rejected(RejectReason::StatementLimit {
                limit: Limit::Memory { bytes: 64 }
            }),
            refusal(Refusal::TooLarge, 64)
        );
        assert_eq!(Error::closed(), refusal(Refusal::ShutDown, 64));
        assert_eq!(Error::closed(), stopped(Stopped::ShutDown));
        assert_eq!(
            Error::rejected(RejectReason::DeadlineExceeded),
            stopped(Stopped::DeadlineExceeded)
        );
    }
}
