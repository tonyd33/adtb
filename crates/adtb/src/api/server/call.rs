//! The call a task runs: a self-contained transaction of its own, run in steps on the engine's
//! workers, its output formed into batches as the task's capacity allows.

use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Weak};
use std::task::Waker;

use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use crate::exec::{Arena, Exceeded, Grant, Resumable, Step, Stopped, Val};
use crate::mvcc::{Watch, Xid};
use crate::runtime::Runtime;
use crate::write::{self, Deferral, Local, Outcome, Produced, Production, Rerun, Text, Txn};

use super::super::arrow::BATCH_ROWS;
use super::super::registered::Bound;
use super::super::text::{DynRows, Prepared};
use super::super::{Closes, Engine, Error, Halted, Position, TxnId};
use super::result::{Completion, Layout};
use super::task::{Out, Request, Shared, TaskOptions, stopped};

/// What a task calls.
pub(super) enum Work {
    Registered(Box<Bound>),
    Text(Box<Prepared>),
}

impl Work {
    fn prepared(&self) -> &Prepared {
        match self {
            Work::Registered(b) => &b.prepared,
            Work::Text(p) => p,
        }
    }
}

/// Where a call is.
enum Phase<R: Runtime> {
    Start,
    Deferring(Deferring),
    Writing(Writing),
    Output(Output<R>),
    /// A read's output is formed; its transaction stays open until the drain is acknowledged.
    Drain(Option<(TxnId, Txn)>),
    Over,
}

/// A deferrable read waiting for its snapshot to be safe.
struct Deferring {
    id: TxnId,
    tx: Txn,
    /// The transaction the waker is registered on.
    watched: Option<(Xid, Waker)>,
}

/// A write's attempt in progress.
#[derive(Default)]
struct Writing {
    tx: Option<(TxnId, Txn)>,
    attempts: u32,
    rerun: Option<Rerun>,
    /// The transaction the waker is registered on.
    watched: Option<(Xid, Waker)>,
}

/// Each result's layout and elements, in item order.
type Results<R> = VecDeque<(Arc<Layout>, Source<R>)>;

/// Results being formed into batches.
struct Output<R: Runtime> {
    results: Results<R>,
    /// Whether the front result's layout has been handed out.
    announced: bool,
    /// A read's transaction, held until the drain is acknowledged.
    tx: Option<(TxnId, Txn)>,
}

enum Source<R: Runtime> {
    /// Elements formed whole, and the reservation they hold.
    Whole(std::vec::IntoIter<Val>, Arena),
    Stream(Box<Stream<R>>),
}

/// A set result produced a unit at a time, each element handed out once.
struct Stream<R: Runtime> {
    production: Production<R>,
    more: bool,
    pending: VecDeque<Val>,
    /// The elements handed out so far, with the reservation they hold.
    sent: BTreeSet<Val>,
    held: Arena,
    schema: Arc<Schema>,
    elem: Ty,
}

pub(super) struct Call<R: Runtime> {
    engine: Weak<Engine<R>>,
    close: Closes,
    shared: Arc<Shared<R>>,
    work: Work,
    opts: TaskOptions,
    grant: Option<Arc<Grant>>,
    phase: Phase<R>,
}

/// How a phase of a step ended.
enum Then {
    Next,
    Step(Step),
}

/// Whether the call may form another batch.
enum Room {
    Open,
    /// It waits for capacity.
    Full,
    /// Its committed output was abandoned.
    Abandoned,
    /// Its owner asked it to stop before it committed.
    Stopped(Error),
}

impl<R: Runtime> Call<R> {
    pub(super) fn new(
        engine: &Arc<Engine<R>>,
        shared: Arc<Shared<R>>,
        work: Work,
        opts: TaskOptions,
    ) -> Call<R> {
        Call {
            engine: Arc::downgrade(engine),
            close: engine.close.clone(),
            shared,
            work,
            opts,
            grant: None,
            phase: Phase::Start,
        }
    }

