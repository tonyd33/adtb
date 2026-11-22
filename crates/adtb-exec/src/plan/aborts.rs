use adtb_lang::analysis::{Totality, literal_count};
use adtb_lang::kernel::{Agg, Kind, Terms, Tm};
use adtb_lang::types::{Base, Ty};

use super::{Build, Chain, End, Eval, Pipeline, Query, Sink, Spine, Step};

/// How evaluating part of a plan may abort, ordered by what `try` leaves of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Abort {
    Never,
    /// It may fault, and `try` catches the fault.
    Faults,
    /// It may fail, be rejected, not terminate or reach an `Unreachable`.
    Escapes,
}

/// The judge of how a plan's parts may abort, from the terms they evaluate as a [`Totality`]
/// judges them.
pub(super) struct Aborts<'a, 's> {
    terms: &'a Terms,
    total: &'a mut Totality<'s>,
}

impl<'a, 's> Aborts<'a, 's> {
    pub(super) fn new(terms: &'a Terms, total: &'a mut Totality<'s>) -> Self {
        Aborts { terms, total }
    }

    /// Returns whether an evaluation of the builds or pipelines of `q`, or of its sink's
    /// per-element parts, may abort.
    pub(super) fn query_parts(&mut self, q: &Query) -> bool {
        self.parts(q) != Abort::Never
    }

    fn term(&mut self, t: Tm) -> Abort {
        if self.total.total(self.terms, t) {
            Abort::Never
        } else if self.total.escapes(self.terms, t) {
            Abort::Escapes
        } else {
            Abort::Faults
        }
    }

    fn all(&mut self, ts: &[Tm]) -> Abort {
        ts.iter()
            .map(|t| self.term(*t))
            .fold(Abort::Never, Abort::max)
    }

