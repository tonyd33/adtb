use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::Arc;

use adtb_lang::types::Base;

use crate::runtime::Runtime;

use super::code::Catch;
use super::frame::Frame;
use super::ir::{AggOp, Eval, Fold, GroupAgg, Sink};
use super::run::{Ctx, holding};
use super::spill::Distinct;
use super::value::{F32, F64, Val};
use super::wide::Wide;
use super::{Error, Fault};

fn fits(b: Base, v: i128) -> Option<i64> {
    let (lo, hi): (i128, i128) = match b {
        Base::I8 => (i8::MIN.into(), i8::MAX.into()),
        Base::I16 => (i16::MIN.into(), i16::MAX.into()),
        Base::I32 => (i32::MIN.into(), i32::MAX.into()),
        _ => (i64::MIN.into(), i64::MAX.into()),
    };
    (lo..=hi).contains(&v).then_some(v as i64)
}

fn mistyped(want: &str, v: &Val) -> Error {
    Error::Internal(format!("an aggregate's range binds {want}, found {v:?}"))
}

fn float(v: &Val) -> Result<f64, Error> {
    match v {
        Val::F32(x) => Ok(f64::from(x.get())),
        Val::F64(x) => Ok(x.get()),
        other => Err(mistyped("floats", other)),
    }
}

/// An aggregate folded one value at a time.
pub struct Acc {
    op: AggOp,
    n: usize,
    int: i128,
    dec: Wide,
    f32: f32,
    f64: f64,
    best: Option<Val>,
}

impl Acc {
    pub fn new(op: AggOp) -> Acc {
        Acc {
            op,
            n: 0,
            int: 0,
            dec: Wide::ZERO,
            f32: 0.0,
            f64: 0.0,
            best: None,
        }
    }

    /// Fold in one value of the aggregate's range.
    pub fn push(&mut self, m: Val) -> Result<(), Error> {
        self.n += 1;
        match self.op {
            AggOp::Count => {}
            AggOp::SumInt(_) => {
                let Val::Int(i) = m else {
                    return Err(mistyped("integers", &m));
                };
                self.int += i128::from(i);
            }
            AggOp::SumDecimal => {
                let Val::Decimal(d) = m else {
                    return Err(mistyped("decimals", &m));
                };
                self.dec = self
                    .dec
                    .add(Wide::from_i128(d))
                    .ok_or(Error::Fault(Fault::ArithmeticOverflow))?;
            }
            AggOp::SumF32 | AggOp::AvgF32 { .. } => self.f32 += float(&m)? as f32,
            AggOp::SumF64 | AggOp::AvgF64 { .. } => self.f64 += float(&m)?,
            AggOp::Min { .. } => {
                if self.best.as_ref().is_none_or(|b| m < *b) {
                    self.best = Some(m);
                }
            }
            AggOp::Max { .. } => {
                if self.best.as_ref().is_none_or(|b| m >= *b) {
                    self.best = Some(m);
                }
            }
        }
        Ok(())
    }

    /// Returns the aggregate of the values folded in.
    pub fn finish(self) -> Result<Val, Error> {
        let wrap = |w: bool, v: Option<Val>| match (w, v) {
            (true, Some(v)) => Ok(Val::some(v)),
            (true, None) => Ok(Val::none()),
            (false, Some(v)) => Ok(v),
            (false, None) => Err(Error::Internal(
                "an aggregate that is not an Option ranges over a nonempty range".into(),
            )),
        };
        let n = self.n;
        match self.op {
            AggOp::Count => Ok(Val::Int(n as i64)),
            AggOp::SumInt(b) => Ok(Val::Int(
                fits(b, self.int).ok_or(Error::Fault(Fault::ArithmeticOverflow))?,
            )),
            AggOp::SumDecimal => Ok(Val::Decimal(
                self.dec
                    .digits38()
                    .ok_or(Error::Fault(Fault::ArithmeticOverflow))?,
            )),
            AggOp::SumF32 => Ok(Val::F32(F32::new(self.f32))),
            AggOp::SumF64 => Ok(Val::F64(F64::new(self.f64))),
            AggOp::Min { wrap: w } | AggOp::Max { wrap: w } => wrap(w, self.best),
            AggOp::AvgF32 { wrap: w } => {
                wrap(w, (n > 0).then(|| Val::F32(F32::new(self.f32 / n as f32))))
            }
            AggOp::AvgF64 { wrap: w } => {
                wrap(w, (n > 0).then(|| Val::F64(F64::new(self.f64 / n as f64))))
            }
        }
    }
}

/// Returns aggregate `op` of `ms`, one value per binding of its range.
pub fn aggregate(op: AggOp, ms: Vec<Val>) -> Result<Val, Error> {
    let mut acc = Acc::new(op);
    for m in ms {
        acc.push(m)?;
    }
    acc.finish()
}

