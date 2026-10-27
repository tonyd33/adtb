use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::kernel::{Agg, Tm};
use adtb_lang::schema::Schema;
use adtb_lang::types::{Base, RelId, Ty};

use crate::plan::{self, Catalog, Element, HeapId, IndexId, Plan};

use super::Error;
use super::code::{Code, Compiler, Shared};
use super::store::{Conv, conv};
use super::types::{base, ctors, field, option_arg};

pub type Expr = Arc<Code>;

/// An `Eval` of a plan with its expressions compiled.
#[derive(Debug)]
pub enum Eval {
    Expr(Expr),
    Let(u32, Box<Eval>, Box<Eval>),
    If(Expr, Box<Eval>, Box<Eval>),
    Case(Expr, Vec<Option<(u32, Eval)>>),
    Try(Box<Eval>),
    Lookup(Lookup),
    Query(Arc<Query>),
    Fix(Box<Fix>),
    /// The groups, then the key whose group's rows it returns.
    GroupRows(Expr, Expr),
    At(Box<At>),
}

/// A read of the past: `body` evaluated at the state `point` names.
#[derive(Debug)]
pub struct At {
    /// Whether `point` is an `Instant` rather than a `Position`.
    pub instant: bool,
    pub point: Expr,
    /// The version `body` is typed at, or `None` for the version of the state `point` names.
    pub pins: Option<u64>,
    pub body: Eval,
    /// The heaps `body` reads.
    pub heaps: BTreeSet<HeapId>,
}

#[derive(Debug)]
pub struct Lookup {
    pub index: IndexId,
    pub key: Vec<Expr>,
    pub routes: Vec<Fetch>,
}

/// A fetch of an index entry's row from the heap of its constructor.
#[derive(Debug)]
pub struct Fetch {
    pub ctor: u32,
    pub heap: HeapId,
    /// The position in the row of each of the index's key elements that is a field.
    pub key_at: Vec<Option<usize>>,
}

#[derive(Debug)]
pub struct Query {
    pub builds: Vec<Build>,
    pub spine: Spine,
    pub sink: Sink,
    pub total: bool,
}

#[derive(Debug)]
pub enum Spine {
    Pipeline(Arc<Pipeline>),
    Union(Vec<Spine>),
    If(Expr, Box<Spine>, Box<Spine>),
    Case(Expr, Vec<Option<(u32, Spine)>>),
    Let(u32, Eval, Box<Spine>),
}

#[derive(Debug)]
pub struct Build {
    pub var: u32,
    pub key: Vec<Expr>,
    pub pipelines: Vec<Arc<Pipeline>>,
}

#[derive(Debug)]
pub struct Pipeline {
    pub source: Source,
    pub chain: Arc<Chain>,
}

#[derive(Debug)]
pub enum Source {
    Unit,
    Scan { heap: HeapId, var: u32 },
}

#[derive(Debug)]
pub struct Chain {
    pub steps: Vec<Step>,
    pub end: End,
}

#[derive(Debug)]
pub enum Step {
    Filter(Expr),
    Bind(u32, Eval),
    Unnest(u32, Expr),
    Probe {
        build: usize,
        key: Vec<Expr>,
        var: u32,
    },
}

#[derive(Debug)]
pub enum End {
    Nothing,
    Emit(Expr),
    Fork(Vec<Arc<Chain>>),
    If(Expr, Arc<Chain>, Arc<Chain>),
    Case(Expr, Vec<Option<(u32, Arc<Chain>)>>),
    Index {
        index: IndexId,
        key: Vec<Expr>,
        var: u32,
        routes: Vec<(Fetch, Arc<Chain>)>,
    },
    /// An `Index` end whose entries' next field also lies within the bounds, each its value and
    /// whether it is included.
    Range {
        index: IndexId,
        key: Vec<Expr>,
        lo: Option<(Expr, bool)>,
        hi: Option<(Expr, bool)>,
        var: u32,
        routes: Vec<(Fetch, Arc<Chain>)>,
    },
}

/// How an aggregate combines its values, chosen for its types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggOp {
    Count,
    SumInt(Base),
    SumDecimal,
    SumF32,
    SumF64,
    /// `wrap` when the result is an `Option`.
    Min {
        wrap: bool,
    },
    Max {
        wrap: bool,
    },
    AvgF32 {
        wrap: bool,
    },
    AvgF64 {
        wrap: bool,
    },
}

