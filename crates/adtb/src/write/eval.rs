use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use adtb_lang::kernel::{Conflict, Kind, PathStep, Terms, Tm, Var};
use adtb_lang::schema::{Generator, OnDelete, Stage};
use adtb_lang::types::{RelId, Ty};

use crate::exec::types::{ctors, field};
use crate::exec::{self, Val, from_stored, to_key};
use crate::key::Tid;
use crate::runtime::Runtime;

use super::apply::Undo;
use super::fresh::{self, Occurrences};
use super::schema::{RefField, Rel};
use super::{Database, Error, Outcome, Rejection, Rows, Txn};

pub type Env = BTreeMap<Var, Val>;

/// One effect of a write.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Eff {
    Ins(RelId, Val),
    Del(RelId, Val),
    Rep(RelId, Val, Val),
}

pub type Effects = BTreeSet<Eff>;

/// What a constraint is, for the reason a violation of it gives.
#[derive(Debug)]
pub enum Violation {
    /// Rows of a relation share these keys.
    Key(RelId, BTreeSet<Val>),
    Other(BTreeSet<Val>),
}

/// The violations a statement found, by constraint name.
pub type Violations = BTreeMap<String, Violation>;

pub fn violate_key(v: &mut Violations, name: &str, rel: RelId, key: Val) {
    match v
        .entry(name.to_string())
        .or_insert_with(|| Violation::Key(rel, BTreeSet::new()))
    {
        Violation::Key(_, keys) => {
            keys.insert(key);
        }
        Violation::Other(_) => unreachable!("a key's name names no other constraint"),
    }
}

pub fn violate(v: &mut Violations, name: &str, rows: impl IntoIterator<Item = Val>) {
    let rows: BTreeSet<Val> = rows.into_iter().collect();
    if rows.is_empty() {
        return;
    }
    match v
        .entry(name.to_string())
        .or_insert_with(|| Violation::Other(BTreeSet::new()))
    {
        Violation::Other(report) => report.extend(rows),
        Violation::Key(..) => unreachable!("a constraint's name names no key"),
    }
}

/// Returns the pairs of distinct rows of every group with more than one, as a unique's report.
pub fn pairs<'a>(groups: impl IntoIterator<Item = &'a BTreeMap<Val, Val>>) -> BTreeSet<Val> {
    let mut out = BTreeSet::new();
    for g in groups {
        for x in g.values() {
            for y in g.values() {
                if x != y {
                    out.insert(Val::record(vec![x.clone(), y.clone()]));
                }
            }
        }
    }
    out
}

/// A record of a source that an insert with `rejects into` rejected.
pub struct Rejected {
    line: i64,
    source: String,
    raw: String,
    reason: String,
}

/// The arena a term belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Arena {
    Statement,
    Program,
}

/// A term planned with the variables bound around it.
type PlanKey = (Arena, Tm, Vec<Var>);

/// One statement of a transaction.
pub struct Stmt<'a, R: Runtime> {
    pub db: &'a Database<R>,
    pub tx: &'a Txn,
    terms: Terms,
    fresh: Occurrences,
    root: Tm,
    clock: i64,
    pub store: Arc<exec::Store<R>>,
    drawn: RefCell<BTreeMap<(Arena, Val), Val>>,
    load: Cell<Option<i64>>,
    pub undo: RefCell<Vec<Undo>>,
    plans: RefCell<HashMap<PlanKey, Arc<exec::Program>>>,
}

impl<'a, R: Runtime> Stmt<'a, R> {
    pub fn new(db: &'a Database<R>, tx: &'a Txn, terms: &Terms, root: Tm, clock: i64) -> Self {
        let mut terms = terms.clone();
        let mut fresh = Occurrences::new();
        let root = fresh::rewrite(&mut terms, root, &[], &mut fresh);
        Stmt {
            db,
            tx,
            terms,
            fresh,
            root,
            clock,
            store: db.store(tx),
            drawn: RefCell::default(),
            load: Cell::default(),
            undo: RefCell::default(),
            plans: RefCell::default(),
        }
    }

    /// Returns a statement that only reads.
    pub fn reader(db: &'a Database<R>, tx: &'a Txn) -> Self {
        Stmt {
            db,
            tx,
            terms: Terms::default(),
            fresh: Occurrences::new(),
            root: Tm(0),
            clock: 0,
            store: db.store(tx),
            drawn: RefCell::default(),
            load: Cell::default(),
            undo: RefCell::default(),
            plans: RefCell::default(),
        }
    }

