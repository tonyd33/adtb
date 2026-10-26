use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use adtb_lang::schema::{OnDelete, Stage as SchemaStage, TypeDefKind};
use adtb_lang::types::{CtorSet, RelId, Ty, TypeId};

use crate::eval::{Abort, Arena, Change, Database, Env, Interpreter, R, Run, bool_of, into_set};
use crate::stmt::{Eff, Effects, Outcome, Rejection, Stage};
use crate::textual::print;
use crate::value::Value;

/// A direct reference field of a constructor: its name, whether it is a header field, its target
/// relation and constructor set, whether it is an `Option`, and its delete action.
struct RefField {
    name: String,
    header: bool,
    target: RelId,
    ctors: Option<CtorSet>,
    optional: bool,
    action: OnDelete,
}

impl RefField {
    /// Returns the key the field of `row` names, if any.
    fn names<'v>(&self, row: &'v Value) -> Option<&'v Value> {
        let v = row.field(&self.name)?;
        if !self.optional {
            return Some(v);
        }
        match v {
            Value::Ctor(1, fs) => fs.first().map(|(_, k)| k),
            _ => None,
        }
    }
}

/// What a constraint is, for the reason a violation of it gives: a relation's key, with the
/// keys that violate it, or any other.
enum Constraint {
    Key(RelId, BTreeSet<Value>),
    Other,
}

struct Violation {
    kind: Constraint,
    report: BTreeSet<Value>,
}

/// A value of a declared sum stored inside a row, at a field path.
struct SumValue {
    path: Vec<String>,
    sum: TypeId,
    ctor: u32,
    value: Value,
}

/// The inserted and deleted rows of each constructor relation.
struct Delta {
    plus: Database,
    minus: Database,
}

fn add(db: &mut Database, rel: RelId, v: Value) {
    let c = v.tag().expect("a row is a constructor value");
    db.entry((rel, c)).or_default().insert(v);
}

fn apply(d: &Database, delta: &Delta) -> Database {
    let mut out = d.clone();
    for (rc, rows) in &delta.minus {
        if let Some(have) = out.get_mut(rc) {
            have.retain(|v| !rows.contains(v));
        }
    }
    for (rc, rows) in &delta.plus {
        out.entry(*rc).or_default().extend(rows.iter().cloned());
    }
    out.retain(|_, rows| !rows.is_empty());
    out
}

fn normalise(mut delta: Delta) -> Delta {
    for (rc, plus) in &mut delta.plus {
        if let Some(minus) = delta.minus.get_mut(rc) {
            let both: Vec<Value> = plus.intersection(minus).cloned().collect();
            for v in both {
                plus.remove(&v);
                minus.remove(&v);
            }
        }
    }
    delta.plus.retain(|_, rows| !rows.is_empty());
    delta.minus.retain(|_, rows| !rows.is_empty());
    delta
}

/// Returns `v` with field `name` replaced by `to`, if it has one.
fn with_field(v: Value, name: &str, to: Value) -> Value {
    match v {
        Value::Ctor(c, fs) => Value::Ctor(
            c,
            fs.into_iter()
                .map(|(n, x)| if n == name { (n, to.clone()) } else { (n, x) })
                .collect(),
        ),
        other => other,
    }
}

/// Returns the value at `path` inside `v`, following one field per step.
fn at<'v>(v: &'v Value, path: &[String]) -> Option<&'v Value> {
    path.iter().try_fold(v, |x, f| x.field(f))
}

