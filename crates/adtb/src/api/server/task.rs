//! The calls the server submits: each an owned task that the engine's workers run in steps, whose
//! results the transport observes in batches under the capacity it supplies.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, OnceLock, Weak};
use std::task::{Wake, Waker};
use std::time::Duration;

use adtb_lang::ast::{self, AdhocItem};
use adtb_lang::schema::{ModuleDef, Schema, ValueDef};
use adtb_lang::textual::{Form, print};
use adtb_lang::types::{RelId, Ty};
use adtb_sys::lock::Lock;

use crate::exec::{Admitted, Entry, Grant, Priority, Refusal, Refused, Stopped, Val};
use crate::runtime::Runtime;

use super::super::text::Prepared;
use super::super::text::TextError;
use super::super::{
    CallOptions, CancelToken, Engine, Error, Limit, OWNER_ROLE, RejectReason, RetryReason,
};
use super::adhoc::refused_text;
use super::call::{Call, RoleCall, Work};
use super::result::{Completion, Format, Layout, ResultBatch};
use super::session::{Readmission, text_hash};
use super::tables::{CallOrigin, CallRow};
use super::{RoleAuthority, RoleSession};

/// The bytes a queued task's metadata counts for besides its arguments.
pub(super) const TASK_BYTES: usize = 256;

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
        let authority = RoleAuthority {
            engine: self.engine.clone(),
        };
        let assertion = opts.call.assert.clone();
        let hash = text_hash(text);
        let origin = CallOrigin::AdHoc(hash);
        let caller = authority.admit_caller(self, &hash, assertion.as_ref())?;
        let e = self.engine.upgrade().ok_or_else(Error::closed)?;
        let names: Vec<String> = params.iter().map(|(n, ..)| (*n).to_owned()).collect();
        let parsed = Prepared::parsed(text);
        let mut issuer = match &parsed {
            Ok(items) => authority.admit_items(self, &e, items, &names, &caller, &hash)?,
            Err(_) => None,
        };
        let _open = e.enter()?;
        let held = (self.name != OWNER_ROLE)
            .then(|| authority.holds(self))
            .transpose()?;
        let unseen = held.as_ref().filter(|h| !h.schema);
        let refused = || {
            if let Some(quarantined) = e.refusal() {
                return quarantined;
            }
            match e.audit_denied(&self.name, &caller, &origin) {
                Ok(()) => refused_text(&self.name),
                Err(err) => err,
            }
        };
        let failed = |args: &[u8; 32], err: Error| {
            e.audit_call(&CallRow {
                role: &self.name,
                caller: &caller,
                origin: &origin,
                args,
                failed: Some(&err),
            })
            .map_or_else(|audit| audit, |()| err)
        };
        if unseen.is_some_and(|h| {
            params
                .iter()
                .any(|(_, ty, _)| !declared(&e.schema, ty) || !h.names.may_type(&e.schema, ty))
        }) {
            return Err(refused());
        }
        if let Some((name, ..)) = params.iter().find(|(_, ty, _)| !declared(&e.schema, ty)) {
            let err = Error::invalid(format!("parameter `{name}` has no type of the schema"));
            return Err(failed(&names_hash(&names), err));
        }
        let typed: Vec<_> = params.iter().map(|(n, t, v)| (*n, t, v)).collect();
        let args = match args_hash(&e.schema, &typed) {
            Ok(args) => args,
            Err(err) => return Err(failed(&names_hash(&names), err)),
        };
        let capability_only = parsed
            .as_ref()
            .is_ok_and(|items| items.iter().all(is_capability_command));
        let hides = unseen.is_some() && parsed.is_ok() && !capability_only;
        let view = unseen.map(|h| {
            let commanded = match &parsed {
                Ok(items) if capability_only => commanded(&e.schema, items),
                _ => Vec::new(),
            };
            h.names.naming(commanded).view(&e.schema)
        });
        let checked = parsed
            .map_err(|err| TextError::Other(e.refusal().unwrap_or(err)))
            .and_then(|items| {
                let p = Prepared::naming(
                    &e.schema,
                    view.as_ref().unwrap_or(&e.schema),
                    &e.program,
                    &e.explained(),
                    &items,
                    params,
                    caller.clone(),
                )?;
                let p = e.admit_text(Ok(p)).map_err(TextError::Other)?;
                Ok((p, items))
            });
        let (mut p, items) = match checked {
            Ok(checked) => checked,
            Err(TextError::Checker(_)) if hides => return Err(refused()),
            Err(TextError::Checker(err)) => {
                let err = e.refusal().unwrap_or(err);
                return Err(failed(&args, err));
            }
            Err(TextError::Other(err)) => return Err(failed(&args, err)),
        };
        if let Some(issuer) = &mut issuer {
            issuer.holds = Some(authority.admit_prepared(self, &e, &p, &caller, issuer, &args)?);
        }
        p.issuer = issuer;
        let readmission = (self.name != OWNER_ROLE).then(|| Readmission::Text {
            session: self.clone(),
            text: hash,
            items,
            params: names,
            args,
            assertion,
        });
        let call = RoleCall {
            role: self.name.clone(),
            session: self.serial,
            origin,
            args,
            readmission,
        };
        let bytes = TASK_BYTES + text.len() + p.bytes();
        let work = Work::Text(Box::new(p));
        submit(&e, work, Some(call), opts, Priority::AdHoc, bytes)
    }
}