    pub fn rel(&self, r: RelId) -> &'a Rel {
        self.db
            .rels
            .get(&r)
            .unwrap_or_else(|| panic!("the catalog describes relation {r:?}"))
    }

    fn arena(&self, a: Arena) -> &Terms {
        match a {
            Arena::Statement => &self.terms,
            Arena::Program => &self.db.program,
        }
    }

    pub fn run(&self, params: &[(Var, Val)]) -> Result<Outcome, Error> {
        let env: Env = params.iter().cloned().collect();
        let mut effects = Effects::new();
        let t = self.root;
        let result = match self.terms.kind(t) {
            Kind::Transact(b, _) => self.block(*b, &env, &mut effects)?,
            _ if *self.terms.ty(t) == Ty::Change => {
                self.write(t, &env, &mut effects)?;
                Val::record(Vec::new())
            }
            _ => {
                return Ok(Outcome {
                    result: self.eval(Arena::Statement, t, &env)?,
                    inserted: Rows::new(),
                    deleted: Rows::new(),
                });
            }
        };
        self.boundary(effects, result)
    }

    /// Returns the value of term `t` of arena `a` in `env`, its placeholders filled.
    pub fn eval(&self, a: Arena, t: Tm, env: &Env) -> Result<Val, Error> {
        let params: Vec<Var> = env.keys().copied().collect();
        let key = (a, t, params.clone());
        let cached = self.plans.borrow().get(&key).cloned();
        let program = match cached {
            Some(p) => p,
            None => {
                let db = self.db;
                let p = crate::plan::plan(
                    &db.schema,
                    &db.catalog,
                    self.arena(a),
                    t,
                    &params,
                    crate::plan::Options::default(),
                )
                .map_err(|e| Error::Unsupported(e.to_string()))?;
                let x = Arc::new(exec::compile(&db.schema, &db.catalog, &p, None)?);
                self.plans.borrow_mut().insert(key, x.clone());
                x
            }
        };
        let call = self.db.call(&self.store)?;
        let bound: Vec<(Var, Val)> = env.iter().map(|(v, x)| (*v, x.clone())).collect();
        let v = exec::execute(self.db.rt.clone(), program, call, &bound)?;
        let ty = self.arena(a).ty(t).clone();
        self.fill(a, &ty, v)
    }

    fn fill(&self, a: Arena, ty: &Ty, v: Val) -> Result<Val, Error> {
        let occurrences = match a {
            Arena::Statement => &self.fresh,
            Arena::Program => &self.db.program_fresh,
        };
        let mut draw = |p: &Val| {
            let key = (a, p.clone());
            if let Some(v) = self.drawn.borrow().get(&key) {
                return Ok(v.clone());
            }
            let occ = p.fields()[0].int();
            let (g, r) = occurrences[&occ];
            let v = self.draw(g, r)?;
            self.drawn.borrow_mut().insert(key, v.clone());
            Ok(v)
        };
        fresh::fill(&self.db.schema, ty, v, &mut draw)
    }

    fn draw(&self, g: Generator, r: RelId) -> Result<Val, Error> {
        Ok(match g {
            Generator::Clock => Val::Instant(self.clock),
            Generator::Sequence => Val::Int(self.db.next_sequence(r)?),
            Generator::Uuidv7 => {
                let ms = self.clock.div_euclid(1000) as u64;
                let n = self
                    .db
                    .uuids
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let mut b = [0u8; 16];
                b[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
                let rand = self.db.rt.next_u64();
                b[6] = 0x70 | (rand as u8 & 0x0f);
                b[7] = (rand >> 8) as u8;
                b[8..].copy_from_slice(&n.to_be_bytes());
                b[8] = 0x80 | (b[8] & 0x3f);
                Val::Bytes(Arc::from(b.as_slice()))
            }
        })
    }

    fn set(&self, t: Tm, env: &Env) -> Result<BTreeSet<Val>, Error> {
        match self.eval(Arena::Statement, t, env)? {
            Val::Set(s) => Ok(s.as_ref().clone()),
            other => panic!("a write's rows are a set, found {other:?}"),
        }
    }

    /// Evaluates block term `t`, adding the effects of its writes to `out`. Returns its result.
    fn block(&self, t: Tm, env: &Env, out: &mut Effects) -> Result<Val, Error> {
        match self.terms.kind(t) {
            Kind::Block(w, k) => {
                let (w, k) = (*w, *k);
                self.write(w, env, out)?;
                self.block(k, env, out)
            }
            Kind::LetBang(x, w, k) => {
                let (x, w, k) = (*x, *w, *k);
                let v = self.bind_write(w, env, out)?;
                self.block(k, &bind(env, x, v), out)
            }
            Kind::Let(x, e, k) => {
                let (x, e, k) = (*x, *e, *k);
                let v = self.eval(Arena::Statement, e, env)?;
                self.block(k, &bind(env, x, v), out)
            }
            _ => self.eval(Arena::Statement, t, env),
        }
    }

    /// Evaluates write `t`, adding its effects to `out`.
    fn write(&self, t: Tm, env: &Env, out: &mut Effects) -> Result<(), Error> {
        match self.terms.kind(t).clone() {
            Kind::Skip => Ok(()),
            Kind::Insert(r, rows) => {
                for v in self.records(rows, env, None)? {
                    out.insert(Eff::Ins(r, v));
                }
                Ok(())
            }
            Kind::Delete(r, rows) => {
                for v in self.set(rows, env)? {
                    if self.in_snapshot(r, &v)? {
                        out.insert(Eff::Del(r, v));
                    }
                }
                Ok(())
            }
            Kind::Replace(..) | Kind::Upsert { .. } | Kind::Rejects { .. } => {
                self.bind_write(t, env, out).map(drop)
            }
            Kind::Par(ws) => ws.iter().try_for_each(|w| self.write(*w, env, out)),
            Kind::Fail(m) => {
                let Val::Text(message) = self.eval(Arena::Statement, m, env)? else {
                    panic!("`fail` takes a Text")
                };
                Err(Error::Rejected(Rejection::UserError {
                    message: message.to_string(),
                }))
            }
            Kind::If(c, a, b) => {
                let w = if self.eval(Arena::Statement, c, env)?.truth() {
                    a
                } else {
                    b
                };
                self.write(w, env, out)
            }
            Kind::Case { scrutinee, arms } => {
                let v = self.eval(Arena::Statement, scrutinee, env)?;
                let c = v.ctor_of();
                let (_, x, w) = arms
                    .iter()
                    .find(|(d, _, _)| *d == c)
                    .expect("an arm per constructor");
                self.write(*w, &bind(env, *x, v), out)
            }
            Kind::Let(x, e, w) => {
                let v = self.eval(Arena::Statement, e, env)?;
                self.write(w, &bind(env, x, v), out)
            }
            Kind::Unreachable => panic!("evaluation reached a position exhaustiveness excludes"),
            k => panic!("{k:?} is not a write"),
        }
    }

    /// Evaluates a `Replace` or `Upsert`, adding its effects to `out`. Returns the value a `let`
    /// binds to it.
    fn bind_write(&self, t: Tm, env: &Env, out: &mut Effects) -> Result<Val, Error> {
        match self.terms.kind(t).clone() {
            Kind::Replace(r, rows, f) => {
                let Kind::Lam(x, body) = *self.terms.kind(f) else {
                    panic!("a replace's new value is a `Lam`")
                };
                let mut news = BTreeSet::new();
                for v in self.set(rows, env)? {
                    let n = self.eval(Arena::Statement, body, &bind(env, x, v.clone()))?;
                    if self.key_of(r, &n) != self.key_of(r, &v) {
                        return Err(Error::Rejected(Rejection::KeyChanged { relation: r }));
                    }
                    if n != v && self.in_snapshot(r, &v)? {
                        out.insert(Eff::Rep(r, v, n.clone()));
                    }
                    news.insert(n);
                }
                Ok(Val::set(news))
            }
            Kind::Upsert { rows, .. } => {
                let rows = self.records(rows, env, None)?;
                self.upsert(t, rows, env, out)
            }
            Kind::Rejects { into, insert } => {
                let rows = match self.terms.kind(insert) {
                    Kind::Insert(_, rows) | Kind::Upsert { rows, .. } => *rows,
                    k => panic!("`rejects into` wraps an insert or upsert, not {k:?}"),
                };
                let mut rejected = Vec::new();
                let rows = self.records(rows, env, Some(&mut rejected))?;
                let bound = match self.terms.kind(insert) {
                    Kind::Insert(r, _) => {
                        out.extend(rows.iter().map(|v| Eff::Ins(*r, v.clone())));
                        Val::set(rows)
                    }
                    _ => self.upsert(insert, rows, env, out)?,
                };
                if !rejected.is_empty() {
                    let load = self.load(into)?;
                    for j in rejected {
                        out.insert(Eff::Ins(into, self.rejected_row(into, load, j)));
                    }
                }
                Ok(bound)
            }
            k => panic!("{k:?} binds no rows"),
        }
    }

    /// Returns the rows an insert's `rows` term computes. When `rows` ranges over one source
    /// relation and its evaluation fails, each record is computed on its own, in the order of
    /// the source's key: a record whose computation reaches `reject` goes to `rejected`, or
    /// rejects the statement with `BadInput` naming it when there is none.
    fn records(
        &self,
        rows: Tm,
        env: &Env,
        mut rejected: Option<&mut Vec<Rejected>>,
    ) -> Result<BTreeSet<Val>, Error> {
        let source = match self.terms.kind(rows) {
            Kind::For(x, src, body) => match self.terms.kind(*src) {
                Kind::Rel(r, _) => Some((*x, *r, *src, *body)),
                _ => None,
            },
            _ => None,
        };
        let Some((x, r0, src, body)) = source else {
            return self.set(rows, env);
        };
        match self.set(rows, env) {
            Err(Error::Rejected(Rejection::BadInput { .. }) | Error::Fault(_)) => {}
            other => return other,
        }
        let mut records: Vec<Val> = self.set(src, env)?.into_iter().collect();
        records.sort_by_cached_key(|p| self.key_of(r0, p));
        let source = self.db.schema.relation(r0).name.clone();
        let mut out = BTreeSet::new();
        for (i, p) in records.into_iter().enumerate() {
            let line = i as i64 + 1;
            match self.set(body, &bind(env, x, p.clone())) {
                Ok(vs) => out.extend(vs),
                Err(Error::Rejected(Rejection::BadInput { detail, .. })) => match &mut rejected {
                    Some(js) => {
                        let ty = self.terms.var_info(x).ty.clone();
                        js.push(Rejected {
                            line,
                            source: source.clone(),
                            raw: exec::print(&self.db.schema, &ty, &p)?,
                            reason: detail,
                        });
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

    /// Returns the `load` of the rows the statement inserts into `into`: one value per
    /// statement, past every value issued before and every one `into` holds.
    fn load(&self, into: RelId) -> Result<i64, Error> {
        if let Some(l) = self.load.get() {
            return Ok(l);
        }
        let at = self.rel(into).ctors[0]
            .fields
            .iter()
            .position(|(n, _)| n == "load")
            .expect("`RejectedRow` has a `load`");
        let held = self
            .rows(into, None)?
            .into_iter()
            .filter_map(|(_, v)| match v.fields()[at] {
                Val::Int(l) => Some(l),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let l = self.db.next_load(held)?;
        self.load.set(Some(l));
        Ok(l)
    }

    fn rejected_row(&self, into: RelId, load: i64, j: Rejected) -> Val {
        let fields = self.rel(into).ctors[0]
            .fields
            .iter()
            .map(|(n, _)| match n.as_str() {
                "load" => Val::Int(load),
                "line" => Val::Int(j.line),
                "source" => Val::text(&j.source),
                "raw" => Val::text(&j.raw),
                "reason" => Val::text(&j.reason),
                other => panic!("`RejectedRow` has no field `{other}`"),
            })
            .collect();
        Val::ctor(0, fields)
    }

    fn upsert(
        &self,
        t: Tm,
        rows: BTreeSet<Val>,
        env: &Env,
        out: &mut Effects,
    ) -> Result<Val, Error> {
        let Kind::Upsert {
            rel,
            on,
            old,
            new,
            otherwise,
            single,
            ..
        } = self.terms.kind(t).clone()
        else {
            panic!("an upsert")
        };
        let mut bound = BTreeSet::new();
        for n in rows {
            let olds = self.conflicts(rel, on, &n)?;
            if olds.is_empty() {
                out.insert(Eff::Ins(rel, n.clone()));
                bound.insert(n);
                continue;
            }
            for o in olds {
                let mut ew = Effects::new();
                let inner = bind(&bind(env, old, o.clone()), new, n.clone());
                self.write(otherwise, &inner, &mut ew)?;
                let k = self.key_of(rel, &o);
                let written: Vec<Val> = ew
                    .iter()
                    .filter_map(|e| match e {
                        Eff::Rep(r, _, v) | Eff::Ins(r, v) if *r == rel => Some(v),
                        _ => None,
                    })
                    .filter(|v| self.key_of(rel, v) == k)
                    .cloned()
                    .collect();
                if written.is_empty() {
                    bound.insert(o);
                } else {
                    bound.extend(written);
                }
                out.extend(ew);
            }
        }
        if single {
            Ok(bound
                .into_iter()
                .next()
                .expect("a single-row upsert binds a row"))
        } else {
            Ok(Val::set(bound))
        }
    }

    /// Returns the rows of `rel` in the snapshot whose values of `on` equal those of `n`.
    fn conflicts(&self, rel: RelId, on: Conflict, n: &Val) -> Result<Vec<Val>, Error> {
        match on {
            Conflict::Key => {
                let k = self.key_of(rel, n);
                Ok(self
                    .by_key(rel, &k)?
                    .into_iter()
                    .map(|(_, _, v)| v)
                    .collect())
            }
            Conflict::Unique(i) => {
                let r = self.rel(rel);
                let pos = r
                    .uniques
                    .iter()
                    .position(|u| u.pos == i as usize)
                    .expect("an upsert names a unique");
                let u = &r.uniques[pos];
                let Some(want) = self.unique_values(rel, pos, n)? else {
                    return Ok(Vec::new());
                };
                let mut out = Vec::new();
                for (_, o) in self.rows(rel, Some(&u.scope))? {
                    if self.unique_values(rel, pos, &o)?.as_ref() == Some(&want) {
                        out.push(o);
                    }
                }
                Ok(out)
            }
        }
    }

    /// Returns the values of the fields of unique `u` of `rel` on `row`, or `None` when `row`
    /// holds another constructor than a field's path names.
    pub fn unique_values(
        &self,
        rel: RelId,
        u: usize,
        row: &Val,
    ) -> Result<Option<Vec<Val>>, Error> {
        let r = self.rel(rel);
        let unique = &r.uniques[u];
        if !unique.scope.contains(&row.ctor_of()) {
            return Ok(None);
        }
        let mut out = Vec::new();
        for (var, f) in &unique.fields {
            if !self.holds_path(&r.row_ty(row.ctor_of()), row, &f.path)? {
                return Ok(None);
            }
            let env: Env = [(*var, row.clone())].into();
            out.push(self.eval(Arena::Program, f.value, &env)?);
        }
        Ok(Some(out))
    }

    fn holds_path(&self, ty: &Ty, v: &Val, path: &[PathStep]) -> Result<bool, Error> {
        let (mut ty, mut at) = (ty.clone(), v.clone());
        for step in path {
            match step {
                PathStep::Field(f) => {
                    let (a, t) = field(&self.db.schema, &ty, f)?;
                    let i = match &at {
                        Val::Ctor(c, _) => a.of(*c),
                        _ => a.of(0),
                    };
                    at = at.fields()[i].clone();
                    ty = t;
                }
                PathStep::Ctor(c) => {
                    if at.ctor_of() != *c {
                        return Ok(false);
                    }
                    ty = match crate::exec::types::repr(&self.db.schema, &ty) {
                        Ty::Data { def, args, .. } => Ty::Data {
                            def,
                            args,
                            ctors: Some(vec![*c]),
                        },
                        Ty::Sum { ctors, .. } => Ty::Sum {
                            ctors,
                            sel: Some(vec![*c]),
                        },
                        other => other,
                    };
                }
            }
        }
        Ok(true)
    }

    /// Returns the key of `row`, a row of `rel`.
    pub fn key_of(&self, rel: RelId, row: &Val) -> Val {
        let r = self.rel(rel);
        let at = &r.ctors[row.ctor_of() as usize].key_at;
        let fs = row.fields();
        if r.single_key {
            fs[at[0]].clone()
        } else {
            Val::record(at.iter().map(|i| fs[*i].clone()).collect())
        }
    }

    /// Returns the row of `rel` with key `key` the statement's reads see, with its constructor
    /// and the TID of its version.
    pub fn by_key(&self, rel: RelId, key: &Val) -> Result<Option<(u32, Tid, Val)>, Error> {
        let r = self.rel(rel);
        let parts: Vec<Val> = if r.single_key {
            vec![key.clone()]
        } else {
            let mut names = r.key_fields.clone();
            names.sort();
            let fs = key.fields();
            r.key_fields
                .iter()
                .map(|n| fs[names.iter().position(|m| m == n).expect("a key field")].clone())
                .collect()
        };
        let types = self.store.key_types(r.key_index);
        let prefix: Vec<crate::key::Value> = types
            .iter()
            .zip(&parts)
            .map(|(t, v)| to_key(t, v))
            .collect();
        let all: BTreeSet<u32> = r.all_ctors().into_iter().collect();
        for (c, tid) in self.store.probe(r.key_index, &prefix, &all)? {
            let Some((at, stored)) = self.store.fetch(r.ctors[c as usize].heap, tid)? else {
                continue;
            };
            let v = self.row(rel, c, &stored);
            if self.key_of(rel, &v) == *key {
                return Ok(Some((c, at, v)));
            }
        }
        Ok(None)
    }

    /// Returns stored row `stored` of constructor `c` of `rel`.
    pub fn row(&self, rel: RelId, c: u32, stored: &[crate::key::Value]) -> Val {
        let ctor = &self.rel(rel).ctors[c as usize];
        Val::ctor(
            c,
            ctor.convs
                .iter()
                .zip(stored)
                .map(|(conv, v)| from_stored(conv, v))
                .collect(),
        )
    }

    /// Returns every row of `rel` the statement's reads see, of the constructors `of`, or all.
    pub fn rows(&self, rel: RelId, of: Option<&[u32]>) -> Result<Vec<(u32, Val)>, Error> {
        let r = self.rel(rel);
        let mut out = Vec::new();
        for (c, ctor) in r.ctors.iter().enumerate() {
            let c = c as u32;
            if of.is_some_and(|s| !s.contains(&c)) {
                continue;
            }
            for b in self.store.scan(ctor.heap, self.db.options.morsel_rows)? {
                for i in 0..b.len() {
                    out.push((c, self.row(rel, c, &b.row(i))));
                }
            }
        }
        Ok(out)
    }

    /// Returns whether `v` is a row of `rel` the statement's reads see.
    fn in_snapshot(&self, rel: RelId, v: &Val) -> Result<bool, Error> {
        let k = self.key_of(rel, v);
        Ok(self.by_key(rel, &k)?.is_some_and(|(_, _, u)| u == *v))
    }

    /// Returns the key a reference field of `row` names, if any.
    pub fn names(f: &RefField, row: &Val) -> Option<Val> {
        let v = &row.fields()[f.at];
        if !f.optional {
            return Some(v.clone());
        }
        match v {
            Val::Ctor(1, fs) => fs.first().cloned(),
            _ => None,
        }
    }

    /// Returns the relations with a reference field into one of `targets`, with those fields
    /// by constructor.
    pub fn referrers(&self, targets: &BTreeSet<RelId>) -> Vec<(RelId, Vec<Vec<&'a RefField>>)> {
        self.db
            .rels
            .values()
            .filter_map(|r| {
                let fields: Vec<Vec<&RefField>> = r
                    .ctors
                    .iter()
                    .map(|c| {
                        c.refs
                            .iter()
                            .filter(|f| targets.contains(&f.target))
                            .collect()
                    })
                    .collect();
                fields
                    .iter()
                    .any(|fs: &Vec<&RefField>| !fs.is_empty())
                    .then_some((r.id, fields))
            })
            .collect()
    }

    /// Derives the delete actions of the written effects, forms the statement's change set,
    /// applies it, and checks it at the statement boundary.
    fn boundary(&self, written: Effects, result: Val) -> Result<Outcome, Error> {
        let mut minus_written: BTreeSet<(RelId, Val)> = BTreeSet::new();
        let mut kept_keys: BTreeSet<(RelId, Val)> = BTreeSet::new();
        let mut removed: BTreeSet<(RelId, Val)> = BTreeSet::new();
        for e in &written {
            match e {
                Eff::Del(r, v) => {
                    minus_written.insert((*r, v.clone()));
                    removed.insert((*r, self.key_of(*r, v)));
                }
                Eff::Rep(r, v, n) => {
                    minus_written.insert((*r, v.clone()));
                    kept_keys.insert((*r, self.key_of(*r, n)));
                }
                Eff::Ins(r, n) => {
                    kept_keys.insert((*r, self.key_of(*r, n)));
                }
            }
        }
        let mut gone: BTreeSet<(RelId, Val)> = removed.difference(&kept_keys).cloned().collect();
        let mut pre_rows: BTreeMap<(RelId, Val), Val> = BTreeMap::new();
        loop {
            let targets: BTreeSet<RelId> = gone.iter().map(|(r, _)| *r).collect();
            let mut more = Vec::new();
            for (r, fields) in self.referrers(&targets) {
                if !fields
                    .iter()
                    .flatten()
                    .any(|f| f.action == OnDelete::Cascade)
                {
                    continue;
                }
                for (c, u) in self.rows(r, None)? {
                    let k = (r, self.key_of(r, &u));
                    if minus_written.contains(&(r, u.clone())) || gone.contains(&k) {
                        continue;
                    }
                    let hit = fields[c as usize].iter().any(|f| {
                        f.action == OnDelete::Cascade
                            && Self::names(f, &u).is_some_and(|x| gone.contains(&(f.target, x)))
                    });
                    if hit {
                        pre_rows.insert(k.clone(), u.clone());
                        more.push(k);
                    }
                }
            }
            if more.is_empty() {
                break;
            }
            gone.extend(more);
        }
        let mut all = written;
        let targets: BTreeSet<RelId> = gone.iter().map(|(r, _)| *r).collect();
        for (r, fields) in self.referrers(&targets) {
            for (c, u) in self.rows(r, None)? {
                if minus_written.contains(&(r, u.clone()))
                    || gone.contains(&(r, self.key_of(r, &u)))
                {
                    continue;
                }
                let reach: Vec<&RefField> = fields[c as usize]
                    .iter()
                    .copied()
                    .filter(|f| Self::names(f, &u).is_some_and(|x| gone.contains(&(f.target, x))))
                    .collect();
                let clears: Vec<&RefField> = reach
                    .iter()
                    .copied()
                    .filter(|f| f.action == OnDelete::Clear)
                    .collect();
                let retag = reach
                    .iter()
                    .any(|f| matches!(f.action, OnDelete::Retag(..)));
                let new = if retag {
                    let (var, body) = self.db.retags[&r];
                    let env: Env = [(var, u.clone())].into();
                    let v = self.eval(Arena::Program, body, &env)?;
                    self.cleared(r, v, &clears)
                } else if !clears.is_empty() {
                    self.cleared(r, u.clone(), &clears)
                } else {
                    continue;
                };
                all.insert(Eff::Rep(r, u, new));
            }
        }
        let mut plus = Rows::new();
        let mut minus = Rows::new();
        let killed: BTreeSet<(RelId, Val)> = gone.clone();
        for e in &all {
            match e {
                Eff::Ins(r, v) | Eff::Rep(r, _, v) => {
                    if !killed.contains(&(*r, self.key_of(*r, v))) {
                        add(&mut plus, *r, v.clone());
                    }
                }
                Eff::Del(..) => {}
            }
            if let Eff::Del(r, v) | Eff::Rep(r, v, _) = e {
                add(&mut minus, *r, v.clone());
            }
        }
        for (r, k) in &gone {
            let row = match pre_rows.get(&(*r, k.clone())) {
                Some(u) => Some(u.clone()),
                None => self.by_key(*r, k)?.map(|(_, _, u)| u),
            };
            if let Some(u) = row {
                add(&mut minus, *r, u);
            }
        }
        normalise(&mut plus, &mut minus);
        self.check_open(&plus)?;
        let mut viol = Violations::new();
        let mut by_key: BTreeMap<(RelId, Val), BTreeSet<Val>> = BTreeMap::new();
        for ((r, _), rows) in &plus {
            for v in rows {
                by_key
                    .entry((*r, self.key_of(*r, v)))
                    .or_default()
                    .insert(v.clone());
            }
        }
        for ((r, k), rows) in by_key {
            if rows.len() > 1 {
                violate_key(&mut viol, &format!("{}.key", self.rel(r).qualified), r, k);
            }
        }
        self.apply(&plus, &minus, &mut viol)?;
        self.check(&plus, &minus, &mut viol)?;
        if let Some((name, v)) = viol.into_iter().next() {
            let reason = match v {
                Violation::Key(relation, keys) => Rejection::KeyViolation {
                    relation,
                    key: keys.into_iter().next().expect("a key violation has a key"),
                },
                Violation::Other(report) => Rejection::Violated {
                    constraint: name,
                    report,
                },
            };
            return Err(Error::Rejected(reason));
        }
        Ok(Outcome {
            result,
            inserted: plus,
            deleted: minus,
        })
    }

    /// Returns `v`, a row of `rel`, with each field of `clears` it has set to `None`.
    fn cleared(&self, rel: RelId, v: Val, clears: &[&RefField]) -> Val {
        let r = self.rel(rel);
        let Val::Ctor(c, fs) = v else {
            panic!("a row is a constructor value")
        };
        let names = &r.ctors[c as usize].fields;
        let mut fs = fs.to_vec();
        for f in clears {
            if let Some(i) = names.iter().position(|(n, _)| *n == f.name) {
                fs[i] = Val::none();
            }
        }
        Val::ctor(c, fs)
    }

    /// Refuses an inserted or retagged row in a constructor not open for writes, and a stored
    /// value of a value-level constructor not open for writes that the pre-state row of its key
    /// does not hold at the same path.
    fn check_open(&self, plus: &Rows) -> Result<(), Error> {
        let schema = &self.db.schema;
        if schema.stages.is_empty() && schema.value_stages.is_empty() {
            return Ok(());
        }
        let closed = |s: Stage| match s {
            Stage::Enabled => None,
            s => Some(s),
        };
        for ((r, c), rows) in plus {
            let rel = self.rel(*r);
            for v in rows {
                let pre = self.by_key(*r, &self.key_of(*r, v))?.map(|(_, _, u)| u);
                if let Some(stage) = closed(schema.stage(*r, *c))
                    && pre.as_ref().is_none_or(|u| u.ctor_of() != *c)
                {
                    return Err(Error::Rejected(Rejection::NotOpen {
                        ctor: format!("{}.{}", rel.name, rel.ctors[*c as usize].name),
                        stage,
                    }));
                }
                let mut stored = Vec::new();
                self.sum_values(v, &rel.elem, &mut Vec::new(), &mut stored);
                for (path, def, ctor, value) in stored {
                    let Some(stage) = closed(schema.value_stage(def, ctor)) else {
                        continue;
                    };
                    let held = pre.as_ref().and_then(|u| at(u, &path)) == Some(&value);
                    if !held {
                        let sum = schema.ty(def);
                        return Err(Error::Rejected(Rejection::NotOpen {
                            ctor: format!("{}.{}", sum.name, sum.ctors()[ctor as usize].name),
                            stage,
                        }));
                    }
                }
            }
        }
        Ok(())
    }

    /// Adds to `out` every value of a declared sum inside `v`, of type `ty`, below the top,
    /// with its path of field positions, its sum and its constructor.
    fn sum_values(
        &self,
        v: &Val,
        ty: &Ty,
        path: &mut Vec<usize>,
        out: &mut Vec<(Vec<usize>, adtb_lang::types::TypeId, u32, Val)>,
    ) {
        let schema = &self.db.schema;
        let t = crate::exec::types::repr(schema, ty);
        let fields: Vec<Ty> = match (&t, v) {
            (Ty::Record(fs), Val::Record(_)) => fs.iter().map(|(_, t)| t.clone()).collect(),
            (Ty::Data { def, .. }, Val::Ctor(c, _)) => {
                if !path.is_empty() {
                    out.push((path.clone(), *def, *c, v.clone()));
                }
                ctors(schema, &t).expect("a sum")[*c as usize]
                    .iter()
                    .map(|(_, t)| t.clone())
                    .collect()
            }
            (Ty::Sum { .. }, Val::Ctor(c, _)) => ctors(schema, &t).expect("a sum")[*c as usize]
                .iter()
                .map(|(_, t)| t.clone())
                .collect(),
            _ => return,
        };
        for (i, (x, t)) in v.fields().iter().zip(fields).enumerate() {
            path.push(i);
            self.sum_values(x, &t, path, out);
            path.pop();
        }
    }
}

fn at<'v>(v: &'v Val, path: &[usize]) -> Option<&'v Val> {
    path.iter().try_fold(v, |x, i| match x {
        Val::Ctor(..) | Val::Record(_) => x.fields().get(*i),
        _ => None,
    })
}

pub fn bind(env: &Env, x: Var, v: Val) -> Env {
    let mut e = env.clone();
    e.insert(x, v);
    e
}

pub fn add(rows: &mut Rows, rel: RelId, v: Val) {
    rows.entry((rel, v.ctor_of())).or_default().insert(v);
}

fn normalise(plus: &mut Rows, minus: &mut Rows) {
    for (rc, p) in plus.iter_mut() {
        if let Some(m) = minus.get_mut(rc) {
            let both: Vec<Val> = p.intersection(m).cloned().collect();
            for v in both {
                p.remove(&v);
                m.remove(&v);
            }
        }
    }
    plus.retain(|_, rows| !rows.is_empty());
    minus.retain(|_, rows| !rows.is_empty());
}
