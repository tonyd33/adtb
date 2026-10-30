use std::collections::{BTreeSet, HashMap};

use adtb_lang::kernel::{Agg, Kind, Tm, Var};
use adtb_lang::types::Ty;

use super::{Planner, QueryCx, SET_ROWS, Scope, prepend};
use crate::plan::{Chain, Error, GroupAgg, Sink, Step};

impl Planner<'_> {
    /// Returns the chain for `for g in src do body`, `src` being `group (var in source by by)`,
    /// whose aggregates over `g`'s rows are computed by the group's sink, when there is one to
    /// compute.
    pub(super) fn fused_group(
        &mut self,
        g: Var,
        src: Tm,
        body: Tm,
        q: &mut QueryCx,
        sc: &Scope,
    ) -> Result<Option<Chain>, Error> {
        let Kind::Group { var, source, by } = self.kind(src) else {
            return Ok(None);
        };
        let mut found = Found::default();
        if !self.uses(body, g, &mut found, &mut BTreeSet::new()) {
            return Ok(None);
        }
        let inside = self.facts.binders(&self.t, body);
        let strict = self.strict(body);
        let mut fused: Vec<(Tm, bool)> = Vec::new();
        for a in &found.aggs {
            let free = self.facts.free(&self.t, *a);
            if free.iter().any(|v| inside.contains(v)) {
                continue;
            }
            let total = self.total(*a);
            if total || strict.contains(a) {
                fused.push((*a, total));
            }
        }
        if fused.iter().filter(|(_, t)| !t).count() > 1 {
            fused.retain(|(_, t)| *t);
        }
        let Ty::Record(group_fields) = self.t.var_info(g).ty.clone() else {
            return Err(Error::Internal(
                "a group is a record of its key and rows".into(),
            ));
        };
        let field_ty = |name: &str| {
            group_fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone())
                .ok_or_else(|| Error::Internal("a group has a key and rows".into()))
        };
        loop {
            if fused.is_empty() {
                return Ok(None);
            }
            let stop: BTreeSet<Tm> = fused.iter().map(|(a, _)| *a).collect();
            let rows_used = self.reaches_rows(body, g, &stop, &mut BTreeSet::new());
            let names: Vec<String> = (0..fused.len()).map(|i| format!("agg{i}")).collect();
            let mut fields: Vec<(String, Ty)> = fused
                .iter()
                .zip(&names)
                .map(|((a, _), n)| (n.clone(), self.ty(*a)))
                .collect();
            fields.push(("key".into(), field_ty("key")?));
            if rows_used {
                fields.push(("rows".into(), field_ty("rows")?));
            }
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            let rec_ty = Ty::Record(fields);
            let g2 = self.fresh("%group", rec_ty.clone());
            let g2_tm = self.var_tm(g2);
            let mut map: HashMap<Tm, Tm> = HashMap::new();
            for ((a, _), n) in fused.iter().zip(&names) {
                let f = self.add(Kind::Field(g2_tm, n.clone()), self.ty(*a));
                map.insert(*a, f);
            }
            for (f, name) in [(&found.keys, "key"), (&found.rows, "rows")] {
                for t in f {
                    let n = self.add(Kind::Field(g2_tm, name.into()), self.ty(*t));
                    map.insert(*t, n);
                }
            }
            let new_body = self.replace(body, &map, &mut HashMap::new());
            if fused.iter().any(|(_, t)| !t) && !self.total(new_body) {
                fused.retain(|(_, t)| *t);
                continue;
            }
            let mut aggs = Vec::new();
            for ((a, _), n) in fused.iter().zip(&names) {
                aggs.push(self.group_agg(*a, n.clone())?);
            }
            let sink = Sink::Group {
                var,
                by: self.eval(by)?,
                rows: rows_used,
                aggs,
            };
            let groups = self.query(source, sink, None)?;
            let v = self.fresh("%groups", Ty::Set(Box::new(rec_ty)));
            let v_tm = self.var_tm(v);
            let rest = self.chain(new_body, q, &sc.bind(g2).times(SET_ROWS))?;
            return Ok(Some(prepend(
                vec![Step::Bind(v, groups), Step::Unnest(g2, v_tm)],
                rest,
            )));
        }
    }

    /// Returns whether `t` uses the rows of group `g` outside the terms of `stop`.
    fn reaches_rows(
        &mut self,
        t: Tm,
        g: Var,
        stop: &BTreeSet<Tm>,
        seen: &mut BTreeSet<Tm>,
    ) -> bool {
        if stop.contains(&t) || !seen.insert(t) {
            return false;
        }
        if let Kind::Field(x, l) = self.kind(t)
            && l == "rows"
            && matches!(self.t.kind(x), Kind::Var(v) if *v == g)
        {
            return true;
        }
        self.t
            .children(t)
            .into_iter()
            .any(|c| self.reaches_rows(c, g, stop, seen))
    }

    fn group_agg(&mut self, a: Tm, field: String) -> Result<GroupAgg, Error> {
        let Kind::Agg {
            op,
            var,
            source,
            body,
        } = self.kind(a)
        else {
            panic!("a fused aggregate is an aggregate")
        };
        let Kind::For(row, _, inner) = self.kind(source) else {
            panic!("a fused aggregate ranges over the group's rows")
        };
        let (conds, elem) = self.peel(inner);
        let Kind::Singleton(element) = self.kind(elem) else {
            panic!("a fused aggregate's range selects one element per row")
        };
        let mut filter = Vec::new();
        for c in conds {
            filter.push(self.eval(c)?);
        }
        Ok(GroupAgg {
            field,
            op,
            ty: self.ty(a),
            body_ty: self.ty(body),
            row,
            filter,
            elem: var,
            element: self.eval(element)?,
            body: self.eval(body)?,
        })
    }

    /// Records the uses of group variable `g` in `t`; returns false when one is neither its key, its rows nor an aggregate over its rows.
    fn uses(&mut self, t: Tm, g: Var, found: &mut Found, seen: &mut BTreeSet<Tm>) -> bool {
        if !seen.insert(t) {
            return true;
        }
        let is_g = |p: &Self, x: Tm| matches!(p.t.kind(x), Kind::Var(v) if *v == g);
        match self.kind(t) {
            Kind::Var(v) => v != g,
            Kind::Field(x, l) if is_g(self, x) => {
                match l.as_str() {
                    "key" => found.keys.push(t),
                    _ => found.rows.push(t),
                }
                true
            }
            Kind::Agg {
                op, source, body, ..
            } if !matches!(op, Agg::User(_))
                && self.rows_agg(source, g)
                && !self.mentions(body, &[g]) =>
            {
                found.aggs.push(t);
                if let Kind::For(_, rows, _) = self.kind(source) {
                    found.rows.push(rows);
                }
                true
            }
            _ => self
                .t
                .children(t)
                .into_iter()
                .all(|c| self.uses(c, g, found, seen)),
        }
    }

    /// Returns whether `source` selects one distinct element per row of group `g` that passes its conditions.
    fn rows_agg(&mut self, source: Tm, g: Var) -> bool {
        let Kind::For(row, rows, inner) = self.kind(source) else {
            return false;
        };
        let Kind::Field(x, l) = self.kind(rows) else {
            return false;
        };
        if l != "rows"
            || !matches!(self.t.kind(x), Kind::Var(v) if *v == g)
            || self.mentions(inner, &[g])
        {
            return false;
        }
        let (_, elem) = self.peel(inner);
        let Kind::Singleton(e) = self.kind(elem) else {
            return false;
        };
        let is_row = |p: &Self, t: Tm| matches!(p.t.kind(t), Kind::Var(v) if *v == row);
        match self.kind(e) {
            Kind::Var(v) => v == row,
            Kind::Record(fs) | Kind::Ctor { fields: fs, .. } => {
                fs.iter().any(|(_, f)| is_row(self, *f))
            }
            _ => false,
        }
    }

    /// Returns the subterms of `t` evaluated whenever `t` is.
    fn strict(&self, t: Tm) -> BTreeSet<Tm> {
        let mut out = BTreeSet::new();
        let mut stack = vec![t];
        while let Some(t) = stack.pop() {
            if !out.insert(t) {
                continue;
            }
            match self.t.kind(t) {
                Kind::If(c, ..) => stack.push(*c),
                Kind::Case { scrutinee, .. } => stack.push(*scrutinee),
                Kind::For(_, s, _)
                | Kind::Agg { source: s, .. }
                | Kind::Group { source: s, .. } => stack.push(*s),
                Kind::Fix(..) | Kind::Top { .. } | Kind::Present { .. } | Kind::Try(_) => {}
                _ => stack.extend(self.t.children(t)),
            }
        }
        out
    }

    fn replace(&mut self, t: Tm, map: &HashMap<Tm, Tm>, memo: &mut HashMap<Tm, Tm>) -> Tm {
        if let Some(n) = map.get(&t) {
            memo.insert(t, *n);
            return *n;
        }
        if let Some(n) = memo.get(&t) {
            return *n;
        }
        let cs = self.t.children(t);
        let new: Vec<Tm> = cs.iter().map(|c| self.replace(*c, map, memo)).collect();
        let out = if new == cs {
            t
        } else {
            let kind = self.t.kind(t).with_children(&new);
            self.add(kind, self.ty(t))
        };
        memo.insert(t, out);
        out
    }
}

#[derive(Default)]
struct Found {
    keys: Vec<Tm>,
    rows: Vec<Tm>,
    aggs: Vec<Tm>,
}
