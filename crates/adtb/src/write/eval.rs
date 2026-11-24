use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::ControlFlow;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use adtb_file::Record;
use adtb_lang::capability::RootKind;
use adtb_lang::kernel::{Kind, Terms, Tm, Valid, ValidTerms, Var};
use adtb_lang::schema::prelude::ctor;
use adtb_lang::schema::{Committed, Generator, Schema, Stage};
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::exec::types::ctors;
use crate::exec::{self, Footprint, Held, Val, from_stored_row, to_key};
use crate::plan::IndexId;
use crate::runtime::Runtime;

use super::apply::UndoLog;
use super::change::ChangeSet;
use super::fresh::{self, Occurrences};
use super::input::{self, Input, Local, is_temp, temp_key};
use super::schema::{RefField, Rel};
use super::violation::{Violations, refuse, violate_key};
use super::{Broken, Counts, Database, Done, Error, Rejection, TermKind, Txn};

pub type Env = BTreeMap<Var, Val>;

/// One effect of a write.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Eff {
    Ins(RelId, Val),
    Del(RelId, Val),
    Rep(RelId, Val, Val),
}

impl Eff {
    fn rel(&self) -> RelId {
        match self {
            Eff::Ins(r, _) | Eff::Del(r, _) | Eff::Rep(r, ..) => *r,
        }
    }
}

impl Footprint for Eff {
    fn footprint(&self) -> usize {
        match self {
            Eff::Ins(r, v) | Eff::Del(r, v) => r.footprint() + v.footprint(),
            Eff::Rep(r, v, n) => r.footprint() + v.footprint() + n.footprint(),
        }
    }
}

/// The effects of a statement's writes, held in the grant of its rows.
pub type Effects = Held<BTreeSet<Eff>>;

/// Rows of a relation by key, each with its constructor, held in a statement's grant.
pub type Referring = Held<BTreeMap<Val, (u32, Val)>>;

/// The arena a term belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Arena {
    Statement,
    /// The normal program, in a root of the kind given.
    Program(RootKind),
}

impl Footprint for Arena {
    fn footprint(&self) -> usize {
        size_of::<Arena>()
    }
}

/// A term planned with the variables bound around it.
type PlanKey = (Arena, Tm, Vec<Var>);

/// One statement of a transaction.
pub struct Stmt<'a, R: Runtime> {
    pub db: &'a Database<R>,
    pub tx: &'a Txn,
    /// The database's schema, or one extending it with the transaction's temp relations.
    schema: &'a Committed,
    terms: ValidTerms,
    /// The kind of root the statement's terms were normalised as.
    kind: RootKind,
    /// What each variable standing for a temp relation's rows or a file scan stands for.
    inputs: BTreeMap<Var, Input>,
    /// The records of each file scan.
    scans: RefCell<BTreeMap<Var, Vec<Record<Val>>>>,
    /// Why each file scan holding a record that does not convert fails.
    unconverted: RefCell<BTreeMap<Var, Rejection>>,
    /// What the transaction holds outside the file, while the statement runs.
    local: RefCell<Local>,
    fresh: Occurrences,
    root: Tm,
    clock: i64,
    caller: Option<Val>,
    pub options: exec::Options,
    pub stop: super::Stop,
    /// The grant the statement's reads and executions take their memory from.
    grant: Arc<exec::Grant>,
    /// The grant the statement holds the rows it touches in: its call's, or one of no limit.
    rows_in: Arc<exec::Grant>,
    /// The spill file of the statement's own reads.
    spill: exec::Spill<R>,
    pub store: Arc<exec::Store<R>>,
    drawn: RefCell<Held<BTreeMap<(Arena, Val), Val>>>,
    load: Cell<Option<i64>>,
    pub undo: RefCell<UndoLog>,
    plans: RefCell<HashMap<PlanKey, Arc<exec::Program>>>,
}