/// Returns whether `set` selects constructors among `count`, each once, in order.
fn selects(set: Option<&[u32]>, count: usize) -> bool {
    set.is_none_or(|s| {
        !s.is_empty()
            && s.windows(2).all(|w| w[0] < w[1])
            && s.iter().all(|c| (*c as usize) < count)
    })
}

/// Returns whether every declaration and constructor `ty` names is one of `schema`, and it is a
/// value's type.
fn declared(schema: &Schema, ty: &Ty) -> bool {
    match ty {
        Ty::Base(_) | Ty::Unit => true,
        Ty::Data { def, args, ctors } => {
            (def.0 as usize) < schema.types.len()
                && args.len() == schema.ty(*def).params.len()
                && selects(ctors.as_deref(), schema.ty(*def).ctors().len())
                && args.iter().all(|t| declared(schema, t))
        }
        Ty::Key { rel, ctors } => {
            (rel.0 as usize) < schema.relations.len()
                && selects(
                    ctors.as_deref(),
                    schema.ty(schema.relation(*rel).elem_def).ctors().len(),
                )
        }
        Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => declared(schema, t),
        Ty::Group(k, t) => declared(schema, k) && declared(schema, t),
        Ty::Record(fields) => fields.iter().all(|(_, t)| declared(schema, t)),
        Ty::Sum { ctors, sel } => {
            selects(sel.as_deref(), ctors.len())
                && ctors
                    .iter()
                    .all(|(_, fields)| fields.iter().all(|(_, t)| declared(schema, t)))
        }
        Ty::Param(_) | Ty::Fn(..) | Ty::Change => false,
    }
}

/// Returns whether `item` is a command a capability alone decides, holding no expression the
/// role's grants decide.
fn is_capability_command(item: &AdhocItem) -> bool {
    let AdhocItem::Command(command, _) = item else {
        return false;
    };
    match command {
        ast::Command::Vacuum(_)
        | ast::Command::Verify(_)
        | ast::Command::Reindex { .. }
        | ast::Command::Compact
        | ast::Command::FinalizeUpgrade
        | ast::Command::RetireBundle(_)
        | ast::Command::SupersedeBundle(..)
        | ast::Command::RegisterBundle(_)
        | ast::Command::ApplyMigrations(_)
        | ast::Command::PrintSchema
        | ast::Command::RotateKek(_)
        | ast::Command::RotateDek
        | ast::Command::DestroyDek(_)
        | ast::Command::Backup { .. } => true,
        ast::Command::Explain { .. }
        | ast::Command::PinPlan(_)
        | ast::Command::UnpinPlan(_)
        | ast::Command::CancelCall(_)
        | ast::Command::TerminateSession(_)
        | ast::Command::SetSetting(..)
        | ast::Command::Export { .. }
        | ast::Command::ArchiveAudit { .. }
        | ast::Command::Dump { .. }
        | ast::Command::Restore { .. } => false,
    }
}

/// Returns every relation of a user module the relation names in `items` may denote.
fn commanded(schema: &Schema, items: &[AdhocItem]) -> Vec<RelId> {
    let mut out = Vec::new();
    for item in items {
        let AdhocItem::Command(
            ast::Command::Vacuum(name)
            | ast::Command::Verify(Some(name))
            | ast::Command::Reindex { relation: name, .. },
            _,
        ) = item
        else {
            continue;
        };
        let denoted = |module: &ModuleDef| match module.values.get(&name.name.text) {
            Some(ValueDef::Relation(r)) if !module.engine => Some(*r),
            _ => None,
        };
        match &name.module {
            Some(m) => out.extend(
                schema
                    .module_named(&m.text)
                    .and_then(|id| denoted(schema.module(id))),
            ),
            None => out.extend(schema.modules.iter().filter_map(denoted)),
        }
    }
    out
}

/// Returns the audit's arguments hash of a text whose values do not print: that of their names.
fn names_hash(names: &[String]) -> [u8; 32] {
    let mut sorted: Vec<&String> = names.iter().collect();
    sorted.sort();
    let mut hasher = blake3::Hasher::new();
    for name in sorted {
        hasher.update(name.as_bytes());
        hasher.update(&[0]);
    }
    *hasher.finalize().as_bytes()
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
    work: Work,
    role_call: Option<RoleCall<R>>,
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
    let call_id = role_call
        .as_ref()
        .map(|c| e.calls.lock(rt).add(c.session, stopper.clone()));
    let caller = opts.call.cancel.replace(cancel.clone()).map(|t| {
        let id = t.on_cancel(Waker::from(stopper.clone()));
        (t, id)
    });
    let call = Call::new(e, shared.clone(), work, role_call, opts);
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

    use crate::runtime::Threads;

    use super::super::super::{CreateOptions, Database, Log, OpenOptions};
    use super::super::session::Readmission;
    use super::super::tables::RoleId;
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
        let p = Prepared::of_items(&e.schema, &e.program, &e.explained(), &items, &[], caller)
            .expect("checks");
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
        let work = Work::Text(Box::new(p));
        let task = submit(
            &e,
            work,
            Some(call),
            TaskOptions::default(),
            Priority::AdHoc,
            0,
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