#[derive(Debug)]
pub enum Sink {
    Set,
    Exists,
    IsEmpty,
    Agg {
        op: AggOp,
        var: u32,
        body: Eval,
    },
    Fold {
        var: u32,
        body: Eval,
        fold: Fold,
    },
    /// `order` lists, for each field of the group records in name order, which value fills it:
    /// `None` for the key, `Some(None)` for the rows, `Some(Some(i))` for aggregate `i`.
    Group {
        var: u32,
        by: Eval,
        rows: bool,
        aggs: Vec<GroupAgg>,
        order: Vec<Option<Option<usize>>>,
    },
    Top {
        count: Eval,
        keys: Vec<(Eval, bool)>,
        var: u32,
    },
    Present {
        var: u32,
        keys: Vec<(Eval, bool)>,
        take: Option<Eval>,
        after: Option<Eval>,
    },
}

/// A user-defined aggregate: `step` over `state` and `value`, `finish` over `state`.
#[derive(Debug)]
pub struct Fold {
    pub init: Expr,
    pub step: Expr,
    pub finish: Expr,
    pub state: u32,
    pub value: u32,
    pub optional: bool,
}

#[derive(Debug)]
pub struct GroupAgg {
    pub op: AggOp,
    pub row: u32,
    pub filter: Vec<Eval>,
    pub elem: u32,
    pub element: Eval,
    pub body: Eval,
}

#[derive(Debug)]
pub struct Fix {
    pub binds: Vec<FixBind>,
    pub body: Eval,
}

#[derive(Debug)]
pub struct FixBind {
    pub var: u32,
    pub body: Eval,
    pub delta: u32,
    pub step: Option<Eval>,
}

/// What the executor knows of a heap a plan reads.
#[derive(Debug)]
pub struct HeapInfo {
    pub ctor: u32,
    /// How each field of a row converts.
    pub fields: Vec<Conv>,
}

/// A plan compiled for the executor.
#[derive(Debug)]
pub struct Program {
    pub root: Eval,
    /// The type of the root's value.
    pub ty: Ty,
    pub params: Vec<u32>,
    pub vars: usize,
    pub heaps: BTreeMap<HeapId, HeapInfo>,
    pub shared: Shared,
    pub reads: plan::Reads,
}

/// The fault constructors of the prelude's `Fault`, by the executor's fault order.
pub(super) fn fault_ctors(schema: &Schema) -> Vec<u32> {
    let def = schema.ty(schema.prelude_type("Fault"));
    super::Fault::ALL
        .iter()
        .map(|f| {
            def.ctor_index(f.name())
                .unwrap_or_else(|| panic!("the prelude declares fault {}", f.name()))
        })
        .collect()
}

struct Lower<'a> {
    c: Compiler<'a>,
    heaps: BTreeMap<HeapId, (RelId, u32)>,
    indexes: BTreeMap<IndexId, (RelId, Vec<Element>)>,
}

/// Compiles `plan`, planned against `schema` and `catalog`, for the executor. `caller` is the
/// value `Caller` denotes, if the statement has one.
///
/// Fails with `Unsupported` when the plan uses a form or primitive the executor does not run.
pub fn compile(
    schema: &Schema,
    catalog: &Catalog,
    plan: &Plan,
    caller: Option<super::Val>,
) -> Result<Program, Error> {
    let mut heaps = BTreeMap::new();
    let mut indexes = BTreeMap::new();
    for (r, d) in &catalog.relations {
        for (c, h) in &d.heaps {
            heaps.insert(*h, (*r, *c));
        }
        for i in &d.indexes {
            indexes.insert(i.id, (*r, i.elements.clone()));
        }
    }
    let mut l = Lower {
        c: Compiler::new(schema, &plan.terms),
        heaps,
        indexes,
    };
    let root = l.eval(&plan.root)?;
    let mut infos = BTreeMap::new();
    for h in &plan.reads.heaps {
        let (rel, ctor) = l.heaps[h];
        let row = l.row_type(rel, ctor);
        let cs = ctors(schema, &row).expect("a row type is a sum");
        let fields = cs[ctor as usize]
            .iter()
            .map(|(_, t)| conv(schema, t))
            .collect();
        infos.insert(*h, HeapInfo { ctor, fields });
    }
    Ok(Program {
        root,
        ty: plan.ty.clone(),
        params: plan.params.iter().map(|v| v.0).collect(),
        vars: plan.terms.vars.len(),
        heaps: infos,
        shared: Shared {
            caller,
            fault_ctors: fault_ctors(schema),
            hosts: Arc::default(),
        },
        reads: plan.reads.clone(),
    })
}

