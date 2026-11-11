use std::collections::BTreeSet;

use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::types::Ty;

use super::catalog::{Catalog, Element, IndexDesc, RelationDesc};
use super::terms::Facts;
use super::{
    Broken, Build, Chain, End, Eval, Fix, HeapId, IndexId, Pipeline, Plan, Query, Reads, Route,
    Sink, Source, Spine, Step,
};

/// Refuse `plan` when it breaks an invariant of every plan that a walk of its structure decides:
/// every variable bound where it is read, every case naming each constructor its scrutinee may
/// hold once, every scan and route reading its constructor's heap, every range probing an
/// ordered index on a field, every read listed, and every read of the past holding no index
/// probe and no other read of the past.
pub(super) fn validate(
    schema: &Schema,
    catalog: &Catalog,
    facts: &mut Facts,
    plan: &Plan,
) -> Result<(), Broken> {
    let mut bound = vec![0u32; plan.terms.vars.len()];
    for v in &plan.params {
        bound[v.0 as usize] += 1;
    }
    let mut w = Walk {
        schema,
        catalog,
        terms: &plan.terms,
        facts,
        bound,
        reads: &plan.reads,
        past: None,
        seen: Seen::default(),
    };
    let out = w.eval(&plan.root);
    #[cfg(any(test, feature = "testing"))]
    counts::add(&w.seen, out.is_err());
    out
}

/// Which checks a walk applied at least once.
#[derive(Clone, Copy, Debug, Default)]
struct Seen {
    bound: bool,
    arms: bool,
    scans: bool,
    routes: bool,
    ranges: bool,
    listed: bool,
    past: bool,
}

struct Walk<'a> {
    schema: &'a Schema,
    catalog: &'a Catalog,
    terms: &'a Terms,
    facts: &'a mut Facts,
    /// How many binders in scope bind each variable.
    bound: Vec<u32>,
    reads: &'a Reads,
    /// The reads of the read of the past being walked.
    past: Option<&'a Reads>,
    seen: Seen,
}

type R = Result<(), Broken>;

impl<'a> Walk<'a> {
    fn bind(&mut self, v: Var) {
        self.bound[v.0 as usize] += 1;
    }

    fn unbind(&mut self, v: Var) {
        self.bound[v.0 as usize] -= 1;
    }

    /// Run `f` with `vs` bound.
    fn under(&mut self, vs: &[Var], f: impl FnOnce(&mut Self) -> R) -> R {
        vs.iter().for_each(|v| self.bind(*v));
        let out = f(self);
        vs.iter().for_each(|v| self.unbind(*v));
        out
    }

    fn expr(&mut self, t: Tm) -> R {
        self.seen.bound = true;
        let free = self.facts.free(self.terms, t);
        match free.iter().find(|v| self.bound[v.0 as usize] == 0) {
            Some(v) => Err(Broken::Unbound(*v)),
            None => Ok(()),
        }
    }

    fn exprs(&mut self, ts: &[Tm]) -> R {
        ts.iter().try_for_each(|t| self.expr(*t))
    }

    fn eval(&mut self, e: &'a Eval) -> R {
        match e {
            Eval::Expr(t) => self.expr(*t),
            Eval::Let(x, a, b) => {
                self.eval(a)?;
                self.under(&[*x], |w| w.eval(b))
            }
            Eval::If(c, a, b) => {
                self.expr(*c)?;
                self.eval(a)?;
                self.eval(b)
            }
            Eval::Case(s, arms) => {
                self.case(*s, arms.iter().map(|(c, _, _)| *c))?;
                arms.iter()
                    .try_for_each(|(_, v, a)| self.under(&[*v], |w| w.eval(a)))
            }
            Eval::Try(a) => self.eval(a),
            Eval::Lookup(l) => {
                self.exprs(&l.key)?;
                let rel = self.key_index(l.index)?;
                let ctors = l.routes.iter().map(|(c, h)| (*c, *h));
                self.routes(l.index, rel, &|c| rel.heaps.contains_key(&c), ctors)
            }
            Eval::Query(q) => self.query(q),
            Eval::Fix(f) => self.fix(f),
            Eval::GroupRows { groups, key } => self.exprs(&[*groups, *key]),
            Eval::At(a) => {
                self.seen.past = true;
                if self.past.is_some() {
                    return Err(Broken::NestedPast);
                }
                self.expr(a.point)?;
                self.past = Some(&a.reads);
                let out = self.eval(&a.body);
                self.past = None;
                out
            }
        }
    }

