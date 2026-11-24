use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::kernel::{Conflict, Kind, Tm, Var};
use adtb_lang::types::{RelId, Ty};

use crate::exec::types::ctors;
use crate::exec::{self, Footprint, Held, Val};
use crate::runtime::Runtime;

use super::eval::{Eff, Effects, Env, Stmt, bind};
use super::input::{Input, is_temp};
use super::{Broken, Error, Rejection, TermKind};

/// A record of a source that an insert with `rejects into` rejected.
pub struct Rejected {
    line: i64,
    source: String,
    raw: String,
    reason: String,
}

impl Footprint for Rejected {
    fn footprint(&self) -> usize {
        size_of::<Rejected>() + self.source.len() + self.raw.len() + self.reason.len()
    }
}

impl<R: Runtime> Stmt<'_, R> {
    /// Evaluate a `rejects into` write of `insert`, adding its effects to `out`, each rejected
    /// record as a row of `into`. Returns the value a `let` binds to it.
    pub(super) fn rejects(
        &self,
        into: RelId,
        insert: Tm,
        env: &Env,
        out: &mut Effects,
    ) -> Result<Held<Val>, Error> {
        let rows = match self.terms().kind(insert) {
            Kind::Insert(_, rows) | Kind::Upsert { rows, .. } => *rows,
            k => {
                return Err(Broken::RejectsWraps(TermKind(k.clone())).into());
            }
        };
        let mut rejected: Held<Vec<Rejected>> = self.rows_in().held();
        let rows = self.records(rows, env, Some(&mut rejected))?;
        let bound = match self.terms().kind(insert) {
            Kind::Insert(r, _) => {
                for v in rows.iter() {
                    out.insert(Eff::Ins(*r, v.clone()))?;
                }
                rows.into_value()?
            }
            _ => self.upsert(insert, rows, env, out)?,
        };
        if !rejected.is_empty() {
            let load = self.load(into)?;
            for j in rejected {
                out.insert(Eff::Ins(into, self.rejected_row(into, load, j)?))?;
            }
        }
        Ok(bound)
    }

    /// Returns the rows an insert's `rows` term computes. When `rows` ranges over one source
    /// relation and its evaluation fails, each record is computed on its own, in the order of
    /// the source's key: a record whose computation reaches `reject` goes to `rejected`, or
    /// rejects the statement with `BadInput` naming it when there is none. Over a file scan,
    /// each record is computed on its own in the order of the files, and a record that does not
    /// convert is rejected as well.
    pub(super) fn records(
        &self,
        rows: Tm,
        env: &Env,
        mut rejected: Option<&mut Held<Vec<Rejected>>>,
    ) -> Result<Held<BTreeSet<Val>>, Error> {
        let source = match self.terms().kind(rows) {
            Kind::For(x, src, body) => match self.terms().kind(*src) {
                Kind::Rel(r, _) => Some((*x, *r, *src, *body)),
                Kind::Var(v) => match self.input(v) {
                    Some(Input::Temp { rel, .. } | Input::Volatile { rel, .. }) => {
                        Some((*x, *rel, *src, *body))
                    }
                    Some(Input::File { .. }) => {
                        return self.file_records(*v, (*x, *body), env, rejected);
                    }
                    None => None,
                },
                _ => None,
            },
            _ => None,
        };
        let Some((x, r0, src, body)) = source else {
            return self.set(rows, env);
        };
        match self.set(rows, env) {
            Err(
                Error::Rejected(Rejection::BadInput { .. })
                | Error::Exec(exec::Error::Rejected(_) | exec::Error::Fault(_)),
            ) => {}
            other => return other,
        }
        let mut keyed: Held<Vec<(Val, Val)>> = self.grant().held();
        for p in self.set(src, env)? {
            keyed.push((self.key_of(r0, &p)?, p))?;
        }
        keyed.sort();
        let source = self.schema().relation(r0).name.clone();
        let mut out: Held<BTreeSet<Val>> = self.grant().held();
        for (i, (_, p)) in keyed.into_iter().enumerate() {
            let line = i as i64 + 1;
            match self.set(body, &bind(env, x, p.clone())) {
                Ok(vs) => out.append(vs),
                Err(
                    Error::Rejected(Rejection::BadInput { detail, .. })
                    | Error::Exec(exec::Error::Rejected(detail)),
                ) => match &mut rejected {
                    Some(js) => {
                        let ty = self.terms().var_info(x).ty.clone();
                        js.push(Rejected {
                            line,
                            source: source.clone(),
                            raw: exec::print(self.schema(), &ty, &p)?,
                            reason: detail,
                        })?;
                    }
                    None => {
                        return Err(Error::Rejected(Rejection::BadInput {
                            source,
                            line,
                            detail,
                        }));
                    }
                },
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// Returns the rows `body` computes for each record of the file scan `scan`, bound to `x`. A
    /// record that does not convert, or whose computation reaches `reject`, goes to `rejected`,
    /// or rejects the statement with `BadInput` naming it when there is none.
    fn file_records(
        &self,
        scan: Var,
        (x, body): (Var, Tm),
        env: &Env,
        mut rejected: Option<&mut Held<Vec<Rejected>>>,
    ) -> Result<Held<BTreeSet<Val>>, Error> {
        let records = self.scan_records(scan)?;
        let mut out: Held<BTreeSet<Val>> = self.grant().held();
        for r in records {
            let reason = match r.row {
                Ok(p) => match self.set(body, &bind(env, x, p)) {
                    Ok(vs) => {
                        out.append(vs);
                        continue;
                    }
                    Err(
                        Error::Rejected(Rejection::BadInput { detail, .. })
                        | Error::Exec(exec::Error::Rejected(detail)),
                    ) => detail,
                    Err(e) => return Err(e),
                },
                Err(reason) => reason,
            };
            match &mut rejected {
                Some(js) => js.push(Rejected {
                    line: r.line,
                    source: r.source,
                    raw: r.raw,
                    reason,
                })?,
                None => {
                    return Err(Error::Rejected(Rejection::BadInput {
                        source: r.source,
                        line: r.line,
                        detail: reason,
                    }));
                }
            }
        }
        Ok(out)
    }

    /// Returns the `load` of the rows the statement inserts into `into`: one value per
    /// statement, past every value issued before and every one `into` holds.
    fn load(&self, into: RelId) -> Result<i64, Error> {
        if let Some(l) = self.drawn_load().get() {
            return Ok(l);
        }
        let at = self
            .rejected_fields(into)?
            .iter()
            .position(|n| n == "load")
            .expect("`RejectedRow` has a `load`");
        let rows: Vec<Val> = if is_temp(self.schema(), into) {
            self.local()
                .borrow()
                .rows(into, None)?
                .into_iter()
                .collect()
        } else {
            self.rows(into, None)?.into_iter().map(|(_, v)| v).collect()
        };
        let mut held = 0;
        for v in rows {
            if let Val::Int(l) = v.fields()?[at] {
                held = held.max(l);
            }
        }
        let l = self.db.next_load(held)?;
        self.drawn_load().set(Some(l));
        Ok(l)
    }

    /// Returns the names of the fields of a row of `into`, a relation of `RejectedRow`, in
    /// declaration order.
    fn rejected_fields(&self, into: RelId) -> Result<Vec<String>, Error> {
        let elem = &self.schema().relation(into).elem;
        let cs = ctors(self.schema(), elem).ok_or(Broken::RejectedRowNotRecord)?;
        Ok(cs[0].iter().map(|(n, _)| n.clone()).collect())
    }

    fn rejected_row(&self, into: RelId, load: i64, j: Rejected) -> Result<Val, Error> {
        let fields = self
            .rejected_fields(into)?
            .iter()
            .map(|n| match n.as_str() {
                "load" => Val::Int(load),
                "line" => Val::Int(j.line),
                "source" => Val::text(&j.source),
                "raw" => Val::text(&j.raw),
                "reason" => Val::text(&j.reason),
                other => panic!("`RejectedRow` has no field `{other}`"),
            })
            .collect();
        Ok(Val::ctor(0, fields))
    }

    pub(super) fn upsert(
        &self,
        t: Tm,
        rows: Held<BTreeSet<Val>>,
        env: &Env,
        out: &mut Effects,
    ) -> Result<Held<Val>, Error> {
        let Kind::Upsert {
            rel,
            on,
            old,
            new,
            otherwise,
            single,
            ..
        } = self.terms().kind(t).clone()
        else {
            panic!("an upsert")
        };
        let mut bound: Held<BTreeSet<Val>> = self.grant().held();
        for n in rows {
            let olds = self.conflicts(rel, on, &n)?;
            if olds.is_empty() {
                out.insert(Eff::Ins(rel, n.clone()))?;
                bound.insert(n)?;
                continue;
            }
            for o in olds {
                if let Ty::Data {
                    ctors: Some(visible),
                    ..
                } = &self.terms().var_info(old).ty
                    && !visible.contains(&o.ctor_of()?)
                {
                    out.insert(Eff::Ins(rel, n.clone()))?;
                    bound.insert(n.clone())?;
                    continue;
                }
                let mut ew: Effects = self.rows_in().held();
                let inner = bind(&bind(env, old, o.clone()), new, n.clone());
                self.write(otherwise, &inner, &mut ew)?;
                let k = self.key_of(rel, &o)?;
                let mut wrote = false;
                for e in ew.iter() {
                    if let Eff::Rep(r, _, v) | Eff::Ins(r, v) = e
                        && *r == rel
                        && self.key_of(rel, v)? == k
                    {
                        bound.insert(v.clone())?;
                        wrote = true;
                    }
                }
                if !wrote {
                    bound.insert(o)?;
                }
                out.append(ew);
            }
        }
        if single {
            let row = (bound.into_iter().next()).expect("a single-row upsert binds a row");
            Ok(self.grant().hold(row)?)
        } else {
            Ok(bound.into_value()?)
        }
    }

    /// Returns the rows of `rel` in the snapshot whose values of `on` equal those of `n`.
    fn conflicts(&self, rel: RelId, on: Conflict, n: &Val) -> Result<Vec<Val>, Error> {
        match on {
            Conflict::Key if is_temp(self.schema(), rel) => {
                let k = self.key_of(rel, n)?;
                Ok(self
                    .local()
                    .borrow()
                    .by_key(rel, &k)
                    .cloned()
                    .into_iter()
                    .collect())
            }
            Conflict::Key => {
                let k = self.key_of(rel, n)?;
                Ok(self
                    .by_key(rel, &k)?
                    .into_iter()
                    .map(|(_, _, v)| v)
                    .collect())
            }
            Conflict::Unique(i) => {
                let r = self.rel(rel);
                let u = r
                    .uniques
                    .iter()
                    .find(|u| u.pos == i as usize)
                    .ok_or(Broken::NoUnique(i))?;
                let (_, ix) = r
                    .indexes
                    .iter()
                    .find(|(id, _)| *id == u.index)
                    .unwrap_or_else(|| panic!("a unique's index is one of its relation's"));
                let c = n.ctor_of()?;
                let types = self.store.key_types(u.index)?;
                let row = self.stored(r.ctors[c as usize].heap, n)?;
                let Some(key) = ix.keys.key(c, &row, &types)? else {
                    return Ok(Vec::new());
                };
                let schema = self.db.bindings.indexes()[&u.index].index.schema();
                let want = schema.encode(&key)?;
                let scope = r.routes(u.scope.iter().copied());
                let mut out = BTreeMap::new();
                for n in self.probe(u.index, &key, &scope)? {
                    let d = n.ctor;
                    let Some((_, stored)) = self.store.fetch(r.ctors[d as usize].heap, &n)? else {
                        continue;
                    };
                    let found = ix.keys.key(d, &stored, &types)?;
                    let found = found.map(|k| schema.encode(&k)).transpose()?;
                    if found.as_ref() == Some(&want) {
                        let o = self.row(rel, d, &stored)?;
                        out.insert(self.key_of(rel, &o)?, o);
                    }
                }
                Ok(out.into_values().collect())
            }
        }
    }
}