impl Lower<'_> {
    fn schema(&self) -> &Schema {
        self.c.schema()
    }

    fn row_type(&self, rel: RelId, ctor: u32) -> Ty {
        match &self.schema().relation(rel).elem {
            Ty::Data { def, args, .. } => Ty::Data {
                def: *def,
                args: args.clone(),
                ctors: Some(vec![ctor]),
            },
            t => panic!("a relation's element type is a declared sum, found {t:?}"),
        }
    }

    fn expr(&mut self, t: Tm) -> Result<Expr, Error> {
        self.c.code(t)
    }

    fn exprs(&mut self, ts: &[Tm]) -> Result<Vec<Expr>, Error> {
        ts.iter().map(|t| self.expr(*t)).collect()
    }

    fn arms<P, Q>(
        &mut self,
        scrut: Tm,
        arms: &[(u32, adtb_lang::kernel::Var, P)],
        mut each: impl FnMut(&mut Self, &P) -> Result<Q, Error>,
    ) -> Result<Vec<Option<(u32, Q)>>, Error> {
        let n = ctors(self.schema(), self.c.terms().ty(scrut)).map_or(0, |c| c.len());
        let mut out: Vec<Option<(u32, Q)>> = (0..n).map(|_| None).collect();
        for (c, v, a) in arms {
            out[*c as usize] = Some((v.0, each(self, a)?));
        }
        Ok(out)
    }

    fn eval(&mut self, e: &plan::Eval) -> Result<Eval, Error> {
        Ok(match e {
            plan::Eval::Expr(t) => Eval::Expr(self.expr(*t)?),
            plan::Eval::Let(v, a, b) => {
                Eval::Let(v.0, Box::new(self.eval(a)?), Box::new(self.eval(b)?))
            }
            plan::Eval::If(c, a, b) => Eval::If(
                self.expr(*c)?,
                Box::new(self.eval(a)?),
                Box::new(self.eval(b)?),
            ),
            plan::Eval::Case(s, arms) => {
                Eval::Case(self.expr(*s)?, self.arms(*s, arms, Lower::eval)?)
            }
            plan::Eval::Try(a) => Eval::Try(Box::new(self.eval(a)?)),
            plan::Eval::Lookup(lk) => Eval::Lookup(Lookup {
                index: lk.index,
                key: self.exprs(&lk.key)?,
                routes: lk
                    .routes
                    .iter()
                    .map(|(c, h)| self.fetch(lk.index, *c, *h))
                    .collect::<Result<_, _>>()?,
            }),
            plan::Eval::Query(q) => Eval::Query(Arc::new(self.query(q)?)),
            plan::Eval::GroupRows { groups, key } => {
                Eval::GroupRows(self.expr(*groups)?, self.expr(*key)?)
            }
            plan::Eval::At(a) => Eval::At(Box::new(At {
                instant: a.instant,
                point: self.expr(a.point)?,
                pins: a.pins,
                body: self.eval(&a.body)?,
                heaps: a.heaps.clone(),
            })),
            plan::Eval::Fix(f) => Eval::Fix(Box::new(Fix {
                binds: f
                    .binds
                    .iter()
                    .map(|b| {
                        Ok(FixBind {
                            var: b.var.0,
                            body: self.eval(&b.body)?,
                            delta: b.delta.0,
                            step: b.step.as_ref().map(|s| self.eval(s)).transpose()?,
                        })
                    })
                    .collect::<Result<_, Error>>()?,
                body: self.eval(&f.body)?,
            })),
        })
    }

    fn fetch(&mut self, index: IndexId, ctor: u32, heap: HeapId) -> Result<Fetch, Error> {
        let (rel, elements) = self.indexes[&index].clone();
        let row = self.row_type(rel, ctor);
        let key_at = elements
            .iter()
            .map(|e| match e {
                Element::Field(f) => field(self.schema(), &row, f)
                    .and_then(|(at, _)| at.of(ctor))
                    .map(Some),
                Element::Derived { .. } => Ok(None),
            })
            .collect::<Result<_, _>>()?;
        Ok(Fetch { ctor, heap, key_at })
    }

    fn query(&mut self, q: &plan::Query) -> Result<Query, Error> {
        let builds = q
            .builds
            .iter()
            .map(|b| {
                Ok(Build {
                    var: b.var.0,
                    key: self.exprs(&b.key)?,
                    pipelines: b
                        .pipelines
                        .iter()
                        .map(|p| self.pipeline(p).map(Arc::new))
                        .collect::<Result<_, Error>>()?,
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok(Query {
            builds,
            spine: self.spine(&q.spine)?,
            sink: self.sink(&q.sink)?,
            total: q.total,
        })
    }

    fn spine(&mut self, s: &plan::Spine) -> Result<Spine, Error> {
        Ok(match s {
            plan::Spine::Pipeline(p) => Spine::Pipeline(Arc::new(self.pipeline(p)?)),
            plan::Spine::Union(ss) => {
                Spine::Union(ss.iter().map(|s| self.spine(s)).collect::<Result<_, _>>()?)
            }
            plan::Spine::If(c, a, b) => Spine::If(
                self.expr(*c)?,
                Box::new(self.spine(a)?),
                Box::new(self.spine(b)?),
            ),
            plan::Spine::Case(sc, arms) => {
                Spine::Case(self.expr(*sc)?, self.arms(*sc, arms, Lower::spine)?)
            }
            plan::Spine::Let(v, e, b) => Spine::Let(v.0, self.eval(e)?, Box::new(self.spine(b)?)),
        })
    }

    fn pipeline(&mut self, p: &plan::Pipeline) -> Result<Pipeline, Error> {
        let source = match &p.source {
            plan::Source::Unit => Source::Unit,
            plan::Source::Scan { heap, var, .. } => Source::Scan {
                heap: *heap,
                var: var.0,
            },
        };
        Ok(Pipeline {
            source,
            chain: self.chain(&p.chain)?,
        })
    }

    fn chain(&mut self, c: &plan::Chain) -> Result<Arc<Chain>, Error> {
        let steps = c
            .steps
            .iter()
            .map(|s| {
                Ok(match s {
                    plan::Step::Filter(t) => Step::Filter(self.expr(*t)?),
                    plan::Step::Bind(v, e) => Step::Bind(v.0, self.eval(e)?),
                    plan::Step::Unnest(v, t) => Step::Unnest(v.0, self.expr(*t)?),
                    plan::Step::Probe { build, key, var } => Step::Probe {
                        build: build.0 as usize,
                        key: self.exprs(key)?,
                        var: var.0,
                    },
                })
            })
            .collect::<Result<_, Error>>()?;
        Ok(Arc::new(Chain {
            steps,
            end: self.end(&c.end)?,
        }))
    }

    fn end(&mut self, e: &plan::End) -> Result<End, Error> {
        Ok(match e {
            plan::End::Nothing => End::Nothing,
            plan::End::Emit(t) => End::Emit(self.expr(*t)?),
            plan::End::Fork(cs) => {
                End::Fork(cs.iter().map(|c| self.chain(c)).collect::<Result<_, _>>()?)
            }
            plan::End::If(c, a, b) => End::If(self.expr(*c)?, self.chain(a)?, self.chain(b)?),
            plan::End::Case(s, arms) => {
                End::Case(self.expr(*s)?, self.arms(*s, arms, Lower::chain)?)
            }
            plan::End::Index {
                index,
                key,
                var,
                routes,
            } => End::Index {
                index: *index,
                key: self.exprs(key)?,
                var: var.0,
                routes: self.routes(*index, routes)?,
            },
            plan::End::Range {
                index,
                key,
                lo,
                hi,
                var,
                routes,
            } => End::Range {
                index: *index,
                key: self.exprs(key)?,
                lo: self.bound(lo)?,
                hi: self.bound(hi)?,
                var: var.0,
                routes: self.routes(*index, routes)?,
            },
        })
    }

    fn routes(
        &mut self,
        index: IndexId,
        routes: &[plan::Route],
    ) -> Result<Vec<(Fetch, Arc<Chain>)>, Error> {
        routes
            .iter()
            .map(|r| Ok((self.fetch(index, r.ctor, r.heap)?, self.chain(&r.chain)?)))
            .collect()
    }

    fn bound(&mut self, b: &Option<plan::RangeBound>) -> Result<Option<(Expr, bool)>, Error> {
        b.map(|b| Ok((self.expr(b.value)?, b.inclusive)))
            .transpose()
    }

    fn agg(&self, op: Agg, ty: &Ty, body_ty: &Ty) -> Result<AggOp, Error> {
        let s = self.schema();
        let wrap = ty != body_ty && option_arg(s, ty).is_some();
        Ok(match op {
            Agg::Count => AggOp::Count,
            Agg::Sum => match base(s, ty) {
                Some(b @ (Base::I8 | Base::I16 | Base::I32 | Base::I64)) => AggOp::SumInt(b),
                Some(Base::Decimal(_)) => AggOp::SumDecimal,
                Some(Base::F32) => AggOp::SumF32,
                Some(Base::F64) => AggOp::SumF64,
                other => return Err(Error::Unsupported(format!("`sum` at {other:?}"))),
            },
            Agg::Min => AggOp::Min { wrap },
            Agg::Max => AggOp::Max { wrap },
            Agg::Avg => match base(s, body_ty) {
                Some(Base::F32) => AggOp::AvgF32 { wrap },
                _ => AggOp::AvgF64 { wrap },
            },
            Agg::User(_) => {
                return Err(Error::Unsupported("a user-defined aggregate".into()));
            }
        })
    }

    fn sink(&mut self, s: &plan::Sink) -> Result<Sink, Error> {
        Ok(match s {
            plan::Sink::Set => Sink::Set,
            plan::Sink::Exists => Sink::Exists,
            plan::Sink::IsEmpty => Sink::IsEmpty,
            plan::Sink::Agg {
                op,
                var,
                body,
                ty,
                body_ty,
            } => Sink::Agg {
                op: self.agg(*op, ty, body_ty)?,
                var: var.0,
                body: self.eval(body)?,
            },
            plan::Sink::Fold { var, body, fold } => Sink::Fold {
                var: var.0,
                body: self.eval(body)?,
                fold: Fold {
                    init: self.expr(fold.init)?,
                    step: self.expr(fold.step)?,
                    finish: self.expr(fold.finish)?,
                    state: fold.state.0,
                    value: fold.value.0,
                    optional: fold.optional,
                },
            },
            plan::Sink::Group {
                var,
                by,
                rows,
                aggs,
            } => {
                let mut names: Vec<(String, Option<Option<usize>>)> = vec![("key".into(), None)];
                if *rows {
                    names.push(("rows".into(), Some(None)));
                }
                let mut compiled = Vec::new();
                for (i, a) in aggs.iter().enumerate() {
                    names.push((a.field.clone(), Some(Some(i))));
                    compiled.push(GroupAgg {
                        op: self.agg(a.op, &a.ty, &a.body_ty)?,
                        row: a.row.0,
                        filter: a
                            .filter
                            .iter()
                            .map(|f| self.eval(f))
                            .collect::<Result<_, _>>()?,
                        elem: a.elem.0,
                        element: self.eval(&a.element)?,
                        body: self.eval(&a.body)?,
                    });
                }
                names.sort_by(|a, b| a.0.cmp(&b.0));
                Sink::Group {
                    var: var.0,
                    by: self.eval(by)?,
                    rows: *rows,
                    aggs: compiled,
                    order: names.into_iter().map(|(_, o)| o).collect(),
                }
            }
            plan::Sink::Top { count, keys, var } => Sink::Top {
                count: self.eval(count)?,
                keys: self.keys(keys)?,
                var: var.0,
            },
            plan::Sink::Present {
                var,
                keys,
                take,
                after,
            } => Sink::Present {
                var: var.0,
                keys: self.keys(keys)?,
                take: take.as_ref().map(|t| self.eval(t)).transpose()?,
                after: after.as_ref().map(|t| self.eval(t)).transpose()?,
            },
        })
    }

    fn keys(&mut self, keys: &[(plan::Eval, bool)]) -> Result<Vec<(Eval, bool)>, Error> {
        keys.iter().map(|(k, d)| Ok((self.eval(k)?, *d))).collect()
    }
}