impl<'a, R: Runtime> Stmt<'a, R> {
    pub fn new(
        db: &'a Database<R>,
        tx: &'a Txn,
        text: super::Text<'a>,
        root: Tm,
        options: exec::Options,
        (stop, grant, rows): (super::Stop, Arc<exec::Grant>, Arc<exec::Grant>),
    ) -> Result<Self, Error> {
        let schema = text.schema;
        let mut terms = text.terms.clone();
        let mut fresh = Occurrences::new();
        let root = fresh::rewrite(schema, &mut terms, root, &[], &mut fresh);
        let (root, inputs) = input::rewrite(schema, &mut terms, root);
        let free = text
            .params
            .iter()
            .map(|(v, _)| *v)
            .chain(inputs.keys().copied())
            .collect();
        let terms = ValidTerms::new(schema, terms, &[(root, free)]).map_err(Broken::IllTyped)?;
        let scans_files = (inputs.values()).any(|i| matches!(i, Input::File { .. }));
        let rows_in = if scans_files {
            super::grant::unbounded()
        } else {
            rows
        };
        Ok(Stmt {
            db,
            tx,
            schema,
            terms,
            kind: text.items.kind(),
            inputs,
            scans: RefCell::default(),
            unconverted: RefCell::default(),
            local: RefCell::default(),
            fresh,
            root,
            clock: text.clock,
            caller: text.caller.cloned(),
            options,
            stop,
            spill: exec::Spill::new(db.rt.clone(), grant.clone(), options.spill_bytes),
            grant,
            store: db.store(tx),
            drawn: RefCell::new(rows_in.held()),
            undo: RefCell::new(UndoLog::new(&rows_in)),
            rows_in,
            load: Cell::default(),
            plans: RefCell::default(),
        })
    }

    /// Returns the grant the statement's reads and executions take their memory from.
    pub(super) fn grant(&self) -> &Arc<exec::Grant> {
        &self.grant
    }

    /// Returns the grant the statement holds the rows it touches in.
    pub(super) fn rows_in(&self) -> &Arc<exec::Grant> {
        &self.rows_in
    }

    /// Returns a statement that only reads, under `grant`.
    pub fn reader(db: &'a Database<R>, tx: &'a Txn, grant: Arc<exec::Grant>) -> Self {
        let (terms, root) = ValidTerms::unit(&db.schema);
        Stmt {
            db,
            tx,
            schema: &db.schema,
            terms,
            kind: RootKind::Adhoc,
            inputs: BTreeMap::new(),
            scans: RefCell::default(),
            unconverted: RefCell::default(),
            local: RefCell::default(),
            fresh: Occurrences::new(),
            root,
            clock: 0,
            caller: None,
            options: db.options,
            stop: super::Stop::default(),
            spill: exec::Spill::new(db.rt.clone(), grant.clone(), db.options.spill_bytes),
            drawn: RefCell::new(grant.held()),
            undo: RefCell::new(UndoLog::new(&grant)),
            rows_in: grant.clone(),
            grant,
            store: db.store(tx),
            load: Cell::default(),
            plans: RefCell::default(),
        }
    }

