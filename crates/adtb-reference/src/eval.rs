use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use adtb_file::{Disk, Files, Record};
use adtb_lang::check::Program;
use adtb_lang::kernel::{Agg, Const, FileFormat, Kind, Terms, Tm, Transition, Var};
use adtb_lang::schema::{Schema, TypeDefKind};
use adtb_lang::textual::PrintError;
use adtb_lang::types::{Base, CtorSet, FnId, RelId, Ty, TypeId};

use crate::big::Big;
use crate::num::{digits38, in_range};
use crate::prim;
use crate::stmt::{Rejection, Stage, Stmt};
use crate::unicode::Compiled;
use crate::value::{Value, canon32, canon64};

/// A database: the rows of each constructor relation, by relation variable and constructor index.
pub type Database = BTreeMap<(RelId, u32), BTreeSet<Value>>;

/// Returns the size of the stored form of a row of a constructor of a relation, and the row
/// limit that constructor's rows must fit, both in bytes.
pub type RowLimit<'a> = dyn Fn(RelId, u32, &Value) -> (usize, usize) + 'a;

/// A constructor of the prelude sum `Fault`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    DivisionByZero,
    ArithmeticOverflow,
    OutOfRange,
    NotFinite,
    NoSuchDate,
    UnknownZone,
    InvalidText,
    InvalidSpan,
    NegativeCount,
    NoSuchRow,
    FunctionFailed,
}

impl Fault {
    pub fn name(self) -> &'static str {
        match self {
            Fault::DivisionByZero => "DivisionByZero",
            Fault::ArithmeticOverflow => "ArithmeticOverflow",
            Fault::OutOfRange => "OutOfRange",
            Fault::NotFinite => "NotFinite",
            Fault::NoSuchDate => "NoSuchDate",
            Fault::UnknownZone => "UnknownZone",
            Fault::InvalidText => "InvalidText",
            Fault::InvalidSpan => "InvalidSpan",
            Fault::NegativeCount => "NegativeCount",
            Fault::NoSuchRow => "NoSuchRow",
            Fault::FunctionFailed => "FunctionFailed",
        }
    }
}

/// The limit a statement ran past, as the prelude's `Limit` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// A time limit or deadline, in microseconds.
    Time {
        after: i64,
    },
    Memory {
        bytes: i64,
    },
    /// The fixpoint round limit.
    Rounds {
        rounds: i64,
    },
}

/// Why an evaluation produced no value.
#[derive(Clone, Debug, PartialEq)]
pub enum Abort {
    Fault(Fault),
    /// The statement ran past a limit, rejected with `StatementLimit`.
    Limit(Limit),
    /// The statement is rejected for this reason.
    Reject(Rejection),
    /// A form the interpreter does not evaluate yet.
    NotYet(String),
    /// The term or a value breaks the invariant this text names, one the checker guarantees.
    Internal(String),
}

impl From<PrintError> for Abort {
    fn from(e: PrintError) -> Abort {
        Abort::Internal(e.message)
    }
}

impl From<Fault> for Abort {
    fn from(f: Fault) -> Abort {
        Abort::Fault(f)
    }
}

pub type R<T> = Result<T, Abort>;

pub(crate) fn not_yet<T>(what: &str) -> R<T> {
    Err(Abort::NotYet(what.to_string()))
}

pub(crate) fn internal<T>(what: impl Into<String>) -> R<T> {
    Err(Abort::Internal(what.into()))
}

