use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::kernel::{Conflict, Kind, PathStep, Prim, Terms, Tm, Var};
use adtb_lang::schema::Generator;
use adtb_lang::types::{RelId, Ty};

use crate::eval::{
    Abort, Arena, Change, Database, Env, Here, Interpreter, R, Run, case_arm, not_yet,
};
use crate::eval::{bool_of, internal};
use crate::textual::print;
use crate::value::Value;

/// A constructor of the prelude sum `RejectReason` that a statement can be rejected with.
#[derive(Clone, Debug, PartialEq)]
pub enum Rejection {
    /// `constraint` is the violated constraint's name; `report` the rows its query returns.
    Violated {
        constraint: String,
        report: BTreeSet<Value>,
    },
    /// `key` is the key in the textual syntax.
    KeyViolation {
        relation: String,
        key: String,
    },
    KeyChanged {
        relation: String,
    },
    /// `ctor` is qualified by its relation (`identities.Apple`) or by its sum (`Provider.Okta`).
    NotOpen {
        ctor: String,
        stage: Stage,
    },
    UserError {
        message: String,
    },
    BadInput {
        source: String,
        line: i64,
        detail: String,
    },
}

/// The stage of a constructor that refuses writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Declared,
    Deprecated,
}

/// The supply of generated values, kept across statements.
///
/// Invariant: a `sequence` or `uuidv7` value it draws for `r` differs from every value it has
/// issued for `r` and from every key of `r` in the snapshot of every statement it has served.
#[derive(Clone, Debug, Default)]
pub struct Supply {
    /// The instant, in microseconds, that every clock generator of the next statement takes.
    pub clock: i64,
    issued: BTreeMap<RelId, BTreeSet<Value>>,
    held: BTreeMap<RelId, BTreeSet<Value>>,
    loads: i64,
    uuids: u64,
}

impl Supply {
    fn hold(&mut self, it: &Interpreter, db: &Database) {
        for ((r, _), rows) in db {
            if it.schema.relation(*r).generate.is_none() {
                continue;
            }
            let held = self.held.entry(*r).or_default();
            held.extend(rows.iter().map(|row| it.key_of(*r, row)));
        }
    }

    fn used(&self, r: RelId, v: &Value) -> bool {
        self.issued.get(&r).is_some_and(|s| s.contains(v))
            || self.held.get(&r).is_some_and(|s| s.contains(v))
    }

    fn sequence(&mut self, r: RelId) -> i64 {
        let ints = self
            .issued
            .get(&r)
            .into_iter()
            .chain(self.held.get(&r))
            .flatten()
            .filter_map(|v| match v {
                Value::Key(k) => match k.as_ref() {
                    Value::Int(i) => Some(*i),
                    _ => None,
                },
                Value::Int(i) => Some(*i),
                _ => None,
            });
        ints.max().unwrap_or(0) + 1
    }

    fn uuidv7(&mut self, r: RelId) -> Value {
        let ms = self.clock.div_euclid(1000) as u64;
        loop {
            let n = self.uuids;
            self.uuids += 1;
            let mut b = [0u8; 16];
            b[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
            b[6] = 0x70;
            b[8..].copy_from_slice(&n.to_be_bytes());
            b[8] = 0x80 | (b[8] & 0x3f);
            let v = Value::key(Value::Bytes(b.to_vec()));
            if !self.used(r, &v) {
                return v;
            }
        }
    }
}

/// What a statement that succeeds returns.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub result: Value,
    /// The snapshot and the normalised change set.
    pub change: Change,
    /// The state after the statement.
    pub db: Database,
}

/// One effect of a write.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Eff {
    Ins(RelId, Value),
    Del(RelId, Value),
    Rep(RelId, Value, Value),
    /// Delete every row of the relation with this key.
    Kill(RelId, Value),
}

pub(crate) type Effects = BTreeSet<Eff>;