    pub fn rel(&self, r: RelId) -> &'a Rel {
        self.db
            .rels
            .get(&r)
            .unwrap_or_else(|| panic!("the catalog describes relation {r:?}"))
    }

    fn arena(&self, a: Arena) -> Valid<'_> {
        match a {
            Arena::Statement => self.terms.valid(),
            Arena::Program(_) => self.db.program.valid(),
        }
    }

    /// Run the statement, each variable of `params` bound, over `local`, what its transaction
    /// holds outside the file.
    ///
    /// On an `Err`, `local` may hold scans of the statement and changes to temp relations of
    /// its writes, and the statement's effects on the file are left for [`undo`](Stmt::undo).
    pub(super) fn run(&self, params: &[(Var, Val)], local: &mut Local) -> Result<Done, Error> {
        std::mem::swap(&mut *self.local.borrow_mut(), local);
        let out = self.run_here(params);
        std::mem::swap(&mut *self.local.borrow_mut(), local);
        out
    }

    fn run_here(&self, params: &[(Var, Val)]) -> Result<Done, Error> {
        let mut env: Env = params.iter().cloned().collect();
        let _inputs = self.bind_inputs(&mut env)?;
        let mut effects: Effects = self.rows_in.held();
        let t = self.root;
        let result = match self.terms.kind(t) {
            Kind::Transact(b, _) => self.block(*b, &env, &mut effects)?,
            _ if *self.terms.ty(t) == Ty::Change => {
                self.write(t, &env, &mut effects)?;
                self.grant.hold(Val::record(Vec::new()))?
            }
            _ => {
                return Ok(Done {
                    result: self.eval(Arena::Statement, t, &env)?,
                    inserted: Counts::new(),
                    deleted: Counts::new(),
                });
            }
        };
        let temps = effects.split_off_where(|e| is_temp(self.schema, e.rel()));
        let (mut inserted, mut deleted) = self.boundary(effects)?;
        self.local.borrow_mut().apply(self.schema, temps.iter())?;
        let mut set = ChangeSet::new(&self.rows_in);
        for e in temps {
            match e {
                Eff::Ins(r, v) => set.insert(r, v)?,
                Eff::Del(r, v) => set.delete(r, v)?,
                Eff::Rep(r, v, n) => {
                    set.delete(r, v)?;
                    set.insert(r, n)?;
                }
            }
        }
        set.normalise();
        let (plus, minus) = set.counts();
        inserted.extend(plus);
        deleted.extend(minus);
        Ok(Done {
            result,
            inserted,
            deleted,
        })
    }

    /// Bind in `env` the variable of each read of a temp relation to its rows, and of each file
    /// scan to its rows when every record converts. Returns the rows bound, held in the grant of
    /// the statement's rows.
    fn bind_inputs(&self, env: &mut Env) -> Result<Vec<Held<Val>>, Error> {
        let bound: BTreeSet<Var> = env.keys().copied().collect();
        let mut facts = crate::plan::Facts::default();
        let mut held = Vec::new();
        let mut hold = |env: &mut Env, v: Var, rows: BTreeSet<Val>| -> Result<(), Error> {
            let rows = self.rows_in.hold(Val::set(rows))?;
            env.insert(v, (*rows).clone());
            held.push(rows);
            Ok(())
        };
        for (v, input) in &self.inputs {
            match input {
                Input::Temp { rel, ctors } => {
                    let rows = self.local.borrow().rows(*rel, ctors.as_ref())?;
                    hold(env, *v, rows)?;
                }
                Input::Volatile { rel, ctors } => {
                    let mut rows = BTreeSet::new();
                    for row in self.db.volatile_rows(*rel)? {
                        let c = row.ctor_of()?;
                        if ctors.as_ref().is_none_or(|s| s.contains(&c)) {
                            rows.insert(row);
                        }
                    }
                    hold(env, *v, rows)?;
                }
                Input::File { format, path, row } => {
                    if !facts.free(&self.terms, *path).is_subset(&bound) {
                        return Err(Error::Exec(exec::Error::Unsupported(
                            "a file scan whose path depends on a row".into(),
                        )));
                    }
                    let Val::Text(at) = self.eval(Arena::Statement, *path, env)?.into_inner()
                    else {
                        return Err(Broken::ScanPathNotText.into());
                    };
                    let records = self.local.borrow_mut().scan(
                        self.schema,
                        &self.db.files,
                        self.stop.confined,
                        (*format, &at, row),
                    )?;
                    let mut rows = BTreeSet::new();
                    let mut failed = None;
                    for r in &records {
                        match &r.row {
                            Ok(x) => {
                                rows.insert(x.clone());
                            }
                            Err(detail) => {
                                failed = Some(Rejection::BadInput {
                                    source: r.source.clone(),
                                    line: r.line,
                                    detail: detail.clone(),
                                });
                                break;
                            }
                        }
                    }
                    match failed {
                        Some(why) => {
                            self.unconverted.borrow_mut().insert(*v, why);
                        }
                        None => hold(env, *v, rows)?,
                    }
                    self.scans.borrow_mut().insert(*v, records);
                }
            }
        }
        Ok(held)
    }

    /// Returns the value of term `t` of arena `a` in `env`, its placeholders filled, held in the
    /// statement's grant.
    pub fn eval(&self, a: Arena, t: Tm, env: &Env) -> Result<Held<Val>, Error> {
        if a == Arena::Statement {
            let unconverted = self.unconverted.borrow();
            let mut facts = crate::plan::Facts::default();
            if let Some(why) = unconverted
                .iter()
                .find_map(|(v, why)| facts.mentions(&self.terms, t, &[*v]).then_some(why))
            {
                return Err(Error::Rejected(why.clone()));
            }
        }
        let params: Vec<Var> = env.keys().copied().collect();
        let key = (a, t, params);
        let cached = self.plans.borrow().get(&key).cloned();
        let program = match cached {
            Some(p) => p,
            None => {
                let kind = match a {
                    Arena::Statement => self.kind,
                    Arena::Program(kind) => kind,
                };
                let x =
                    self.db
                        .program(self.schema, self.arena(a), (t, kind), self.caller.clone())?;
                let x = Arc::new(x);
                self.plans.borrow_mut().insert(key, x.clone());
                x
            }
        };
        let call = self
            .db
            .call(&self.store, self.options, &self.stop, &self.grant)?;
        let bound = program.bind(|v| env.get(&v).cloned())?;
        let v = exec::execute(self.db.rt.clone(), bound, call)?;
        v.try_map(|v| self.fill(a, v))
    }

    /// Returns the plan of term `t` of the statement.
    pub fn plan_of(&self, t: Tm) -> Result<crate::plan::Plan, Error> {
        self.db
            .plan(self.schema, self.arena(Arena::Statement), (t, self.kind))
    }

    pub fn schema(&self) -> &'a Committed {
        self.schema
    }

    pub fn terms(&self) -> &Terms {
        &self.terms
    }

    pub fn root(&self) -> Tm {
        self.root
    }

    pub(super) fn input(&self, v: &Var) -> Option<&Input> {
        self.inputs.get(v)
    }

    /// Returns the records file scan `scan` read.
    pub(super) fn scan_records(&self, scan: Var) -> Result<Vec<Record<Val>>, Error> {
        let scans = self.scans.borrow();
        Ok(scans.get(&scan).cloned().ok_or(Broken::ScanUnread)?)
    }

    pub(super) fn local(&self) -> &RefCell<Local> {
        &self.local
    }

    /// Returns the `load` the statement drew, `None` until it draws one.
    pub(super) fn drawn_load(&self) -> &Cell<Option<i64>> {
        &self.load
    }

    /// Returns `v`, a value of arena `a`, with every placeholder replaced by its drawn value.
    fn fill(&self, a: Arena, v: Val) -> Result<Val, Error> {
        let occurrences = match a {
            Arena::Statement => &self.fresh,
            Arena::Program(_) => &self.db.program_fresh,
        };
        let mut draw = |occ: u32, p: &Val| {
            let key = (a, p.clone());
            if let Some(v) = self.drawn.borrow().get(&key) {
                return Ok(v.clone());
            }
            let (g, r) = occurrences[&occ];
            let v = self.draw(g, r)?;
            self.drawn.borrow_mut().insert(key, v.clone())?;
            Ok(v)
        };
        fresh::fill(v, &mut draw)
    }

    fn draw(&self, g: Generator, r: RelId) -> Result<Val, Error> {
        Ok(match g {
            Generator::Clock => Val::Instant(self.clock),
            Generator::Sequence => Val::Int(self.db.next_sequence(r)?),
            Generator::Uuidv7 => {
                let ms = self.clock.div_euclid(1000) as u64;
                let n = self.db.uuids.fetch_add(1, Ordering::Relaxed);
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

    pub(super) fn set(&self, t: Tm, env: &Env) -> Result<Held<BTreeSet<Val>>, Error> {
        let v = self.eval(Arena::Statement, t, env)?;
        if !matches!(*v, Val::Set(_)) {
            return Err(Broken::RowsNotSet(v.into_inner()).into());
        }
        Ok(v.into_set()?)
    }

    /// Evaluate block term `t`, adding the effects of its writes to `out`. Returns its result.
    fn block(&self, t: Tm, env: &Env, out: &mut Effects) -> Result<Held<Val>, Error> {
        let (mut t, mut env) = (t, env.clone());
        let mut bound = Vec::new();
        loop {
            t = match self.terms.kind(t) {
                Kind::Block(w, k) => {
                    self.write(*w, &env, out)?;
                    *k
                }
                Kind::LetBang(x, w, k) => {
                    let v = self.bind_write(*w, &env, out)?;
                    env = bind(&env, *x, (*v).clone());
                    bound.push(v);
                    *k
                }
                Kind::Let(x, e, k) => {
                    let v = self.eval(Arena::Statement, *e, &env)?;
                    env = bind(&env, *x, (*v).clone());
                    bound.push(v);
                    *k
                }
                _ => return self.eval(Arena::Statement, t, &env),
            };
        }
    }

    /// Evaluate write `t`, adding its effects to `out`.
    pub(super) fn write(&self, t: Tm, env: &Env, out: &mut Effects) -> Result<(), Error> {
        match self.terms.kind(t).clone() {
            Kind::Skip => Ok(()),
            Kind::Insert(r, rows) => {
                for v in self.records(rows, env, None)? {
                    out.insert(Eff::Ins(r, v))?;
                }
                Ok(())
            }
            Kind::Delete(r, rows) => {
                for v in self.set(rows, env)? {
                    if self.in_snapshot(r, &v)? {
                        out.insert(Eff::Del(r, v))?;
                    }
                }
                Ok(())
            }
            Kind::Replace(..) | Kind::Upsert { .. } | Kind::Rejects { .. } => {
                self.bind_write(t, env, out).map(drop)
            }
            Kind::Par(ws) => ws.iter().try_for_each(|w| self.write(*w, env, out)),
            Kind::Fail(m) => {
                let Val::Text(message) = self.eval(Arena::Statement, m, env)?.into_inner() else {
                    return Err(Broken::FailNotText.into());
                };
                Err(Error::Rejected(Rejection::UserError {
                    message: message.to_string(),
                }))
            }
            Kind::If(c, a, b) => {
                let w = if self.eval(Arena::Statement, c, env)?.truth()? {
                    a
                } else {
                    b
                };
                self.write(w, env, out)
            }
            Kind::Case { scrutinee, arms } => {
                let v = self.eval(Arena::Statement, scrutinee, env)?;
                let c = v.ctor_of()?;
                let (_, x, w) = arms
                    .iter()
                    .find(|(d, _, _)| *d == c)
                    .ok_or(Broken::NoArm(c))?;
                self.write(*w, &bind(env, *x, (*v).clone()), out)
            }
            Kind::Let(x, e, w) => {
                let v = self.eval(Arena::Statement, e, env)?;
                self.write(w, &bind(env, x, (*v).clone()), out)
            }
            Kind::Unreachable => Err(Broken::Unreachable.into()),
            k => Err(Broken::NotAWrite(TermKind(k)).into()),
        }
    }

    /// Evaluate a `Replace` or `Upsert`, adding its effects to `out`. Returns the value a `let`
    /// binds to it.
    fn bind_write(&self, t: Tm, env: &Env, out: &mut Effects) -> Result<Held<Val>, Error> {
        match self.terms.kind(t).clone() {
            Kind::Replace(r, rows, f) => {
                let Kind::Lam(x, body) = *self.terms.kind(f) else {
                    return Err(Broken::ReplaceNotLam.into());
                };
                let mut news: Held<BTreeSet<Val>> = self.rows_in.held();
                for v in self.set(rows, env)? {
                    let n = self.eval(Arena::Statement, body, &bind(env, x, v.clone()))?;
                    if self.key_of(r, &n)? != self.key_of(r, &v)? {
                        return Err(Error::Rejected(Rejection::KeyChanged { relation: r }));
                    }
                    if *n != v && self.in_snapshot(r, &v)? {
                        out.insert(Eff::Rep(r, v, (*n).clone()))?;
                    }
                    news.insert(n.into_inner())?;
                }
                Ok(news.into_value()?)
            }
            Kind::Upsert { rows, .. } => {
                let rows = self.records(rows, env, None)?;
                self.upsert(t, rows, env, out)
            }
            Kind::Rejects { into, insert } => self.rejects(into, insert, env, out),
            k => Err(Broken::NotABoundWrite(TermKind(k)).into()),
        }
    }

    /// Returns the key of `row`, a row of `rel`.
    pub fn key_of(&self, rel: RelId, row: &Val) -> Result<Val, Error> {
        if is_temp(self.schema, rel) {
            return temp_key(self.schema, rel, row);
        }
        let r = self.rel(rel);
        let at = &r.ctors[row.ctor_of()? as usize].key_at;
        let fs = row.fields()?;
        Ok(if r.single_key {
            fs[at[0]].clone()
        } else {
            Val::record(at.iter().map(|i| fs[*i].clone()).collect())
        })
    }

    /// Returns stored row `stored` of constructor `c` of `rel`.
    pub fn row(
        &self,
        rel: RelId,
        c: u32,
        stored: &[crate::key::Value],
    ) -> Result<Val, exec::Error> {
        let ctor = &self.rel(rel).ctors[c as usize];
        Ok(Val::ctor(c, from_stored_row(&ctor.convs, stored)?))
    }

    /// Fail with what stops the statement, if it is stopped.
    pub(super) fn stopped(&self) -> Result<(), exec::Error> {
        self.stop.check(self.db.rt.now())
    }

    /// Returns the entries of index `id` as [`exec::Store::probe`] does, bounded by the statement.
    pub(super) fn probe(
        &self,
        id: IndexId,
        prefix: &[crate::key::Value],
        routes: &exec::Routes,
    ) -> Result<Held<Vec<exec::Named>>, exec::Error> {
        let bounds = exec::ProbeBounds {
            grant: &self.grant,
            check: &|| self.stopped(),
        };
        self.store.probe(id, prefix, routes, bounds)
    }

    /// Returns every row of `rel` the statement's reads see, of the constructors `of`, or all,
    /// held in the statement's grant.
    pub fn rows(&self, rel: RelId, of: Option<&[u32]>) -> Result<Held<Vec<(u32, Val)>>, Error> {
        let r = self.rel(rel);
        let mut out: Held<Vec<(u32, Val)>> = self.grant.held();
        for (c, ctor) in r.ctors.iter().enumerate() {
            let c = c as u32;
            if of.is_some_and(|s| !s.contains(&c)) {
                continue;
            }
            let listing = self.store.pages(ctor.heap)?;
            let morsel = self.db.options.morsel_rows;
            self.store
                .scan(ctor.heap, listing, morsel, &self.grant, &self.spill, |b| {
                    for i in 0..b.len() {
                        out.push((c, self.row(rel, c, &b.row(i))?))?;
                    }
                    Ok(ControlFlow::Continue(()))
                })?;
        }
        Ok(out)
    }

    /// Returns whether `v` is a row of `rel` the statement's reads see.
    fn in_snapshot(&self, rel: RelId, v: &Val) -> Result<bool, Error> {
        let k = self.key_of(rel, v)?;
        if is_temp(self.schema, rel) {
            return Ok(self.local.borrow().by_key(rel, &k) == Some(v));
        }
        Ok(self.by_key(rel, &k)?.is_some_and(|(_, _, u)| u == *v))
    }

    /// Returns the key a reference field of `row` names, if any.
    pub fn names(f: &RefField, row: &Val) -> Result<Option<Val>, Error> {
        let v = &row.fields()?[f.at];
        if !f.optional {
            return Ok(Some(v.clone()));
        }
        Ok(match v {
            Val::Ctor(ctor::SOME, fs) => fs.first().cloned(),
            _ => None,
        })
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

    /// Returns each row of `rel` the statement's reads see in which a field of `fields`, by
    /// constructor, names a key of `keys`, each with its target, found by probing the field's
    /// index. The rows are held in the statement's grant, by key.
    pub fn referring(
        &self,
        rel: RelId,
        fields: &[Vec<&RefField>],
        keys: &BTreeSet<(RelId, Val)>,
    ) -> Result<Referring, Error> {
        let r = self.rel(rel);
        let mut probed = BTreeSet::new();
        let mut out: Referring = self.grant.held();
        for (c, fs) in fields.iter().enumerate() {
            for f in fs {
                let targets = keys.iter().filter(|(t, _)| *t == f.target).map(|(_, k)| k);
                let at = (!f.header).then_some(c);
                if !probed.insert((at, f.name.as_str())) {
                    continue;
                }
                let index = f.index.ok_or_else(|| Broken::ReferenceUnindexed {
                    relation: r.qualified.clone(),
                    field: f.name.clone(),
                })?;
                let routes = if f.header {
                    r.routes(r.all_ctors())
                } else {
                    r.routes([c as u32])
                };
                let types = self.store.key_types(index)?;
                for k in targets {
                    let value = if f.optional {
                        Val::some(k.clone())
                    } else {
                        k.clone()
                    };
                    let prefix = [to_key(&types[0], &value)?];
                    for n in self.probe(index, &prefix, &routes)? {
                        let d = n.ctor;
                        let Some((_, stored)) = self.store.fetch(r.ctors[d as usize].heap, &n)?
                        else {
                            continue;
                        };
                        let v = self.row(rel, d, &stored)?;
                        if Self::names(f, &v)?.as_ref() == Some(k) {
                            out.insert(self.key_of(rel, &v)?, (d, v))?;
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    /// Apply change set `set` and check it at the statement boundary. Returns how many rows of
    /// each constructor it inserts, then deletes.
    pub(super) fn settle(&self, set: ChangeSet) -> Result<(Counts, Counts), Error> {
        self.check_open(&set)?;
        let set = self.check_sizes(set)?;
        let mut viol = Violations::new(&self.rows_in);
        let mut keys: Held<BTreeSet<(RelId, Val)>> = self.rows_in.held();
        for ((r, _), v) in set.plus() {
            let k = self.key_of(*r, v)?;
            if !keys.insert((*r, k.clone()))? {
                let name = format!("{}.key", self.rel(*r).qualified);
                violate_key(&mut viol, &name, *r, k)?;
            }
        }
        drop(keys);
        self.apply(&set, &mut viol)?;
        self.check(set.change(), &mut viol)?;
        refuse(viol)?;
        Ok(set.change().counts())
    }

    /// Refuse an inserted or retagged row in a constructor not open for writes, and a stored
    /// value of a value-level constructor not open for writes that the pre-state row of its key
    /// does not hold at the same path.
    fn check_open(&self, set: &ChangeSet) -> Result<(), Error> {
        let schema = &self.db.schema;
        if schema.all_enabled() {
            return Ok(());
        }
        let closed = |s: Stage| match s {
            Stage::Enabled => None,
            s => Some(s),
        };
        for ((r, c), v) in set.plus() {
            let rel = self.rel(*r);
            let pre = self.by_key(*r, &self.key_of(*r, v)?)?.map(|(_, _, u)| u);
            if let Some(stage) = closed(schema.stage(*r, *c))
                && pre.as_ref().map(Val::ctor_of).transpose()? != Some(*c)
            {
                return Err(Error::Rejected(Rejection::NotOpen {
                    ctor: format!("{}.{}", rel.name, rel.ctors[*c as usize].name),
                    stage,
                }));
            }
            let mut stored = Vec::new();
            self.sum_values(v, &rel.elem, &mut Vec::new(), &mut stored)?;
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
        Ok(())
    }

    /// Add to `out` every value of a declared sum inside `v`, of type `ty`, below the top,
    /// with its path of field positions, its sum and its constructor.
    fn sum_values(
        &self,
        v: &Val,
        ty: &Ty,
        path: &mut Vec<usize>,
        out: &mut Vec<(Vec<usize>, TypeId, u32, Val)>,
    ) -> Result<(), Error> {
        let schema = &self.db.schema;
        let t = exec::types::repr(schema, ty);
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
            _ => return Ok(()),
        };
        for (i, (x, t)) in v.fields()?.iter().zip(fields).enumerate() {
            path.push(i);
            self.sum_values(x, &t, path, out)?;
            path.pop();
        }
        Ok(())
    }
}

fn at<'v>(v: &'v Val, path: &[usize]) -> Option<&'v Val> {
    path.iter().try_fold(v, |x, i| match x {
        Val::Ctor(_, fs) | Val::Record(fs) => fs.get(*i),
        _ => None,
    })
}

pub fn bind(env: &Env, x: Var, v: Val) -> Env {
    let mut e = env.clone();
    e.insert(x, v);
    e
}

impl<R: Runtime> Database<R> {
    /// Returns the plan of term `t` of `terms`, in a root of `kind`.
    pub(super) fn plan(
        &self,
        schema: &Schema,
        terms: Valid<'_>,
        (t, kind): (Tm, RootKind),
    ) -> Result<crate::plan::Plan, Error> {
        let usable = self.usable_catalog();
        crate::plan::plan(
            schema,
            &self.normal,
            usable.as_deref().unwrap_or(&self.catalog),
            terms,
            t,
            kind,
            crate::plan::Options::default(),
        )
        .map_err(|e| match e {
            e @ crate::plan::Error::Internal(_) => Broken::StatementPlan(e).into(),
            e => Error::Exec(exec::Error::Unsupported(e.to_string())),
        })
    }

    /// Returns the program of term `t` of `terms`, in a root of `kind`, run for `caller`, its
    /// host functions bound.
    pub(super) fn program(
        &self,
        schema: &Schema,
        terms: Valid<'_>,
        (t, kind): (Tm, RootKind),
        caller: Option<Val>,
    ) -> Result<exec::Program, Error> {
        let plan = self.plan(schema, terms, (t, kind))?;
        let mut program = exec::compile(schema, &self.catalog, &plan, caller)?;
        program.shared.hosts = self.hosts.clone();
        Ok(program)
    }
}