/// Returns the order of two key lists under directions `desc`.
pub fn cmp_keys(a: &[Val], b: &[Val], desc: &[bool]) -> Ordering {
    for ((x, y), d) in a.iter().zip(b).zip(desc) {
        let o = x.cmp(y);
        let o = if *d { o.reverse() } else { o };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// An element with its keys, ordered by the keys under `desc` and then by the element.
struct Ranked {
    keys: Vec<Val>,
    v: Val,
    desc: Arc<[bool]>,
}

impl Ranked {
    fn size(&self) -> usize {
        self.keys.iter().map(Val::size).sum::<usize>() + self.v.size()
    }
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Ranked) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Ranked {}

impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Ranked) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ranked {
    fn cmp(&self, other: &Ranked) -> Ordering {
        cmp_keys(&self.keys, &other.keys, &self.desc).then_with(|| self.v.cmp(&other.v))
    }
}

/// A user-defined aggregate folded one value at a time.
struct Folding<'a> {
    fold: &'a Fold,
    f: Frame,
    state: Option<Val>,
}

impl<R: Runtime> Ctx<R> {
    /// Returns the value `sink` forms from the elements of `set`, reading them a morsel at a
    /// time.
    pub(super) fn sink(
        self: &Arc<Self>,
        sink: &Sink,
        set: Distinct,
        env: &Frame,
        count: Option<Val>,
    ) -> Result<Val, Error> {
        Ok(match sink {
            Sink::Set => Val::set(set.into_set(&self.spill)?),
            Sink::Exists => Val::bool(!set.is_empty()),
            Sink::IsEmpty => Val::bool(set.is_empty()),
            Sink::Agg { op, var, body } => {
                let mut acc = Acc::new(*op);
                self.each_value(set, *var, body, env, |m| acc.push(m))?;
                acc.finish()?
            }
            Sink::Fold { var, body, fold } => {
                let mut folding = Folding {
                    fold,
                    f: env.clone(),
                    state: None,
                };
                self.each_value(set, *var, body, env, |m| self.fold_in(&mut folding, m))?;
                self.fold_out(folding)?
            }
            Sink::Group {
                var,
                by,
                aggs,
                order,
                ..
            } => self.group(set, *var, by, aggs, order, env)?,
            Sink::Top { keys, var, .. } => {
                let n = count.expect("a top's count").int()?;
                let bound = usize::try_from(n).unwrap_or(0);
                let top = self.ranked(set, keys, *var, env, bound, |_| true)?;
                if n < 0 {
                    return Err(Error::Fault(Fault::NegativeCount));
                }
                Val::set(top.into_iter().collect())
            }
            Sink::Present {
                var,
                keys,
                take,
                after,
            } => {
                let take = take.as_ref().map(|t| self.eval_env(t, env, false));
                let after = after.as_ref().map(|t| self.eval_env(t, env, false));
                let bounds = present_bounds(take, after, keys.len());
                let (n, cursor) = match &bounds {
                    Ok((n, c)) => (*n, c.clone()),
                    Err(_) => (0, None),
                };
                let desc: Vec<bool> = keys.iter().map(|(_, d)| *d).collect();
                let listed = self.ranked(set, keys, *var, env, n, |k| {
                    cursor
                        .as_ref()
                        .is_none_or(|c| cmp_keys(c, k, &desc) == Ordering::Less)
                })?;
                bounds?;
                Val::List(Arc::from(listed))
            }
        })
    }

    /// Evaluate `body` at each element of `set` bound to `var`, a morsel at a time, and pass
    /// each value to `each`.
    fn each_value(
        self: &Arc<Self>,
        set: Distinct,
        var: u32,
        body: &Eval,
        env: &Frame,
        mut each: impl FnMut(Val) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut m = set.merge(&self.spill)?;
        loop {
            let chunk = m.chunk(self.options.morsel_rows.max(1))?;
            if chunk.is_empty() {
                return Ok(());
            }
            let mut f = env.widen(chunk.len());
            f.bind_all(var, chunk);
            let mut sel = f.all();
            for (_, v) in self.eval_rows(body, &mut f, &mut sel, &mut Catch::off(), false)? {
                each(v)?;
            }
        }
    }

    fn fold_in(&self, s: &mut Folding<'_>, m: Val) -> Result<(), Error> {
        let state = match s.state.take() {
            Some(v) => v,
            None => s.fold.init.eval_one(&mut s.f, self.sh())?,
        };
        s.f.bind_one(s.fold.state, state);
        s.f.bind_one(s.fold.value, m);
        s.state = Some(s.fold.step.eval_one(&mut s.f, self.sh())?);
        Ok(())
    }

    fn fold_out(&self, mut s: Folding<'_>) -> Result<Val, Error> {
        let state = match s.state.take() {
            Some(v) => v,
            None if s.fold.optional => return Ok(Val::none()),
            None => s.fold.init.eval_one(&mut s.f, self.sh())?,
        };
        s.f.bind_one(s.fold.state, state);
        let v = s.fold.finish.eval_one(&mut s.f, self.sh())?;
        Ok(if s.fold.optional { Val::some(v) } else { v })
    }

