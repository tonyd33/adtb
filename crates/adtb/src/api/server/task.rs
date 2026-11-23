//! The calls the server submits: each an owned task that the engine's workers run in steps, whose
//! results the transport observes in batches under the capacity it supplies.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, OnceLock, Weak};
use std::task::{Wake, Waker};
use std::time::Duration;

use adtb_lang::schema::Schema;
use adtb_lang::textual::{Form, print};
use adtb_lang::types::Ty;
use adtb_sys::lock::Lock;

use crate::exec::{self, Grant, Refusal, Refused, Stopped, Val};
use crate::runtime::Runtime;

use super::super::admitted::Admitted;
use super::super::{CallOptions, CancelToken, Engine, Error, Limit, RejectReason, RetryReason};
use super::RoleSession;
use super::call::{Call, RoleCall, Work};
use super::result::{Completion, Format, Layout, ResultBatch};

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
    grant: Weak<Grant>,
    /// Set while the call holds its grant, from its start until it is dropped.
    granted: bool,
    /// Wakers a step of the call notifies once it has left the engine.
    pub(super) deferred: Vec<Waker>,
}

impl State {
    /// Record `grant` as the call's, held until [`State::released`].
    pub(super) fn granted(&mut self, grant: &Arc<Grant>) {
        self.grant = Arc::downgrade(grant);
        self.granted = true;
    }

    /// Record that the call no longer holds its grant.
    pub(super) fn released(&mut self) {
        self.granted = false;
    }

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
    fn request(&self, admitted: &exec::Admitted<R>, r: Request, token: &CancelToken) {
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
    admitted: OnceLock<exec::Admitted<R>>,
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
    admitted: exec::Admitted<R>,
    /// The token of the call's own statements, cancelled with the caller's.
    cancel: CancelToken,
    /// The caller's token, and the registration on it that stops the call.
    caller: Option<(CancelToken, u64)>,
    call: Option<i64>,
}

impl<R: Runtime> std::fmt::Debug for Task<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Task")
            .field("admitted", &self.admitted)
            .finish_non_exhaustive()
    }
}

/// The calls of role sessions that have not ended.
pub(in super::super) struct Running<R: Runtime> {
    next: i64,
    calls: BTreeMap<i64, (u64, Arc<Stopper<R>>)>,
}

impl<R: Runtime> Default for Running<R> {
    fn default() -> Self {
        Running {
            next: 1,
            calls: BTreeMap::new(),
        }
    }
}

impl<R: Runtime> Running<R> {
    /// Returns the key of a new call of `session`, forgetting those that have ended.
    fn add(&mut self, session: u64, stopper: Arc<Stopper<R>>) -> i64 {
        self.calls.retain(|_, (_, s)| s.shared.strong_count() > 0);
        let id = self.next;
        self.next += 1;
        self.calls.insert(id, (session, stopper));
        id
    }

    /// Returns the session of call `id`, unless the call has ended.
    pub(in super::super) fn session(&self, id: i64) -> Option<u64> {
        let (session, s) = self.calls.get(&id)?;
        s.shared.upgrade().map(|_| *session)
    }

    /// Cancel call `id` unless the call has ended.
    pub(in super::super) fn stop(&self, id: i64) {
        if let Some((_, s)) = self.calls.get(&id) {
            s.wake_by_ref();
        }
    }
}

impl<R: Runtime> Task<R> {
    /// Returns the key of the call's row of `sys::calls`, for a role session's call.
    pub fn call(&self) -> Option<i64> {
        self.call
    }

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
            ) if !s.granted => return Observation::Done(c.clone()),
            Some(c) if s.drained && !s.granted => return Observation::Done(c.clone()),
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

impl<R: Runtime> RoleSession<R> {
    /// Submit ad hoc `text` as a task, each free name typed and bound by `params`.
    ///
    /// A role but the embedded owner holds `adhoc`, and its text needs what each command requires
    /// and the grants for all it reads and writes, checked at submit and again when the task leaves
    /// the queue. Refuse other text with `Denied`, recorded in the audit, and a call the admission
    /// queue has no room for, without running it.
    pub fn submit_text(
        &self,
        text: &str,
        params: &[(&str, Ty, Val)],
        opts: TaskOptions,
    ) -> Result<Task<R>, Error> {
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let (admitted, call, _open) = self.admit_text(&e, text, params, &opts)?;
        submit(&e, (admitted, Work::Text), Some(call), opts)
    }
}