    fn fix(&mut self, f: &'a Fix) -> R {
        let vars: Vec<Var> = f.binds.iter().flat_map(|b| [b.var, b.delta]).collect();
        self.under(&vars, |w| {
            for b in &f.binds {
                w.eval(&b.body)?;
                if let Some(s) = &b.step {
                    w.eval(s)?;
                }
            }
            w.eval(&f.body)
        })
    }

    fn query(&mut self, q: &'a Query) -> R {
        q.builds.iter().try_for_each(|b| self.build(b))?;
        self.spine(&q.spine)?;
        self.sink(&q.sink)
    }

    fn build(&mut self, b: &'a Build) -> R {
        b.pipelines.iter().try_for_each(|p| self.pipeline(p))?;
        self.under(&[b.var], |w| w.exprs(&b.key))
    }

    fn sink(&mut self, s: &'a Sink) -> R {
        match s {
            Sink::Set | Sink::Exists | Sink::IsEmpty => Ok(()),
            Sink::Agg { var, body, .. } => self.under(&[*var], |w| w.eval(body)),
            Sink::Fold { var, body, fold } => {
                self.under(&[*var], |w| w.eval(body))?;
                self.expr(fold.init)?;
                self.under(&[fold.state, fold.value], |w| w.expr(fold.step))?;
                self.under(&[fold.state], |w| w.expr(fold.finish))
            }
            Sink::Group { var, by, aggs, .. } => {
                self.under(&[*var], |w| w.eval(by))?;
                for a in aggs {
                    self.under(&[a.row], |w| {
                        a.filter.iter().try_for_each(|f| w.eval(f))?;
                        w.eval(&a.element)
                    })?;
                    self.under(&[a.elem], |w| w.eval(&a.body))?;
                }
                Ok(())
            }
            Sink::Top { count, keys, var } => {
                self.eval(count)?;
                self.under(&[*var], |w| keys.iter().try_for_each(|(k, _)| w.eval(k)))
            }
            Sink::Present {
                var,
                keys,
                take,
                after,
            } => {
                self.under(&[*var], |w| keys.iter().try_for_each(|(k, _)| w.eval(k)))?;
                take.iter().chain(after).try_for_each(|x| self.eval(x))
            }
        }
    }

    fn spine(&mut self, s: &'a Spine) -> R {
        match s {
            Spine::Pipeline(p) => self.pipeline(p),
            Spine::Union(ss) => ss.iter().try_for_each(|s| self.spine(s)),
            Spine::If(c, a, b) => {
                self.expr(*c)?;
                self.spine(a)?;
                self.spine(b)
            }
            Spine::Case(s, arms) => {
                self.case(*s, arms.iter().map(|(c, _, _)| *c))?;
                arms.iter()
                    .try_for_each(|(_, v, a)| self.under(&[*v], |w| w.spine(a)))
            }
            Spine::Let(x, e, b) => {
                self.eval(e)?;
                self.under(&[*x], |w| w.spine(b))
            }
        }
    }

    fn pipeline(&mut self, p: &'a Pipeline) -> R {
        match &p.source {
            Source::Unit => self.chain(&p.chain),
            Source::Scan { heap, ctor, var } => {
                self.seen.scans = true;
                match self.catalog.heap_of(*heap) {
                    Some((_, c)) if c == *ctor => {}
                    _ => return Err(Broken::ScanHeap(*heap)),
                }
                self.listed_heap(*heap)?;
                self.under(&[*var], |w| w.chain(&p.chain))
            }
        }
    }

    fn chain(&mut self, ch: &'a Chain) -> R {
        let mut bound = Vec::new();
        let out = self.steps(ch, &mut bound);
        bound.iter().for_each(|v| self.unbind(*v));
        out
    }