/// The transition a constraint body reads: the state before the statement and the rows it
/// inserted and deleted. The interpreter's database is the state after.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Change {
    pub prior: Database,
    pub inserted: Database,
    pub deleted: Database,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arena {
    Program,
    Query,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Env(Option<Rc<Frame>>);

#[derive(Debug)]
struct Frame {
    var: Var,
    value: Value,
    next: Env,
}

impl Drop for Frame {
    /// Drop the frames this one alone holds in a loop, in constant stack.
    fn drop(&mut self) {
        let mut next = self.next.0.take();
        while let Some(f) = next {
            next = match Rc::try_unwrap(f) {
                Ok(mut f) => f.next.0.take(),
                Err(_) => None,
            };
        }
    }
}

impl Env {
    pub(crate) fn bind(&self, var: Var, value: Value) -> Env {
        Env(Some(Rc::new(Frame {
            var,
            value,
            next: self.clone(),
        })))
    }

    fn get(&self, var: Var) -> &Value {
        let mut at = &self.0;
        while let Some(f) = at {
            if f.var == var {
                return &f.value;
            }
            at = &f.next.0;
        }
        panic!("variable {var:?} is free in a closed kernel term")
    }
}

/// Where a term lives: its arena, and the type arguments its type parameters stand for.
#[derive(Clone, Debug)]
pub(crate) struct Here {
    pub(crate) arena: Arena,
    pub(crate) targs: Rc<[Ty]>,
}

#[derive(Debug)]
pub struct Closure {
    here: Here,
    var: Var,
    body: Tm,
    env: Env,
}

#[derive(Clone, Copy)]
pub(crate) struct Measure<'a>(pub(crate) &'a RowLimit<'a>);

impl std::fmt::Debug for Measure<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Measure")
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Source<'a>(pub(crate) &'a dyn Files);

impl std::fmt::Debug for Source<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Source")
    }
}

/// The reference interpreter over one database value.
#[derive(Debug)]
pub struct Interpreter<'a> {
    pub(crate) schema: &'a Schema,
    pub(crate) program: &'a Program,
    pub(crate) db: &'a Database,
    caller: Option<Value>,
    change: Option<&'a Change>,
    pub(crate) fix_limit: usize,
    pub(crate) closed: BTreeMap<(TypeId, u32), Stage>,
    pub(crate) row_limit: Option<Measure<'a>>,
    files: Option<Source<'a>>,
    pub(crate) compiled: Rc<Compiled>,
}