    /// Returns the records of a `Group` sink. The elements of `set` pass through a set of key
    /// and element pairs, which spills, and come back one group at a time.
    fn group(
        self: &Arc<Self>,
        set: Distinct,
        var: u32,
        by: &Eval,
        aggs: &[GroupAgg],
        order: &[Option<Option<usize>>],
        env: &Frame,
    ) -> Result<Val, Error> {
        let mut pairs = Distinct::new(&self.grant);
        let mut m = set.merge(&self.spill)?;
        loop {
            let chunk = m.chunk(self.options.morsel_rows.max(1))?;
            if chunk.is_empty() {
                break;
            }
            let mut f = env.widen(chunk.len());
            f.bind_all(var, chunk.clone());
            let mut sel = f.all();
            for (r, k) in self.eval_rows(by, &mut f, &mut sel, &mut Catch::off(), false)? {
                pairs.insert(Val::record(vec![k, chunk[r as usize].clone()]), &self.spill)?;
            }
        }
        drop(m);
        let mut held = self.grant.arena();
        let mut out = BTreeSet::new();
        let mut emit = |k: Val, members: BTreeSet<Val>| -> Result<(), Error> {
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
            Ok(())
        };
        let mut groups = pairs.merge(&self.spill)?;
        let mut cur: Option<(Val, BTreeSet<Val>)> = None;
        let mut members_held = self.grant.arena();
        while let Some(p) = groups.pop()? {
            let fs = p.fields()?;
            let (k, e) = (fs[0].clone(), fs[1].clone());
            if let Some((ck, ms)) = cur.take_if(|(ck, _)| *ck != k) {
                members_held.release(usize::MAX);
                emit(ck, ms)?;
            }
            members_held.grow(e.size())?;
            cur.get_or_insert_with(|| (k, BTreeSet::new())).1.insert(e);
        }
        if let Some((k, ms)) = cur {
            members_held.release(usize::MAX);
            emit(k, ms)?;
        }
        drop(groups);
        Ok(Val::set(out))
    }

    /// Returns the elements of `set` whose keys `keep` admits, ordered by `keys` over `var` and
    /// then by the element, at most the first `n` of them. Holds at most `n` elements.
    fn ranked(
        self: &Arc<Self>,
        set: Distinct,
        keys: &[(Eval, bool)],
        var: u32,
        env: &Frame,
        n: usize,
        keep: impl Fn(&[Val]) -> bool,
    ) -> Result<Vec<Val>, Error> {
        let desc: Arc<[bool]> = keys.iter().map(|(_, d)| *d).collect();
        let mut best: BTreeSet<Ranked> = BTreeSet::new();
        let mut held = self.grant.arena();
        let mut m = set.merge(&self.spill)?;
        loop {
            let chunk = m.chunk(self.options.morsel_rows.max(1))?;
            if chunk.is_empty() {
                break;
            }
            for (k, v) in self.keyed(chunk, keys, var, env)? {
                if n == 0 || !keep(&k) {
                    continue;
                }
                let r = Ranked {
                    keys: k,
                    v,
                    desc: desc.clone(),
                };
                if best.len() == n && best.last().is_some_and(|l| r >= *l) {
                    continue;
                }
                held.grow(r.size())?;
                best.insert(r);
                if best.len() > n {
                    let x = best.pop_last().expect("a ranking past its bound");
                    held.release(x.size());
                }
            }
        }
        Ok(best.into_iter().map(|r| r.v).collect())
    }

    /// Returns each element of `elems` with the values of `keys` at it.
    fn keyed(
        self: &Arc<Self>,
        elems: Vec<Val>,
        keys: &[(Eval, bool)],
        var: u32,
        env: &Frame,
    ) -> Result<Vec<(Vec<Val>, Val)>, Error> {
        let mut f = env.widen(elems.len());
        f.bind_all(var, elems.clone());
        let mut ks: Vec<Vec<Val>> = vec![Vec::with_capacity(keys.len()); elems.len()];
        for (k, _) in keys {
            let mut sel_f = f.all();
            for (r, v) in self.eval_rows(k, &mut f, &mut sel_f, &mut Catch::off(), false)? {
                ks[r as usize].push(v);
            }
        }
        Ok(ks.into_iter().zip(elems).collect())
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
}

/// Returns how many elements a `Present` sink takes and the keys of its cursor, from the values
/// of its `take` and `after`, failing as the first of them failed.
fn present_bounds(
    take: Option<Result<Val, Error>>,
    after: Option<Result<Val, Error>>,
    keys: usize,
) -> Result<(usize, Option<Vec<Val>>), Error> {
    let take = take.transpose()?;
    let after = after.transpose()?;
    let cursor = match after {
        Some(Val::Ctor(1, fs)) if keys == 1 => Some(vec![fs[0].clone()]),
        Some(Val::Ctor(1, fs)) => Some(fs[0].fields()?.to_vec()),
        _ => None,
    };
    let n = match take.as_ref().map(Val::int).transpose()? {
        Some(t) if t < 0 => return Err(Error::Fault(Fault::NegativeCount)),
        Some(t) => t as usize,
        None => usize::MAX,
    };
    Ok((n, cursor))
}