    fn evals<'e>(&mut self, es: impl IntoIterator<Item = &'e Eval>) -> Abort {
        es.into_iter()
            .map(|e| self.eval(e))
            .fold(Abort::Never, Abort::max)
    }

    fn parts(&mut self, q: &Query) -> Abort {
        let builds = q.builds.iter().map(|b| self.build(b)).max();
        let per_element = match &q.sink {
            Sink::Set | Sink::Exists | Sink::IsEmpty => Abort::Never,
            Sink::Agg { body, .. } | Sink::Fold { body, .. } => self.eval(body),
            Sink::Group { by, aggs, .. } => {
                let parts = aggs
                    .iter()
                    .flat_map(|a| a.filter.iter().chain([&a.element, &a.body]));
                self.evals(parts).max(self.eval(by))
            }
            Sink::Top { keys, .. } | Sink::Present { keys, .. } => {
                self.evals(keys.iter().map(|(k, _)| k))
            }
        };
        let spine = self.spine(&q.spine);
        builds.unwrap_or(Abort::Never).max(spine).max(per_element)
    }

    fn build(&mut self, b: &Build) -> Abort {
        let key = self.all(&b.key);
        b.pipelines
            .iter()
            .map(|p| self.pipeline(p))
            .fold(key, Abort::max)
    }

    /// Returns how the sink may abort beyond its per-element parts.
    fn sink(&mut self, sink: &Sink) -> Abort {
        let op = |op: &Agg, ty: &Ty| match op {
            Agg::Count | Agg::Min | Agg::Max | Agg::Avg => Abort::Never,
            Agg::Sum if matches!(ty, Ty::Base(Base::F32 | Base::F64)) => Abort::Never,
            Agg::Sum => Abort::Faults,
            Agg::User(_) => Abort::Escapes,
        };
        match sink {
            Sink::Set | Sink::Exists | Sink::IsEmpty => Abort::Never,
            Sink::Agg { op: o, ty, .. } => op(o, ty),
            Sink::Fold { fold, .. } => self.all(&[fold.init, fold.step, fold.finish]),
            Sink::Group { aggs, .. } => aggs
                .iter()
                .map(|a| op(&a.op, &a.ty))
                .fold(Abort::Never, Abort::max),
            Sink::Top { count, .. } => self.count(count).max(self.eval(count)),
            Sink::Present { take, after, .. } => {
                let counted = take.as_ref().map_or(Abort::Never, |n| self.count(n));
                counted.max(self.evals(take.iter().chain(after)))
            }
        }
    }

    /// Returns how the count of a `top` or the take of a `present` may fault.
    fn count(&self, n: &Eval) -> Abort {
        match n {
            Eval::Expr(t) if literal_count(self.terms, *t) => Abort::Never,
            _ => Abort::Faults,
        }
    }

    fn eval(&mut self, e: &Eval) -> Abort {
        match e {
            Eval::Expr(t) => self.term(*t),
            Eval::Let(_, a, b) => self.eval(a).max(self.eval(b)),
            Eval::If(c, a, b) => {
                let live: Vec<&Eval> = [&**a, &**b]
                    .into_iter()
                    .filter(|a| !self.dead_eval(a))
                    .collect();
                self.term(*c).max(self.evals(live))
            }
            Eval::Case(s, arms) => {
                let live: Vec<&Eval> = arms
                    .iter()
                    .map(|(_, _, a)| a)
                    .filter(|a| !self.dead_eval(a))
                    .collect();
                self.term(*s).max(self.evals(live))
            }
            Eval::Try(a) => match self.eval(a) {
                Abort::Never | Abort::Faults => Abort::Never,
                Abort::Escapes => Abort::Escapes,
            },
            Eval::Lookup(l) => self.all(&l.key).max(self.term(l.deref)),
            Eval::Query(q) => self.parts(q).max(self.sink(&q.sink)),
            Eval::Fix(f) => self.term(f.term),
            Eval::GroupRows { groups, key } => self.all(&[*groups, *key]),
            Eval::At(_) => Abort::Escapes,
        }
    }

    fn spine(&mut self, s: &Spine) -> Abort {
        match s {
            Spine::Pipeline(p) => self.pipeline(p),
            Spine::Union(ss) => ss
                .iter()
                .map(|s| self.spine(s))
                .fold(Abort::Never, Abort::max),
            Spine::If(c, a, b) => {
                let live: Vec<&Spine> = [&**a, &**b]
                    .into_iter()
                    .filter(|s| !self.dead_spine(s))
                    .collect();
                live.into_iter()
                    .fold(self.term(*c), |m, s| m.max(self.spine(s)))
            }
            Spine::Case(s, arms) => {
                let live: Vec<&Spine> = arms
                    .iter()
                    .map(|(_, _, a)| a)
                    .filter(|a| !self.dead_spine(a))
                    .collect();
                live.into_iter()
                    .fold(self.term(*s), |m, a| m.max(self.spine(a)))
            }
            Spine::Let(_, e, b) => self.eval(e).max(self.spine(b)),
        }
    }

    fn pipeline(&mut self, p: &Pipeline) -> Abort {
        self.chain(&p.chain)
    }

    fn chain(&mut self, c: &Chain) -> Abort {
        let steps = c
            .steps
            .iter()
            .map(|s| match s {
                Step::Filter(t) | Step::Unnest(_, t) => self.term(*t),
                Step::Bind(_, e) => self.eval(e),
                Step::Probe { key, .. } => self.all(key),
            })
            .fold(Abort::Never, Abort::max);
        steps.max(self.end(&c.end))
    }

    fn end(&mut self, e: &End) -> Abort {
        match e {
            End::Nothing => Abort::Never,
            End::Emit(t) => self.term(*t),
            End::Fork(cs) => cs
                .iter()
                .map(|c| self.chain(c))
                .fold(Abort::Never, Abort::max),
            End::If(c, a, b) => {
                let live: Vec<&Chain> = [&**a, &**b]
                    .into_iter()
                    .filter(|a| !self.dead_chain(a))
                    .collect();
                live.into_iter()
                    .fold(self.term(*c), |m, a| m.max(self.chain(a)))
            }
            End::Case(s, arms) => {
                let live: Vec<&Chain> = arms
                    .iter()
                    .map(|(_, _, a)| a)
                    .filter(|a| !self.dead_chain(a))
                    .collect();
                live.into_iter()
                    .fold(self.term(*s), |m, a| m.max(self.chain(a)))
            }
            End::Index { key, routes, .. } => {
                let reads = self.all(key);
                routes
                    .iter()
                    .map(|r| self.chain(&r.chain))
                    .fold(reads, Abort::max)
            }
            End::Range {
                key,
                lo,
                hi,
                routes,
                ..
            } => {
                let bounds: Vec<Tm> = lo.iter().chain(hi).map(|b| b.value).collect();
                let reads = self.all(key).max(self.all(&bounds));
                routes
                    .iter()
                    .map(|r| self.chain(&r.chain))
                    .fold(reads, Abort::max)
            }
        }
    }

    fn unreachable(&self, t: Tm) -> bool {
        matches!(self.terms.kind(t), Kind::Unreachable)
    }

    /// Returns whether `e` is the plan of an arm that is `Unreachable`.
    fn dead_eval(&self, e: &Eval) -> bool {
        matches!(e, Eval::Expr(t) if self.unreachable(*t))
    }

    /// Returns whether `s` is the plan of a set arm that is `Unreachable`.
    fn dead_spine(&self, s: &Spine) -> bool {
        matches!(s, Spine::Pipeline(p) if self.dead_chain(&p.chain))
    }

    /// Returns whether `c` is the plan of a set arm that is `Unreachable`: it first ranges over
    /// that term.
    fn dead_chain(&self, c: &Chain) -> bool {
        matches!(c.steps.first(), Some(Step::Unnest(_, t)) if self.unreachable(*t))
    }
}
