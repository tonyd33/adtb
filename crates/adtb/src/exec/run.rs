use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use adtb_sys::lock::Lock;

use crate::batch::Batch;
use crate::key::{Tid, Value};
use crate::plan::HeapId;
use crate::runtime::Runtime;

use super::code::Catch;
use super::frame::{Frame, Sel};
use super::ir::{
    Build, Chain, End, Eval, Expr, Fetch, Fix, Fold, GroupAgg, Lookup, Pipeline, Program, Query,
    Sink, Source, Spine, Step,
};
use super::memory::{Arena, Grant};
use super::sched::Scheduler;
use super::sink::{aggregate, cmp_keys, sorted};
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
}

/// A build's hash table: the elements of each key.
struct Table {
    rows: HashMap<Vec<Val>, Vec<Val>>,
    _arena: Arena,
}

/// The values a task emits, held until it merges them into its query's.
struct Out {
    set: BTreeSet<Val>,
    arena: Arena,
}

impl Out {
    fn new(grant: &Arc<Grant>) -> Result<Out, Error> {
        Ok(Out {
            set: BTreeSet::new(),
            arena: grant.take(0)?,
        })
    }

    fn insert(&mut self, v: Val) -> Result<(), Error> {
        let n = v.size();
        if self.set.insert(v) {
            self.arena.grow(n)?;
        }
        Ok(())
    }
}

/// One evaluation of a query.
struct Run {
    query: Arc<Query>,
    env: Frame,
    builds: Vec<Lock<Option<Arc<Table>>>>,
    emitted: Lock<Vec<Out>>,
    error: Lock<Option<Error>>,
    failed: AtomicBool,
    pending: AtomicUsize,
}

