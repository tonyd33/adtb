use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use adtb_sys::lock::Lock;

use crate::batch::Batch;
use crate::key::{Tid, Value};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::Pid;

use super::code::Catch;
use super::frame::{Frame, Sel};
use super::ir::{
    At, Chain, End, Eval, Expr, Fetch, Fix, Lookup, Pipeline, Program, Query, Sink, Source, Spine,
    Step,
};
use super::join::{Deferred, Probed, Table};
use super::memory::Grant;
use super::sched::Scheduler;
use super::spill::{Distinct, Spill};
use super::store::{Store, from_stored, to_key};
use super::value::Val;
use super::{Error, Exceeded, Fault, Options};

/// Everything the evaluations of one statement share.
pub struct Ctx<R: Runtime> {
    pub rt: R,
    pub program: Arc<Program>,
    pub store: Arc<Store<R>>,
    pub grant: Arc<Grant>,
    pub options: Options,
    pub pool: Option<Arc<Scheduler<R>>>,
    pub cancel: Arc<AtomicBool>,
    pub deadline: Option<super::Deadline>,
    pub spill: Spill<R>,
}

/// One evaluation of a query.
pub(super) struct Run {
    pub(super) query: Arc<Query>,
    pub(super) env: Frame,
    pub(super) builds: Vec<Lock<Option<Arc<Table>>>>,
    /// The values the finished tasks emitted, held until the query's sink reads them.
    emitted: Lock<Option<Distinct>>,
    pub(super) deferred: Lock<Deferred>,
    error: Lock<Option<Error>>,
    failed: AtomicBool,
    pending: AtomicUsize,
    /// Set once one value decides the sink, when no remaining row could abort.
    stopped: Option<Arc<AtomicBool>>,
}

impl Run {
    pub(super) fn new(query: Arc<Query>, env: Frame) -> Run {
        let builds = query.builds.iter().map(|_| Lock::new(None)).collect();
        let stops = query.total && matches!(query.sink, Sink::Exists | Sink::IsEmpty);
        Run {
            query,
            env,
            builds,
            emitted: Lock::new(None),
            deferred: Lock::new(Deferred::new()),
            error: Lock::new(None),
            failed: AtomicBool::new(false),
            pending: AtomicUsize::new(0),
            stopped: stops.then(|| Arc::new(AtomicBool::new(false))),
        }
    }

    /// Returns an empty set for values the query's pipelines emit.
    pub(super) fn out(&self, grant: &Arc<Grant>) -> Result<Distinct, Error> {
        Distinct::stopping(grant, self.stopped.clone())
    }

    pub(super) fn stopped(&self) -> bool {
        self.stopped
            .as_ref()
            .is_some_and(|s| s.load(Ordering::Acquire))
    }

    /// Merge what a task emitted into the query's values.
    fn emit<R: Runtime>(&self, rt: &R, sp: &Spill<R>, out: Distinct) -> Result<(), Error> {
        let mut slot = self.emitted.lock(rt);
        match &mut *slot {
            None => *slot = Some(out),
            Some(d) => d.absorb(out, sp)?,
        }
        Ok(())
    }

    fn fail<R: Runtime>(&self, rt: &R, e: Error) {
        self.failed.store(true, Ordering::Release);
        self.error.lock(rt).get_or_insert(e);
    }
}

fn faulted(e: Error, row: u32, catch: &mut Catch) -> Result<(), Error> {
    match e {
        Error::Fault(f) => catch.fault(row, f),
        other => Err(other),
    }
}

impl<R: Runtime> Ctx<R> {
    pub(super) fn sh(&self) -> &super::code::Shared {
        &self.program.shared
    }

    /// Returns the value of the plan's root in `env`.
    pub fn root(self: &Arc<Self>, env: &Frame) -> Result<Val, Error> {
        let program = self.program.clone();
        self.eval_env(&program.root, env, true)
    }

    fn row(&self, heap: HeapId, fields: &[Value]) -> Val {
        let info = &self.program.heaps[&heap];
        Val::ctor(
            info.ctor,
            info.fields
                .iter()
                .zip(fields)
                .map(|(c, v)| from_stored(c, v))
                .collect(),
        )
    }

