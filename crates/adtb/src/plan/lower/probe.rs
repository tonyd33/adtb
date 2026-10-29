use std::collections::HashMap;

use adtb_lang::kernel::{Kind, PathStep, Prim, Tm, Var};
use adtb_lang::types::{CtorSet, RelId, Ty};

use super::{
    Access, BuildKey, Fixed, Fixes, HEAP_ROWS, IndexProbe, Limit, PROBE_COST, PROBE_ROWS, Planner,
    QueryCx, Scope, Span, chain, erase, is_nothing, nothing, prepend, union,
};
use crate::plan::terms::{with_children, with_vars};
use crate::plan::{
    Build, BuildId, Chain, Element, End, Error, IndexDesc, Lookup, Pipeline, RangeBound, Route,
    Source, Spine, Step,
};

/// The rows `x` a probe binds, then `conds` in order, then `rest`.
struct Then<'a> {
    x: Var,
    conds: &'a [Tm],
    rest: Tm,
}

impl Planner<'_> {
    pub(super) fn lookup(&mut self, key: Tm, row_ty: &Ty) -> Result<Lookup, Error> {
        let Ty::Key { rel, ctors } = self.ty(key) else {
            return Err(Error::Internal("a dereference follows a key".into()));
        };
        let ctors = match row_ty {
            Ty::Data { ctors: Some(s), .. } => s.clone(),
            _ => self.ctors_of(rel, &ctors),
        };
        let desc = self.catalog.relation(rel);
        Ok(Lookup {
            index: desc.key_index,
            key: self
                .key_values(rel, key)
                .into_iter()
                .map(|(_, k)| k)
                .collect(),
            routes: ctors
                .iter()
                .map(|c| (*c, self.catalog.heap(rel, *c)))
                .collect(),
        })
    }

    /// Returns the value of each key field of `rel` in the key `k`.
    fn key_values(&mut self, rel: RelId, k: Tm) -> Vec<(String, Tm)> {
        let r = self.schema.relation(rel);
        let def = self.schema.ty(r.elem_def);
        let args = match &r.elem {
            Ty::Data { args, .. } => args.clone(),
            _ => Vec::new(),
        };
        let field_ty = |f: &str| {
            def.field(0, f)
                .unwrap_or_else(|| panic!("key field `{f}` is a field of the relation's rows"))
                .ty
                .subst(&args)
        };
        match r.key.as_slice() {
            [f] if matches!(field_ty(f), Ty::Key { rel: own, .. } if own == rel) => {
                vec![(f.clone(), k)]
            }
            [f] => {
                let ty = field_ty(f);
                vec![(f.clone(), self.add(Kind::Prim(Prim::KeyValue, vec![k]), ty))]
            }
            fs => {
                let mut tys: Vec<(String, Ty)> =
                    fs.iter().map(|f| (f.clone(), field_ty(f))).collect();
                tys.sort_by(|a, b| a.0.cmp(&b.0));
                let rec = self.add(Kind::Prim(Prim::KeyValue, vec![k]), Ty::Record(tys.clone()));
                fs.iter()
                    .map(|f| {
                        let ty = field_ty(f);
                        (f.clone(), self.add(Kind::Field(rec, f.clone()), ty))
                    })
                    .collect()
            }
        }
    }

    /// Returns what of a row `x` of `rel` condition `c` fixes, each with its value, when `c` is an
    /// equality whose other side mentions neither `x` nor a variable of `arms`, each the value at
    /// a path from `x`.
    fn equality(
        &mut self,
        rel: RelId,
        c: Tm,
        x: Var,
        arms: &HashMap<Var, Vec<PathStep>>,
    ) -> Option<Fixes> {
        let Kind::Prim(Prim::Eq, args) = self.kind(c) else {
            return None;
        };
        let [a, b] = args[..] else { return None };
        if erase(&self.ty(a)) != erase(&self.ty(b)) {
            return None;
        }
        let bound: Vec<Var> = std::iter::once(x).chain(arms.keys().copied()).collect();
        for (this, other) in [(a, b), (b, a)] {
            if self.mentions(other, &bound) || !self.mentions(this, &bound) {
                continue;
            }
            if let Kind::Key(row) = self.kind(this)
                && matches!(self.t.kind(row), Kind::Var(v) if *v == x)
            {
                let fields = self.key_values(rel, other).into_iter();
                return Some(fields.map(|(f, v)| (Fixed::field(f, v), v)).collect());
            }
            let path = self.path_of(this, x, arms).filter(|p| !p.is_empty());
            return Some(vec![(Fixed { path, term: this }, other)]);
        }
        None
    }

    /// Returns the path from `x` that `t` reads, each variable of `arms` standing for the value at
    /// its path.
    fn path_of(&self, t: Tm, x: Var, arms: &HashMap<Var, Vec<PathStep>>) -> Option<Vec<PathStep>> {
        match self.t.kind(t) {
            Kind::Var(v) if *v == x => Some(Vec::new()),
            Kind::Var(v) => arms.get(v).cloned(),
            Kind::Field(row, f) => {
                let mut p = self.path_of(*row, x, arms)?;
                p.push(PathStep::Field(f.clone()));
                Some(p)
            }
            _ => None,
        }
    }

    /// Add to `out` what of row `x` of `rel` every result of set term `t` needs fixed: under a
    /// case over a path from `x` with one arm that is not empty, the total conditions guarding
    /// that arm's results, and so on inside it, and inside the source of a comprehension over
    /// such a case.
    fn arm_equalities(
        &mut self,
        rel: RelId,
        t: Tm,
        x: Var,
        arms: &mut HashMap<Var, Vec<PathStep>>,
        out: &mut Fixes,
    ) {
        match self.kind(t) {
            Kind::For(_, src, _) if self.rel_source(src).is_none() => {
                self.arm_equalities(rel, src, x, arms, out);
            }
            Kind::Case { .. } => {
                let Some(body) = self.live_arm(t, x, arms, Planner::is_empty_set) else {
                    return;
                };
                let (conds, rest) = self.peel(body);
                let n = self.total_prefix(&conds);
                for cond in &conds[..n] {
                    self.cond_equalities(rel, *cond, x, arms, out);
                }
                self.arm_equalities(rel, rest, x, arms, out);
            }
            _ => {}
        }
    }

    /// Add to `out` what of row `x` of `rel` total condition `c` needs fixed to hold: an
    /// equality, the conjuncts of a conjunction, or under a case over a path from `x` with one
    /// arm that is not false, what that arm needs.
    fn cond_equalities(
        &mut self,
        rel: RelId,
        c: Tm,
        x: Var,
        arms: &mut HashMap<Var, Vec<PathStep>>,
        out: &mut Fixes,
    ) {
        if let Some(pairs) = self.equality(rel, c, x, arms) {
            out.extend(pairs);
            return;
        }
        let Some(body) = self.live_arm(c, x, arms, |p, a| p.literal(a) == Some(false)) else {
            return;
        };
        let from = out.len();
        let mut conjuncts = Vec::new();
        self.conjuncts(body, &mut conjuncts);
        for d in conjuncts {
            self.cond_equalities(rel, d, x, arms, out);
        }
        if let Kind::Case { scrutinee, .. } = self.kind(c)
            && let Some(path) = self.path_of(scrutinee, x, arms)
            && let Some(at) = self.arm_ctor(c, |p, a| p.literal(a) == Some(false))
        {
            self.whole_ctor(scrutinee, &path, at, from, out);
        }
    }

    /// Returns the body of the one arm of case `t` over a path from `x` that `dead` does not
    /// hold of, binding its variable in `arms` to the path through its constructor.
    fn live_arm(
        &self,
        t: Tm,
        x: Var,
        arms: &mut HashMap<Var, Vec<PathStep>>,
        dead: impl Fn(&Self, Tm) -> bool,
    ) -> Option<Tm> {
        let Kind::Case {
            scrutinee,
            arms: cases,
        } = self.t.kind(t)
        else {
            return None;
        };
        let mut path = self
            .path_of(*scrutinee, x, arms)
            .filter(|p| !p.is_empty())?;
        let live: Vec<&(u32, Var, Tm)> = cases.iter().filter(|(_, _, a)| !dead(self, *a)).collect();
        let [(c, v, body)] = live[..] else {
            return None;
        };
        path.push(PathStep::Ctor(*c));
        arms.insert(*v, path);
        Some(*body)
    }

    /// Returns whether `fixed`, of a row `x` of `rel`, is key element `e` of one of its indexes.
    fn is_element(&self, rel: RelId, x: Var, e: &Element, fixed: &Fixed) -> bool {
        match e {
            Element::Field(f) => {
                matches!(fixed.path.as_deref(), Some([PathStep::Field(g)]) if g == f)
            }
            Element::Derived { index, element } => {
                let (x0, f) = &self.normal.index_fields[&(rel, *index, *element)];
                if !f.path.is_empty() {
                    return fixed.path.as_ref() == Some(&f.path);
                }
                let mut vars = HashMap::from([(*x0, x)]);
                self.same_term(fixed.term, f.value, &mut vars)
            }
        }
    }

    /// Returns whether term `a` of the plan's arena is term `b` of the normal program's, each
    /// variable of `b` renamed by `vars` or bound alike in both.
    fn same_term(&self, a: Tm, b: Tm, vars: &mut HashMap<Var, Var>) -> bool {
        let from = &self.normal.terms;
        let (ca, cb) = (self.t.children(a), from.children(b));
        if ca.len() != cb.len() || erase(self.t.ty(a)) != erase(from.ty(b)) {
            return false;
        }
        let zeros = vec![Tm(0); ca.len()];
        let ka = with_children(self.t.kind(a), &zeros);
        let kb = with_children(from.kind(b), &zeros);
        let (mut va, mut vb) = (Vec::new(), Vec::new());
        with_vars(&ka, &mut |v| {
            va.push(v);
            v
        });
        with_vars(&kb, &mut |v| {
            vb.push(v);
            v
        });
        if va.len() != vb.len() {
            return false;
        }
        for (u, w) in vb.iter().zip(&va) {
            if *vars.entry(*u).or_insert(*w) != *w {
                return false;
            }
        }
        if with_vars(&kb, &mut |v| vars[&v]) != ka {
            return false;
        }
        ca.into_iter()
            .zip(cb)
            .all(|(x, y)| self.same_term(x, y, vars))
    }

    /// Returns the best index of `rel` covering `ctors` whose elements the total prefix of
    /// `conds`, or the arms of `rest` as [`Planner::arm_equalities`] reads them, fix for row `x`.
    pub(super) fn index_for(
        &mut self,
        rel: RelId,
        ctors: &CtorSet,
        x: Var,
        conds: &[Tm],
        rest: Tm,
    ) -> Option<IndexProbe> {
        if self.past {
            return None;
        }
        let n = self.total_prefix(conds);
        let mut fixed: Vec<(Option<usize>, Fixes)> = Vec::new();
        let mut in_arms = Vec::new();
        for (i, c) in conds.iter().enumerate().take(n) {
            match self.equality(rel, *c, x, &HashMap::new()) {
                Some(pairs) => fixed.push((Some(i), pairs)),
                None => self.cond_equalities(rel, *c, x, &mut HashMap::new(), &mut in_arms),
            }
        }
        self.arm_equalities(rel, rest, x, &mut HashMap::new(), &mut in_arms);
        if !in_arms.is_empty() {
            fixed.push((None, in_arms));
        }
        let limits: Vec<Limit> = if self.rw.range_probe && !self.bounded {
            conds[..n]
                .iter()
                .filter_map(|c| self.comparison(*c, x))
                .collect()
        } else {
            Vec::new()
        };
        if fixed.is_empty() && limits.is_empty() {
            return None;
        }
        let mut best: Option<((bool, usize, bool), IndexProbe)> = None;
        for index in &self.catalog.relation(rel).indexes {
            if !ctors.iter().all(|c| index.ctors.contains(c)) {
                continue;
            }
            let mut key = Vec::new();
            for e in &index.elements {
                let found = fixed
                    .iter()
                    .flat_map(|(_, ps)| ps)
                    .find(|(f, _)| self.is_element(rel, x, e, f));
                match found {
                    Some((_, v)) => key.push(*v),
                    None => break,
                }
            }
            let full = key.len() == index.elements.len();
            let range = match index.elements.get(key.len()) {
                Some(Element::Field(f)) if index.ordered => {
                    let bound = |below: bool| {
                        limits
                            .iter()
                            .find(|(g, b, _)| g == f && *b == below)
                            .map(|l| l.2)
                    };
                    let span = Span {
                        lo: bound(true),
                        hi: bound(false),
                    };
                    (span.lo.is_some() || span.hi.is_some()).then_some(span)
                }
                _ => None,
            };
            if (key.is_empty() && range.is_none()) || (!full && !index.ordered) {
                continue;
            }
            let covered = &index.elements[..key.len()];
            let used = fixed
                .iter()
                .filter(|(_, ps)| {
                    ps.iter().all(|(f, v)| {
                        covered.iter().zip(&key).any(|(e, k)| {
                            matches!(e, Element::Field(_))
                                && self.is_element(rel, x, e, f)
                                && k == v
                        })
                    })
                })
                .filter_map(|(i, _)| *i)
                .collect();
            let rank = (full && index.unique, key.len(), range.is_some());
            if best.as_ref().is_none_or(|b| rank > b.0) {
                let probe = IndexProbe {
                    index: index.clone(),
                    key,
                    range,
                    used,
                };
                best = Some((rank, probe));
            }
        }
        best.map(|(_, p)| p)
    }

    /// Returns the bound comparison `c` puts on a field of row `x`, when it compares the field with
    /// a term of the same type that does not mention `x`.
    fn comparison(&mut self, c: Tm, x: Var) -> Option<Limit> {
        let Kind::Prim(op, args) = self.kind(c) else {
            return None;
        };
        let (below, inclusive) = match op {
            Prim::Gt => (true, false),
            Prim::Ge => (true, true),
            Prim::Lt => (false, false),
            Prim::Le => (false, true),
            _ => return None,
        };
        let [a, b] = args[..] else { return None };
        if erase(&self.ty(a)) != erase(&self.ty(b)) {
            return None;
        }
        for (this, other, below) in [(a, b, below), (b, a, !below)] {
            if let Kind::Field(row, f) = self.kind(this)
                && matches!(self.t.kind(row), Kind::Var(v) if *v == x)
                && !self.mentions(other, &[x])
            {
                let bound = RangeBound {
                    value: other,
                    inclusive,
                };
                return Some((f, below, bound));
            }
        }
        None
    }

    /// Returns the refusal of a read of the rows `x` of `ctors` of `rel` under `conds` that no
    /// index probe reaches.
    fn unbounded(&mut self, rel: RelId, ctors: &CtorSet, x: Var, conds: &[Tm]) -> Error {
        let n = self.total_prefix(conds);
        let mut fields: Vec<String> = Vec::new();
        for c in &conds[..n] {
            for (f, _) in self
                .equality(rel, *c, x, &HashMap::new())
                .unwrap_or_default()
            {
                if let Some([PathStep::Field(f)]) = f.path.as_deref()
                    && !fields.contains(f)
                {
                    fields.push(f.clone());
                }
            }
        }
        Error::Unbounded {
            rel,
            ctors: ctors.clone(),
            fields,
        }
    }

    /// Returns the pipelines reading the rows `x` of `ctors` of `r` at the start of a query.
    pub(super) fn scan(
        &mut self,
        r: RelId,
        ctors: &CtorSet,
        x: Var,
        body: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Spine, Error> {
        let (conds, rest) = self.pulled(body);
        if (self.rw.index_scan || self.bounded)
            && let Some(probe) = self.index_for(r, ctors, x, &conds, rest)
        {
            let rows = if probe.unique() { 1.0 } else { PROBE_ROWS };
            let IndexProbe {
                index,
                key,
                range,
                used,
            } = probe;
            let left: Vec<Tm> = (0..conds.len())
                .filter(|i| !used.contains(i))
                .map(|i| conds[i])
                .collect();
            let then = Then {
                x,
                conds: &left,
                rest,
            };
            let routes = self.routes(r, ctors, &index.ctors, then, q, &sc.clone().times(rows))?;
            let ends = self.index_ends(&index, key, range, x, routes);
            return Ok(union(
                ends.into_iter()
                    .map(|end| {
                        Spine::Pipeline(Pipeline {
                            source: Source::Unit,
                            chain: chain(Vec::new(), end),
                        })
                    })
                    .collect(),
            ));
        }
        if self.bounded {
            return Err(self.unbounded(r, ctors, x, &conds));
        }
        let heaps = if self.rw.prune {
            ctors.clone()
        } else {
            self.all_ctors(r)
        };
        let mut out = Vec::new();
        for c in heaps {
            let inner = sc.bind_known(x, Some(c)).times(HEAP_ROWS);
            let ch = if ctors.contains(&c) {
                self.filtered(&conds, rest, q, &inner)?
            } else {
                nothing()
            };
            if self.rw.prune && is_nothing(&ch) {
                continue;
            }
            out.push(Spine::Pipeline(Pipeline {
                source: Source::Scan {
                    heap: self.catalog.heap(r, c),
                    ctor: c,
                    var: x,
                },
                chain: ch,
            }));
        }
        Ok(union(out))
    }

    /// Returns one route per constructor an index probe reaches, each continuing with `then`.
    fn routes(
        &mut self,
        r: RelId,
        ctors: &CtorSet,
        covered: &CtorSet,
        then: Then<'_>,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Vec<Route>, Error> {
        let reached = if self.rw.prune { ctors } else { covered };
        let mut out = Vec::new();
        for c in reached {
            let ch = if ctors.contains(c) {
                let inner = sc.bind_known(then.x, Some(*c));
                self.filtered(then.conds, then.rest, q, &inner)?
            } else {
                nothing()
            };
            if self.rw.prune && is_nothing(&ch) {
                continue;
            }
            out.push(Route {
                ctor: *c,
                heap: self.catalog.heap(r, *c),
                chain: ch,
            });
        }
        Ok(out)
    }

    /// Returns the ends probing `index`: one routing to every route, or one per route.
    fn index_ends(
        &mut self,
        index: &IndexDesc,
        key: Vec<Tm>,
        range: Option<Span>,
        x: Var,
        routes: Vec<Route>,
    ) -> Vec<End> {
        let end = |routes| match range {
            Some(Span { lo, hi }) => End::Range {
                index: index.id,
                key: key.clone(),
                lo,
                hi,
                var: x,
                routes,
            },
            None => End::Index {
                index: index.id,
                key: key.clone(),
                var: x,
                routes,
            },
        };
        if routes.is_empty() {
            Vec::new()
        } else if self.rw.fuse_probe {
            vec![end(routes)]
        } else {
            routes.into_iter().map(|r| end(vec![r])).collect()
        }
    }

    /// Returns `s` with its bounds specialised as [`Planner::here`] does.
    fn span_here(&mut self, s: Span, sc: &Scope) -> Result<Span, Error> {
        let mut at = |b: Option<RangeBound>| -> Result<Option<RangeBound>, Error> {
            b.map(|b| {
                Ok(RangeBound {
                    value: self.here(b.value, sc)?,
                    inclusive: b.inclusive,
                })
            })
            .transpose()
        };
        Ok(Span {
            lo: at(s.lo)?,
            hi: at(s.hi)?,
        })
    }

    /// Returns the chain reading the rows `y` of `ctors` of `r` for each row reaching it, then `body`.
    pub(super) fn join(
        &mut self,
        r: RelId,
        ctors: &CtorSet,
        y: Var,
        body: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Chain, Error> {
        let outer = sc;
        let (all, rest) = self.pulled(body);
        let n = self.total_prefix(&all);
        let mut pre = Vec::new();
        let mut conds = Vec::new();
        for (i, c) in all.iter().enumerate() {
            if i < n && !self.mentions(*c, &[y]) {
                pre.push(Step::Filter(self.here(*c, sc)?));
            } else {
                conds.push(*c);
            }
        }
        let sc = &sc.clone().times(0.5f64.powi(pre.len() as i32));
        let n = self.total_prefix(&conds);
        let mut options: Vec<(f64, Access)> = Vec::new();
        if (self.rw.index_join || self.bounded)
            && let Some(probe) = self.index_for(r, ctors, y, &conds, rest)
        {
            let per = if probe.unique() {
                PROBE_COST
            } else {
                PROBE_COST + PROBE_ROWS
            };
            options.push((sc.rows * per, Access::Index(probe)));
        }
        if options.is_empty() && ctors.len() > 1 && (self.rw.index_join || self.bounded) {
            let each: Vec<CtorSet> = ctors.iter().map(|&c| CtorSet::from([c])).collect();
            if each
                .iter()
                .all(|one| self.index_for(r, one, y, &conds, rest).is_some())
            {
                let arms = each
                    .iter()
                    .map(|one| self.join(r, one, y, body, q, outer))
                    .collect::<Result<_, _>>()?;
                return Ok(chain(Vec::new(), End::Fork(arms)));
            }
        }
        if self.bounded && options.is_empty() {
            return Err(self.unbounded(r, ctors, y, &conds));
        }
        let mut keys = Vec::new();
        let mut filters = Vec::new();
        for (i, c) in conds.iter().enumerate().take(n) {
            if self.free_in(*c, &sc.local) {
                if self.rw.hash_join
                    && let Kind::Prim(Prim::Eq, args) = self.kind(*c)
                    && let [a, b] = args[..]
                    && erase(&self.ty(a)) == erase(&self.ty(b))
                {
                    for (build, probe) in [(a, b), (b, a)] {
                        if self.mentions(build, &[y])
                            && !self.free_in(build, &sc.local)
                            && !self.mentions(probe, &[y])
                        {
                            keys.push((i, build, probe));
                            break;
                        }
                    }
                }
            } else if self.mentions(*c, &[y]) {
                filters.push(i);
            }
        }
        let build_rows = ctors.len() as f64 * HEAP_ROWS;
        let kept = build_rows * 0.5f64.powi(filters.len() as i32);
        if !keys.is_empty() && !self.bounded {
            options.push((
                build_rows + sc.rows,
                Access::Build {
                    keys: keys.clone(),
                    filters: filters.clone(),
                },
            ));
        }
        if !self.bounded {
            options.push((
                build_rows + sc.rows * kept,
                Access::Build {
                    keys: Vec::new(),
                    filters: filters.clone(),
                },
            ));
        }
        let (_, access) = options
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("a cross join is always an option");
        match access {
            Access::Index(probe) => {
                let rows = if probe.unique() { 1.0 } else { PROBE_ROWS };
                let IndexProbe {
                    index,
                    key,
                    range,
                    used,
                } = probe;
                let left: Vec<Tm> = (0..conds.len())
                    .filter(|i| !used.contains(i))
                    .map(|i| conds[i])
                    .collect();
                let inner = sc.clone().times(rows);
                let then = Then {
                    x: y,
                    conds: &left,
                    rest,
                };
                let routes = self.routes(r, ctors, &index.ctors, then, q, &inner)?;
                let key = key
                    .into_iter()
                    .map(|k| self.here(k, sc))
                    .collect::<Result<_, _>>()?;
                let range = range.map(|s| self.span_here(s, sc)).transpose()?;
                let mut ends = self.index_ends(&index, key, range, y, routes);
                Ok(match ends.len() {
                    0 if pre.is_empty() => nothing(),
                    0 => chain(pre, End::Nothing),
                    1 => chain(pre, ends.pop().expect("one end")),
                    _ => chain(
                        pre,
                        End::Fork(ends.into_iter().map(|e| chain(Vec::new(), e)).collect()),
                    ),
                })
            }
            Access::Build { keys, filters } => {
                let key_conds: Vec<usize> = keys.iter().map(|(i, _, _)| *i).collect();
                let build_filters: Vec<Tm> = filters.iter().map(|i| conds[*i]).collect();
                let build_keys: Vec<Tm> = keys.iter().map(|(_, b, _)| *b).collect();
                let probe: Vec<Tm> = keys.iter().map(|(_, _, p)| *p).collect();
                let build = self.build(r, ctors, y, build_keys, build_filters, q)?;
                let left: Vec<Tm> = (0..conds.len())
                    .filter(|i| !filters.contains(i) && !key_conds.contains(i))
                    .map(|i| conds[i])
                    .collect();
                let rows = if keys.is_empty() { kept } else { 1.0 };
                let known = (ctors.len() == 1).then(|| ctors[0]);
                let inner = sc.bind_known(y, known).times(rows);
                let rest = self.filtered(&left, rest, q, &inner)?;
                let probe = probe
                    .into_iter()
                    .map(|k| self.here(k, sc))
                    .collect::<Result<_, _>>()?;
                pre.push(Step::Probe {
                    build,
                    key: probe,
                    var: y,
                });
                Ok(prepend(pre, rest))
            }
        }
    }

    fn build(
        &mut self,
        r: RelId,
        ctors: &CtorSet,
        y: Var,
        key: Vec<Tm>,
        filters: Vec<Tm>,
        q: &mut QueryCx,
    ) -> Result<BuildId, Error> {
        let memo = BuildKey {
            rel: r,
            ctors: ctors.clone(),
            var: y,
            key: key.clone(),
            filters: filters.clone(),
        };
        if let Some(b) = q.memo.get(&memo) {
            return Ok(*b);
        }
        let heaps = if self.rw.prune {
            ctors.clone()
        } else {
            self.all_ctors(r)
        };
        let row = self.var_tm(y);
        let mut pipelines = Vec::new();
        for c in heaps {
            let end = if ctors.contains(&c) {
                End::Emit(row)
            } else {
                End::Nothing
            };
            let steps = if ctors.contains(&c) {
                filters.iter().map(|f| Step::Filter(*f)).collect()
            } else {
                Vec::new()
            };
            if self.rw.prune && matches!(end, End::Nothing) {
                continue;
            }
            pipelines.push(Pipeline {
                source: Source::Scan {
                    heap: self.catalog.heap(r, c),
                    ctor: c,
                    var: y,
                },
                chain: chain(steps, end),
            });
        }
        let id = BuildId(q.builds.len() as u32);
        q.builds.push(Build {
            var: y,
            key,
            pipelines,
        });
        q.memo.insert(memo, id);
        Ok(id)
    }
}