impl<'a> Interpreter<'a> {
    pub fn new(schema: &'a Schema, program: &'a Program, db: &'a Database) -> Interpreter<'a> {
        Interpreter {
            schema,
            program,
            db,
            caller: None,
            change: None,
            fix_limit: 10_000,
            closed: BTreeMap::new(),
            row_limit: None,
            files: None,
            compiled: Rc::default(),
        }
    }

    /// Sets the value `caller` denotes.
    pub fn with_caller(mut self, caller: Value) -> Self {
        self.caller = Some(caller);
        self
    }

    /// Sets the transition that `inserted`, `deleted` and `prior` read; without one they read the
    /// empty change.
    pub fn with_change(mut self, change: &'a Change) -> Self {
        self.change = Some(change);
        self
    }

    /// Sets the constructors not open for writes, each with its stage.
    pub fn with_closed(mut self, closed: BTreeMap<(TypeId, u32), Stage>) -> Self {
        self.closed = closed;
        self
    }

    /// Sets the files a file scan reads; without them it reads the file system, a relative path
    /// against the working directory.
    pub fn with_files(mut self, files: &'a dyn Files) -> Self {
        self.files = Some(Source(files));
        self
    }

    /// Returns the records of the files `path` names, in `format`, read at `row`.
    /// A file that cannot be read fails the scan with `BadInput` naming it at line 0.
    pub(crate) fn file_records(
        &self,
        format: FileFormat,
        path: &str,
        row: &Ty,
    ) -> R<Vec<Record<Value>>> {
        let read = match self.files {
            Some(Source(files)) => adtb_file::records(self.schema, files, format, path, row),
            None => adtb_file::records(self.schema, &Disk::default(), format, path, row),
        };
        read.map_err(|e| match e {
            adtb_file::Error::Unreadable { source, detail } => Abort::Reject(Rejection::BadInput {
                source,
                line: 0,
                detail,
            }),
            adtb_file::Error::NotYet(what) => Abort::NotYet(what.to_string()),
        })
    }

    /// Sets the measure of stored rows; without one no row is too large.
    pub fn with_row_limit(mut self, limit: &'a RowLimit<'a>) -> Self {
        self.row_limit = Some(Measure(limit));
        self
    }

    /// Sets how many steps a `fix` may take before the statement is rejected for its limit.
    pub fn with_fix_limit(mut self, steps: usize) -> Self {
        self.fix_limit = steps;
        self
    }

    /// Evaluates term `t` of `terms` with each variable of `params` bound to its value.
    pub fn eval(&self, terms: &Terms, t: Tm, params: &[(Var, Value)]) -> R<Value> {
        let run = Run {
            it: self,
            query: terms,
            stmt: None,
        };
        let env = params
            .iter()
            .fold(Env::default(), |e, (v, x)| e.bind(*v, x.clone()));
        run.eval(&run.here(Arena::Query), t, &env)
    }

    /// Evaluates term `t`, a function of the arguments, and applies it to `args` in order, or to
    /// `()` when there are none.
    pub fn apply(&self, terms: &Terms, t: Tm, args: Vec<Value>) -> R<Value> {
        let run = Run {
            it: self,
            query: terms,
            stmt: None,
        };
        let f = run.eval(&run.here(Arena::Query), t, &Env::default())?;
        run.apply_all(f, args)
    }

    pub fn call(&self, f: FnId, args: Vec<Value>) -> R<Value> {
        let run = Run {
            it: self,
            query: &self.program.terms,
            stmt: None,
        };
        run.call(f, Rc::from(Vec::new()), args)
    }

    /// Returns the prelude `Fault` constructor value of `f`.
    pub fn fault_value(&self, f: Fault) -> Value {
        let def = self.schema.ty(self.schema.prelude_type("Fault"));
        let c = def
            .ctor_index(f.name())
            .expect("the prelude declares every fault");
        Value::Ctor(c, Vec::new())
    }

    pub(crate) fn is_prelude(&self, ty: &Ty, name: &str) -> bool {
        matches!(ty, Ty::Data { def, .. } if *def == self.schema.prelude_type(name))
    }

    /// Returns the key of `row`, a row of `rel`.
    pub fn key_of(&self, rel: RelId, row: &Value) -> Value {
        let r = self.schema.relation(rel);
        let field = |f: &str| {
            row.field(f)
                .unwrap_or_else(|| panic!("a row of `{}` has its key field `{f}`", r.name))
                .clone()
        };
        match r.key.as_slice() {
            [f] if self.schema.self_key(rel, f) => field(f),
            [f] => Value::key(field(f)),
            fs => Value::key(Value::record(fs.iter().map(|f| (f.as_str(), field(f))))),
        }
    }

    fn ctors(&self, rel: RelId, set: &Option<CtorSet>) -> CtorSet {
        match set {
            Some(s) => s.clone(),
            None => {
                let n = self
                    .schema
                    .ty(self.schema.relation(rel).elem_def)
                    .ctors()
                    .len();
                (0..n as u32).collect()
            }
        }
    }

    pub(crate) fn rows(&self, db: &Database, rel: RelId, set: &Option<CtorSet>) -> BTreeSet<Value> {
        let mut out = BTreeSet::new();
        for c in self.ctors(rel, set) {
            if let Some(rs) = db.get(&(rel, c)) {
                out.extend(rs.iter().cloned());
            }
        }
        out
    }

    /// The field names of constructor `c` of a declared sum, header fields first, each in
    /// declaration order.
    pub(crate) fn field_order(&self, ty: &Ty, c: u32) -> Option<Vec<String>> {
        let Ty::Data { def, .. } = ty else {
            return None;
        };
        let TypeDefKind::Sum { header, ctors, .. } = &self.schema.ty(*def).kind else {
            return None;
        };
        let own = &ctors.get(c as usize)?.fields;
        Some(header.iter().chain(own).map(|f| f.name.clone()).collect())
    }
}

pub(crate) struct Run<'r, 'a> {
    pub(crate) it: &'r Interpreter<'a>,
    pub(crate) query: &'r Terms,
    pub(crate) stmt: Option<&'r Stmt>,
}

fn int(v: &Value) -> R<i64> {
    match v {
        Value::Int(i) => Ok(*i),
        other => internal(format!("expected an integer, found {other:?}")),
    }
}

pub(crate) fn bool_of(v: &Value) -> R<bool> {
    match v.as_bool() {
        Some(b) => Ok(b),
        None => internal(format!("expected a Bool, found {v:?}")),
    }
}

pub(crate) fn into_set(v: Value) -> R<BTreeSet<Value>> {
    match v {
        Value::Set(s) => Ok(s),
        other => internal(format!("expected a set, found {other:?}")),
    }
}

/// Returns the variable and body of the arm of a `Case` that the scrutinee's value takes.
pub(crate) fn case_arm(arms: &[(u32, Var, Tm)], v: &Value) -> R<(Var, Tm)> {
    let Some(c) = v.tag() else {
        return internal(format!("a case scrutinee is a constructor value: {v:?}"));
    };
    match arms.iter().find(|(d, _, _)| *d == c) {
        Some((_, x, body)) => Ok((*x, *body)),
        None => internal(format!("a case names constructor {c} of its scrutinee")),
    }
}

/// Compares two lists of ordering values, each key ascending unless its flag says descending.
fn cmp_keys(a: &[Value], b: &[Value], desc: &[bool]) -> Ordering {
    for ((x, y), d) in a.iter().zip(b).zip(desc) {
        let o = x.cmp(y);
        let o = if *d { o.reverse() } else { o };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

impl Run<'_, '_> {
    pub(crate) fn here(&self, arena: Arena) -> Here {
        Here {
            arena,
            targs: Rc::from(Vec::new()),
        }
    }

    pub(crate) fn terms(&self, a: Arena) -> &Terms {
        match a {
            Arena::Program => &self.it.program.terms,
            Arena::Query => self.query,
        }
    }

    pub(crate) fn ty(&self, h: &Here, t: Tm) -> Ty {
        let ty = self.terms(h.arena).ty(t);
        if h.targs.is_empty() {
            ty.clone()
        } else {
            ty.subst(&h.targs)
        }
    }

    pub(crate) fn set(&self, h: &Here, t: Tm, env: &Env) -> R<BTreeSet<Value>> {
        into_set(self.eval(h, t, env)?)
    }

    fn eval_all(&self, h: &Here, ts: &[Tm], env: &Env) -> R<Vec<Value>> {
        ts.iter().map(|t| self.eval(h, *t, env)).collect()
    }

    pub(crate) fn apply(&self, f: Value, arg: Value) -> R<Value> {
        let Value::Fn(c) = f else {
            return internal(format!("applied a value that is not a function: {f:?}"));
        };
        self.eval(&c.here, c.body, &c.env.bind(c.var, arg))
    }

    fn apply_all(&self, f: Value, args: Vec<Value>) -> R<Value> {
        if args.is_empty() {
            return self.apply(f, Value::unit());
        }
        args.into_iter().try_fold(f, |g, a| self.apply(g, a))
    }

    /// Folds the inputs `ms` of user-defined aggregate `i` from its `init` with its `step`, then
    /// finishes the state; `None` over no input when `ty`, the result type, is an `Option` of the
    /// declared result.
    fn user_aggregate(&self, i: u32, ms: Vec<Value>, ty: &Ty) -> R<Value> {
        let def = &self.it.schema.aggregates[i as usize];
        let parts = self.it.program.aggregates[i as usize].expect("a checked aggregate");
        let direct = *ty == def.result;
        if ms.is_empty() && !direct {
            return Ok(Value::none());
        }
        let h = Here {
            arena: Arena::Program,
            targs: Rc::from(Vec::new()),
        };
        let mut state = self.eval(&h, parts.init, &Env::default())?;
        let step = self.eval(&h, parts.step, &Env::default())?;
        for m in ms {
            state = self.apply_all(step.clone(), vec![state, m])?;
        }
        let finish = self.eval(&h, parts.finish, &Env::default())?;
        let v = self.apply(finish, state)?;
        Ok(if direct { v } else { Value::some(v) })
    }

    fn call(&self, f: FnId, targs: Rc<[Ty]>, args: Vec<Value>) -> R<Value> {
        let Some(tm) = self.it.program.functions[f.0 as usize] else {
            return not_yet(&format!(
                "`{}` is a Wasm or host function",
                self.it.schema.function(f).name
            ));
        };
        let h = Here {
            arena: Arena::Program,
            targs,
        };
        let g = self.eval(&h, tm, &Env::default())?;
        self.apply_all(g, args)
    }

    /// Returns the value of term `t` under `env`. Follow the taken branch of each `case`, `if`
    /// and `let` in a loop: the stack grows only with the nesting of other terms.
    pub(crate) fn eval(&self, h: &Here, t: Tm, env: &Env) -> R<Value> {
        let (mut t, mut env) = (t, env.clone());
        loop {
            t = match self.terms(h.arena).kind(t) {
                Kind::Case { scrutinee, arms } => {
                    let v = self.eval(h, *scrutinee, &env)?;
                    let (x, body) = case_arm(arms, &v)?;
                    env = env.bind(x, v);
                    body
                }
                Kind::If(c, a, b) => {
                    if bool_of(&self.eval(h, *c, &env)?)? {
                        *a
                    } else {
                        *b
                    }
                }
                Kind::Let(x, e, b) => {
                    let v = self.eval(h, *e, &env)?;
                    env = env.bind(*x, v);
                    *b
                }
                _ => return self.step(h, t, &env),
            };
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per term kind; each arm would have to be a function"
    )]
    fn step(&self, h: &Here, t: Tm, env: &Env) -> R<Value> {
        let terms = self.terms(h.arena);
        let it = self.it;
        match terms.kind(t) {
            Kind::Var(v) => Ok(env.get(*v).clone()),
            Kind::Const(c) => constant(c, &self.ty(h, t)),
            Kind::Prim(p, args) => {
                let vs = self.eval_all(h, args, env)?;
                let tys: Vec<Ty> = args.iter().map(|a| self.ty(h, *a)).collect();
                prim::apply(it, *p, vs, &self.ty(h, t), &tys)
            }
            Kind::Record(fs) => {
                let mut out = Vec::new();
                for (n, x) in fs {
                    out.push((n.clone(), self.eval(h, *x, env)?));
                }
                Ok(Value::Record(out))
            }
            Kind::Field(x, name) => {
                let v = self.eval(h, *x, env)?;
                match v.field(name) {
                    Some(f) => Ok(f.clone()),
                    None => internal(format!("{v:?} has no field `{name}`")),
                }
            }
            Kind::Ctor { ctor, fields } => {
                let mut given = Vec::new();
                for (n, x) in fields {
                    given.push((n.clone(), self.eval(h, *x, env)?));
                }
                let given = match it.field_order(&self.ty(h, t), *ctor) {
                    Some(order) => order
                        .into_iter()
                        .map(|n| {
                            let Some(i) = given.iter().position(|(g, _)| *g == n) else {
                                return internal(format!("constructor field `{n}` is given"));
                            };
                            Ok(given.swap_remove(i))
                        })
                        .collect::<R<_>>()?,
                    None => given,
                };
                Ok(Value::Ctor(*ctor, given))
            }
            Kind::Case { .. } | Kind::If(..) | Kind::Let(..) => self.eval(h, t, env),
            Kind::Lam(x, b) => Ok(Value::Fn(Rc::new(Closure {
                here: h.clone(),
                var: *x,
                body: *b,
                env: env.clone(),
            }))),
            Kind::App(f, a) => {
                let g = self.eval(h, *f, env)?;
                let x = self.eval(h, *a, env)?;
                self.apply(g, x)
            }
            Kind::Call {
                func,
                type_args,
                args,
            } => {
                let vs = self.eval_all(h, args, env)?;
                let targs: Vec<Ty> = type_args.iter().map(|a| a.subst(&h.targs)).collect();
                self.call(*func, Rc::from(targs), vs)
            }
            Kind::Caller => match &it.caller {
                Some(c) => Ok(c.clone()),
                None => not_yet("`caller` with no caller given"),
            },
            Kind::Key(x) => {
                let row = self.eval(h, *x, env)?;
                let Ty::Key { rel, .. } = self.ty(h, t) else {
                    return internal("`key` has a key type");
                };
                Ok(it.key_of(rel, &row))
            }
            Kind::KeyOf(_, x) => Ok(Value::key(self.eval(h, *x, env)?)),
            Kind::Deref { key, .. } => {
                let k = self.eval(h, *key, env)?;
                let Ty::Key { rel, ctors } = self.ty(h, *key) else {
                    return internal("`deref` follows a key");
                };
                let ctors = match self.ty(h, t) {
                    Ty::Data { ctors: Some(s), .. } => Some(s),
                    _ => ctors,
                };
                it.rows(it.db, rel, &ctors)
                    .into_iter()
                    .find(|row| it.key_of(rel, row) == k)
                    .ok_or(Abort::Fault(Fault::NoSuchRow))
            }
            Kind::Tag(x) => {
                let v = self.eval(h, *x, env)?;
                let Some(c) = v.tag() else {
                    return internal("`tag` takes a constructor value");
                };
                Ok(Value::Ctor(c, Vec::new()))
            }
            Kind::Try(x) => match self.eval(h, *x, env) {
                Ok(v) => Ok(Value::Ctor(0, vec![("value".into(), v)])),
                Err(Abort::Fault(f)) => {
                    Ok(Value::Ctor(1, vec![("error".into(), it.fault_value(f))]))
                }
                Err(other) => Err(other),
            },
            Kind::Rel(r, s) => Ok(Value::Set(it.rows(it.db, *r, s))),
            Kind::Transition(tr, r, s) => {
                let empty = Database::new();
                let db = match (tr, it.change) {
                    (Transition::Prior, None) => it.db,
                    (Transition::Prior, Some(c)) => &c.prior,
                    (Transition::Inserted, Some(c)) => &c.inserted,
                    (Transition::Deleted, Some(c)) => &c.deleted,
                    (_, None) => &empty,
                };
                Ok(Value::Set(it.rows(db, *r, s)))
            }
            Kind::Restrict(x, s) => {
                let a = self.set(h, *x, env)?;
                Ok(Value::Set(
                    a.into_iter()
                        .filter(|v| v.tag().is_some_and(|c| s.contains(&c)))
                        .collect(),
                ))
            }
            Kind::EmptySet => Ok(Value::Set(BTreeSet::new())),
            Kind::Singleton(x) => Ok(Value::set([self.eval(h, *x, env)?])),
            Kind::Union(a, b) => {
                let mut x = self.set(h, *a, env)?;
                x.extend(self.set(h, *b, env)?);
                Ok(Value::Set(x))
            }
            Kind::For(x, src, body) => {
                let mut out = BTreeSet::new();
                for v in self.set(h, *src, env)? {
                    out.extend(self.set(h, *body, &env.bind(*x, v))?);
                }
                Ok(Value::Set(out))
            }
            Kind::IsEmpty(x) => Ok(Value::bool(self.set(h, *x, env)?.is_empty())),
            Kind::Exists(x) => Ok(Value::bool(!self.set(h, *x, env)?.is_empty())),
            Kind::Agg {
                op,
                var,
                source,
                body,
            } => {
                let a = self.set(h, *source, env)?;
                let mut ms = Vec::with_capacity(a.len());
                for v in a {
                    ms.push(self.eval(h, *body, &env.bind(*var, v))?);
                }
                match op {
                    Agg::User(i) => self.user_aggregate(*i, ms, &self.ty(h, t)),
                    _ => aggregate(*op, ms, &self.ty(h, t), &self.ty(h, *body)),
                }
            }
            Kind::Group { var, source, by } => {
                let mut groups: BTreeMap<Value, BTreeSet<Value>> = BTreeMap::new();
                for v in self.set(h, *source, env)? {
                    let k = self.eval(h, *by, &env.bind(*var, v.clone()))?;
                    groups.entry(k).or_default().insert(v);
                }
                Ok(Value::set(groups.into_iter().map(|(k, rows)| {
                    Value::Record(vec![("key".into(), k), ("rows".into(), Value::Set(rows))])
                })))
            }
            Kind::Top {
                count,
                keys,
                var,
                source,
            } => {
                let c = int(&self.eval(h, *count, env)?)?;
                let a = self.set(h, *source, env)?;
                let sorted = self.ordered(h, a, keys, *var, env)?;
                if c < 0 {
                    return Err(Fault::NegativeCount.into());
                }
                Ok(Value::set(
                    sorted.into_iter().take(c as usize).map(|(_, v)| v),
                ))
            }
            Kind::Present {
                var,
                source,
                keys,
                take,
                after,
            } => {
                let a = self.set(h, *source, env)?;
                let sorted = self.ordered(h, a, keys, *var, env)?;
                let take = match take {
                    Some(n) => Some(int(&self.eval(h, *n, env)?)?),
                    None => None,
                };
                let cursor = match after {
                    Some(c) => match self.eval(h, *c, env)? {
                        Value::Ctor(0, _) => None,
                        Value::Ctor(1, mut fs) => {
                            let Some((_, v)) = fs.pop() else {
                                return internal("`Some` has a value");
                            };
                            Some(match (keys.len(), v) {
                                (1, v) => vec![v],
                                (_, Value::Record(fs)) => fs.into_iter().map(|(_, v)| v).collect(),
                                (_, v) => {
                                    return internal(format!(
                                        "a compound cursor is a record, found {v:?}"
                                    ));
                                }
                            })
                        }
                        v => return internal(format!("a cursor is an `Option`, found {v:?}")),
                    },
                    None => None,
                };
                if take.is_some_and(|n| n < 0) {
                    return Err(Fault::NegativeCount.into());
                }
                let desc: Vec<bool> = keys.iter().map(|(_, d)| *d).collect();
                let rest = sorted.into_iter().filter(|(k, _)| match &cursor {
                    Some(c) => cmp_keys(c, k, &desc) == Ordering::Less,
                    None => true,
                });
                let n = take.map_or(usize::MAX, |n| n as usize);
                Ok(Value::List(rest.take(n).map(|(_, v)| v).collect()))
            }
            Kind::Fix(binds, body) => {
                let mut cur: Vec<Value> =
                    binds.iter().map(|_| Value::Set(BTreeSet::new())).collect();
                let mut steps = 0;
                loop {
                    let inner = binds
                        .iter()
                        .zip(&cur)
                        .fold(env.clone(), |e, ((x, _), s)| e.bind(*x, s.clone()));
                    let next: Vec<Value> = binds
                        .iter()
                        .map(|(_, q)| self.eval(h, *q, &inner))
                        .collect::<R<_>>()?;
                    if next == cur {
                        return self.eval(h, *body, &inner);
                    }
                    steps += 1;
                    if steps > it.fix_limit {
                        return Err(Abort::Limit(Limit::Rounds {
                            rounds: it.fix_limit as i64,
                        }));
                    }
                    cur = next;
                }
            }
            Kind::At { .. } => not_yet("`at` reads history, and this database value has none"),
            Kind::Unreachable => internal("evaluation reached a position exhaustiveness excludes"),
            Kind::Fresh(g, r) => {
                let stmt = self
                    .stmt
                    .expect("`fresh` occurs only in the fills of a statement's writes");
                Ok(stmt.draw(*g, *r, &self.ty(h, t)))
            }
            Kind::Only(x) => {
                let mut s = self.set(h, *x, env)?.into_iter();
                match s.next() {
                    Some(v) => Ok(v),
                    None => internal("`only` takes a set of one element"),
                }
            }
            Kind::Reject(m) => {
                let Value::Text(detail) = self.eval(h, *m, env)? else {
                    return internal("`reject` takes a Text");
                };
                Err(Abort::Reject(Rejection::BadInput {
                    source: String::new(),
                    line: 0,
                    detail,
                }))
            }
            Kind::Skip
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Par(_)
            | Kind::Fail(_)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => {
                internal("a write or block is evaluated by `Interpreter::run`, never as a query")
            }
        }
    }

    /// Returns the elements of `a` with their ordering values, sorted by them.
    fn ordered(
        &self,
        h: &Here,
        a: BTreeSet<Value>,
        keys: &[(Tm, bool)],
        var: Var,
        env: &Env,
    ) -> R<Vec<(Vec<Value>, Value)>> {
        let mut out = Vec::with_capacity(a.len());
        for v in a {
            let inner = env.bind(var, v.clone());
            let ks = keys
                .iter()
                .map(|(k, _)| self.eval(h, *k, &inner))
                .collect::<R<Vec<_>>>()?;
            out.push((ks, v));
        }
        let desc: Vec<bool> = keys.iter().map(|(_, d)| *d).collect();
        out.sort_by(|a, b| cmp_keys(&a.0, &b.0, &desc));
        Ok(out)
    }
}

fn constant(c: &Const, ty: &Ty) -> R<Value> {
    Ok(match (c, ty) {
        (Const::Int(i), Ty::Base(Base::Decimal(s))) => {
            Value::Decimal(i * 10i128.pow(u32::from(*s)), *s)
        }
        (Const::Int(i), Ty::Base(Base::F32)) => Value::F32(*i as f32),
        (Const::Int(i), Ty::Base(Base::F64)) => Value::F64(*i as f64),
        (Const::Int(i), _) => Value::Int(*i as i64),
        (Const::Decimal(d), Ty::Base(Base::Decimal(s))) => Value::Decimal(*d, *s),
        (Const::Decimal(d), t) => return internal(format!("decimal constant {d} at {t:?}")),
        (Const::Float(f), Ty::Base(Base::F32)) => Value::F32(canon32(*f as f32)),
        (Const::Float(f), _) => Value::F64(canon64(*f)),
        (Const::Text(s), _) => Value::Text(s.clone()),
        (Const::Duration(d), _) => Value::Duration(*d),
    })
}

/// Returns the value of an aggregate over the body values of its bindings, at the node's type
/// `ty`, where a semigroup aggregate's type is its body type `body` exactly when its range is
/// nonempty.
fn aggregate(op: Agg, ms: Vec<Value>, ty: &Ty, body: &Ty) -> R<Value> {
    let direct = ty == body;
    let wrap = |v: Value| if direct { v } else { Value::some(v) };
    match op {
        Agg::Count => Ok(Value::Int(ms.len() as i64)),
        Agg::Sum => sum(ms, ty),
        Agg::Min | Agg::Max => {
            let m = if op == Agg::Min {
                ms.into_iter().min()
            } else {
                ms.into_iter().max()
            };
            match m {
                Some(v) => Ok(wrap(v)),
                None if direct => internal("an aggregate over a nonempty range saw no binding"),
                None => Ok(Value::none()),
            }
        }
        Agg::Avg => {
            if ms.is_empty() {
                if direct {
                    return internal("an aggregate over a nonempty range saw no binding");
                }
                return Ok(Value::none());
            }
            let n = ms.len();
            let v = match &ms[0] {
                Value::F32(_) => {
                    let s = ms.iter().try_fold(0f32, |a, m| match m {
                        Value::F32(x) => Ok(a + x),
                        _ => internal("`avg` over F32"),
                    })?;
                    Value::F32(canon32(s / n as f32))
                }
                _ => {
                    let s = ms.iter().try_fold(0f64, |a, m| match m {
                        Value::F64(x) => Ok(a + x),
                        _ => internal("`avg` over F64"),
                    })?;
                    Value::F64(canon64(s / n as f64))
                }
            };
            Ok(wrap(v))
        }
        Agg::User(_) => unreachable!("a user-defined aggregate folds with its own terms"),
    }
}

fn sum(ms: Vec<Value>, ty: &Ty) -> R<Value> {
    match ty {
        Ty::Base(b @ (Base::I8 | Base::I16 | Base::I32 | Base::I64)) => {
            let total = ms.iter().try_fold(Big::zero(), |a, m| {
                Ok::<_, Abort>(a.add(&Big::from_i128(i128::from(int(m)?))))
            })?;
            total
                .to_i128()
                .and_then(|v| in_range(v, *b))
                .map(Value::Int)
                .ok_or(Abort::Fault(Fault::ArithmeticOverflow))
        }
        Ty::Base(Base::Decimal(s)) => {
            let total = ms.iter().try_fold(Big::zero(), |a, m| match m {
                Value::Decimal(d, t) if t == s => Ok(a.add(&Big::from_i128(*d))),
                other => internal(format!("`sum` at Decimal {s} over {other:?}")),
            })?;
            digits38(&total)
                .map(|d| Value::Decimal(d, *s))
                .ok_or(Abort::Fault(Fault::ArithmeticOverflow))
        }
        Ty::Base(Base::F32) => {
            let s = ms.iter().try_fold(0f32, |a, m| match m {
                Value::F32(x) => Ok(a + x),
                _ => internal("`sum` over F32"),
            })?;
            Ok(Value::F32(canon32(s)))
        }
        Ty::Base(Base::F64) => {
            let s = ms.iter().try_fold(0f64, |a, m| match m {
                Value::F64(x) => Ok(a + x),
                _ => internal("`sum` over F64"),
            })?;
            Ok(Value::F64(canon64(s)))
        }
        t => internal(format!("`sum` at {t:?}")),
    }
}