    fn advance(&mut self, e: &Engine<R>, waker: &Waker) -> Result<Step, Error> {
        loop {
            let then = match std::mem::replace(&mut self.phase, Phase::Over) {
                Phase::Start => self.start(e)?,
                Phase::Deferring(d) => self.defer(e, d, waker)?,
                Phase::Writing(w) => self.write(e, w, waker)?,
                Phase::Output(o) => self.output(e, o, waker)?,
                Phase::Drain(tx) => self.drain(e, tx, waker)?,
                Phase::Over => Then::Step(Step::Done),
            };
            if let Then::Step(s) = then {
                return Ok(s);
            }
        }
    }

    fn start(&mut self, e: &Engine<R>) -> Result<Then, Error> {
        let requested = self.shared.defer(|s| {
            s.started = s.request.is_none();
            (s.request, [None, None])
        });
        if let Some(r) = requested {
            return Err(r.error());
        }
        let grant =
            e.db.admit_grant()
                .map_err(|err| e.write_error(TxnId(0), err))?;
        self.shared.defer(|s| {
            s.grant = Arc::downgrade(&grant);
            ((), [None, None])
        });
        self.grant = Some(grant);
        self.opts.call.stops().check(&e.rt)?;
        e.reads_after(self.opts.call.after)?;
        if let Work::Registered(b) = &self.work
            && b.due
        {
            e.renew(b.name());
        }
        let p = self.work.prepared();
        self.phase = if p.writes {
            Phase::Writing(Writing {
                attempts: 1,
                ..Writing::default()
            })
        } else if p.deferrable {
            let id = e.txn_id();
            let tx =
                e.db.begin_deferrable()
                    .map_err(|err| e.write_error(id, err))?;
            Phase::Deferring(Deferring {
                id,
                tx,
                watched: None,
            })
        } else {
            let id = e.txn_id();
            let tx = e.begin_read(id, false)?;
            Phase::Output(self.read(e, id, tx)?)
        };
        Ok(Then::Next)
    }

    /// Wait, without the worker, until the deferrable read's snapshot is safe, then begin its
    /// output.
    fn defer(&mut self, e: &Engine<R>, d: Deferring, waker: &Waker) -> Result<Then, Error> {
        let Deferring { id, mut tx, .. } = d;
        loop {
            match e.db.defer(tx).map_err(|err| e.write_error(id, err))? {
                Deferral::Safe(safe) => {
                    self.phase = Phase::Output(self.read(e, id, safe)?);
                    return Ok(Then::Next);
                }
                Deferral::Wait { tx: waiting, on } => match e.db.watch(None, on, waker) {
                    Ok(Watch::Ended) => tx = waiting,
                    Ok(Watch::Pending) => {
                        self.phase = Phase::Deferring(Deferring {
                            id,
                            tx: waiting,
                            watched: Some((on, waker.clone())),
                        });
                        return Ok(Then::Step(Step::Wait));
                    }
                    Err(f) => {
                        e.abort(id, waiting)?;
                        return Err(e.write_error(id, f.into()));
                    }
                },
            }
        }
    }

    fn bounds(&self, e: &Engine<R>) -> write::Bounds {
        write::Bounds {
            grant: self.grant.clone(),
            ..self
                .opts
                .call
                .stops()
                .bounds(&e.rt, self.opts.call.fix_rounds)
        }
    }