/// The state of one statement: its supply and its one `load` value.
#[derive(Debug)]
pub(crate) struct Stmt {
    supply: RefCell<Supply>,
    load: Cell<Option<i64>>,
}

impl Stmt {
    /// Returns a new value of generator `g` of relation `r`, at the fill's type `ty`.
    pub(crate) fn draw(&self, g: Generator, r: RelId, ty: &Ty) -> Value {
        let mut s = self.supply.borrow_mut();
        let v = match g {
            Generator::Clock => return Value::Instant(s.clock),
            Generator::Sequence => {
                let n = Value::Int(s.sequence(r));
                if matches!(ty, Ty::Key { .. }) {
                    Value::key(n)
                } else {
                    n
                }
            }
            Generator::Uuidv7 => s.uuidv7(r),
        };
        s.issued.entry(r).or_default().insert(v.clone());
        v
    }

    fn load(&self, it: &Interpreter, into: RelId) -> i64 {
        if let Some(l) = self.load.get() {
            return l;
        }
        let mut s = self.supply.borrow_mut();
        let present = it
            .rows(it.db, into, &None)
            .iter()
            .filter_map(|row| match row.field("load") {
                Some(Value::Int(l)) => Some(*l),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let l = s.loads.max(present) + 1;
        s.loads = l;
        self.load.set(Some(l));
        l
    }
}

/// A record of a source that an insert with `rejects into` rejected.
struct Rejected {
    line: i64,
    source: String,
    raw: String,
    reason: String,
}

impl<'a> Interpreter<'a> {
    /// Runs term `t` of `terms` as one statement over the database, each variable of `params`
    /// bound to its value, drawing generated values from `supply`.
    ///
    /// `t` is a `Transact`, a write, or a query, whose outcome changes nothing.
    pub fn run(
        &self,
        terms: &Terms,
        t: Tm,
        params: &[(Var, Value)],
        supply: &mut Supply,
    ) -> R<Outcome> {
        let env = params
            .iter()
            .fold(Env::default(), |e, (v, x)| e.bind(*v, x.clone()));
        self.statement(terms, t, env, supply)
    }

    /// Runs the registered transaction `t` of `terms`, a `Lam` chain over its parameters, on
    /// `args` in order.
    pub fn run_call(
        &self,
        terms: &Terms,
        t: Tm,
        args: Vec<Value>,
        supply: &mut Supply,
    ) -> R<Outcome> {
        let mut args = if args.is_empty() {
            vec![Value::unit()]
        } else {
            args
        }
        .into_iter();
        let mut env = Env::default();
        let mut body = t;
        while let Kind::Lam(x, b) = terms.kind(body) {
            let arg = args
                .next()
                .expect("a registered transaction takes one argument per parameter");
            env = env.bind(*x, arg);
            body = *b;
        }
        self.statement(terms, body, env, supply)
    }

    fn statement(&self, terms: &Terms, t: Tm, env: Env, supply: &mut Supply) -> R<Outcome> {
        supply.hold(self, self.db);
        let stmt = Stmt {
            supply: RefCell::new(std::mem::take(supply)),
            load: Cell::new(None),
        };
        let run = Run {
            it: self,
            query: terms,
            stmt: Some(&stmt),
        };
        let out = run.statement(t, &env);
        *supply = stmt.supply.into_inner();
        out
    }
}

impl Run<'_, '_> {
    fn statement(&self, t: Tm, env: &Env) -> R<Outcome> {
        let h = self.here(Arena::Query);
        let mut effects = Effects::new();
        let result = match self.query.kind(t) {
            Kind::Transact(b, _) => self.block(&h, *b, env, &mut effects)?,
            _ if *self.query.ty(t) == Ty::Change => {
                self.write(&h, t, env, &mut effects)?;
                Value::unit()
            }
            _ => {
                let result = self.eval(&h, t, env)?;
                return Ok(Outcome {
                    result,
                    change: Change {
                        prior: self.it.db.clone(),
                        ..Change::default()
                    },
                    db: self.it.db.clone(),
                });
            }
        };
        self.boundary(effects, result)
    }

    /// Evaluates block term `t`, adding the effects of its writes to `out`. Returns its result.
    fn block(&self, h: &Here, t: Tm, env: &Env, out: &mut Effects) -> R<Value> {
        match self.terms(h.arena).kind(t) {
            Kind::Block(w, k) => {
                self.write(h, *w, env, out)?;
                self.block(h, *k, env, out)
            }
            Kind::LetBang(x, w, k) => {
                let v = self.bind_write(h, *w, env, out)?;
                self.block(h, *k, &env.bind(*x, v), out)
            }
            Kind::Let(x, e, k) => {
                let v = self.eval(h, *e, env)?;
                self.block(h, *k, &env.bind(*x, v), out)
            }
            _ => self.eval(h, t, env),
        }
    }

    /// Evaluates write `t`, adding its effects to `out`.
    pub(crate) fn write(&self, h: &Here, t: Tm, env: &Env, out: &mut Effects) -> R<()> {
        let terms = self.terms(h.arena);
        match terms.kind(t) {
            Kind::Skip => Ok(()),
            Kind::Insert(r, rows) => {
                for v in self.records(h, *rows, env, None)? {
                    out.insert(Eff::Ins(*r, v));
                }
                Ok(())
            }
            Kind::Delete(r, rows) => {
                for v in self.set(h, *rows, env)? {
                    if self.in_snapshot(*r, &v) {
                        out.insert(Eff::Del(*r, v));
                    }
                }
                Ok(())
            }
            Kind::Replace(..) | Kind::Upsert { .. } | Kind::Rejects { .. } => {
                self.bind_write(h, t, env, out).map(drop)
            }
            Kind::Par(ws) => ws.iter().try_for_each(|w| self.write(h, *w, env, out)),
            Kind::Fail(m) => {
                let Value::Text(message) = self.eval(h, *m, env)? else {
                    return internal("`fail` takes a Text");
                };
                Err(Abort::Reject(Rejection::UserError { message }))
            }
            Kind::If(c, a, b) => {
                let w = if bool_of(&self.eval(h, *c, env)?)? {
                    *a
                } else {
                    *b
                };
                self.write(h, w, env, out)
            }
            Kind::Case { scrutinee, arms } => {
                let v = self.eval(h, *scrutinee, env)?;
                let (x, w) = case_arm(arms, &v)?;
                self.write(h, w, &env.bind(x, v), out)
            }
            Kind::Let(x, e, w) => {
                let v = self.eval(h, *e, env)?;
                self.write(h, *w, &env.bind(*x, v), out)
            }
            Kind::Unreachable => internal("evaluation reached a position exhaustiveness excludes"),
            k => internal(format!("{k:?} is not a write")),
        }
    }

    /// Evaluates a `Replace`, `Upsert` or `Rejects`, adding its effects to `out`. Returns the
    /// value a `let` binds to it.
    fn bind_write(&self, h: &Here, t: Tm, env: &Env, out: &mut Effects) -> R<Value> {
        let terms = self.terms(h.arena);
        match terms.kind(t) {
            Kind::Replace(r, rows, f) => {
                let f = self.eval(h, *f, env)?;
                let mut news = BTreeSet::new();
                for v in self.set(h, *rows, env)? {
                    let n = self.apply(f.clone(), v.clone())?;
                    if self.it.key_of(*r, &n) != self.it.key_of(*r, &v) {
                        return Err(Abort::Reject(Rejection::KeyChanged {
                            relation: self.it.schema.relation(*r).name.clone(),
                        }));
                    }
                    if n != v && self.in_snapshot(*r, &v) {
                        out.insert(Eff::Rep(*r, v, n.clone()));
                    }
                    news.insert(n);
                }
                Ok(Value::Set(news))
            }
            Kind::Upsert { rows, .. } => {
                let rows = self.records(h, *rows, env, None)?;
                self.upsert(h, t, rows, env, out)
            }
            Kind::Rejects { into, insert } => {
                let rows = match terms.kind(*insert) {
                    Kind::Insert(_, rows) | Kind::Upsert { rows, .. } => *rows,
                    k => {
                        return internal(format!(
                            "`rejects into` wraps an insert or upsert, not {k:?}"
                        ));
                    }
                };
                let mut rejected = Vec::new();
                let rows = self.records(h, rows, env, Some(&mut rejected))?;
                let bound = match terms.kind(*insert) {
                    Kind::Insert(r, _) => {
                        out.extend(rows.iter().map(|v| Eff::Ins(*r, v.clone())));
                        Value::Set(rows)
                    }
                    _ => self.upsert(h, *insert, rows, env, out)?,
                };
                for j in rejected {
                    let load = self.stmt.expect("a statement").load(self.it, *into);
                    out.insert(Eff::Ins(*into, self.rejected_row(*into, load, j)));
                }
                Ok(bound)
            }
            k => internal(format!("{k:?} binds no rows")),
        }
    }

    fn rejected_row(&self, into: RelId, load: i64, j: Rejected) -> Value {
        let mut given = vec![
            ("load", Value::Int(load)),
            ("line", Value::Int(j.line)),
            ("source", Value::Text(j.source)),
            ("raw", Value::Text(j.raw)),
            ("reason", Value::Text(j.reason)),
        ];
        let order = self
            .it
            .field_order(&self.it.schema.relation(into).elem, 0)
            .expect("`RejectedRow` is a record");
        let fields = order
            .into_iter()
            .map(|n| {
                let i = given
                    .iter()
                    .position(|(g, _)| *g == n)
                    .unwrap_or_else(|| panic!("`RejectedRow` has no field `{n}` of ours"));
                let (_, v) = given.swap_remove(i);
                (n, v)
            })
            .collect();
        Value::Ctor(0, fields)
    }

    /// Returns the rows an insert's `rows` term computes. Over a source relation, each record
    /// is computed on its own, in the order of the source's key: a record whose computation
    /// reaches `reject` goes to `rejected`, or rejects the statement with `BadInput` when there
    /// is none.
    fn records(
        &self,
        h: &Here,
        rows: Tm,
        env: &Env,
        mut rejected: Option<&mut Vec<Rejected>>,
    ) -> R<BTreeSet<Value>> {
        let terms = self.terms(h.arena);
        let source = match terms.kind(rows) {
            Kind::For(x, src, body) => match terms.kind(*src) {
                Kind::Rel(r, s) => Some((*x, *r, s.clone(), *body)),
                Kind::Prim(Prim::ReadFile { .. }, _) => return not_yet("a file scan"),
                _ => None,
            },
            _ => None,
        };
        let Some((x, r0, s, body)) = source else {
            assert!(
                rejected.is_none(),
                "the rows of `rejects into` range over one source relation"
            );
            return self.set(h, rows, env);
        };
        let it = self.it;
        let mut records: Vec<Value> = it.rows(it.db, r0, &s).into_iter().collect();
        records.sort_by_cached_key(|p| it.key_of(r0, p));
        let source = it.schema.relation(r0).name.clone();
        let mut out = BTreeSet::new();
        for (i, p) in records.into_iter().enumerate() {
            let line = i as i64 + 1;
            match self.set(h, body, &env.bind(x, p.clone())) {
                Ok(vs) => out.extend(vs),
                Err(Abort::Reject(Rejection::BadInput { detail, .. })) => match &mut rejected {
                    Some(js) => {
                        let ty = terms.var_info(x).ty.subst(&h.targs);
                        js.push(Rejected {
                            line,
                            source: source.clone(),
                            raw: print(it.schema, &p, &ty),
                            reason: detail,
                        });
                    }
                    None => {
                        return Err(Abort::Reject(Rejection::BadInput {
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

    /// Evaluates the upsert `t` over its proposed `rows`, adding its effects to `out`. Returns
    /// its binding.
    fn upsert(
        &self,
        h: &Here,
        t: Tm,
        rows: BTreeSet<Value>,
        env: &Env,
        out: &mut Effects,
    ) -> R<Value> {
        let Kind::Upsert {
            rel,
            on,
            old,
            new,
            otherwise,
            single,
            ..
        } = self.terms(h.arena).kind(t)
        else {
            panic!("an upsert")
        };
        let rel = *rel;
        let it = self.it;
        let mut bound = BTreeSet::new();
        for n in rows {
            let olds = self.conflicts(rel, *on, &n)?;
            if olds.is_empty() {
                out.insert(Eff::Ins(rel, n.clone()));
                bound.insert(n);
                continue;
            }
            for o in olds {
                let mut ew = Effects::new();
                let inner = env.bind(*old, o.clone()).bind(*new, n.clone());
                self.write(h, *otherwise, &inner, &mut ew)?;
                let k = it.key_of(rel, &o);
                let written: Vec<Value> = ew
                    .iter()
                    .filter_map(|e| match e {
                        Eff::Rep(r, _, v) | Eff::Ins(r, v) if *r == rel => Some(v),
                        _ => None,
                    })
                    .filter(|v| it.key_of(rel, v) == k)
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
        if *single {
            Ok(bound
                .into_iter()
                .next()
                .expect("a single-row upsert binds a row"))
        } else {
            Ok(Value::Set(bound))
        }
    }

    /// Returns the rows of `rel` in the snapshot whose values of `on` equal those of `n`.
    fn conflicts(&self, rel: RelId, on: Conflict, n: &Value) -> R<Vec<Value>> {
        let it = self.it;
        match on {
            Conflict::Key => {
                let k = it.key_of(rel, n);
                Ok(it
                    .rows(it.db, rel, &None)
                    .into_iter()
                    .filter(|o| it.key_of(rel, o) == k)
                    .collect())
            }
            Conflict::Unique(i) => {
                let scope = &it.schema.relation(rel).indexes[i as usize].scope;
                let Some(want) = self.unique_values(rel, i as usize, n)? else {
                    return Ok(Vec::new());
                };
                let mut out = Vec::new();
                for o in it.rows(it.db, rel, scope) {
                    if self.unique_values(rel, i as usize, &o)?.as_ref() == Some(&want) {
                        out.push(o);
                    }
                }
                Ok(out)
            }
        }
    }

    /// Returns the values of the fields of index `i` of `rel` on `row`, or `None` when `row` holds
    /// another constructor than a field's path names.
    pub(crate) fn unique_values(&self, rel: RelId, i: usize, row: &Value) -> R<Option<Vec<Value>>> {
        let it = self.it;
        let n = it.schema.relation(rel).indexes[i].fields.len();
        let h = self.here(Arena::Program);
        let mut out = Vec::with_capacity(n);
        for k in 0..n {
            let field = &it.program.index_fields[&(rel, i, k)];
            if !holds_path(row, &field.path) {
                return Ok(None);
            }
            let f = self.eval(&h, field.value, &Env::default())?;
            out.push(self.apply(f, row.clone())?);
        }
        Ok(Some(out))
    }

    /// Returns whether `v` is a row of `r` in the snapshot.
    fn in_snapshot(&self, r: RelId, v: &Value) -> bool {
        let c = v.tag().expect("a row is a constructor value");
        self.it.db.get(&(r, c)).is_some_and(|rows| rows.contains(v))
    }
}

/// Returns whether `v` holds every constructor `path` names.
fn holds_path(v: &Value, path: &[PathStep]) -> bool {
    let mut at = v;
    for step in path {
        match step {
            PathStep::Field(f) => match at.field(f) {
                Some(next) => at = next,
                None => return false,
            },
            PathStep::Ctor(c) => {
                if at.tag() != Some(*c) {
                    return false;
                }
            }
        }
    }
    true
}