    fn batch_rows(&self, heap: HeapId, b: &Batch) -> Vec<Val> {
        let info = &self.program.heaps[&heap];
        (0..b.len())
            .map(|i| {
                Val::ctor(
                    info.ctor,
                    info.fields
                        .iter()
                        .enumerate()
                        .map(|(c, conv)| from_stored(conv, &b.value(c, i)))
                        .collect(),
                )
            })
            .collect()
    }

    pub(super) fn check_cancel(&self) -> Result<(), Error> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        match &self.deadline {
            Some(d) if self.rt.now() >= d.at => Err(d.error.clone()),
            _ => Ok(()),
        }
    }

    pub(super) fn eval_env(
        self: &Arc<Self>,
        e: &Eval,
        env: &Frame,
        par: bool,
    ) -> Result<Val, Error> {
        let mut f = env.clone();
        let mut sel = vec![0];
        let mut v = self.eval_rows(e, &mut f, &mut sel, &mut Catch::off(), par)?;
        Ok(v.pop().expect("an evaluation with no catch has a value").1)
    }

    /// Evaluate `codes` in turn at the rows of `sel`. Returns each remaining row's values.
    pub(super) fn eval_keys(
        &self,
        codes: &[Expr],
        f: &mut Frame,
        sel: &mut Sel,
        catch: &mut Catch,
    ) -> Result<HashMap<u32, Vec<Val>>, Error> {
        let mut out: HashMap<u32, Vec<Val>> = sel.iter().map(|r| (*r, Vec::new())).collect();
        for c in codes {
            for (r, v) in c.eval(f, sel, catch, self.sh())? {
                out.get_mut(&r).expect("a row of the selection").push(v);
            }
        }
        out.retain(|_, vs| vs.len() == codes.len());
        Ok(out)
    }

    /// Evaluate `e` at the rows of `sel`, removing rows that fault when `catch` is on. `par`
    /// lets a query it evaluates run on the scheduler.
    pub(super) fn eval_rows(
        self: &Arc<Self>,
        e: &Eval,
        f: &mut Frame,
        sel: &mut Sel,
        catch: &mut Catch,
        par: bool,
    ) -> Result<Vec<(u32, Val)>, Error> {
        if sel.is_empty() {
            return Ok(Vec::new());
        }
        match e {
            Eval::Expr(c) => c.eval(f, sel, catch, self.sh()),
            Eval::Let(x, a, b) => {
                let v = self.eval_rows(a, f, sel, catch, par)?;
                f.bind(*x, v);
                self.eval_rows(b, f, sel, catch, par)
            }
            Eval::If(c, a, b) => {
                let vs = c.eval(f, sel, catch, self.sh())?;
                let (mut t, mut e): (Sel, Sel) = (Vec::new(), Vec::new());
                for (r, v) in vs {
                    if v.truth()? { t.push(r) } else { e.push(r) }
                }
                let since = catch.faults.len();
                let mut out = self.eval_rows(a, f, &mut t, catch, par)?;
                out.extend(self.eval_rows(b, f, &mut e, catch, par)?);
                catch.drop_faulted(since, sel);
                Ok(out)
            }
            Eval::Case(s, arms) => {
                let vs = s.eval(f, sel, catch, self.sh())?;
                let mut by: BTreeMap<u32, Vec<(u32, Val)>> = BTreeMap::new();
                for (r, v) in vs {
                    by.entry(v.ctor_of()?).or_default().push((r, v));
                }
                let since = catch.faults.len();
                let mut out = Vec::new();
                for (c, rows) in by {
                    let (var, arm) = super::arm(arms, c)?;
                    let mut s: Sel = rows.iter().map(|(r, _)| *r).collect();
                    f.bind(*var, rows);
                    out.extend(self.eval_rows(arm, f, &mut s, catch, par)?);
                }
                catch.drop_faulted(since, sel);
                Ok(out)
            }
            Eval::Try(a) => {
                let mut inner = Catch::on();
                let mut s = sel.clone();
                let mut out: Vec<(u32, Val)> = self
                    .eval_rows(a, f, &mut s, &mut inner, par)?
                    .into_iter()
                    .map(|(r, v)| (r, Val::ctor(0, vec![v])))
                    .collect();
                out.extend(
                    inner
                        .faults
                        .iter()
                        .map(|(r, x)| (*r, Val::ctor(1, vec![self.sh().fault_value(*x)]))),
                );
                Ok(out)
            }
            Eval::Lookup(l) => {
                let keys = self.eval_keys(&l.key, f, sel, catch)?;
                let since = catch.faults.len();
                let mut out = Vec::new();
                for &r in sel.iter() {
                    match self.lookup(l, &keys[&r]) {
                        Ok(v) => out.push((r, v)),
                        Err(e) => faulted(e, r, catch)?,
                    }
                }
                catch.drop_faulted(since, sel);
                Ok(out)
            }
            Eval::Query(q) => {
                let since = catch.faults.len();
                let mut out = Vec::new();
                for &r in sel.iter() {
                    match self.query(q, f.row(r), par && f.len() == 1) {
                        Ok(v) => out.push((r, v)),
                        Err(e) => faulted(e, r, catch)?,
                    }
                }
                catch.drop_faulted(since, sel);
                Ok(out)
            }
            Eval::GroupRows(groups, key) => {
                let ks: HashMap<u32, Val> =
                    key.eval(f, sel, catch, self.sh())?.into_iter().collect();
                let mut out = Vec::new();
                for (r, g) in groups.eval(f, sel, catch, self.sh())? {
                    let Some(k) = ks.get(&r) else { continue };
                    out.push((r, group_rows(&g, k)?));
                }
                Ok(out)
            }
            Eval::Fix(fx) => {
                let since = catch.faults.len();
                let mut out = Vec::new();
                for &r in sel.iter() {
                    match self.fix(fx, &f.row(r), par && f.len() == 1) {
                        Ok(v) => out.push((r, v)),
                        Err(e) => faulted(e, r, catch)?,
                    }
                }
                catch.drop_faulted(since, sel);
                Ok(out)
            }
            Eval::At(a) => {
                let points = a.point.eval(f, sel, catch, self.sh())?;
                let since = catch.faults.len();
                let mut out = Vec::new();
                for (r, at) in points {
                    match self.at(a, &at, &f.row(r)) {
                        Ok(v) => out.push((r, v)),
                        Err(e) => faulted(e, r, catch)?,
                    }
                }
                catch.drop_faulted(since, sel);
                Ok(out)
            }
        }
    }

    /// Returns the value of `a`'s body in `env` at the state `point` names, read on the calling
    /// thread.
    fn at(self: &Arc<Self>, a: &At, point: &Val, env: &Frame) -> Result<Val, Error> {
        let Some(past) = &self.store.past else {
            return Err(Error::Internal(
                "a read of the past holds no other read of the past".into(),
            ));
        };
        let store = past.store(point, a.instant, a.pins, &a.heaps)?;
        let cx = Arc::new(Ctx {
            rt: self.rt.clone(),
            program: self.program.clone(),
            store,
            grant: self.grant.clone(),
            options: self.options,
            pool: None,
            cancel: self.cancel.clone(),
            deadline: self.deadline.clone(),
            spill: Spill::new(
                self.rt.clone(),
                self.grant.clone(),
                self.options.spill_bytes,
            ),
        });
        cx.eval_env(&a.body, env, false)
    }

    /// Returns the row whose key index entry has key `key`, from the heap of its constructor.
    fn lookup(&self, l: &Lookup, key: &[Val]) -> Result<Val, Error> {
        let types = self.store.key_types(l.index);
        let k: Vec<Value> = types
            .iter()
            .zip(key)
            .map(|(t, v)| to_key(t, v))
            .collect::<Result<_, _>>()?;
        let routes: BTreeSet<u32> = l.routes.iter().map(|r| r.ctor).collect();
        for (c, tid) in self.store.probe(l.index, &k, &routes)? {
            let route = l.routes.iter().find(|r| r.ctor == c).expect("a route");
            if let Some((_, v)) = self.fetch(route, tid, key)? {
                return Ok(v);
            }
        }
        let heaps: Vec<(u32, HeapId)> = l.routes.iter().map(|r| (r.ctor, r.heap)).collect();
        if let Some((heap, row)) = self.store.fetch_past(l.index, &k, &heaps)? {
            return Ok(self.row(heap, &row));
        }
        Err(Error::Fault(Fault::NoSuchRow))
    }

    /// Returns the visible version the chain from `tid` reaches in `route`'s heap, with its
    /// TID, when its key fields still equal `key`. The plan rechecks every other key element.
    fn fetch(&self, route: &Fetch, tid: Tid, key: &[Val]) -> Result<Option<(Tid, Val)>, Error> {
        let Some((at, row)) = self.store.fetch(route.heap, tid)? else {
            return Ok(None);
        };
        let v = self.row(route.heap, &row);
        let fields = v.fields()?;
        let same = key
            .iter()
            .zip(&route.key_at)
            .all(|(k, i)| i.is_none_or(|i| &fields[i] == k));
        Ok(same.then_some((at, v)))
    }

    fn fix(self: &Arc<Self>, fx: &Fix, env: &Frame, par: bool) -> Result<Val, Error> {
        let n = fx.binds.len();
        let with = |cur: &[Val], deltas: &[Val]| {
            let mut e = env.clone();
            for (i, b) in fx.binds.iter().enumerate() {
                e.bind_one(b.var, cur[i].clone());
                e.bind_one(b.delta, deltas[i].clone());
            }
            e
        };
        let limit = self.options.fix_rounds;
        let mut held = self.grant.arena();
        let mut hold = |sets: &[Val]| -> Result<(), Error> {
            let bytes: usize = sets.iter().map(Val::size).sum();
            let more = bytes.saturating_sub(held.bytes());
            held.grow(more)
        };
        let mut cur: Vec<Val> = vec![Val::empty_set(); n];
        let mut rounds = 0;
        if fx.binds.iter().all(|b| b.step.is_some()) {
            let inner = with(&cur, &cur);
            let mut deltas = fx
                .binds
                .iter()
                .map(|b| self.eval_env(&b.body, &inner, par))
                .collect::<Result<Vec<_>, _>>()?;
            cur = deltas.clone();
            hold(&cur)?;
            while any_rows(&deltas)? {
                rounds += 1;
                if rounds > limit {
                    return Err(Error::StatementLimit(Exceeded::Rounds { limit }));
                }
                self.check_cancel()?;
                let inner = with(&cur, &deltas);
                let mut found = Vec::with_capacity(n);
                for b in &fx.binds {
                    let step = b.step.as_ref().expect("a semi-naive step");
                    found.push(self.eval_env(step, &inner, par)?);
                }
                for (i, s) in found.into_iter().enumerate() {
                    let old = cur[i].as_set()?;
                    let new: BTreeSet<Val> = s
                        .as_set()?
                        .iter()
                        .filter(|x| !old.contains(x))
                        .cloned()
                        .collect();
                    let mut all = old.clone();
                    all.extend(new.iter().cloned());
                    cur[i] = Val::set(all);
                    deltas[i] = Val::set(new);
                }
                hold(&cur)?;
            }
            return self.eval_env(&fx.body, &with(&cur, &cur), par);
        }
        loop {
            let inner = with(&cur, &cur);
            let next = fx
                .binds
                .iter()
                .map(|b| self.eval_env(&b.body, &inner, par))
                .collect::<Result<Vec<_>, _>>()?;
            if next == cur {
                return self.eval_env(&fx.body, &inner, par);
            }
            rounds += 1;
            if rounds > limit {
                return Err(Error::StatementLimit(Exceeded::Rounds { limit }));
            }
            self.check_cancel()?;
            hold(&next)?;
            cur = next;
        }
    }

    /// Collects into `out` the pipelines `s` selects, each with its environment.
    pub(super) fn spine(
        self: &Arc<Self>,
        s: &Spine,
        mut env: Frame,
        par: bool,
        out: &mut Vec<(Arc<Pipeline>, Frame)>,
    ) -> Result<(), Error> {
        match s {
            Spine::Pipeline(p) => out.push((p.clone(), env)),
            Spine::Union(ss) => {
                for s in ss {
                    self.spine(s, env.clone(), par, out)?;
                }
            }
            Spine::If(c, a, b) => {
                let v = c.eval_one(&mut env, self.sh())?;
                self.spine(if v.truth()? { a } else { b }, env, par, out)?;
            }
            Spine::Case(sc, arms) => {
                let v = sc.eval_one(&mut env, self.sh())?;
                let c = v.ctor_of()?;
                let (var, arm) = super::arm(arms, c)?;
                env.bind_one(*var, v);
                self.spine(arm, env, par, out)?;
            }
            Spine::Let(x, e, b) => {
                let v = self.eval_env(e, &env, par)?;
                env.bind_one(*x, v);
                self.spine(b, env, par, out)?;
            }
        }
        Ok(())
    }

    fn query(self: &Arc<Self>, q: &Arc<Query>, env: Frame, par: bool) -> Result<Val, Error> {
        let count = match &q.sink {
            Sink::Top { count, .. } => Some(self.eval_env(count, &env, par)?),
            _ => None,
        };
        if q.total
            && let Some(c) = &count
            && c.int()? <= 0
        {
            return self.sink(&q.sink, Distinct::new(&self.grant)?, &env, count);
        }
        let mut chosen = Vec::new();
        self.spine(&q.spine, env.clone(), par, &mut chosen)?;
        let run = Arc::new(Run::new(q.clone(), env.clone()));
        match &self.pool {
            Some(pool) if par => self.parallel(pool, &run, chosen)?,
            _ => {
                let mut out = run.out(&self.grant)?;
                for (p, e) in &chosen {
                    self.pipeline(&run, p, e, &mut out)?;
                }
                run.emit(&self.rt, &self.spill, out)?;
            }
        }
        let mut set = run.out(&self.grant)?;
        self.drain(&run, &mut set)?;
        for b in &run.builds {
            b.lock(&self.rt).take();
        }
        if let Some(o) = run.emitted.lock(&self.rt).take() {
            set.absorb(o, &self.spill)?;
        }
        self.sink(&q.sink, set, &env, count)
    }

    fn parallel(
        self: &Arc<Self>,
        pool: &Arc<Scheduler<R>>,
        run: &Arc<Run>,
        chosen: Vec<(Arc<Pipeline>, Frame)>,
    ) -> Result<(), Error> {
        let mut small = Vec::new();
        for (p, env) in chosen {
            let Source::Scan { heap, .. } = &p.source else {
                small.push((p, env));
                continue;
            };
            let pages = self.store.pages(*heap)?;
            if pages.len() <= self.options.pack_pages {
                small.push((p, env));
                continue;
            }
            for range in pages.chunks(self.options.range_pages.max(1)) {
                let (p, env, range) = (p.clone(), env.clone(), range.to_vec());
                self.spawn(pool, run, move |cx, run| {
                    cx.range_task(run, &p, &env, &range)
                });
            }
        }
        if !small.is_empty() {
            self.spawn(pool, run, move |cx, run| {
                let mut out = run.out(&cx.grant)?;
                for (p, env) in &small {
                    cx.pipeline(run, p, env, &mut out)?;
                }
                run.emit(&cx.rt, &cx.spill, out)?;
                Ok(())
            });
        }
        while run.pending.load(Ordering::Acquire) > 0 {
            if !pool.help() {
                self.rt.yield_now();
            }
        }
        match run.error.lock(&self.rt).take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Submit `task` as work of `run`, skipped once the run has failed.
    fn spawn(
        self: &Arc<Self>,
        pool: &Arc<Scheduler<R>>,
        run: &Arc<Run>,
        task: impl FnOnce(&Arc<Self>, &Arc<Run>) -> Result<(), Error> + Send + 'static,
    ) {
        run.pending.fetch_add(1, Ordering::AcqRel);
        let (cx, run) = (self.clone(), run.clone());
        pool.submit(Box::new(move || {
            if !run.failed.load(Ordering::Acquire) && !run.stopped() {
                let r = cx.check_cancel().and_then(|()| task(&cx, &run));
                if let Err(e) = r {
                    run.fail(&cx.rt, e);
                }
            }
            run.pending.fetch_sub(1, Ordering::AcqRel);
        }));
    }

    /// Run `p` over the rows on `pages`, a range of its heap's pages.
    fn range_task(
        self: &Arc<Self>,
        run: &Arc<Run>,
        p: &Arc<Pipeline>,
        env: &Frame,
        pages: &[Pid],
    ) -> Result<(), Error> {
        let Source::Scan { heap, .. } = &p.source else {
            unreachable!("a range task scans")
        };
        let mut out = run.out(&self.grant)?;
        self.store
            .scan_pages(*heap, pages, self.options.morsel_rows, |b| {
                self.morsel(run, p, env, &b, &mut out)
            })?;
        run.emit(&self.rt, &self.spill, out)
    }

    pub(super) fn pipeline(
        self: &Arc<Self>,
        run: &Run,
        p: &Pipeline,
        env: &Frame,
        out: &mut Distinct,
    ) -> Result<(), Error> {
        if run.stopped() {
            return Ok(());
        }
        match &p.source {
            Source::Unit => {
                self.check_cancel()?;
                self.chain(run, &p.chain, 0, env.clone(), vec![0], out)
            }
            Source::Scan { heap, .. } => self.store.scan(*heap, self.options.morsel_rows, |b| {
                self.morsel(run, p, env, &b, out)
            }),
        }
    }

    pub(super) fn morsel(
        self: &Arc<Self>,
        run: &Run,
        p: &Pipeline,
        env: &Frame,
        b: &Batch,
        out: &mut Distinct,
    ) -> Result<ControlFlow<()>, Error> {
        self.check_cancel()?;
        if run.stopped() {
            return Ok(ControlFlow::Break(()));
        }
        let Source::Scan { heap, var } = &p.source else {
            unreachable!("a morsel is of a scan")
        };
        let rows = self.batch_rows(*heap, b);
        let _held = self.grant.take(rows.iter().map(Val::size).sum())?;
        let mut f = env.widen(rows.len());
        f.bind_all(*var, rows);
        let sel = f.all();
        self.chain(run, &p.chain, 0, f, sel, out)?;
        Ok(if run.stopped() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        })
    }

    /// Pass the rows of `sel` through the steps of `c` from step `from` on, then its end.
    pub(super) fn chain(
        self: &Arc<Self>,
        run: &Run,
        c: &Arc<Chain>,
        from: usize,
        mut f: Frame,
        mut sel: Sel,
        out: &mut Distinct,
    ) -> Result<(), Error> {
        let off = &mut Catch::off();
        for (i, step) in c.steps.iter().enumerate().skip(from) {
            if sel.is_empty() {
                return Ok(());
            }
            match step {
                Step::Filter(code) => {
                    sel = holding(code.eval(&mut f, &mut sel, off, self.sh())?)?;
                }
                Step::Bind(x, e) => {
                    let vs = self.eval_rows(e, &mut f, &mut sel, off, false)?;
                    f.bind(*x, vs);
                }
                Step::Unnest(x, s) => {
                    let sets = s.eval(&mut f, &mut sel, off, self.sh())?;
                    let mut rows = Expand::new(&f, *x, self.options.morsel_rows);
                    for (r, v) in sets {
                        for e in v.as_set()? {
                            if let Some((g, s)) = rows.push(r, e.clone()) {
                                self.chain(run, c, i + 1, g, s, out)?;
                            }
                        }
                    }
                    if let Some((g, s)) = rows.rest() {
                        self.chain(run, c, i + 1, g, s, out)?;
                    }
                    return Ok(());
                }
                Step::Probe { build, key, var } => {
                    let table = self.table(run, *build)?;
                    let keys = self.eval_keys(key, &mut f, &mut sel, off)?;
                    let input = Probed {
                        f: &f,
                        sel: &sel,
                        keys: &keys,
                        var: *var,
                    };
                    return self.probe(run, c, i, &table, &input, out);
                }
            }
        }
        self.end(run, &c.end, f, sel, out)
    }

    fn end(
        self: &Arc<Self>,
        run: &Run,
        end: &End,
        mut f: Frame,
        mut sel: Sel,
        out: &mut Distinct,
    ) -> Result<(), Error> {
        if sel.is_empty() {
            return Ok(());
        }
        let off = &mut Catch::off();
        match end {
            End::Nothing => Ok(()),
            End::Emit(t) => {
                for (_, v) in t.eval(&mut f, &mut sel, off, self.sh())? {
                    out.insert(v, &self.spill)?;
                }
                Ok(())
            }
            End::Fork(cs) => {
                for c in cs {
                    self.chain(run, c, 0, f.clone(), sel.clone(), out)?;
                }
                Ok(())
            }
            End::If(c, a, b) => {
                let (mut t, mut e) = (Vec::new(), Vec::new());
                for (r, v) in c.eval(&mut f, &mut sel, off, self.sh())? {
                    if v.truth()? { t.push(r) } else { e.push(r) }
                }
                self.chain(run, a, 0, f.clone(), t, out)?;
                self.chain(run, b, 0, f, e, out)
            }
            End::Case(s, arms) => {
                let mut by: BTreeMap<u32, Vec<(u32, Val)>> = BTreeMap::new();
                for (r, v) in s.eval(&mut f, &mut sel, off, self.sh())? {
                    by.entry(v.ctor_of()?).or_default().push((r, v));
                }
                for (c, rows) in by {
                    let (var, arm) = super::arm(arms, c)?;
                    let s: Sel = rows.iter().map(|(r, _)| *r).collect();
                    let mut g = f.clone();
                    g.bind(*var, rows);
                    self.chain(run, arm, 0, g, s, out)?;
                }
                Ok(())
            }
            End::Index {
                index,
                key,
                var,
                routes,
            } => {
                let keys = self.eval_keys(key, &mut f, &mut sel, off)?;
                let ctors: BTreeSet<u32> = routes.iter().map(|(r, _)| r.ctor).collect();
                let found = |_: u32, prefix: &[Value]| self.store.probe(*index, prefix, &ctors);
                let at = IndexEnd {
                    index: *index,
                    keys: &keys,
                    var: *var,
                    routes,
                };
                self.routed(run, &at, &f, &sel, found, out)
            }
            End::Range {
                index,
                key,
                lo,
                hi,
                var,
                routes,
            } => {
                let keys = self.eval_keys(key, &mut f, &mut sel, off)?;
                let types = self.store.key_types(*index);
                let next = &types[key.len()];
                let mut bound =
                    |b: &Option<(Expr, bool)>| -> Result<HashMap<u32, (Value, bool)>, Error> {
                        let Some((e, inclusive)) = b else {
                            return Ok(HashMap::new());
                        };
                        let mut out = HashMap::new();
                        for (r, v) in e.eval(&mut f, &mut sel, off, self.sh())? {
                            out.insert(r, (to_key(next, &v)?, *inclusive));
                        }
                        Ok(out)
                    };
                let (los, his) = (bound(lo)?, bound(hi)?);
                let ctors: BTreeSet<u32> = routes.iter().map(|(r, _)| r.ctor).collect();
                let found = |r: u32, prefix: &[Value]| {
                    let (l, h) = (los.get(&r).cloned(), his.get(&r).cloned());
                    self.store.range(*index, prefix, l, h, &ctors)
                };
                let at = IndexEnd {
                    index: *index,
                    keys: &keys,
                    var: *var,
                    routes,
                };
                self.routed(run, &at, &f, &sel, found, out)
            }
        }
    }

    /// Fetch the rows of the entries `found` returns for each row of `sel` and its key prefix,
    /// and pass each to the chain of its constructor's route, a morsel at a time.
    fn routed(
        self: &Arc<Self>,
        run: &Run,
        at: &IndexEnd,
        f: &Frame,
        sel: &Sel,
        mut found: impl FnMut(u32, &[Value]) -> Result<Vec<(u32, Tid)>, Error>,
        out: &mut Distinct,
    ) -> Result<(), Error> {
        let types = self.store.key_types(at.index);
        let mut hits: Vec<Expand<'_>> = at
            .routes
            .iter()
            .map(|_| Expand::new(f, at.var, self.options.morsel_rows))
            .collect();
        for &r in sel {
            let k = &at.keys[&r];
            let prefix: Vec<Value> = types
                .iter()
                .zip(k)
                .map(|(t, v)| to_key(t, v))
                .collect::<Result<_, _>>()?;
            let mut seen = BTreeSet::new();
            for (c, tid) in found(r, &prefix)? {
                let i = at
                    .routes
                    .iter()
                    .position(|(rt, _)| rt.ctor == c)
                    .expect("a route of the entry's constructor");
                let Some((t, v)) = self.fetch(&at.routes[i].0, tid, k)? else {
                    continue;
                };
                if seen.insert(t)
                    && let Some((g, s)) = hits[i].push(r, v)
                {
                    self.chain(run, &at.routes[i].1, 0, g, s, out)?;
                }
            }
        }
        for (rows, (_, chain)) in hits.into_iter().zip(at.routes) {
            if let Some((g, s)) = rows.rest() {
                self.chain(run, chain, 0, g, s, out)?;
            }
        }
        Ok(())
    }
}