    fn text<'a>(&self, e: &Engine<R>, p: &'a Prepared) -> Text<'a> {
        Text {
            schema: p.schema(),
            terms: p.terms(),
            items: p.items(),
            params: p.params(),
            clock: e.clock(),
            caller: Some(p.caller()),
        }
    }

    /// Start the output of the read `tx`, the transaction `id`: produced a unit at a time for
    /// one set-valued query, else formed whole.
    fn read(&mut self, e: &Engine<R>, id: TxnId, tx: Txn) -> Result<Output<R>, Error> {
        let p = self.work.prepared();
        let streams = p.commands().is_empty()
            && p.items().len() == 1
            && matches!(p.terms().ty(p.items()[0]), Ty::Set(_) | Ty::NonEmpty(_));
        let produced = if streams {
            e.db.produce(&tx, self.text(e, p), &self.bounds(e))
        } else {
            Ok(None)
        };
        let results = match produced {
            Ok(Some(Produced::Stream(production))) => self.stream(production),
            Ok(Some(Produced::Whole(result))) => {
                let outcome = Outcome {
                    result,
                    inserted: write::Rows::new(),
                    deleted: write::Rows::new(),
                };
                p.results(p.schema(), vec![outcome], Vec::new())
                    .and_then(|rows| self.whole(e, rows))
            }
            Ok(None) => e
                .statements(id, &tx, p, &mut Local::default(), self.bounds(e))
                .map_err(|(err, _)| err)
                .and_then(|rows| self.whole(e, rows)),
            Err(err) => Err(e.write_error(id, err)),
        };
        match results {
            Ok(results) => Ok(Output {
                results,
                announced: false,
                tx: Some((id, tx)),
            }),
            Err(err) => {
                e.abort(id, tx)?;
                Err(err)
            }
        }
    }

    fn stream(&self, production: Production<R>) -> Result<Results<R>, Error> {
        let p = self.work.prepared();
        let (Ty::Set(elem) | Ty::NonEmpty(elem)) = p.terms().ty(p.items()[0]) else {
            unreachable!("a streamed result is a set")
        };
        let elem = elem.as_ref().clone();
        let (schema, ty) = match &self.work {
            Work::Registered(b) => {
                let rows = b.elements(p.schema(), elem.clone(), Vec::new())?;
                (b.schema().clone(), rows.ty().clone())
            }
            Work::Text(p) => (p.schema().clone(), elem.clone()),
        };
        let layout = Arc::new(Layout::new(&schema, ty, self.opts.format)?);
        let stream = Stream {
            production,
            more: true,
            pending: VecDeque::new(),
            sent: BTreeSet::new(),
            held: self.arena(),
            schema: p.schema().clone(),
            elem,
        };
        Ok(VecDeque::from([(layout, Source::Stream(Box::new(stream)))]))
    }

    /// Returns `rows`, the results of the call's items, as whole sources charged to its grant.
    fn whole(&self, e: &Engine<R>, rows: Vec<DynRows>) -> Result<Results<R>, Error> {
        let (schema, rows) = match &self.work {
            Work::Registered(b) => (b.schema().clone(), vec![b.result(e, rows)?]),
            Work::Text(p) => (p.schema().clone(), rows),
        };
        let mut out = VecDeque::new();
        for r in rows {
            let layout = Arc::new(Layout::new(&schema, r.ty().clone(), self.opts.format)?);
            let vals: Vec<Val> = r.collect::<Result<_, _>>()?;
            let mut held = self.arena();
            held.grow(vals.iter().map(Val::size).sum())
                .map_err(|err| e.write_error(TxnId(0), err.into()))?;
            out.push_back((layout, Source::Whole(vals.into_iter(), held)));
        }
        Ok(out)
    }

    fn arena(&self) -> Arena {
        self.grant
            .as_ref()
            .expect("a started call has its grant")
            .arena()
    }

    /// Run the write's attempts until one commits, fails, or waits for a transaction.
    fn write(&mut self, e: &Engine<R>, mut w: Writing, waker: &Waker) -> Result<Then, Error> {
        loop {
            let (id, tx) = match w.tx.take() {
                Some(t) => t,
                None => {
                    self.opts.call.stops().check(&e.rt)?;
                    w.rerun = None;
                    let id = e.txn_id();
                    (id, e.begin(id)?)
                }
            };
            w.watched = None;
            let bounds = write::Bounds {
                suspend: true,
                rerun: w.rerun,
                ..self.bounds(e)
            };
            let p = self.work.prepared();
            match e.resumable(id, &tx, p, &mut Local::default(), bounds) {
                Ok(rows) => return self.commit(e, w, id, tx, rows),
                Err(Halted::Blocked { on, rerun }) => {
                    w.rerun = Some(rerun);
                    let until = self.opts.call.limit.map(|l| (l, rerun.limit_at(l)));
                    if let Some((after, at)) = until
                        && at <= e.rt.now()
                    {
                        e.abort(id, tx)?;
                        let limit = write::Error::StatementLimit(Exceeded::Time { after });
                        return Err(e.write_error(id, limit));
                    }
                    match e.db.watch(tx.xid(), on, waker) {
                        Ok(Watch::Ended) => w.tx = Some((id, tx)),
                        Ok(Watch::Pending) => {
                            w.watched = Some((on, waker.clone()));
                            w.tx = Some((id, tx));
                            let step = match until {
                                Some((_, at)) => Step::WaitUntil(at),
                                None => Step::Wait,
                            };
                            self.phase = Phase::Writing(w);
                            return Ok(Then::Step(step));
                        }
                        Err(f) => {
                            e.abort(id, tx)?;
                            self.retry(e, &mut w, e.write_error(id, f.into()))?;
                        }
                    }
                }
                Err(Halted::Failed(err, _)) => {
                    e.abort(id, tx)?;
                    self.retry(e, &mut w, err)?;
                }
            }
        }
    }

    /// Count another attempt after the failure `err` of an aborted one, or return `err` when
    /// it is no serialization failure, the call has no reruns left, or its owner asked it to
    /// stop.
    fn retry(&self, e: &Engine<R>, w: &mut Writing, err: Error) -> Result<(), Error> {
        let requested = self.shared.state.lock(&self.shared.rt).request;
        if let Some(r) = requested {
            return Err(r.error());
        }
        let reruns = e.reruns(self.work.prepared(), self.opts.call.retry);
        if err.is_serialization() && u64::from(w.attempts) <= reruns {
            w.attempts += 1;
            Ok(())
        } else {
            Err(err)
        }
    }

    /// Commit the attempt that returned `rows`, unless the owner asked the call to stop first.
    fn commit(
        &mut self,
        e: &Engine<R>,
        mut w: Writing,
        id: TxnId,
        tx: Txn,
        rows: Vec<DynRows>,
    ) -> Result<Then, Error> {
        let results = match self
            .opts
            .call
            .stops()
            .check(&e.rt)
            .and_then(|()| self.whole(e, rows))
        {
            Ok(r) => r,
            Err(err) => {
                e.abort(id, tx)?;
                return Err(err);
            }
        };
        let requested = self.shared.defer(|s| {
            s.committing = s.request.is_none();
            (s.request, [None, None])
        });
        if let Some(r) = requested {
            e.abort(id, tx)?;
            return Err(r.error());
        }
        let done = e.db.commit(tx);
        let after = self.shared.defer(|s| {
            s.committing = false;
            if let Ok(at) = &done {
                s.committed = true;
                s.settled = Some(Completion::Committed {
                    position: Position::from(*at),
                    attempts: w.attempts,
                    incomplete: false,
                });
                if matches!(s.request, Some(Request::Idle(_) | Request::Drop)) {
                    s.abandon();
                    return (true, s.wakers());
                }
            }
            (false, [None, None])
        });
        match done {
            Ok(_) if after => Ok(Then::Step(Step::Done)),
            Ok(_) => {
                self.phase = Phase::Output(Output {
                    results,
                    announced: false,
                    tx: None,
                });
                Ok(Then::Next)
            }
            Err(err) => {
                self.retry(e, &mut w, e.write_error(id, err))?;
                self.phase = Phase::Writing(w);
                Ok(Then::Next)
            }
        }
    }

    /// Form batches while capacity remains, then wait for more or end the output. On a failure,
    /// abort the read.
    fn output(&mut self, e: &Engine<R>, mut o: Output<R>, waker: &Waker) -> Result<Then, Error> {
        match self.fill(e, &mut o, waker) {
            Ok(Some(step)) => {
                self.phase = Phase::Output(o);
                Ok(Then::Step(step))
            }
            Ok(None) => {
                self.phase = match o.tx {
                    Some(tx) => Phase::Drain(Some(tx)),
                    None => Phase::Over,
                };
                Ok(Then::Next)
            }
            Err(err) => {
                if let Some((id, tx)) = o.tx.take() {
                    e.abort(id, tx)?;
                }
                Err(err)
            }
        }
    }

    /// Form batches of `o` while capacity remains. Returns the step that waits for more, or
    /// `None` once the output is formed.
    fn fill(&self, e: &Engine<R>, o: &mut Output<R>, waker: &Waker) -> Result<Option<Step>, Error> {
        while let Some((layout, source)) = o.results.front_mut() {
            let layout = layout.clone();
            let announce = !std::mem::replace(&mut o.announced, true);
            let room = self.shared.defer(|s| {
                if let Some(r) = s.request.filter(|_| !s.committed) {
                    return (Room::Stopped(r.error()), [None, None]);
                }
                if s.produced {
                    return (Room::Abandoned, [None, None]);
                }
                if announce {
                    s.out.push_back(Out::Layout(layout.clone()));
                }
                let observer = if announce { s.observer.take() } else { None };
                if s.room == 0 {
                    s.producer = Some(waker.clone());
                    return (Room::Full, [observer, None]);
                }
                (Room::Open, [observer, None])
            });
            match room {
                Room::Open => {}
                Room::Full => return Ok(Some(Step::Wait)),
                Room::Abandoned => return Ok(None),
                Room::Stopped(err) => return Err(err),
            }
            let id = o.tx.as_ref().map_or(TxnId(0), |(id, _)| *id);
            let rows = self.take(e, id, source)?;
            if rows.is_empty() {
                if source.finished() {
                    o.results.pop_front();
                    o.announced = false;
                }
                continue;
            }
            let bytes = rows.iter().map(Val::size).sum();
            if let Source::Whole(_, held) = source {
                held.release(bytes);
            }
            let mut held = self.arena();
            held.grow(bytes)
                .map_err(|err| e.write_error(id, err.into()))?;
            let batch = layout.batch(rows, held)?;
            self.shared.defer(|s| {
                s.room = s.room.saturating_sub(bytes);
                s.out.push_back(Out::Batch(batch));
                ((), [s.observer.take(), None])
            });
        }
        self.shared.defer(|s| {
            s.produced = true;
            ((), [s.observer.take(), None])
        });
        Ok(None)
    }

    /// Returns up to a batch of the next elements of `source`, running a unit of a production
    /// when none is pending.
    fn take(&self, e: &Engine<R>, id: TxnId, source: &mut Source<R>) -> Result<Vec<Val>, Error> {
        match source {
            Source::Whole(it, _) => Ok(it.by_ref().take(BATCH_ROWS).collect()),
            Source::Stream(s) => {
                if s.pending.is_empty() && s.more {
                    let mut fresh = Vec::new();
                    s.more = s
                        .production
                        .next(&mut fresh)
                        .map_err(|err| e.write_error(id, err))?;
                    let fresh = match &self.work {
                        Work::Registered(b) => b
                            .elements(&s.schema, s.elem.clone(), fresh)?
                            .collect::<Result<Vec<_>, _>>()?,
                        Work::Text(_) => fresh,
                    };
                    for v in fresh {
                        if !s.sent.contains(&v) {
                            s.held
                                .grow(v.size())
                                .map_err(|err| e.write_error(id, err.into()))?;
                            s.sent.insert(v.clone());
                            s.pending.push_back(v);
                        }
                    }
                }
                let n = s.pending.len().min(BATCH_ROWS);
                Ok(s.pending.drain(..n).collect())
            }
        }
    }

    /// Wait for the drain acknowledgement, then commit the read and settle it.
    fn drain(
        &mut self,
        e: &Engine<R>,
        tx: Option<(TxnId, Txn)>,
        waker: &Waker,
    ) -> Result<Then, Error> {
        let drained = self.shared.defer(|s| {
            if !s.drained {
                s.producer = Some(waker.clone());
            }
            (s.drained, [None, None])
        });
        if !drained {
            self.phase = Phase::Drain(tx);
            return Ok(Then::Step(Step::Wait));
        }
        if let Some((id, tx)) = tx {
            let at = e.db.commit(tx).map_err(|err| e.write_error(id, err))?;
            self.settle(Completion::Read {
                snapshot: Position::from(at),
            });
        }
        Ok(Then::Step(Step::Done))
    }

    fn settle(&self, c: Completion) {
        self.shared.defer(|s| {
            s.settle(c);
            ((), [s.observer.take(), None])
        });
    }

    /// Abort what the call holds and settle it as failed with `err`, or as its owner's request
    /// asked, unless it has committed. Keep the wakers to wake for [`Shared::notify`].
    fn fail(&mut self, err: Error) {
        self.release();
        self.shared.defer(|s| {
            if s.committed {
                s.abandon();
            } else {
                let err = s.request.map_or(err, Request::error);
                s.settle(Completion::Failed(err));
            }
            ((), s.wakers())
        });
    }

    /// Withdraw the call's waker and abort its open transaction.
    fn release(&mut self) {
        let phase = std::mem::replace(&mut self.phase, Phase::Over);
        let Some(e) = self.engine.upgrade() else {
            return;
        };
        let tx = match phase {
            Phase::Deferring(d) => {
                if let Some((on, waker)) = &d.watched {
                    e.db.withdraw(None, *on, waker);
                }
                Some((d.id, d.tx))
            }
            Phase::Writing(w) => {
                if let (Some((_, tx)), Some((on, waker))) = (&w.tx, &w.watched) {
                    e.db.withdraw(tx.xid(), *on, waker);
                }
                w.tx
            }
            Phase::Output(o) => o.tx,
            Phase::Drain(tx) => tx,
            Phase::Start | Phase::Over => None,
        };
        if let Some((id, tx)) = tx
            && let Err(err) = e.abort(id, tx)
        {
            self.shared.with(|s| {
                s.settle(Completion::Failed(err));
                ((), [None, None])
            });
        }
    }
}

impl<R: Runtime> Source<R> {
    fn finished(&self) -> bool {
        match self {
            Source::Whole(it, _) => it.len() == 0,
            Source::Stream(s) => !s.more && s.pending.is_empty(),
        }
    }
}

impl<R: Runtime> Resumable for Call<R> {
    fn run(&mut self, waker: &Waker) -> Step {
        let step = match self.engine.upgrade() {
            Some(e) => match e.enter() {
                Ok(_open) => self.advance(&e, waker),
                Err(err) => Err(err),
            },
            None => Err(Error::closed()),
        };
        let step = step.unwrap_or_else(|err| {
            self.fail(err);
            Step::Done
        });
        self.shared.notify();
        step
    }

    /// Settle the call as stopped. While the engine closes, its observer is woken once the close
    /// has ended.
    fn stop(mut self: Box<Self>, why: Stopped) {
        self.fail(stopped(why));
        let wakers = self.shared.deferred();
        let now = match self.close.lock(&self.shared.rt).as_mut() {
            Some(c) => {
                c.kept.extend(wakers);
                Vec::new()
            }
            None => wakers,
        };
        now.into_iter().for_each(Waker::wake);
    }
}