impl Interpreter<'_> {
    fn qualified(&self, module: adtb_lang::types::ModuleId, name: &str) -> String {
        format!("{}::{name}", self.schema.module(module).name)
    }

    /// Returns the reference fields of constructor `c` of `rel`, header fields first.
    fn ref_fields(&self, rel: RelId, c: u32) -> Vec<RefField> {
        let r = self.schema.relation(rel);
        let def = self.schema.ty(r.elem_def);
        let header = def.header().iter().map(|f| (f, true));
        let own = def.ctors()[c as usize].fields.iter().map(|f| (f, false));
        let option = self.schema.prelude_type("Option");
        header
            .chain(own)
            .filter_map(|(f, header)| {
                if r.key.contains(&f.name) && self.self_key(rel, &f.name) {
                    return None;
                }
                let (key, optional) = match f.ty.subst(r.elem_args()) {
                    k @ Ty::Key { .. } => (k, false),
                    Ty::Data { def, mut args, .. } if def == option && args.len() == 1 => {
                        match args.pop() {
                            Some(k @ Ty::Key { .. }) => (k, true),
                            _ => return None,
                        }
                    }
                    _ => return None,
                };
                let Ty::Key { rel: target, ctors } = key else {
                    unreachable!("matched a key type")
                };
                Some(RefField {
                    name: f.name.clone(),
                    header,
                    target,
                    ctors,
                    optional,
                    action: f.on_delete.clone(),
                })
            })
            .collect()
    }

    /// Returns the change set of effects `e` over the snapshot, before normalisation.
    fn change_set(&self, e: &Effects) -> Delta {
        let killed: BTreeSet<(RelId, &Value)> = e
            .iter()
            .filter_map(|x| match x {
                Eff::Kill(r, k) => Some((*r, k)),
                _ => None,
            })
            .collect();
        let mut delta = Delta {
            plus: Database::new(),
            minus: Database::new(),
        };
        for x in e {
            match x {
                Eff::Ins(r, v) | Eff::Rep(r, _, v) => {
                    if !killed.contains(&(*r, &self.key_of(*r, v))) {
                        add(&mut delta.plus, *r, v.clone());
                    }
                }
                Eff::Del(..) | Eff::Kill(..) => {}
            }
            if let Eff::Del(r, v) | Eff::Rep(r, v, _) = x {
                add(&mut delta.minus, *r, v.clone());
            }
        }
        for ((r, _), rows) in self.db {
            for u in rows {
                if killed.contains(&(*r, &self.key_of(*r, u))) {
                    add(&mut delta.minus, *r, u.clone());
                }
            }
        }
        delta
    }

    /// Adds to `out` every value of a declared sum inside `v`, of type `ty`, below the top, with
    /// its field path, its sum and its constructor.
    fn sum_values(&self, v: &Value, ty: &Ty, path: &mut Vec<String>, out: &mut Vec<SumValue>) {
        let fields: Vec<(&String, &Value, Ty)> = match (v, ty) {
            (Value::Record(fs), Ty::Record(tys)) => fs
                .iter()
                .zip(tys)
                .map(|((n, x), (_, t))| (n, x, t.clone()))
                .collect(),
            (Value::Ctor(c, fs), Ty::Sum { ctors, .. }) => fs
                .iter()
                .zip(&ctors[*c as usize].1)
                .map(|((n, x), (_, t))| (n, x, t.clone()))
                .collect(),
            (_, Ty::Data { def, args, .. }) => match &self.schema.ty(*def).kind {
                TypeDefKind::Newtype { base, .. } => {
                    return self.sum_values(v, &base.subst(args), path, out);
                }
                TypeDefKind::Sum { header, ctors, .. } => {
                    let Value::Ctor(c, fs) = v else {
                        panic!("a value of a sum is a constructor value: {v:?}")
                    };
                    if !path.is_empty() {
                        out.push(SumValue {
                            path: path.clone(),
                            sum: *def,
                            ctor: *c,
                            value: v.clone(),
                        });
                    }
                    header
                        .iter()
                        .chain(&ctors[*c as usize].fields)
                        .zip(fs)
                        .map(|(fd, (n, x))| (n, x, fd.ty.subst(args)))
                        .collect()
                }
            },
            _ => Vec::new(),
        };
        for (n, x, t) in fields {
            path.push(n.clone());
            self.sum_values(x, &t, path, out);
            path.pop();
        }
    }

    /// Calls `f` on every value of newtype `n` inside `v`, of type `ty`, at any depth.
    fn newtype_values(
        &self,
        v: &Value,
        ty: &Ty,
        n: TypeId,
        f: &mut dyn FnMut(&Value) -> R<()>,
    ) -> R<()> {
        match ty {
            Ty::Data { def, args, .. } => match &self.schema.ty(*def).kind {
                TypeDefKind::Newtype { base, .. } => {
                    if *def == n {
                        f(v)?;
                    }
                    self.newtype_values(v, &base.subst(args), n, f)
                }
                TypeDefKind::Sum { header, ctors, .. } => {
                    let Value::Ctor(c, fs) = v else {
                        panic!("a value of a sum is a constructor value: {v:?}")
                    };
                    let fields = header.iter().chain(&ctors[*c as usize].fields);
                    for (fd, (_, x)) in fields.zip(fs) {
                        self.newtype_values(x, &fd.ty.subst(args), n, f)?;
                    }
                    Ok(())
                }
            },
            Ty::Record(tys) => {
                let Value::Record(fs) = v else {
                    panic!("a value of a record type is a record: {v:?}")
                };
                for ((_, x), (_, t)) in fs.iter().zip(tys) {
                    self.newtype_values(x, t, n, f)?;
                }
                Ok(())
            }
            Ty::Sum { ctors, .. } => {
                let Value::Ctor(c, fs) = v else {
                    panic!("a value of a sum is a constructor value: {v:?}")
                };
                for ((_, x), (_, t)) in fs.iter().zip(&ctors[*c as usize].1) {
                    self.newtype_values(x, t, n, f)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Refuses an inserted or retagged row in a constructor not open for writes, and a stored
    /// value of a value-level constructor not open for writes that the pre-state row of its key
    /// does not hold at the same path.
    fn check_open(&self, plus: &Database) -> R<()> {
        let schema = self.schema;
        if self.closed.is_empty() && schema.stages.is_empty() && schema.value_stages.is_empty() {
            return Ok(());
        }
        let closed = |t: TypeId, c: u32, at: Option<RelId>| -> Option<Stage> {
            let stage = match at {
                Some(r) => schema.stage(r, c),
                None => schema.value_stage(t, c),
            };
            match stage {
                SchemaStage::Declared => Some(Stage::Declared),
                SchemaStage::Deprecated => Some(Stage::Deprecated),
                SchemaStage::Enabled => self.closed.get(&(t, c)).copied(),
            }
        };
        for ((r, c), rows) in plus {
            let rel = self.schema.relation(*r);
            let def = self.schema.ty(rel.elem_def);
            for v in rows {
                let k = self.key_of(*r, v);
                let pre = self
                    .rows(self.db, *r, &None)
                    .into_iter()
                    .find(|u| self.key_of(*r, u) == k);
                if let Some(stage) = closed(rel.elem_def, *c, Some(*r))
                    && pre.as_ref().is_none_or(|u| u.tag() != Some(*c))
                {
                    return Err(Abort::Reject(Rejection::NotOpen {
                        ctor: format!("{}.{}", rel.name, def.ctors()[*c as usize].name),
                        stage,
                    }));
                }
                let mut stored = Vec::new();
                self.sum_values(v, &rel.elem, &mut Vec::new(), &mut stored);
                for w in stored {
                    if let Some(stage) = closed(w.sum, w.ctor, None)
                        && pre.as_ref().and_then(|u| at(u, &w.path)) != Some(&w.value)
                    {
                        let sum = self.schema.ty(w.sum);
                        return Err(Abort::Reject(Rejection::NotOpen {
                            ctor: format!("{}.{}", sum.name, sum.ctors()[w.ctor as usize].name),
                            stage,
                        }));
                    }
                }
            }
        }
        Ok(())
    }
}

impl Run<'_, '_> {
    /// Derives the delete actions of the written effects, forms the statement's change set, and
    /// tests it at the statement boundary.
    pub(crate) fn boundary(&self, written: Effects, result: Value) -> R<Outcome> {
        let it = self.it;
        let d = it.db;
        let dw = apply(d, &it.change_set(&written));
        let dw_keys: BTreeSet<(RelId, Value)> = dw
            .iter()
            .flat_map(|((r, _), rows)| rows.iter().map(|u| (*r, it.key_of(*r, u))))
            .collect();
        let kept = |r: RelId, c: u32, u: &Value| dw.get(&(r, c)).is_some_and(|s| s.contains(u));
        let mut gone: BTreeSet<(RelId, Value)> = d
            .iter()
            .flat_map(|((r, _), rows)| rows.iter().map(|u| (*r, it.key_of(*r, u))))
            .filter(|rk| !dw_keys.contains(rk))
            .collect();
        let names_gone = |f: &RefField, u: &Value, gone: &BTreeSet<(RelId, Value)>| {
            f.names(u)
                .is_some_and(|k| gone.contains(&(f.target, k.clone())))
        };
        loop {
            let mut more = Vec::new();
            for ((r, c), rows) in d {
                let fields = it.ref_fields(*r, *c);
                for u in rows {
                    let k = (*r, it.key_of(*r, u));
                    if !kept(*r, *c, u) || gone.contains(&k) {
                        continue;
                    }
                    if fields
                        .iter()
                        .any(|f| f.action == OnDelete::Cascade && names_gone(f, u, &gone))
                    {
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
        for ((r, c), rows) in d {
            let fields = it.ref_fields(*r, *c);
            for u in rows {
                if !kept(*r, *c, u) || gone.contains(&(*r, it.key_of(*r, u))) {
                    continue;
                }
                let reach: Vec<&RefField> =
                    fields.iter().filter(|f| names_gone(f, u, &gone)).collect();
                let clears = reach.iter().filter(|f| f.action == OnDelete::Clear);
                let new = if reach
                    .iter()
                    .any(|f| matches!(f.action, OnDelete::Retag(..)))
                {
                    let lam = it.program.retags[r];
                    let f = self.eval(&self.here(Arena::Program), lam, &Env::default())?;
                    let retagged = self.apply(f, u.clone())?;
                    clears.fold(retagged, |v, g| with_field(v, &g.name, Value::none()))
                } else if reach.iter().any(|f| f.action == OnDelete::Clear) {
                    clears.fold(u.clone(), |v, g| with_field(v, &g.name, Value::none()))
                } else {
                    continue;
                };
                all.insert(Eff::Rep(*r, u.clone(), new));
            }
        }
        all.extend(gone.into_iter().map(|(r, k)| Eff::Kill(r, k)));
        let delta = it.change_set(&all);
        let mut existing: BTreeMap<RelId, BTreeSet<Value>> = BTreeMap::new();
        for e in &all {
            let Eff::Ins(r, v) = e else { continue };
            let k = it.key_of(*r, v);
            let held = it.rows(d, *r, &None).into_iter().any(|u0| {
                it.key_of(*r, &u0) == k
                    && !delta
                        .minus
                        .get(&(*r, u0.tag().expect("a row")))
                        .is_some_and(|m| m.contains(&u0))
            });
            if held {
                existing.entry(*r).or_default().insert(k);
            }
        }
        let delta = normalise(delta);
        it.check_open(&delta.plus)?;
        let db = apply(d, &delta);
        let change = Change {
            prior: d.clone(),
            inserted: delta.plus,
            deleted: delta.minus,
        };
        let mut viol = it.violations(&db, &change)?;
        for (r, keys) in existing {
            let name = it.qualified(it.schema.relation(r).module, &it.schema.relation(r).name);
            let v = viol.entry(format!("{name}.key")).or_insert(Violation {
                kind: Constraint::Key(r, BTreeSet::new()),
                report: BTreeSet::new(),
            });
            if let Constraint::Key(_, held) = &mut v.kind {
                held.extend(keys);
            }
        }
        let Some((name, v)) = viol.into_iter().next() else {
            return Ok(Outcome { result, change, db });
        };
        let reason = match v.kind {
            Constraint::Key(r, keys) => {
                let key = keys.into_iter().next().expect("a key violation has a key");
                Rejection::KeyViolation {
                    relation: it.schema.relation(r).name.clone(),
                    key: print(
                        it.schema,
                        &key,
                        &Ty::Key {
                            rel: r,
                            ctors: None,
                        },
                    ),
                }
            }
            Constraint::Other => Rejection::Violated {
                constraint: name,
                report: v.report,
            },
        };
        Err(Abort::Reject(reason))
    }
}

impl Interpreter<'_> {
    /// Returns every constraint of the schema that transition `change` into `db` violates, by
    /// name, each with its report.
    fn violations(&self, db: &Database, change: &Change) -> R<BTreeMap<String, Violation>> {
        let mut after = Interpreter::new(self.schema, self.program, db)
            .with_change(change)
            .with_fix_limit(self.fix_limit);
        after.compiled = Rc::clone(&self.compiled);
        let run = Run {
            it: &after,
            query: &self.program.terms,
            stmt: None,
        };
        let schema = self.schema;
        let mut out = BTreeMap::new();
        let mut found = |name: String, kind: Constraint, report: BTreeSet<Value>| {
            if !report.is_empty() {
                out.insert(name, Violation { kind, report });
            }
        };
        let key_of_a = |rel: RelId, report: &BTreeSet<Value>| -> BTreeSet<Value> {
            report
                .iter()
                .map(|p| self.key_of(rel, p.field("a").expect("a pair")))
                .collect()
        };
        let pairs = |groups: BTreeMap<Vec<Value>, Vec<Value>>| -> BTreeSet<Value> {
            let mut report = BTreeSet::new();
            for rows in groups.values() {
                for x in rows {
                    for y in rows {
                        if x != y {
                            report.insert(Value::record([("a", x.clone()), ("b", y.clone())]));
                        }
                    }
                }
            }
            report
        };
        for (i, r) in schema.relations.iter().enumerate() {
            if r.elem_def.0 == u32::MAX {
                continue;
            }
            let rel = RelId(i as u32);
            let name = self.qualified(r.module, &r.name);
            let def = schema.ty(r.elem_def);
            let mut by_key: BTreeMap<Vec<Value>, Vec<Value>> = BTreeMap::new();
            for row in after.rows(db, rel, &None) {
                by_key
                    .entry(vec![after.key_of(rel, &row)])
                    .or_default()
                    .push(row);
            }
            let report = pairs(by_key);
            let keys = key_of_a(rel, &report);
            found(format!("{name}.key"), Constraint::Key(rel, keys), report);
            for (ix_at, ix) in r.indexes.iter().enumerate() {
                if !ix.unique {
                    continue;
                }
                let mut groups: BTreeMap<Vec<Value>, Vec<Value>> = BTreeMap::new();
                for row in after.rows(db, rel, &ix.scope) {
                    if let Some(vs) = run.unique_values(rel, ix_at, &row)? {
                        groups.entry(vs).or_default().push(row);
                    }
                }
                let groups = groups
                    .into_iter()
                    .map(|(k, rows)| {
                        let mut by_key: BTreeMap<Value, Value> = BTreeMap::new();
                        for row in rows {
                            by_key.entry(after.key_of(rel, &row)).or_insert(row);
                        }
                        (k, by_key.into_values().collect())
                    })
                    .collect();
                found(
                    self.unique_name(&name, rel, ix_at),
                    Constraint::Other,
                    pairs(groups),
                );
            }
            let mut refs: BTreeMap<String, BTreeSet<Value>> = BTreeMap::new();
            for c in 0..def.ctors().len() as u32 {
                for f in self.ref_fields(rel, c) {
                    let field = if f.header {
                        format!("{name}.{}", f.name)
                    } else {
                        format!("{name}.{}.{}", def.ctors()[c as usize].name, f.name)
                    };
                    let targets: BTreeSet<Value> = after
                        .rows(db, f.target, &f.ctors)
                        .iter()
                        .map(|t| after.key_of(f.target, t))
                        .collect();
                    let report = refs.entry(field).or_default();
                    for u in db.get(&(rel, c)).into_iter().flatten() {
                        if f.names(u).is_some_and(|k| !targets.contains(k)) {
                            report.insert(u.clone());
                        }
                    }
                }
            }
            for (field, report) in refs {
                found(field, Constraint::Other, report);
            }
        }
        for (n, (var, p)) in &self.program.domains {
            let h = run.here(Arena::Program);
            let mut report = BTreeSet::new();
            for ((r, _), rows) in db {
                let elem = &schema.relation(*r).elem;
                for x in rows {
                    let mut bad = false;
                    self.newtype_values(x, elem, *n, &mut |y| {
                        let ok = run.eval(&h, *p, &Env::default().bind(*var, y.clone()))?;
                        bad |= !bool_of(&ok)?;
                        Ok(())
                    })?;
                    if bad {
                        report.insert(x.clone());
                    }
                }
            }
            let t = schema.ty(*n);
            found(
                format!("{}.domain", self.qualified(t.module, &t.name)),
                Constraint::Other,
                report,
            );
        }
        let h = run.here(Arena::Program);
        let named = schema
            .constraints
            .iter()
            .map(|c| (c.module, &c.name))
            .zip(&self.program.constraints)
            .chain(
                schema
                    .checks
                    .iter()
                    .map(|c| (c.module, &c.name))
                    .zip(&self.program.checks),
            );
        for ((module, name), tm) in named {
            let report = into_set(run.eval(&h, *tm, &Env::default())?)?;
            found(self.qualified(module, name), Constraint::Other, report);
        }
        Ok(out)
    }

    /// Returns the canonical name of the unique at index `i` of `rel`, whose qualified name is
    /// `name`.
    fn unique_name(&self, name: &str, rel: RelId, i: usize) -> String {
        let r = self.schema.relation(rel);
        let def = self.schema.ty(r.elem_def);
        let ix = &r.indexes[i];
        let paths: Option<Vec<String>> = ix.fields.iter().map(path).collect();
        let Some(paths) = paths else {
            let id = self.schema.ids.indexes[&(rel, i)];
            return format!("{name}.unique#{id}");
        };
        let all: CtorSet = (0..def.ctors().len() as u32).collect();
        let mut ctors: Vec<&str> = ix
            .scope
            .as_ref()
            .unwrap_or(&all)
            .iter()
            .map(|c| def.ctors()[*c as usize].name.as_str())
            .collect();
        ctors.sort_unstable();
        let set = match ctors.as_slice() {
            [one] => one.to_string(),
            many => format!("({})", many.join(" | ")),
        };
        format!("{name}.unique({}) in {set}", paths.join(", "))
    }
}

/// Returns the field path an index field is written as, if it is one.
fn path(e: &adtb_lang::ast::Expr) -> Option<String> {
    use adtb_lang::ast::ExprKind;
    match &e.kind {
        ExprKind::Name(q) if q.module.is_none() => Some(q.name.text.clone()),
        ExprKind::Field(x, f) => Some(format!("{}.{}", path(x)?, f.text)),
        ExprKind::Select(x, adtb_lang::ast::CtorSel::One(c)) => {
            Some(format!("{}.{}", path(x)?, c.text))
        }
        _ => None,
    }
}
