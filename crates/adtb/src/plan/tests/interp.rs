use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use adtb_lang::check::Program;
use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_lang::schema::Schema;
use adtb_lang::types::{Base, RelId, Ty};
use adtb_reference::{Abort, Database, Fault, Interpreter, Limit, R, Value};

use crate::plan::*;

/// Evaluates plans over an in-memory database, one row at a time.
pub struct Interp<'a> {
    schema: &'a Schema,
    program: &'a Program,
    db: &'a Database,
    terms: RefCell<Terms>,
    heaps: BTreeMap<HeapId, (RelId, u32)>,
    indexes: BTreeMap<IndexId, (RelId, IndexDesc)>,
    fix_limit: usize,
}

type Env = BTreeMap<Var, Value>;

/// The key and the value of each element of a build.
type Table = Vec<(Vec<Value>, Value)>;

fn bind(env: &Env, v: Var, x: Value) -> Env {
    let mut e = env.clone();
    e.insert(v, x);
    e
}

fn set(v: Value) -> BTreeSet<Value> {
    match v {
        Value::Set(s) => s,
        other => panic!("expected a set, found {other:?}"),
    }
}

fn truth(v: &Value) -> bool {
    v.as_bool()
        .unwrap_or_else(|| panic!("expected a Bool, found {v:?}"))
}

/// The builds of one evaluation of a query, each made when first probed.
struct Built<'q> {
    query: &'q Query,
    env: Env,
    tables: RefCell<HashMap<BuildId, Table>>,
}