/// Returns the BLAKE3-256 hash of the canonical text of `params`, by name.
pub(super) fn args_hash(schema: &Schema, params: &[(&str, &Ty, &Val)]) -> Result<[u8; 32], Error> {
    let mut sorted: Vec<_> = params.iter().collect();
    sorted.sort_by_key(|(name, ..)| *name);
    let mut hasher = blake3::Hasher::new();
    for (name, ty, value) in sorted {
        let shown = print(*value, &Form::of(schema, ty)).map_err(|_| {
            Error::invalid(format!("parameter `{name}` does not print as its type"))
        })?;
        hasher.update(name.as_bytes());
        hasher.update(b"=");
        hasher.update(shown.as_bytes());
        hasher.update(b"\n");
    }
    Ok(*hasher.finalize().as_bytes())
}

pub(super) fn submit<R: Runtime>(
    e: &Arc<Engine<R>>,
    (admitted, work): (Admitted, Work),
    role_call: Option<RoleCall<R>>,
    mut opts: TaskOptions,
) -> Result<Task<R>, Error> {
    let sched = e.scheduler();
    let rt = &e.rt;
    let entry = admitted.entry();
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
    let call_id = role_call
        .as_ref()
        .map(|c| e.calls.lock(rt).add(c.session, stopper.clone()));
    let caller = opts.call.cancel.replace(cancel.clone()).map(|t| {
        let id = t.on_cancel(Waker::from(stopper.clone()));
        (t, id)
    });
    let call = Call::new(e, shared.clone(), (admitted, work), role_call, opts);
    match sched.admit(entry, Box::new(call)) {
        Ok(admitted) => {
            if stopper.admitted.set(admitted.clone()).is_err() {
                panic!("a call is admitted once");
            }
            if caller.as_ref().is_some_and(|(t, _)| t.is_cancelled()) {
                stopper.wake_by_ref();
            }
            Ok(Task {
                shared,
                admitted,
                cancel,
                caller,
                call: call_id,
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

    use crate::exec::Priority;
    use crate::runtime::Threads;

    use super::super::super::admitted::{Authority, Queued};
    use super::super::super::text::Prepared;
    use super::super::super::{CreateOptions, Database, Log, OWNER_ROLE, OpenOptions};
    use super::super::gate::Readmission;
    use super::super::tables::{CallOrigin, RoleId};
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
    fn a_close_reentered_on_the_closing_thread_returns_at_once() {
        let (dir, db) = opened("task-reentry");
        let e = &*db.engine;
        *e.close.lock(&e.rt) = Some(super::super::super::Close {
            by: e.rt.current(),
            kept: Vec::new(),
        });
        db.clone().close().unwrap();
        let open = db
            .query_text("count(i for i in items)", &[], &CallOptions::default())
            .unwrap();
        let counts: Vec<_> = open
            .into_iter()
            .map(crate::test_world::api::printed)
            .collect();
        assert_eq!(vec![vec!["0"]], counts);
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
    fn a_readmission_failing_but_for_denied_records_a_failed_call() {
        let scratch = adtb_testkit::scratch::Scratch::new("readmit-fixtures", "readmit");
        let rt = OsRuntime::new(scratch.path(), Backend::Pool { threads: 1 });
        let log = Log::new(["module example\nadd module example\n"]);
        let options = OpenOptions {
            create: Some(CreateOptions::default()),
            workers: Some(1),
            pool_bytes: Some(4 * 1024 * 1024),
            ..OpenOptions::default()
        };
        let db = Database::open_in(rt, "readmit.adt", &log, &options).expect("open");
        let e = db.engine.clone();
        let caller = e.principal(None).expect("principal");
        let items = Prepared::parsed("verify").expect("parses");
        let p =
            Prepared::of_items(&e.compiled, &e.explained(), &items, &[], caller).expect("checks");
        let call = RoleCall {
            role: "w".to_owned(),
            session: 0,
            origin: CallOrigin::AdHoc([0; 32]),
            args: [0; 32],
            readmission: Some(Readmission::Text {
                session: RoleSession {
                    serial: 0,
                    engine: Weak::new(),
                    id: RoleId(1),
                    name: "w".to_owned(),
                },
                text: [0; 32],
                items: Vec::new(),
                params: Vec::new(),
                args: [0; 32],
                assertion: None,
            }),
        };
        let queued = Queued::new(Priority::AdHoc, None, None, 0);
        let admitted = e
            .admit_program(p, &queued, Authority::Owner)
            .expect("it is admitted");
        let task = submit(
            &e,
            (admitted, Work::Text),
            Some(call),
            TaskOptions::default(),
        )
        .expect("the queue has room");
        let got = loop {
            match task.observe(Waker::noop()) {
                Observation::Done(done) => break done,
                _ => std::thread::yield_now(),
            }
        };
        assert!(
            matches!(&got, Completion::Failed(e) if *e == Error::closed()),
            "{got:?}"
        );
        let mut rows = db
            .query_text(
                "from a in catalog::audit.Call where a.role == \"w\" select a.seq",
                &[],
                &CallOptions::default(),
            )
            .expect("the audit reads");
        assert_eq!(1, rows.remove(0).count());
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