    /// Walk the steps and the end of `ch`, adding to `bound` each variable a step binds.
    fn steps(&mut self, ch: &'a Chain, bound: &mut Vec<Var>) -> R {
        for s in &ch.steps {
            let v = match s {
                Step::Filter(t) => {
                    self.expr(*t)?;
                    continue;
                }
                Step::Bind(x, e) => {
                    self.eval(e)?;
                    *x
                }
                Step::Unnest(x, t) => {
                    self.expr(*t)?;
                    *x
                }
                Step::Probe { key, var, .. } => {
                    self.exprs(key)?;
                    *var
                }
            };
            self.bind(v);
            bound.push(v);
        }
        self.end(&ch.end)
    }

    fn end(&mut self, e: &'a End) -> R {
        match e {
            End::Nothing => Ok(()),
            End::Emit(t) => self.expr(*t),
            End::Fork(cs) => cs.iter().try_for_each(|c| self.chain(c)),
            End::If(c, a, b) => {
                self.expr(*c)?;
                self.chain(a)?;
                self.chain(b)
            }
            End::Case(s, arms) => {
                self.case(*s, arms.iter().map(|(c, _, _)| *c))?;
                arms.iter()
                    .try_for_each(|(_, v, a)| self.under(&[*v], |w| w.chain(a)))
            }
            End::Index {
                index,
                key,
                var,
                routes,
            } => self.probe(*index, key, *var, routes, false),
            End::Range {
                index,
                key,
                lo,
                hi,
                var,
                routes,
            } => {
                for b in lo.iter().chain(hi) {
                    self.expr(b.value)?;
                }
                self.probe(*index, key, *var, routes, true)
            }
        }
    }

    /// Check a probe of `index` by the values `key` routed by `routes`, a range of the field
    /// after `key` when `range` is set.
    fn probe(
        &mut self,
        index: IndexId,
        key: &[Tm],
        var: Var,
        routes: &'a [Route],
        range: bool,
    ) -> R {
        if self.past.is_some() {
            return Err(Broken::PastIndex(index));
        }
        self.exprs(key)?;
        let (rel, desc) = self.index(index)?;
        if range {
            self.range(index, desc, key.len())?;
        }
        let covers = |c| desc.ctors.contains(&c);
        self.routes(index, rel, &covers, routes.iter().map(|r| (r.ctor, r.heap)))?;
        self.under(&[var], |w| {
            routes.iter().try_for_each(|r| w.chain(&r.chain))
        })
    }

    fn range(&mut self, index: IndexId, desc: &IndexDesc, prefix: usize) -> R {
        self.seen.ranges = true;
        if !desc.ordered {
            return Err(Broken::RangeNotOrdered(index));
        }
        match desc.elements.get(prefix) {
            Some(Element::Field(_)) => Ok(()),
            _ => Err(Broken::RangeNotField(index)),
        }
    }

    /// Check that the routes `ctors` of a probe of `index`, of relation `rel`, name distinct
    /// constructors the index `covers`, each with its heap, and that the reads list the index
    /// and the heap of every constructor of `rel`.
    fn routes(
        &mut self,
        index: IndexId,
        rel: &RelationDesc,
        covers: &dyn Fn(u32) -> bool,
        ctors: impl Iterator<Item = (u32, HeapId)>,
    ) -> R {
        self.seen.routes = true;
        let mut seen = Vec::new();
        for (ctor, heap) in ctors {
            if !covers(ctor) {
                return Err(Broken::RouteNotOfIndex { index, ctor });
            }
            if seen.contains(&ctor) {
                return Err(Broken::RouteTwice { index, ctor });
            }
            seen.push(ctor);
            if rel.heaps.get(&ctor) != Some(&heap) {
                return Err(Broken::RouteHeap { ctor, heap });
            }
        }
        self.listed_index(index)?;
        rel.heaps.values().try_for_each(|h| self.listed_heap(*h))
    }

    fn listed_heap(&mut self, h: HeapId) -> R {
        self.seen.listed = true;
        if !self.reads.heaps.contains(&h) {
            return Err(Broken::UnlistedHeap(h));
        }
        match self.past {
            Some(p) if !p.heaps.contains(&h) => Err(Broken::PastUnlistedHeap(h)),
            _ => Ok(()),
        }
    }

