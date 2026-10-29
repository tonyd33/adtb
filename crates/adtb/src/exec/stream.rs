//! A query's result produced a unit of work at a time, for a reader that takes it in parts.

use std::collections::VecDeque;
use std::sync::Arc;

use adtb_lang::kernel::Var;

use crate::runtime::Runtime;
use crate::storage::Pid;

use super::frame::Frame;
use super::ir::{Eval, Pipeline, Program, Sink, Source};
use super::run::{Ctx, Run};
use super::spill::Spill;
use super::value::Val;
use super::{Call, Deadline, Error};

/// What [`produce`] made of a program.
pub enum Produced<R: Runtime> {
    /// The program's whole value: its root is not a query forming a set.
    Whole(Val),
    Stream(Production<R>),
}

impl<R: Runtime> std::fmt::Debug for Produced<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Produced::Whole(v) => f.debug_tuple("Whole").field(v).finish(),
            Produced::Stream(p) => f.debug_tuple("Stream").field(p).finish(),
        }
    }
}

impl<R: Runtime> std::fmt::Debug for Production<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Production")
            .field("units", &self.units.len())
            .finish_non_exhaustive()
    }
}

enum Unit {
    Pipeline(Arc<Pipeline>, Frame),
    Pages(Arc<Pipeline>, Frame, Vec<Pid>),
    Drain,
}

/// The elements of a set-forming root query, made one unit at a time: a pipeline with no source
/// or over a small heap, a range of a heap's pages, then the rows a partitioned join deferred.
///
/// Elements of different units may repeat.
pub struct Production<R: Runtime> {
    cx: Arc<Ctx<R>>,
    run: Arc<Run>,
    units: VecDeque<Unit>,
}

/// Returns `program` with its parameters bound by `params`, as a production when its root is a
/// query forming a set, else as its whole value.
pub fn produce<R: Runtime>(
    rt: R,
    program: Arc<Program>,
    call: Call<R>,
    params: &[(Var, Val)],
) -> Result<Produced<R>, Error> {
    let cx = Arc::new(Ctx {
        spill: Spill::new(rt.clone(), call.grant.clone(), call.options.spill_bytes),
        rt,
        program,
        store: call.store,
        grant: call.grant,
        options: call.options,
        pool: None,
        cancel: call.cancel,
        deadline: call.deadline,
    });
    let params: Vec<(u32, Val)> = params.iter().map(|(v, x)| (v.0, x.clone())).collect();
    let q = match &cx.program.root {
        Eval::Query(q) if matches!(q.sink, Sink::Set) => q.clone(),
        _ => return Ok(Produced::Whole(cx.root(&params)?)),
    };
    let mut env = Frame::unit(cx.program.vars);
    for (v, x) in &params {
        env.bind_one(*v, x.clone());
    }
    let mut chosen = Vec::new();
    cx.spine(&q.spine, env.clone(), false, &mut chosen)?;
    let mut units = VecDeque::new();
    for (p, env) in chosen {
        match &p.source {
            Source::Scan { heap, .. } => {
                let pages = cx.store.pages(*heap)?;
                for range in pages.chunks(cx.options.range_pages.max(1)) {
                    units.push_back(Unit::Pages(p.clone(), env.clone(), range.to_vec()));
                }
            }
            Source::Unit => units.push_back(Unit::Pipeline(p, env)),
        }
    }
    units.push_back(Unit::Drain);
    let run = Arc::new(Run::new(q, env));
    Ok(Produced::Stream(Production { cx, run, units }))
}

impl<R: Runtime> Production<R> {
    /// Run the next unit, stopping it at `deadline`, appending the distinct elements it makes to
    /// `out`. Returns whether a unit is left.
    pub fn next(&mut self, deadline: Option<Deadline>, out: &mut Vec<Val>) -> Result<bool, Error> {
        let Some(unit) = self.units.pop_front() else {
            return Ok(false);
        };
        Arc::get_mut(&mut self.cx)
            .expect("a production's context is shared only while a unit runs")
            .deadline = deadline;
        let (cx, run) = (&self.cx, &self.run);
        cx.check_cancel()?;
        let mut set = run.out(&cx.grant)?;
        match unit {
            Unit::Pipeline(p, env) => cx.pipeline(run, &p, &env, &mut set)?,
            Unit::Pages(p, env, pages) => {
                let Source::Scan { heap, .. } = &p.source else {
                    unreachable!("a unit of pages scans")
                };
                cx.store
                    .scan_pages(*heap, &pages, cx.options.morsel_rows, |b| {
                        cx.morsel(run, &p, &env, &b, &mut set)
                    })?;
            }
            Unit::Drain => {
                cx.drain(run, &mut set)?;
                for b in &run.builds {
                    b.lock(&cx.rt).take();
                }
            }
        }
        out.extend(set.into_set(&cx.spill)?);
        Ok(!self.units.is_empty())
    }
}