impl<'a> Interp<'a> {
    pub fn new(
        schema: &'a Schema,
        program: &'a Program,
        db: &'a Database,
        catalog: &Catalog,
        plan: &Plan,
    ) -> Interp<'a> {
        let mut heaps = BTreeMap::new();
        let mut indexes = BTreeMap::new();
        for (r, d) in &catalog.relations {
            for (c, h) in &d.heaps {
                heaps.insert(*h, (*r, *c));
            }
            for i in &d.indexes {
                indexes.insert(i.id, (*r, i.clone()));
            }
        }
        Interp {
            schema,
            program,
            db,
            terms: RefCell::new(plan.terms.clone()),
            heaps,
            indexes,
            fix_limit: 10_000,
        }
    }

    pub fn run(&self, plan: &Plan, params: &[(Var, Value)]) -> R<Value> {
        let env: Env = params.iter().cloned().collect();
        self.eval(&plan.root, &env)
    }

    fn expr(&self, t: Tm, env: &Env) -> R<Value> {
        let bound: Vec<(Var, Value)> = env.iter().map(|(v, x)| (*v, x.clone())).collect();
        Interpreter::new(self.schema, self.program, self.db).eval(&self.terms.borrow(), t, &bound)
    }

    fn rows(&self, heap: HeapId) -> Vec<Value> {
        let key = self.heaps[&heap];
        self.db
            .get(&key)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns the rows of the index's constructors whose leading fields equal `key`, with their constructors.
    fn entries(&self, index: IndexId, key: &[Value]) -> Vec<(u32, Value)> {
        let (r, desc) = &self.indexes[&index];
        let mut out = Vec::new();
        for c in &desc.ctors {
            for row in self.db.get(&(*r, *c)).into_iter().flatten() {
                let hit = desc
                    .fields
                    .iter()
                    .zip(key)
                    .all(|(f, k)| row.field(f).expect("an indexed field") == k);
                if hit {
                    out.push((*c, row.clone()));
                }
            }
        }
        out
    }

    fn eval(&self, e: &Eval, env: &Env) -> R<Value> {
        match e {
            Eval::Expr(t) => self.expr(*t, env),
            Eval::Let(v, a, b) => {
                let x = self.eval(a, env)?;
                self.eval(b, &bind(env, *v, x))
            }
            Eval::If(c, a, b) => {
                if truth(&self.expr(*c, env)?) {
                    self.eval(a, env)
                } else {
                    self.eval(b, env)
                }
            }
            Eval::Case(s, arms) => {
                let v = self.expr(*s, env)?;
                let c = v.tag().expect("a scrutinee is a constructor value");
                let (_, x, arm) = arms
                    .iter()
                    .find(|(d, _, _)| *d == c)
                    .expect("an arm per constructor");
                self.eval(arm, &bind(env, *x, v))
            }
            Eval::Try(a) => match self.eval(a, env) {
                Ok(v) => Ok(Value::Ctor(0, vec![("value".into(), v)])),
                Err(Abort::Fault(f)) => {
                    let fault = Interpreter::new(self.schema, self.program, self.db).fault_value(f);
                    Ok(Value::Ctor(1, vec![("error".into(), fault)]))
                }
                Err(other) => Err(other),
            },
            Eval::Lookup(l) => {
                let key = l
                    .key
                    .iter()
                    .map(|k| self.expr(*k, env))
                    .collect::<R<Vec<_>>>()?;
                self.entries(l.index, &key)
                    .into_iter()
                    .find(|(c, _)| l.routes.iter().any(|(d, _)| d == c))
                    .map(|(_, row)| row)
                    .ok_or(Abort::Fault(Fault::NoSuchRow))
            }
            Eval::Query(q) => self.query(q, env),
            Eval::Fix(f) => self.fix(f, env),
        }
    }

    fn fix(&self, f: &Fix, env: &Env) -> R<Value> {
        let empty = || Value::Set(BTreeSet::new());
        let with = |cur: &[Value], deltas: &[Value]| {
            let mut e = env.clone();
            for (i, b) in f.binds.iter().enumerate() {
                e.insert(b.var, cur[i].clone());
                e.insert(b.delta, deltas[i].clone());
            }
            e
        };
        let n = f.binds.len();
        let mut cur: Vec<Value> = vec![empty(); n];
        let mut steps = 0;
        if f.binds.iter().all(|b| b.step.is_some()) {
            let inner = with(&cur, &cur);
            let mut deltas = f
                .binds
                .iter()
                .map(|b| self.eval(&b.body, &inner))
                .collect::<R<Vec<_>>>()?;
            cur = deltas.clone();
            while deltas.iter().any(|d| !set(d.clone()).is_empty()) {
                steps += 1;
                if steps > self.fix_limit {
                    return Err(Abort::Limit(Limit::Rounds {
                        rounds: self.fix_limit as i64,
                    }));
                }
                let inner = with(&cur, &deltas);
                let mut next = Vec::new();
                for (i, b) in f.binds.iter().enumerate() {
                    let found = set(self.eval(b.step.as_ref().expect("a step"), &inner)?);
                    let old = set(cur[i].clone());
                    next.push(
                        found
                            .into_iter()
                            .filter(|x| !old.contains(x))
                            .collect::<BTreeSet<_>>(),
                    );
                }
                for (i, new) in next.into_iter().enumerate() {
                    let mut all = set(cur[i].clone());
                    all.extend(new.iter().cloned());
                    cur[i] = Value::Set(all);
                    deltas[i] = Value::Set(new);
                }
            }
            return self.eval(&f.body, &with(&cur, &cur));
        }
        loop {
            let inner = with(&cur, &cur);
            let next = f
                .binds
                .iter()
                .map(|b| self.eval(&b.body, &inner))
                .collect::<R<Vec<_>>>()?;
            if next == cur {
                return self.eval(&f.body, &inner);
            }
            steps += 1;
            if steps > self.fix_limit {
                return Err(Abort::Limit(Limit::Rounds {
                    rounds: self.fix_limit as i64,
                }));
            }
            cur = next;
        }
    }

    fn query(&self, q: &Query, env: &Env) -> R<Value> {
        if let Sink::Top { count, .. } = &q.sink {
            let c = self.eval(count, env)?;
            return self.finish(q, env, Some(c));
        }
        self.finish(q, env, None)
    }

    fn finish(&self, q: &Query, env: &Env, count: Option<Value>) -> R<Value> {
        let built = Built {
            query: q,
            env: env.clone(),
            tables: RefCell::new(HashMap::new()),
        };
        let mut out = BTreeSet::new();
        self.spine(&q.spine, env, &built, &mut out)?;
        match &q.sink {
            Sink::Set => Ok(Value::Set(out)),
            Sink::Exists => Ok(Value::bool(!out.is_empty())),
            Sink::IsEmpty => Ok(Value::bool(out.is_empty())),
            Sink::Agg {
                op,
                var,
                body,
                ty,
                body_ty,
            } => {
                let mut ms = Vec::new();
                for x in out {
                    ms.push(self.eval(body, &bind(env, *var, x))?);
                }
                self.aggregate(*op, ty, body_ty, ms)
            }
            Sink::Fold { var, body, fold } => {
                if fold.optional && out.is_empty() {
                    return Ok(Value::none());
                }
                let mut state = self.expr(fold.init, env)?;
                for x in out {
                    let m = self.eval(body, &bind(env, *var, x))?;
                    let e = bind(&bind(env, fold.state, state), fold.value, m);
                    state = self.expr(fold.step, &e)?;
                }
                let v = self.expr(fold.finish, &bind(env, fold.state, state))?;
                Ok(if fold.optional { Value::some(v) } else { v })
            }
            Sink::Group {
                var,
                by,
                rows,
                aggs,
            } => {
                let mut groups: BTreeMap<Value, BTreeSet<Value>> = BTreeMap::new();
                for x in out {
                    let k = self.eval(by, &bind(env, *var, x.clone()))?;
                    groups.entry(k).or_default().insert(x);
                }
                let mut result = BTreeSet::new();
                for (k, members) in groups {
                    let mut fields = vec![("key".to_string(), k)];
                    for a in aggs {
                        fields.push((a.field.clone(), self.group_agg(a, &members, env)?));
                    }
                    if *rows {
                        fields.push(("rows".into(), Value::Set(members)));
                    }
                    fields.sort_by(|a, b| a.0.cmp(&b.0));
                    result.insert(Value::Record(fields));
                }
                Ok(Value::Set(result))
            }
            Sink::Top { keys, var, .. } => {
                let sorted = self.sorted(out, keys, *var, env)?;
                let c = count.expect("a top's count");
                let n = match c {
                    Value::Int(n) => n,
                    other => panic!("a count is an integer, found {other:?}"),
                };
                if n < 0 {
                    return Err(Fault::NegativeCount.into());
                }
                Ok(Value::Set(sorted.into_iter().take(n as usize).collect()))
            }
            Sink::Present {
                var,
                keys,
                take,
                after,
            } => {
                let keyed = self.keyed(out, keys, *var, env)?;
                let take = take.as_ref().map(|t| self.eval(t, env)).transpose()?;
                let after = after.as_ref().map(|t| self.eval(t, env)).transpose()?;
                self.present(keyed, keys, take, after)
            }
        }
    }

    fn group_agg(&self, a: &GroupAgg, members: &BTreeSet<Value>, env: &Env) -> R<Value> {
        let mut elems = BTreeSet::new();
        'rows: for row in members {
            let at = bind(env, a.row, row.clone());
            for f in &a.filter {
                if !truth(&self.eval(f, &at)?) {
                    continue 'rows;
                }
            }
            elems.insert(self.eval(&a.element, &at)?);
        }
        let mut ms = Vec::new();
        for x in elems {
            ms.push(self.eval(&a.body, &bind(env, a.elem, x))?);
        }
        self.aggregate(a.op, &a.ty, &a.body_ty, ms)
    }

    fn keyed(
        &self,
        elems: BTreeSet<Value>,
        keys: &[(Eval, bool)],
        var: Var,
        env: &Env,
    ) -> R<Vec<Value>> {
        let mut out = Vec::new();
        for x in elems {
            let at = bind(env, var, x.clone());
            let mut fields = vec![("e".to_string(), x)];
            for (i, (k, _)) in keys.iter().enumerate() {
                fields.push((format!("k{i:03}"), self.eval(k, &at)?));
            }
            out.push(Value::Record(fields));
        }
        Ok(out)
    }

    fn sorted(
        &self,
        elems: BTreeSet<Value>,
        keys: &[(Eval, bool)],
        var: Var,
        env: &Env,
    ) -> R<Vec<Value>> {
        let keyed = self.keyed(elems, keys, var, env)?;
        match self.present(keyed, keys, None, None)? {
            Value::List(xs) => Ok(xs),
            other => panic!("a sort is a list, found {other:?}"),
        }
    }

    fn present(
        &self,
        keyed: Vec<Value>,
        keys: &[(Eval, bool)],
        take: Option<Value>,
        after: Option<Value>,
    ) -> R<Value> {
        let mut t = self.terms.borrow_mut();
        let s = t.var("%s", Ty::Unit);
        let r = t.var("%r", Ty::Unit);
        let tv = t.var("%take", Ty::Base(Base::I64));
        let av = t.var("%after", Ty::Unit);
        let source = t.add(Kind::Var(s), Ty::Set(Box::new(Ty::Unit)));
        let row = t.add(Kind::Var(r), Ty::Unit);
        let ks = keys
            .iter()
            .enumerate()
            .map(|(i, (_, d))| (t.add(Kind::Field(row, format!("k{i:03}")), Ty::Unit), *d))
            .collect();
        let take_tm = take
            .as_ref()
            .map(|_| t.add(Kind::Var(tv), Ty::Base(Base::I64)));
        let after_tm = after.as_ref().map(|_| t.add(Kind::Var(av), Ty::Unit));
        let tm = t.add(
            Kind::Present {
                var: r,
                source,
                keys: ks,
                take: take_tm,
                after: after_tm,
            },
            Ty::Set(Box::new(Ty::Unit)),
        );
        drop(t);
        let mut e = Env::new();
        e.insert(s, Value::Set(keyed.into_iter().collect()));
        if let Some(x) = take {
            e.insert(tv, x);
        }
        if let Some(x) = after {
            e.insert(av, x);
        }
        match self.expr(tm, &e)? {
            Value::List(xs) => Ok(Value::List(
                xs.into_iter()
                    .map(|x| x.field("e").expect("an element").clone())
                    .collect(),
            )),
            other => panic!("a presented result is a list, found {other:?}"),
        }
    }

    fn aggregate(&self, op: Agg, ty: &Ty, body_ty: &Ty, ms: Vec<Value>) -> R<Value> {
        let mut t = self.terms.borrow_mut();
        let elem = Ty::Record(vec![
            ("i".into(), Ty::Base(Base::I64)),
            ("v".into(), body_ty.clone()),
        ]);
        let s = t.var("%s", Ty::Set(Box::new(elem.clone())));
        let r = t.var("%r", elem.clone());
        let source = t.add(Kind::Var(s), Ty::Set(Box::new(elem.clone())));
        let row = t.add(Kind::Var(r), elem);
        let body = t.add(Kind::Field(row, "v".into()), body_ty.clone());
        let tm = t.add(
            Kind::Agg {
                op,
                var: r,
                source,
                body,
            },
            ty.clone(),
        );
        drop(t);
        let rows = ms
            .into_iter()
            .enumerate()
            .map(|(i, v)| Value::Record(vec![("i".into(), Value::Int(i as i64)), ("v".into(), v)]));
        let mut e = Env::new();
        e.insert(s, Value::set(rows));
        self.expr(tm, &e)
    }

    fn spine(&self, s: &Spine, env: &Env, b: &Built, out: &mut BTreeSet<Value>) -> R<()> {
        match s {
            Spine::Pipeline(p) => self.pipeline(p, env, b, out),
            Spine::Union(ss) => {
                for s in ss {
                    self.spine(s, env, b, out)?;
                }
                Ok(())
            }
            Spine::If(c, x, y) => {
                if truth(&self.expr(*c, env)?) {
                    self.spine(x, env, b, out)
                } else {
                    self.spine(y, env, b, out)
                }
            }
            Spine::Case(sc, arms) => {
                let v = self.expr(*sc, env)?;
                let c = v.tag().expect("a scrutinee is a constructor value");
                let (_, x, arm) = arms
                    .iter()
                    .find(|(d, _, _)| *d == c)
                    .expect("an arm per constructor");
                self.spine(arm, &bind(env, *x, v), b, out)
            }
            Spine::Let(v, e, body) => {
                let x = self.eval(e, env)?;
                self.spine(body, &bind(env, *v, x), b, out)
            }
        }
    }

    fn pipeline(&self, p: &Pipeline, env: &Env, b: &Built, out: &mut BTreeSet<Value>) -> R<()> {
        match &p.source {
            Source::Unit => self.chain(&p.chain, env, b, out),
            Source::Scan { heap, ctor, var } => {
                assert_eq!(
                    self.heaps[heap].1, *ctor,
                    "a scan names its heap's constructor"
                );
                for row in self.rows(*heap) {
                    self.chain(&p.chain, &bind(env, *var, row), b, out)?;
                }
                Ok(())
            }
        }
    }

    fn table(&self, id: BuildId, b: &Built) -> R<Table> {
        if let Some(t) = b.tables.borrow().get(&id) {
            return Ok(t.clone());
        }
        let build = &b.query.builds[id.0 as usize];
        let mut elems = BTreeSet::new();
        let none = Built {
            query: b.query,
            env: b.env.clone(),
            tables: RefCell::new(HashMap::new()),
        };
        for p in &build.pipelines {
            self.pipeline(p, &b.env, &none, &mut elems)?;
        }
        let mut table = Vec::new();
        for x in elems {
            let at = bind(&b.env, build.var, x.clone());
            let k = build
                .key
                .iter()
                .map(|k| self.expr(*k, &at))
                .collect::<R<Vec<_>>>()?;
            table.push((k, x));
        }
        b.tables.borrow_mut().insert(id, table.clone());
        Ok(table)
    }

    fn chain(&self, c: &Chain, env: &Env, b: &Built, out: &mut BTreeSet<Value>) -> R<()> {
        self.steps(&c.steps, &c.end, env, b, out)
    }

    fn steps(
        &self,
        steps: &[Step],
        end: &End,
        env: &Env,
        b: &Built,
        out: &mut BTreeSet<Value>,
    ) -> R<()> {
        let Some((first, rest)) = steps.split_first() else {
            return self.end(end, env, b, out);
        };
        match first {
            Step::Filter(c) => {
                if truth(&self.expr(*c, env)?) {
                    self.steps(rest, end, env, b, out)?;
                }
                Ok(())
            }
            Step::Bind(v, e) => {
                let x = self.eval(e, env)?;
                self.steps(rest, end, &bind(env, *v, x), b, out)
            }
            Step::Unnest(v, s) => {
                for x in set(self.expr(*s, env)?) {
                    self.steps(rest, end, &bind(env, *v, x), b, out)?;
                }
                Ok(())
            }
            Step::Probe { build, key, var } => {
                let k = key
                    .iter()
                    .map(|k| self.expr(*k, env))
                    .collect::<R<Vec<_>>>()?;
                for (bk, x) in self.table(*build, b)? {
                    if bk == k {
                        self.steps(rest, end, &bind(env, *var, x), b, out)?;
                    }
                }
                Ok(())
            }
        }
    }

    fn end(&self, end: &End, env: &Env, b: &Built, out: &mut BTreeSet<Value>) -> R<()> {
        match end {
            End::Nothing => Ok(()),
            End::Emit(t) => {
                out.insert(self.expr(*t, env)?);
                Ok(())
            }
            End::Fork(cs) => {
                for c in cs {
                    self.chain(c, env, b, out)?;
                }
                Ok(())
            }
            End::If(c, x, y) => {
                if truth(&self.expr(*c, env)?) {
                    self.chain(x, env, b, out)
                } else {
                    self.chain(y, env, b, out)
                }
            }
            End::Case(s, arms) => {
                let v = self.expr(*s, env)?;
                let c = v.tag().expect("a scrutinee is a constructor value");
                let (_, x, arm) = arms
                    .iter()
                    .find(|(d, _, _)| *d == c)
                    .expect("an arm per constructor");
                self.chain(arm, &bind(env, *x, v), b, out)
            }
            End::Index {
                index,
                key,
                var,
                routes,
            } => {
                let k = key
                    .iter()
                    .map(|k| self.expr(*k, env))
                    .collect::<R<Vec<_>>>()?;
                for (c, row) in self.entries(*index, &k) {
                    if let Some(r) = routes.iter().find(|r| r.ctor == c) {
                        assert_eq!(
                            self.heaps[&r.heap],
                            (self.indexes[index].0, c),
                            "a route fetches from its constructor's heap"
                        );
                        self.chain(&r.chain, &bind(env, *var, row), b, out)?;
                    }
                }
                Ok(())
            }
        }
    }
}