/// An index end's probe: its index, each row's key values, its variable and its routes.
struct IndexEnd<'a> {
    index: IndexId,
    keys: &'a HashMap<u32, Vec<Val>>,
    var: u32,
    routes: &'a [(Fetch, Arc<Chain>)],
}

/// Returns the `rows` of the group of `groups`, a set of `{ key, rows }` records, whose key is
/// `key`, or the empty set when none is.
fn group_rows(groups: &Val, key: &Val) -> Result<Val, Error> {
    let first = Val::record(vec![key.clone(), Val::empty_set()]);
    match groups.as_set()?.range(first..).next() {
        Some(Val::Record(fs)) if fs.len() == 2 => Ok(match &fs[0] == key {
            true => fs[1].clone(),
            false => Val::empty_set(),
        }),
        Some(other) => Err(super::value::mistyped("a group record", other)),
        None => Ok(Val::empty_set()),
    }
}

/// Rows expanded from the rows of a frame, each bound to a value of `var`, cut into frames of
/// at most `per` rows.
pub(super) struct Expand<'a> {
    f: &'a Frame,
    var: u32,
    per: usize,
    from: Vec<u32>,
    elems: Vec<Val>,
}

impl<'a> Expand<'a> {
    pub(super) fn new(f: &'a Frame, var: u32, per: usize) -> Expand<'a> {
        Expand {
            f,
            var,
            per: per.max(1),
            from: Vec::new(),
            elems: Vec::new(),
        }
    }

    /// Add a copy of row `r` with `var` bound to `v`. Returns the frame of the rows added so far
    /// once they fill one.
    pub(super) fn push(&mut self, r: u32, v: Val) -> Option<(Frame, Sel)> {
        self.from.push(r);
        self.elems.push(v);
        (self.from.len() >= self.per).then(|| self.take())
    }

    fn take(&mut self) -> (Frame, Sel) {
        let mut g = self.f.gather(&std::mem::take(&mut self.from));
        g.bind_all(self.var, std::mem::take(&mut self.elems));
        let s = g.all();
        (g, s)
    }

    /// Returns the frame of the rows added since the last one returned, if any.
    pub(super) fn rest(mut self) -> Option<(Frame, Sel)> {
        (!self.from.is_empty()).then(|| self.take())
    }
}

/// Returns whether any of `sets` holds a row.
fn any_rows(sets: &[Val]) -> Result<bool, Error> {
    for s in sets {
        if !s.as_set()?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Returns the rows of `vs` whose value is `True`.
pub(super) fn holding(vs: Vec<(u32, Val)>) -> Result<Sel, Error> {
    let mut sel = Vec::new();
    for (r, v) in vs {
        if v.truth()? {
            sel.push(r);
        }
    }
    Ok(sel)
}