impl Run {
    fn new(query: Arc<Query>, env: Frame) -> Run {
        let builds = query.builds.iter().map(|_| Lock::new(None)).collect();
        Run {
            query,
            env,
            builds,
            emitted: Lock::new(Vec::new()),
            error: Lock::new(None),
            failed: AtomicBool::new(false),
            pending: AtomicUsize::new(0),
        }
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
    fn sh(&self) -> &super::code::Shared {
        &self.program.shared
    }

    /// Returns the value of the plan's root with `params` bound.
    pub fn root(self: &Arc<Self>, params: &[(u32, Val)]) -> Result<Val, Error> {
        let mut env = Frame::unit(self.program.vars);
        for (v, x) in params {
            env.bind_one(*v, x.clone());
        }
        let program = self.program.clone();
        self.eval_env(&program.root, &env, true)
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

    fn check_cancel(&self) -> Result<(), Error> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    fn eval_env(self: &Arc<Self>, e: &Eval, env: &Frame, par: bool) -> Result<Val, Error> {
        let mut f = env.clone();
        let mut sel = vec![0];
        let mut v = self.eval_rows(e, &mut f, &mut sel, &mut Catch::off(), par)?;
        Ok(v.pop().expect("an evaluation with no catch has a value").1)
    }

    /// Evaluate `codes` in turn at the rows of `sel`. Returns each remaining row's values.
    fn eval_keys(
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
    fn eval_rows(
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
        }
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
        let mut held = self.grant.take(0)?;
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
    fn spine(
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
        let mut chosen = Vec::new();
        self.spine(&q.spine, env.clone(), par, &mut chosen)?;
        let run = Arc::new(Run::new(q.clone(), env.clone()));
        match &self.pool {
            Some(pool) if par => self.parallel(pool, &run, chosen)?,
            _ => {
                let mut out = Out::new(&self.grant)?;
                for (p, e) in &chosen {
                    self.pipeline(&run, p, e, &mut out)?;
                }
                run.emitted.lock(&self.rt).push(out);
            }
        }
        let outs = std::mem::take(&mut *run.emitted.lock(&self.rt));
        let mut arenas = Vec::with_capacity(outs.len());
        let mut set = BTreeSet::new();
        for o in outs {
            if set.is_empty() {
                set = o.set;
            } else {
                set.extend(o.set);
            }
            arenas.push(o.arena);
        }
        let v = self.sink(&q.sink, set, &env, count);
        drop(arenas);
        v
    }

    fn sink(
        self: &Arc<Self>,
        sink: &Sink,
        set: BTreeSet<Val>,
        env: &Frame,
        count: Option<Val>,
    ) -> Result<Val, Error> {
        let elems = |set: &BTreeSet<Val>, var: u32| {
            let mut f = env.widen(set.len());
            f.bind_all(var, set.iter().cloned().collect());
            f
        };
        Ok(match sink {
            Sink::Set => Val::set(set),
            Sink::Exists => Val::bool(!set.is_empty()),
            Sink::IsEmpty => Val::bool(set.is_empty()),
            Sink::Agg { op, var, body } => {
                let mut f = elems(&set, *var);
                let mut sel_f = f.all();
                let ms = self.eval_rows(body, &mut f, &mut sel_f, &mut Catch::off(), false)?;
                aggregate(*op, ms.into_iter().map(|(_, v)| v).collect())?
            }
            Sink::Fold { var, body, fold } => {
                let mut f = elems(&set, *var);
                let mut sel_f = f.all();
                let ms = self.eval_rows(body, &mut f, &mut sel_f, &mut Catch::off(), false)?;
                self.fold(fold, ms.into_iter().map(|(_, v)| v), env)?
            }
            Sink::Group {
                var,
                by,
                aggs,
                order,
                ..
            } => {
                let all: Vec<Val> = set.iter().cloned().collect();
                let mut f = elems(&set, *var);
                let mut sel_f = f.all();
                let keys = self.eval_rows(by, &mut f, &mut sel_f, &mut Catch::off(), false)?;
                let mut groups: BTreeMap<Val, BTreeSet<Val>> = BTreeMap::new();
                for (r, k) in keys {
                    groups.entry(k).or_default().insert(all[r as usize].clone());
                }
                let mut held = self.grant.take(0)?;
                let mut out = BTreeSet::new();
                for (k, members) in groups {
                    let computed = aggs
                        .iter()
                        .map(|a| self.group_agg(a, &members, env))
                        .collect::<Result<Vec<_>, _>>()?;
                    let mut members = Some(members);
                    let mut key = Some(k);
                    let fields = order
                        .iter()
                        .map(|o| match o {
                            None => key.take().expect("one key field"),
                            Some(None) => Val::set(members.take().expect("one rows field")),
                            Some(Some(i)) => computed[*i].clone(),
                        })
                        .collect();
                    let rec = Val::record(fields);
                    held.grow(rec.size())?;
                    out.insert(rec);
                }
                Val::set(out)
            }
            Sink::Top { keys, var, .. } => {
                let keyed = self.keyed(&set, keys, *var, env)?;
                let n = count.expect("a top's count").int()?;
                if n < 0 {
                    return Err(Error::Fault(Fault::NegativeCount));
                }
                let desc: Vec<bool> = keys.iter().map(|(_, d)| *d).collect();
                Val::set(
                    sorted(keyed, &desc)
                        .into_iter()
                        .take(n as usize)
                        .map(|(_, v)| v)
                        .collect(),
                )
            }
            Sink::Present {
                var,
                keys,
                take,
                after,
            } => {
                let keyed = self.keyed(&set, keys, *var, env)?;
                let take = take
                    .as_ref()
                    .map(|t| self.eval_env(t, env, false))
                    .transpose()?;
                let after = after
                    .as_ref()
                    .map(|t| self.eval_env(t, env, false))
                    .transpose()?;
                let desc: Vec<bool> = keys.iter().map(|(_, d)| *d).collect();
                let cursor: Option<Vec<Val>> = match after {
                    Some(Val::Ctor(1, fs)) if keys.len() == 1 => Some(vec![fs[0].clone()]),
                    Some(Val::Ctor(1, fs)) => Some(fs[0].fields()?.to_vec()),
                    _ => None,
                };
                let n = match take.as_ref().map(Val::int).transpose()? {
                    Some(t) if t < 0 => return Err(Error::Fault(Fault::NegativeCount)),
                    Some(t) => t as usize,
                    None => usize::MAX,
                };
                let listed: Vec<Val> = sorted(keyed, &desc)
                    .into_iter()
                    .filter(|(k, _)| {
                        cursor
                            .as_ref()
                            .is_none_or(|c| cmp_keys(c, k, &desc) == std::cmp::Ordering::Less)
                    })
                    .take(n)
                    .map(|(_, v)| v)
                    .collect();
                Val::List(Arc::from(listed))
            }
        })
    }

    /// Returns each element of `set` with the values of `keys` at it.
    fn keyed(
        self: &Arc<Self>,
        set: &BTreeSet<Val>,
        keys: &[(Eval, bool)],
        var: u32,
        env: &Frame,
    ) -> Result<Vec<(Vec<Val>, Val)>, Error> {
        let all: Vec<Val> = set.iter().cloned().collect();
        let mut f = env.widen(all.len());
        f.bind_all(var, all.clone());
        let mut ks: Vec<Vec<Val>> = vec![Vec::with_capacity(keys.len()); all.len()];
        for (k, _) in keys {
            let mut sel_f = f.all();
            for (r, v) in self.eval_rows(k, &mut f, &mut sel_f, &mut Catch::off(), false)? {
                ks[r as usize].push(v);
            }
        }
        Ok(ks.into_iter().zip(all).collect())
    }

    /// Returns `fold` over `ms`, folded in turn from its initial state.
    fn fold(
        &self,
        fold: &Fold,
        ms: impl ExactSizeIterator<Item = Val>,
        env: &Frame,
    ) -> Result<Val, Error> {
        if fold.optional && ms.len() == 0 {
            return Ok(Val::none());
        }
        let mut f = env.clone();
        let mut state = fold.init.eval_one(&mut f, self.sh())?;
        for m in ms {
            f.bind_one(fold.state, state);
            f.bind_one(fold.value, m);
            state = fold.step.eval_one(&mut f, self.sh())?;
        }
        f.bind_one(fold.state, state);
        let v = fold.finish.eval_one(&mut f, self.sh())?;
        Ok(if fold.optional { Val::some(v) } else { v })
    }

    fn group_agg(
        self: &Arc<Self>,
        a: &GroupAgg,
        members: &BTreeSet<Val>,
        env: &Frame,
    ) -> Result<Val, Error> {
        let mut f = env.widen(members.len());
        f.bind_all(a.row, members.iter().cloned().collect());
        let mut sel = f.all();
        for c in &a.filter {
            let vs = self.eval_rows(c, &mut f, &mut sel, &mut Catch::off(), false)?;
            sel = holding(vs)?;
        }
        let elems: BTreeSet<Val> = self
            .eval_rows(&a.element, &mut f, &mut sel, &mut Catch::off(), false)?
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        let mut g = env.widen(elems.len());
        g.bind_all(a.elem, elems.into_iter().collect());
        let mut sel_g = g.all();
        let ms = self.eval_rows(&a.body, &mut g, &mut sel_g, &mut Catch::off(), false)?;
        aggregate(a.op, ms.into_iter().map(|(_, v)| v).collect())
    }

    /// Returns the table of build `i` of `run`'s query, made on first use.
    fn table(self: &Arc<Self>, run: &Run, i: usize) -> Result<Arc<Table>, Error> {
        let mut slot = run.builds[i].lock(&self.rt);
        if let Some(t) = &*slot {
            return Ok(t.clone());
        }
        let build: &Build = &run.query.builds[i];
        let mut out = Out::new(&self.grant)?;
        for p in &build.pipelines {
            self.pipeline(run, p, &run.env, &mut out)?;
        }
        let elems: Vec<Val> = out.set.into_iter().collect();
        let mut f = run.env.widen(elems.len());
        f.bind_all(build.var, elems.clone());
        let mut sel_f = f.all();
        let keys = self.eval_keys(&build.key, &mut f, &mut sel_f, &mut Catch::off())?;
        let mut rows: HashMap<Vec<Val>, Vec<Val>> = HashMap::new();
        let mut arena = out.arena;
        for (r, e) in elems.into_iter().enumerate() {
            let k = keys[&(r as u32)].clone();
            arena.grow(k.iter().map(Val::size).sum())?;
            rows.entry(k).or_default().push(e);
        }
        let t = Arc::new(Table {
            rows,
            _arena: arena,
        });
        *slot = Some(t.clone());
        Ok(t)
    }

    fn parallel(
        self: &Arc<Self>,
        pool: &Arc<Scheduler<R>>,
        run: &Arc<Run>,
        chosen: Vec<(Arc<Pipeline>, Frame)>,
    ) -> Result<(), Error> {
        let mut small = Vec::new();
        for (p, env) in chosen {
            let pages = match &p.source {
                Source::Unit => 0,
                Source::Scan { heap, .. } => {
                    self.store.heaps[heap].heap.pages(&self.store.pager)?.len()
                }
            };
            if pages <= self.options.pack_pages {
                small.push((p, env));
            } else {
                self.spawn(pool, run, move |cx, run| cx.scan_task(run, &p, &env));
            }
        }
        if !small.is_empty() {
            self.spawn(pool, run, move |cx, run| {
                let mut out = Out::new(&cx.grant)?;
                for (p, env) in &small {
                    cx.pipeline(run, p, env, &mut out)?;
                }
                run.emitted.lock(&cx.rt).push(out);
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
            if !run.failed.load(Ordering::Acquire) {
                let r = cx.check_cancel().and_then(|()| task(&cx, &run));
                if let Err(e) = r {
                    run.fail(&cx.rt, e);
                }
            }
            run.pending.fetch_sub(1, Ordering::AcqRel);
        }));
    }

    /// Scan the heap of `p` and submit a task per morsel but the last, which runs in this one.
    fn scan_task(
        self: &Arc<Self>,
        run: &Arc<Run>,
        p: &Arc<Pipeline>,
        env: &Frame,
    ) -> Result<(), Error> {
        let Source::Scan { heap, .. } = &p.source else {
            unreachable!("a scan task scans")
        };
        let mut batches = self.store.scan(*heap, self.options.morsel_rows)?;
        let last = batches.pop();
        let pool = self
            .pool
            .clone()
            .expect("a scan task runs on the scheduler");
        for b in batches {
            let (p, env) = (p.clone(), env.clone());
            self.spawn(&pool, run, move |cx, run| {
                let mut out = Out::new(&cx.grant)?;
                cx.morsel(run, &p, &env, &b, &mut out)?;
                run.emitted.lock(&cx.rt).push(out);
                Ok(())
            });
        }
        if let Some(b) = last {
            let mut out = Out::new(&self.grant)?;
            self.morsel(run, p, env, &b, &mut out)?;
            run.emitted.lock(&self.rt).push(out);
        }
        Ok(())
    }

    fn pipeline(
        self: &Arc<Self>,
        run: &Run,
        p: &Pipeline,
        env: &Frame,
        out: &mut Out,
    ) -> Result<(), Error> {
        match &p.source {
            Source::Unit => {
                self.check_cancel()?;
                self.chain(run, &p.chain, env.clone(), vec![0], out)
            }
            Source::Scan { heap, .. } => {
                for b in self.store.scan(*heap, self.options.morsel_rows)? {
                    self.morsel(run, p, env, &b, out)?;
                }
                Ok(())
            }
        }
    }

    fn morsel(
        self: &Arc<Self>,
        run: &Run,
        p: &Pipeline,
        env: &Frame,
        b: &Batch,
        out: &mut Out,
    ) -> Result<(), Error> {
        self.check_cancel()?;
        let Source::Scan { heap, var } = &p.source else {
            unreachable!("a morsel is of a scan")
        };
        let rows = self.batch_rows(*heap, b);
        let _held = self.grant.take(rows.iter().map(Val::size).sum())?;
        let mut f = env.widen(rows.len());
        f.bind_all(*var, rows);
        let sel = f.all();
        self.chain(run, &p.chain, f, sel, out)
    }

    fn chain(
        self: &Arc<Self>,
        run: &Run,
        c: &Chain,
        mut f: Frame,
        mut sel: Sel,
        out: &mut Out,
    ) -> Result<(), Error> {
        let off = &mut Catch::off();
        for step in &c.steps {
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
                    let mut from = Vec::new();
                    let mut elems = Vec::new();
                    for (r, v) in s.eval(&mut f, &mut sel, off, self.sh())? {
                        for e in v.as_set()? {
                            from.push(r);
                            elems.push(e.clone());
                        }
                    }
                    f = f.gather(&from);
                    f.bind_all(*x, elems);
                    sel = f.all();
                }
                Step::Probe { build, key, var } => {
                    let table = self.table(run, *build)?;
                    let keys = self.eval_keys(key, &mut f, &mut sel, off)?;
                    let mut from = Vec::new();
                    let mut elems = Vec::new();
                    for &r in &sel {
                        let k = &keys[&r];
                        let hits: Box<dyn Iterator<Item = &Val>> = if k.is_empty() {
                            Box::new(table.rows.values().flatten())
                        } else {
                            Box::new(table.rows.get(k).into_iter().flatten())
                        };
                        for e in hits {
                            from.push(r);
                            elems.push(e.clone());
                        }
                    }
                    f = f.gather(&from);
                    f.bind_all(*var, elems);
                    sel = f.all();
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
        out: &mut Out,
    ) -> Result<(), Error> {
        if sel.is_empty() {
            return Ok(());
        }
        let off = &mut Catch::off();
        match end {
            End::Nothing => Ok(()),
            End::Emit(t) => {
                for (_, v) in t.eval(&mut f, &mut sel, off, self.sh())? {
                    out.insert(v)?;
                }
                Ok(())
            }
            End::Fork(cs) => {
                for c in cs {
                    self.chain(run, c, f.clone(), sel.clone(), out)?;
                }
                Ok(())
            }
            End::If(c, a, b) => {
                let (mut t, mut e) = (Vec::new(), Vec::new());
                for (r, v) in c.eval(&mut f, &mut sel, off, self.sh())? {
                    if v.truth()? { t.push(r) } else { e.push(r) }
                }
                self.chain(run, a, f.clone(), t, out)?;
                self.chain(run, b, f, e, out)
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
                    self.chain(run, arm, g, s, out)?;
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
                let types = self.store.key_types(*index);
                let ctors: BTreeSet<u32> = routes.iter().map(|(r, _)| r.ctor).collect();
                let mut hits: Vec<(Vec<u32>, Vec<Val>)> =
                    vec![(Vec::new(), Vec::new()); routes.len()];
                for &r in &sel {
                    let k = &keys[&r];
                    let prefix: Vec<Value> = types
                        .iter()
                        .zip(k)
                        .map(|(t, v)| to_key(t, v))
                        .collect::<Result<_, _>>()?;
                    let mut seen = BTreeSet::new();
                    for (c, tid) in self.store.probe(*index, &prefix, &ctors)? {
                        let i = routes
                            .iter()
                            .position(|(rt, _)| rt.ctor == c)
                            .expect("a route of the entry's constructor");
                        let Some((at, v)) = self.fetch(&routes[i].0, tid, k)? else {
                            continue;
                        };
                        if seen.insert(at) {
                            hits[i].0.push(r);
                            hits[i].1.push(v);
                        }
                    }
                }
                for ((from, rows), (_, chain)) in hits.into_iter().zip(routes) {
                    if from.is_empty() {
                        continue;
                    }
                    let mut g = f.gather(&from);
                    g.bind_all(*var, rows);
                    let s = g.all();
                    self.chain(run, chain, g, s, out)?;
                }
                Ok(())
            }
        }
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
fn holding(vs: Vec<(u32, Val)>) -> Result<Sel, Error> {
    let mut sel = Vec::new();
    for (r, v) in vs {
        if v.truth()? {
            sel.push(r);
        }
    }
    Ok(sel)
}