    fn listed_index(&mut self, i: IndexId) -> R {
        self.seen.listed = true;
        if !self.reads.indexes.contains(&i) {
            return Err(Broken::UnlistedIndex(i));
        }
        match self.past {
            Some(p) if !p.indexes.contains(&i) => Err(Broken::PastUnlistedIndex(i)),
            _ => Ok(()),
        }
    }

    /// Check that the constructors `arms` names are the constructors a value of `s`'s type may
    /// hold, each once.
    fn case(&mut self, s: Tm, arms: impl Iterator<Item = u32>) -> R {
        self.expr(s)?;
        self.seen.arms = true;
        let ty = self.terms.ty(s);
        let Some((n, may)) = case_ctors(self.schema, ty) else {
            return Err(Broken::CaseNotSum(ty.clone()));
        };
        let mut named = BTreeSet::new();
        for c in arms {
            if c as usize >= n {
                return Err(Broken::ArmNotCtor(c));
            }
            if !named.insert(c) {
                return Err(Broken::ArmTwice(c));
            }
        }
        match may.into_iter().find(|c| !named.contains(c)) {
            Some(c) => Err(Broken::NoArm(c)),
            None => Ok(()),
        }
    }

    /// Returns the relation of index `i` and the index.
    fn index(&self, i: IndexId) -> Result<(&'a RelationDesc, &'a IndexDesc), Broken> {
        let (_, rel) = self.catalog.index_of(i).ok_or(Broken::UnknownIndex(i))?;
        let desc = (rel.indexes.iter().find(|x| x.id == i)).ok_or(Broken::UnknownIndex(i))?;
        Ok((rel, desc))
    }

    /// Returns the relation whose key index `i` is.
    fn key_index(&self, i: IndexId) -> Result<&'a RelationDesc, Broken> {
        match self.catalog.index_of(i) {
            Some((_, rel)) if rel.key_index == i => Ok(rel),
            _ => Err(Broken::LookupNotKey(i)),
        }
    }
}

/// Returns how many constructors a value of `ty` is built from, as the executor numbers them,
/// and those it may hold; `None` for a type with no constructors.
fn case_ctors(schema: &Schema, ty: &Ty) -> Option<(usize, Vec<u32>)> {
    match ty {
        Ty::Data { def, args, ctors } => match &schema.ty(*def).kind {
            TypeDefKind::Newtype { base, .. } => case_ctors(schema, &base.subst(args)),
            TypeDefKind::Sum { ctors: all, .. } => {
                let n = all.len();
                Some((n, ctors.clone().unwrap_or_else(|| all_of(n))))
            }
        },
        Ty::Sum { ctors, sel } => {
            let n = ctors.len();
            Some((n, sel.clone().unwrap_or_else(|| all_of(n))))
        }
        Ty::Tag(t) => case_ctors(schema, t),
        _ => None,
    }
}

fn all_of(n: usize) -> Vec<u32> {
    (0..n as u32).collect()
}

/// Counts, over the plans this process validated, of the plans each check ran on.
#[cfg(any(test, feature = "testing"))]
pub mod counts {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::Seen;

    /// What a count of [`Checked`] counts: plans validated, plans refused, or plans on which
    /// one check ran.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Check {
        Plans,
        Refused,
        Bound,
        Arms,
        Scans,
        Routes,
        Ranges,
        Listed,
        Past,
    }

    impl Check {
        /// The checks, without `Plans` and `Refused`.
        pub const CHECKS: [Check; 7] = [
            Check::Bound,
            Check::Arms,
            Check::Scans,
            Check::Routes,
            Check::Ranges,
            Check::Listed,
            Check::Past,
        ];
    }

    /// The counts of this process's validations, by [`Check`].
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Checked([u64; 9]);

    impl Checked {
        pub fn get(&self, c: Check) -> u64 {
            self.0[c as usize]
        }
    }

    static COUNTS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];

    pub(super) fn add(s: &Seen, refused: bool) {
        let ran = [
            true, refused, s.bound, s.arms, s.scans, s.routes, s.ranges, s.listed, s.past,
        ];
        for (c, r) in COUNTS.iter().zip(ran) {
            c.fetch_add(u64::from(r), Ordering::Relaxed);
        }
    }

    /// Returns the counts so far.
    pub fn checked() -> Checked {
        Checked(COUNTS.each_ref().map(|c| c.load(Ordering::Relaxed)))
    }
}
