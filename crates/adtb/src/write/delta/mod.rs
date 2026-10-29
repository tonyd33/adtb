use std::collections::{BTreeMap, BTreeSet, HashMap};

use adtb_lang::analysis::Totality;
use adtb_lang::kernel::{Agg, Kind, Prim, Terms, Tm, Transition, Var};
use adtb_lang::norm::NormalProgram;
use adtb_lang::schema::Schema;
use adtb_lang::types::{CtorSet, RelId, Ty};

use crate::catalog::indexed_fields;
use crate::plan::explain::{Describe, Node};
use crate::plan::{self, Catalog, Facts, Plan, with_children};

#[cfg(test)]
mod tests;

/// The rows of a constructor set that a delta query reads from the change set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Inserted,
    Deleted,
    /// The inserted rows and the deleted ones.
    Changed,
}

/// A set parameter of a delta query: the rows of `part` of the change set in `ctors` of `rel`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub var: Var,
    pub rel: RelId,
    pub ctors: CtorSet,
    pub part: Part,
}

/// One incremental check of a constraint.
#[derive(Debug)]
pub struct Delta {
    pub inputs: Vec<Input>,
    /// The input every violation the term returns involves a row of, so the term returns nothing
    /// when it is empty; `None` when the term is the whole query.
    pub trigger: Option<Var>,
    pub plan: Plan,
}

/// The incremental checks of one constraint or check.
///
/// Invariant: when every constraint held before a change set, the union of what `deltas` return
/// after it is what the constraint's query returns after it.
#[derive(Debug)]
pub struct Checks {
    pub name: String,
    /// The relation variables the query reads.
    pub reads: BTreeSet<RelId>,
    pub deltas: Vec<Delta>,
}

/// One incremental check as `explain` shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckPlan {
    /// The constructor relation, by qualified name, and the part of the change set whose rows
    /// run the check; `None` when every change runs it.
    pub trigger: Option<(String, Part)>,
    pub plan: Node,
}

impl Delta {
    /// Returns the check as `explain` shows it over `schema` and `catalog`.
    pub fn explain(&self, schema: &Schema, catalog: &Catalog) -> CheckPlan {
        let trigger = self.trigger.map(|t| {
            let i = self
                .inputs
                .iter()
                .find(|i| i.var == t)
                .expect("a check's trigger is one of its inputs");
            (scope_name(schema, i.rel, &i.ctors), i.part)
        });
        CheckPlan {
            trigger,
            plan: Describe::new(schema, catalog).plan(&self.plan),
        }
    }
}

/// Why the constraints of a schema compile to no incremental checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unadmitted {
    /// Each constraint without an index-bounded incremental check, with why.
    Refused(Vec<Refusal>),
    /// A guarantee of the checker does not hold; the one named.
    Internal(String),
}

/// Why one constraint compiles to no incremental check.
enum Outside {
    Refused(Vec<String>),
    Internal(String),
}

/// A constraint the schema cannot admit: it has no index-bounded incremental check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The constraint's qualified name.
    pub constraint: String,
    pub detail: String,
}

/// Returns the incremental checks of every constraint and check of `schema`, their terms added
/// to `terms`, a copy of `normal`'s arena, each planned over `catalog` with every relation read
/// through index probes.
///
/// Fails with a refusal per constraint and reason when some constraint has no such check, and
/// with the internal error when compiling one breaks a guarantee of the checker.
pub fn compile(
    schema: &Schema,
    normal: &NormalProgram,
    terms: &mut Terms,
    catalog: &Catalog,
) -> Result<Vec<Checks>, Unadmitted> {
    let named = schema
        .constraints
        .iter()
        .map(|c| (c.module, &c.name))
        .zip(&normal.constraints)
        .chain(
            schema
                .checks
                .iter()
                .map(|c| (c.module, &c.name))
                .zip(&normal.checks),
        );
    let mut transitions = Vec::new();
    let mut found = Vec::new();
    let mut refused = Vec::new();
    for ((module, name), t) in named {
        let qualified = format!("{}::{name}", schema.module(module).name);
        let outside = super::polarity::non_monotone(schema, terms, *t);
        if !outside.is_empty() {
            refused.extend(outside.into_iter().map(|detail| Refusal {
                constraint: qualified.clone(),
                detail,
            }));
            continue;
        }
        let mut reads = BTreeSet::new();
        let mut own = Vec::new();
        scan(terms, *t, &mut BTreeSet::new(), &mut own, &mut reads);
        found.push((qualified, *t, reads));
        transitions.extend(own);
    }
    let mut cx = Compiler {
        schema,
        normal,
        catalog,
        terms,
        facts: Facts::default(),
        total: Totality::new(schema, false),
        transitions: transitions
            .into_iter()
            .map(|(v, x, r, s)| (v, (x, r, s)))
            .collect(),
        inputs: BTreeMap::new(),
    };
    let mut out = Vec::new();
    for (name, t, reads) in found {
        cx.inputs.clear();
        match cx.constraint(t) {
            Ok(deltas) => out.push(Checks {
                name,
                reads,
                deltas,
            }),
            Err(Outside::Refused(why)) => refused.extend(why.into_iter().map(|detail| Refusal {
                constraint: name.clone(),
                detail,
            })),
            Err(Outside::Internal(what)) => return Err(Unadmitted::Internal(what)),
        }
    }
    if refused.is_empty() {
        Ok(out)
    } else {
        Err(Unadmitted::Refused(refused))
    }
}

fn internal<T>(what: impl Into<String>) -> Result<T, plan::Error> {
    Err(plan::Error::Internal(what.into()))
}

/// Returns the qualified name of `r`, with the constructor set `ctors` when it is not every
/// constructor.
pub fn scope_name(schema: &Schema, r: RelId, ctors: &CtorSet) -> String {
    let rel = schema.relation(r);
    let name = format!("{}::{}", schema.module(rel.module).name, rel.name);
    let def = schema.ty(rel.elem_def);
    if ctors.len() == def.ctors().len() {
        return name;
    }
    let names: Vec<&str> = ctors
        .iter()
        .map(|c| def.ctors()[*c as usize].name.as_str())
        .collect();
    match names.as_slice() {
        [one] => format!("{name}.{one}"),
        many => format!("{name}.({})", many.join(" | ")),
    }
}

/// Replace each transition read under `t` in place by a variable, adding the variables to
/// `out` and the relations every read under `t` names to `reads`.
fn scan(
    terms: &mut Terms,
    t: Tm,
    seen: &mut BTreeSet<Tm>,
    out: &mut Vec<(Var, Transition, RelId, Option<CtorSet>)>,
    reads: &mut BTreeSet<RelId>,
) {
    if !seen.insert(t) {
        return;
    }
    match terms.kind(t).clone() {
        Kind::Transition(x, r, s) => {
            let ty = terms.ty(t).clone();
            let v = terms.var(&format!("%transition{}", t.0), ty);
            terms.nodes[t.0 as usize].kind = Kind::Var(v);
            out.push((v, x, r, s));
            reads.insert(r);
        }
        Kind::Rel(r, _) => {
            reads.insert(r);
        }
        Kind::Deref { key, .. } => {
            if let Ty::Key { rel, .. } = terms.ty(key) {
                reads.insert(*rel);
            }
            scan(terms, key, seen, out, reads);
        }
        _ => {
            for c in terms.children(t) {
                scan(terms, c, seen, out, reads);
            }
        }
    }
}

/// Which state a source of rows reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    State,
    Inserted,
    Deleted,
    Prior,
}

/// One read of a constructor set inside a constraint's query.
#[derive(Clone, Debug)]
struct Occurrence {
    rel: RelId,
    ctors: CtorSet,
    origin: Origin,
    /// The node that reads the rows: a source, or a `Deref`.
    node: Tm,
    at: At,
}

#[derive(Clone, Debug)]
enum At {
    /// The source of a generator of the query's top level, the `For` node given.
    Top(Tm),
    /// A read inside an expression of the query: the row it binds and what of the expression
    /// encloses it.
    Nested { row: Row, within: Within },
}

/// What encloses a read inside an expression of the query.
#[derive(Clone, Debug, Default)]
struct Within {
    /// The generators enclosing it, each with its source when that is a constructor set.
    path: Vec<(Var, Option<Tm>)>,
    /// The conditions guarding it.
    guards: Vec<Tm>,
    /// The arms enclosing it, each a case and the position of the arm, outermost first.
    arms: Vec<(Tm, usize)>,
}

#[derive(Clone, Debug)]
enum Row {
    Gen(Var),
    /// A `Deref` of this key.
    Deref(Tm),
    /// A read that binds no row.
    Bare,
}

/// One step of a chain of generators and conditions.
#[derive(Clone, Copy, Debug)]
enum Link {
    Gen(Var, Tm),
    Cond(Tm),
}

/// A variable the query's top level binds, with the node binding it and its position.
#[derive(Clone, Copy, Debug)]
struct Binder {
    node: Tm,
    order: usize,
}

struct Compiler<'a> {
    schema: &'a Schema,
    normal: &'a NormalProgram,
    catalog: &'a Catalog,
    terms: &'a mut Terms,
    facts: Facts,
    total: Totality<'a>,
    transitions: HashMap<Var, (Transition, RelId, Option<CtorSet>)>,
    /// The set parameters of the constraint being compiled.
    inputs: BTreeMap<(RelId, CtorSet, Part), Var>,
}

impl Compiler<'_> {
    fn add(&mut self, kind: Kind, ty: Ty) -> Tm {
        self.terms.add(kind, ty)
    }

    fn kind(&self, t: Tm) -> Kind {
        self.terms.kind(t).clone()
    }

    fn ty(&self, t: Tm) -> Ty {
        self.terms.ty(t).clone()
    }

    fn all_ctors(&self, r: RelId) -> CtorSet {
        let n = self
            .schema
            .ty(self.schema.relation(r).elem_def)
            .ctors()
            .len() as u32;
        (0..n).collect()
    }

    fn ctors_or_all(&self, r: RelId, s: &Option<CtorSet>) -> CtorSet {
        s.clone().unwrap_or_else(|| self.all_ctors(r))
    }

    fn scope_name(&self, r: RelId, ctors: &CtorSet) -> String {
        scope_name(self.schema, r, ctors)
    }

    fn bool_ty(&self) -> Ty {
        Ty::Data {
            def: self.schema.prelude_type("Bool"),
            args: Vec::new(),
            ctors: None,
        }
    }

    fn is_false(&self, t: Tm) -> bool {
        match self.terms.kind(t) {
            Kind::Ctor { ctor, fields } if fields.is_empty() => {
                let def = self.schema.ty(self.schema.prelude_type("Bool"));
                matches!(self.terms.ty(t), Ty::Data { def: d, .. } if *d == self.schema.prelude_type("Bool"))
                    && Some(*ctor) == def.ctor_index("False")
            }
            _ => false,
        }
    }

    /// Add the conjuncts of `c` to `out`, a `let` of a copyable value at the root moved onto each.
    fn conjuncts(&mut self, c: Tm, out: &mut Vec<Tm>) {
        match *self.terms.kind(c) {
            Kind::If(a, b, f) if self.is_false(f) => {
                self.conjuncts(a, out);
                self.conjuncts(b, out);
            }
            Kind::Let(x, e, b) if self.total.copyable(self.terms, e) => {
                let from = out.len();
                self.conjuncts(b, out);
                for k in &mut out[from..] {
                    *k = self.facts.sink_let(self.terms, x, e, *k);
                }
            }
            _ => out.push(c),
        }
    }

    /// Returns the constructor set `src` reads and the state it reads, when it is one.
    fn source(&self, src: Tm) -> Option<(RelId, CtorSet, Origin)> {
        match self.terms.kind(src) {
            Kind::Rel(r, s) => Some((*r, self.ctors_or_all(*r, s), Origin::State)),
            Kind::Restrict(inner, s) => {
                let (r, cs, o) = self.source(*inner)?;
                Some((r, cs.into_iter().filter(|c| s.contains(c)).collect(), o))
            }
            Kind::Var(v) => {
                let (x, r, s) = self.transitions.get(v)?;
                let origin = match x {
                    Transition::Inserted => Origin::Inserted,
                    Transition::Deleted => Origin::Deleted,
                    Transition::Prior => Origin::Prior,
                };
                Some((*r, self.ctors_or_all(*r, s), origin))
            }
            _ => None,
        }
    }

    /// Returns the variable of the set parameter of `part` of `ctors` of `rel`.
    fn input(&mut self, rel: RelId, ctors: &CtorSet, part: Part, elem: &Ty) -> Var {
        let k = (rel, ctors.clone(), part);
        if let Some(v) = self.inputs.get(&k) {
            return *v;
        }
        let v = self.terms.var(
            &format!("%{part:?}{}", rel.0),
            Ty::Set(Box::new(elem.clone())),
        );
        self.inputs.insert(k, v);
        v
    }

    /// Returns the set parameter a transition variable stands for.
    fn transition_input(&self, v: Var) -> Option<Input> {
        let (x, r, s) = self.transitions.get(&v)?;
        let part = match x {
            Transition::Inserted => Part::Inserted,
            Transition::Deleted => Part::Deleted,
            Transition::Prior => return None,
        };
        Some(Input {
            var: v,
            rel: *r,
            ctors: self.ctors_or_all(*r, s),
            part,
        })
    }

    /// Returns the delta queries of the constraint `name` with query `q`, or every reason it is
    /// outside the bound.
    fn constraint(&mut self, q: Tm) -> Result<Vec<Delta>, Outside> {
        let mut binders = HashMap::new();
        let mut occurrences = Vec::new();
        self.top(q, &mut binders, &mut occurrences);
        let mut terms: Vec<(Tm, Option<Var>, String)> = Vec::new();
        let whole = occurrences.iter().any(|o| {
            matches!(o.at, At::Top(_)) && matches!(o.origin, Origin::Inserted | Origin::Deleted)
        });
        if whole {
            let (links, tail) = self.chain(q);
            let t = self.ordered(links, tail, self.ty(q));
            terms.push((t, None, "the change".to_string()));
        } else {
            for o in &occurrences {
                let what = format!("changes to {}", self.scope_name(o.rel, &o.ctors));
                let made = match (&o.at, o.origin) {
                    (_, Origin::Prior) => continue,
                    (At::Top(f), _) => self.top_delta(q, *f, o),
                    (At::Nested { row, within }, _) => {
                        self.nested_delta(q, o, row, within, &binders)
                    }
                };
                match made {
                    Ok((t, trigger)) => terms.push((t, Some(trigger), what)),
                    Err(e) => return Err(Outside::Refused(vec![self.reason(&what, e)?])),
                }
            }
        }
        let mut deltas = Vec::new();
        let mut why: Vec<String> = Vec::new();
        for (t, trigger, what) in terms {
            let t = match self.priors(t) {
                Ok(t) => t,
                Err(e) => return Err(Outside::Refused(vec![self.reason(&what, e)?])),
            };
            for reason in self.shapes(t) {
                let r = format!("{what}: {reason}");
                if !why.contains(&r) {
                    why.push(r);
                }
            }
            let inputs = self.inputs_of(t);
            let params: Vec<Var> = inputs.iter().map(|i| i.var).collect();
            match plan::plan_bounded(
                self.schema,
                self.normal,
                self.catalog,
                self.terms,
                t,
                &params,
            ) {
                Ok(plan) => deltas.push(Delta {
                    inputs,
                    trigger,
                    plan,
                }),
                Err(e) => {
                    let r = self.reason(&what, e)?;
                    if !why.contains(&r) {
                        why.push(r);
                    }
                }
            }
        }
        if why.is_empty() {
            Ok(deltas)
        } else {
            Err(Outside::Refused(why))
        }
    }

    /// Returns why `what` has no bounded check, for `e`; fails with an internal error as itself.
    fn reason(&self, what: &str, e: plan::Error) -> Result<String, Outside> {
        match e {
            plan::Error::Internal(broken) => Err(Outside::Internal(broken)),
            e => Ok(format!("{what}: {}", self.refusal(&e))),
        }
    }

    fn refusal(&self, e: &plan::Error) -> String {
        match e {
            plan::Error::Unbounded { rel, ctors, fields } if fields.is_empty() => format!(
                "no field connects them to {}, so every row of it is read",
                self.scope_name(*rel, ctors)
            ),
            plan::Error::Unbounded { rel, ctors, fields } => format!(
                "index ({}) in {} is missing",
                fields.join(", "),
                self.scope_name(*rel, ctors)
            ),
            plan::Error::Outside(op) => format!("`{op}` is outside the bound"),
            other => other.to_string(),
        }
    }

    /// Returns the inputs the free variables of `t` stand for.
    fn inputs_of(&mut self, t: Tm) -> Vec<Input> {
        let free = self.facts.free(self.terms, t);
        let mut out = Vec::new();
        for ((rel, ctors, part), var) in &self.inputs {
            if free.contains(var) {
                out.push(Input {
                    var: *var,
                    rel: *rel,
                    ctors: ctors.clone(),
                    part: *part,
                });
            }
        }
        for v in free.iter() {
            if let Some(i) = self.transition_input(*v) {
                out.push(i);
            }
        }
        out
    }

    /// Collect the reads of the query's top level `t`, recording each variable it binds.
    fn top(&mut self, t: Tm, binders: &mut HashMap<Var, Binder>, out: &mut Vec<Occurrence>) {
        let bind = |binders: &mut HashMap<Var, Binder>, x: Var| {
            let order = binders.len();
            binders.insert(x, Binder { node: t, order });
        };
        match self.kind(t) {
            Kind::For(x, src, body) => {
                match self.source(src) {
                    Some((rel, ctors, origin)) => out.push(Occurrence {
                        rel,
                        ctors,
                        origin,
                        node: src,
                        at: At::Top(t),
                    }),
                    None => self.nested(src, &mut Within::default(), out),
                }
                bind(binders, x);
                self.top(body, binders, out);
            }
            Kind::If(c, a, b) => {
                self.nested(c, &mut Within::default(), out);
                self.top(a, binders, out);
                self.top(b, binders, out);
            }
            Kind::Let(x, e, b) => {
                self.nested(e, &mut Within::default(), out);
                bind(binders, x);
                self.top(b, binders, out);
            }
            Kind::Union(a, b) => {
                self.top(a, binders, out);
                self.top(b, binders, out);
            }
            Kind::Case { scrutinee, arms } => {
                self.nested(scrutinee, &mut Within::default(), out);
                for (_, _, a) in arms {
                    self.top(a, binders, out);
                }
            }
            _ => self.nested(t, &mut Within::default(), out),
        }
    }

    /// Collect the reads inside expression `t`, under what of the expression encloses it.
    fn nested(&mut self, t: Tm, within: &mut Within, out: &mut Vec<Occurrence>) {
        let here = |row: Row, within: &Within, guards: Vec<Tm>| At::Nested {
            row,
            within: Within {
                guards,
                ..within.clone()
            },
        };
        match self.kind(t) {
            Kind::For(y, src, body) => {
                let closed = match self.source(src) {
                    Some((rel, ctors, origin)) => {
                        let mut own = within.guards.clone();
                        own.extend(self.peel(body).0);
                        out.push(Occurrence {
                            rel,
                            ctors,
                            origin,
                            node: src,
                            at: here(Row::Gen(y), within, own),
                        });
                        Some(src)
                    }
                    None => {
                        self.nested(src, within, out);
                        None
                    }
                };
                within.path.push((y, closed));
                self.nested(body, within, out);
                within.path.pop();
            }
            Kind::If(c, a, b) => {
                self.nested(c, within, out);
                if matches!(self.terms.kind(b), Kind::EmptySet) || self.is_false(b) {
                    let n = within.guards.len();
                    self.conjuncts(c, &mut within.guards);
                    self.nested(a, within, out);
                    within.guards.truncate(n);
                } else {
                    self.nested(a, within, out);
                    self.nested(b, within, out);
                }
            }
            Kind::Case { scrutinee, arms } => {
                self.nested(scrutinee, within, out);
                for (i, (_, _, a)) in arms.iter().enumerate() {
                    within.arms.push((t, i));
                    self.nested(*a, within, out);
                    within.arms.pop();
                }
            }
            Kind::Deref { key, .. } => {
                if let Ty::Key { rel, ctors } = self.ty(key) {
                    out.push(Occurrence {
                        rel,
                        ctors: self.ctors_or_all(rel, &ctors),
                        origin: Origin::State,
                        node: t,
                        at: here(Row::Deref(key), within, within.guards.clone()),
                    });
                }
                self.nested(key, within, out);
            }
            Kind::Rel(..) | Kind::Restrict(..) | Kind::Var(_) if self.source(t).is_some() => {
                let (rel, ctors, origin) = self.source(t).expect("a source");
                out.push(Occurrence {
                    rel,
                    ctors,
                    origin,
                    node: t,
                    at: here(Row::Bare, within, within.guards.clone()),
                });
            }
            _ => {
                for c in self.terms.children(t) {
                    self.nested(c, within, out);
                }
            }
        }
    }

    /// Returns `q` with the top level generator `f` reading the rows the change set inserts
    /// into its source, and that input.
    ///
    /// Precondition: `f` reads a constructor set in the state after the change.
    fn top_delta(&mut self, q: Tm, f: Tm, o: &Occurrence) -> Result<(Tm, Var), plan::Error> {
        let Kind::For(x, _, body) = self.kind(f) else {
            return internal("a top level read is the source of a `For`");
        };
        let elem = self.terms.var_info(x).ty.clone();
        let d = self.input(o.rel, &o.ctors, Part::Inserted, &elem);
        let set = self.add(Kind::Var(d), Ty::Set(Box::new(elem)));
        let new = self.add(Kind::For(x, set, body), self.ty(f));
        let t = self.replace(q, &HashMap::from([(f, new)]), &mut HashMap::new());
        let (links, tail) = self.chain(t);
        Ok((self.ordered(links, tail, self.ty(q)), d))
    }

    /// Returns `q` restricted to the bindings of its top level that a changed row of the nested
    /// read `o` can affect, and the input of those rows.
    fn nested_delta(
        &mut self,
        q: Tm,
        o: &Occurrence,
        row: &Row,
        within: &Within,
        binders: &HashMap<Var, Binder>,
    ) -> Result<(Tm, Var), plan::Error> {
        let elem = match row {
            Row::Gen(y) => self.terms.var_info(*y).ty.clone(),
            Row::Deref(_) => self.ty(o.node),
            Row::Bare => match self.ty(o.node) {
                Ty::Set(e) | Ty::NonEmpty(e) => *e,
                other => return internal(format!("a read of rows is a set, not {other:?}")),
            },
        };
        let input = match (o.origin, self.kind(o.node)) {
            (Origin::Inserted | Origin::Deleted, Kind::Var(v)) => v,
            _ => self.input(o.rel, &o.ctors, Part::Changed, &elem),
        };
        let input_ty = self.terms.var_info(input).ty.clone();
        let elem = match &input_ty {
            Ty::Set(e) => e.as_ref().clone(),
            other => return internal(format!("a change set input is a set, not {other:?}")),
        };
        let d = self.terms.var("%changed", elem);
        let mut sub: HashMap<Var, Var> = HashMap::new();
        let mut prefix: Vec<(Var, Option<Tm>)> = vec![(d, None)];
        if let Row::Gen(y) = row {
            sub.insert(*y, d);
        }
        for (p, src) in within.path.iter().rev() {
            if let Some(src) = src {
                let ty = self.terms.var_info(*p).ty.clone();
                let fresh = self.terms.var(&self.terms.var_info(*p).name.clone(), ty);
                sub.insert(*p, fresh);
                prefix.push((fresh, Some(*src)));
            }
        }
        let mut candidates: Vec<Tm> = within.guards.clone();
        let mut allowed: BTreeSet<Var> = BTreeSet::new();
        if let Row::Deref(key) = row {
            let mut via = Vec::new();
            let mut conds = Vec::new();
            let key = self.unfold(*key, &mut via, &mut conds);
            let key_ty = self.ty(key);
            let dv = self.add(Kind::Var(d), self.terms.var_info(d).ty.clone());
            let k = self.add(Kind::Key(dv), key_ty);
            let eq = self.add(Kind::Prim(Prim::Eq, vec![k, key]), self.bool_ty());
            candidates.insert(0, eq);
            candidates.extend(conds);
            for (v, src) in via {
                allowed.insert(v);
                prefix.push((v, Some(src)));
            }
        }
        allowed.extend(sub.keys().copied());
        allowed.insert(d);
        allowed.extend(binders.keys().copied());
        let mut kept = Vec::new();
        for c in candidates {
            let c = self.within_arms(c, &within.arms);
            let free = self.facts.free(self.terms, c);
            if free.iter().all(|v| allowed.contains(v))
                && self.facts.scalar(self.terms, c)
                && self.total.total(self.terms, c)
            {
                kept.push(self.subst(c, &sub, &mut HashMap::new()));
            }
        }
        let used: BTreeSet<Var> = kept
            .iter()
            .flat_map(|c| {
                self.facts
                    .free(self.terms, *c)
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
            })
            .collect();
        prefix.retain(|(v, src)| src.is_none() || used.contains(v));
        let (top, _) = self.chain(q);
        let chained: BTreeSet<Var> = top
            .iter()
            .filter_map(|l| match l {
                Link::Gen(v, _) => Some(*v),
                Link::Cond(_) => None,
            })
            .collect();
        let mut links: Vec<Link> = Vec::new();
        for (v, src) in &prefix {
            let src = match src {
                Some(s) => *s,
                None => self.add(Kind::Var(input), input_ty.clone()),
            };
            links.push(Link::Gen(*v, src));
        }
        let mut at_top: HashMap<Tm, Vec<Tm>> = HashMap::new();
        for c in kept {
            let free = self.facts.free(self.terms, c);
            let deepest = free
                .iter()
                .filter_map(|v| binders.get(v).map(|b| (*v, b)))
                .max_by_key(|(_, b)| b.order);
            match deepest {
                Some((v, b)) if !chained.contains(&v) => at_top.entry(b.node).or_default().push(c),
                _ => links.push(Link::Cond(c)),
            }
        }
        let ty = self.ty(q);
        let mut map = HashMap::new();
        for (node, conds) in at_top {
            let wrapped = match self.kind(node) {
                Kind::For(x, src, body) => {
                    let body = self.guard(&conds, body, &ty);
                    Kind::For(x, src, body)
                }
                Kind::Let(x, e, body) => {
                    let body = self.guard(&conds, body, &ty);
                    Kind::Let(x, e, body)
                }
                k => {
                    return internal(format!(
                        "a top level binder is a `For` or a `Let`, not {k:?}"
                    ));
                }
            };
            let n = self.add(wrapped, self.ty(node));
            map.insert(node, n);
        }
        let injected = self.replace(q, &map, &mut HashMap::new());
        let (rest, tail) = self.chain(injected);
        links.extend(rest);
        Ok((self.ordered(links, tail, ty), input))
    }

    /// Returns key `t` with each row it reads through a key replaced by a generator over the
    /// key's relation variable, adding each generator and its source to `via` and the equality
    /// of its key with the key it replaces to `conds`.
    fn unfold(&mut self, t: Tm, via: &mut Vec<(Var, Tm)>, conds: &mut Vec<Tm>) -> Tm {
        if let Kind::Deref { key, .. } = self.kind(t)
            && let Ty::Key { rel, ctors } = self.ty(key)
        {
            let key = self.unfold(key, via, conds);
            let row_ty = self.ty(t);
            let e = self.terms.var("%via", row_ty.clone());
            let src = self.add(Kind::Rel(rel, ctors), Ty::Set(Box::new(row_ty.clone())));
            let row = self.add(Kind::Var(e), row_ty);
            let own = self.add(Kind::Key(row), self.ty(key));
            let eq = self.add(Kind::Prim(Prim::Eq, vec![own, key]), self.bool_ty());
            via.push((e, src));
            conds.push(eq);
            return row;
        }
        let cs = self.terms.children(t);
        let new: Vec<Tm> = cs.iter().map(|c| self.unfold(*c, via, conds)).collect();
        if new == cs {
            t
        } else {
            let kind = with_children(self.terms.kind(t), &new);
            self.add(kind, self.ty(t))
        }
    }

    /// Returns condition `c`, read inside `arms`, as a condition read outside them: for each arm
    /// whose variable it reads, innermost first, a case over that arm's scrutinee holding `c` in
    /// that arm and false in the others.
    fn within_arms(&mut self, c: Tm, arms: &[(Tm, usize)]) -> Tm {
        let mut out = c;
        for (case, at) in arms.iter().rev() {
            let Kind::Case {
                scrutinee,
                arms: cases,
            } = self.kind(*case)
            else {
                continue;
            };
            let (_, v, _) = cases[*at];
            if !self.facts.mentions(self.terms, out, &[v]) {
                continue;
            }
            let no = self.falsity();
            let mut wrapped = Vec::new();
            for (i, (k, w, _)) in cases.iter().enumerate() {
                let ty = self.terms.var_info(*w).ty.clone();
                let fresh = self.terms.var(&self.terms.var_info(*w).name.clone(), ty);
                let body = if i == *at {
                    self.subst(out, &HashMap::from([(v, fresh)]), &mut HashMap::new())
                } else {
                    no
                };
                wrapped.push((*k, fresh, body));
            }
            out = self.add(
                Kind::Case {
                    scrutinee,
                    arms: wrapped,
                },
                self.bool_ty(),
            );
        }
        out
    }

    fn falsity(&mut self) -> Tm {
        let ctor = self
            .schema
            .ty(self.schema.prelude_type("Bool"))
            .ctor_index("False")
            .expect("the prelude's Bool has False");
        self.add(
            Kind::Ctor {
                ctor,
                fields: Vec::new(),
            },
            self.bool_ty(),
        )
    }

    /// Returns the leading generators and conditions of the top level of `q`, and what they
    /// range over.
    fn chain(&mut self, q: Tm) -> (Vec<Link>, Tm) {
        let mut links = Vec::new();
        let mut t = q;
        loop {
            match *self.terms.kind(t) {
                Kind::For(x, src, body) => {
                    links.push(Link::Gen(x, src));
                    t = body;
                }
                Kind::If(c, a, b) if matches!(self.terms.kind(b), Kind::EmptySet) => {
                    let mut cs = Vec::new();
                    self.conjuncts(c, &mut cs);
                    links.extend(cs.into_iter().map(Link::Cond));
                    t = a;
                }
                _ => return (links, t),
            }
        }
    }

    /// Returns the chain `links` over `tail`, a set term of type `ty`, its generators ordered so
    /// that each reads a constructor set through an equality with the rows bound before it
    /// where one exists, those over a change set first.
    ///
    /// A generator whose source is not a constructor set or a change set keeps the order of
    /// the whole chain. A condition comes after every row it reads; a partial one also after
    /// every condition before it.
    fn ordered(&mut self, links: Vec<Link>, tail: Tm, ty: Ty) -> Tm {
        let mut gens: Vec<(Var, Tm)> = Vec::new();
        let mut conds: Vec<Tm> = Vec::new();
        for l in &links {
            match l {
                Link::Gen(v, s) => gens.push((*v, *s)),
                Link::Cond(c) => conds.push(*c),
            }
        }
        let vars: Vec<Var> = gens.iter().map(|(v, _)| *v).collect();
        let fixed = gens
            .iter()
            .any(|(_, s)| self.source(*s).is_none() && !self.is_input(*s));
        let partial: Vec<bool> = conds
            .iter()
            .map(|c| !(self.facts.scalar(self.terms, *c) && self.total.total(self.terms, *c)))
            .collect();
        let mut placed: BTreeSet<Var> = BTreeSet::new();
        let mut done = vec![false; conds.len()];
        let mut steps: Vec<Link> = Vec::new();
        let mut left: Vec<(Var, Tm)> = gens.clone();
        loop {
            for (i, c) in conds.iter().enumerate() {
                if done[i] {
                    continue;
                }
                let free = self.facts.free(self.terms, *c);
                let ready = free.iter().all(|v| !vars.contains(v) || placed.contains(v))
                    && (!partial[i] || done[..i].iter().all(|d| *d));
                if ready {
                    done[i] = true;
                    steps.push(Link::Cond(*c));
                }
            }
            if left.is_empty() {
                break;
            }
            let pick = if fixed {
                0
            } else {
                left.iter()
                    .position(|(_, s)| self.is_input(*s))
                    .or_else(|| {
                        left.iter().position(|(v, _)| {
                            conds
                                .iter()
                                .enumerate()
                                .any(|(i, c)| !done[i] && self.connects(*c, *v, &vars, &placed))
                        })
                    })
                    .unwrap_or(0)
            };
            let (v, s) = left.remove(pick);
            placed.insert(v);
            steps.push(Link::Gen(v, s));
        }
        for (i, c) in conds.iter().enumerate() {
            if !done[i] {
                steps.push(Link::Cond(*c));
            }
        }
        let mut out = tail;
        for step in steps.into_iter().rev() {
            out = match step {
                Link::Gen(v, s) => self.add(Kind::For(v, s, out), ty.clone()),
                Link::Cond(c) => self.guard(&[c], out, &ty),
            };
        }
        out
    }

    /// Returns whether `src` is a set parameter of the change set.
    fn is_input(&self, src: Tm) -> bool {
        matches!(self.terms.kind(src), Kind::Var(v)
            if self.inputs.values().any(|i| i == v) || self.transitions.contains_key(v))
    }

    /// Returns whether `c` is an equality fixing a field or the key of row `y` to a value over
    /// the rows of `placed` among `vars`.
    fn connects(&mut self, c: Tm, y: Var, vars: &[Var], placed: &BTreeSet<Var>) -> bool {
        let Kind::Prim(Prim::Eq, args) = self.kind(c) else {
            return false;
        };
        let [a, b] = args[..] else { return false };
        [(a, b), (b, a)].into_iter().any(|(this, other)| {
            let on_y = match self.terms.kind(this) {
                Kind::Field(row, _) | Kind::Key(row) => {
                    matches!(self.terms.kind(*row), Kind::Var(v) if *v == y)
                }
                _ => false,
            };
            on_y && self
                .facts
                .free(self.terms, other)
                .iter()
                .all(|v| *v != y && (!vars.contains(v) || placed.contains(v)))
        })
    }

    /// Returns `body` guarded by each of `conds`, a set term of type `ty`.
    fn guard(&mut self, conds: &[Tm], body: Tm, ty: &Ty) -> Tm {
        let mut out = body;
        for c in conds.iter().rev() {
            let e = self.add(Kind::EmptySet, ty.clone());
            out = self.add(Kind::If(*c, out, e), ty.clone());
        }
        out
    }

    fn replace(&mut self, t: Tm, map: &HashMap<Tm, Tm>, memo: &mut HashMap<Tm, Tm>) -> Tm {
        if let Some(n) = map.get(&t) {
            return *n;
        }
        if let Some(n) = memo.get(&t) {
            return *n;
        }
        let cs = self.terms.children(t);
        let new: Vec<Tm> = cs.iter().map(|c| self.replace(*c, map, memo)).collect();
        let out = if new == cs {
            t
        } else {
            let kind = with_children(self.terms.kind(t), &new);
            self.add(kind, self.ty(t))
        };
        memo.insert(t, out);
        out
    }

    /// Returns `t` with each free variable of `sub` renamed.
    fn subst(&mut self, t: Tm, sub: &HashMap<Var, Var>, memo: &mut HashMap<Tm, Tm>) -> Tm {
        if let Some(n) = memo.get(&t) {
            return *n;
        }
        let out = match self.kind(t) {
            Kind::Var(v) if sub.contains_key(&v) => self.add(Kind::Var(sub[&v]), self.ty(t)),
            kind => {
                let cs = self.terms.children(t);
                let new: Vec<Tm> = cs.iter().map(|c| self.subst(*c, sub, memo)).collect();
                if new == cs {
                    t
                } else {
                    self.add(with_children(&kind, &new), self.ty(t))
                }
            }
        };
        memo.insert(t, out);
        out
    }

    /// Returns `t` with each read of a `prior` relation replaced by the rows of the state after
    /// the change that it does not insert, with the rows it deletes.
    fn priors(&mut self, t: Tm) -> Result<Tm, plan::Error> {
        let mut map = HashMap::new();
        let mut seen = BTreeSet::new();
        self.find_priors(t, &mut seen, &mut map)?;
        if map.is_empty() {
            return Ok(t);
        }
        Ok(self.replace(t, &map, &mut HashMap::new()))
    }

    fn find_priors(
        &mut self,
        t: Tm,
        seen: &mut BTreeSet<Tm>,
        map: &mut HashMap<Tm, Tm>,
    ) -> Result<(), plan::Error> {
        if !seen.insert(t) {
            return Ok(());
        }
        if let Kind::For(y, src, body) = self.kind(t)
            && let Some((rel, ctors, Origin::Prior)) = self.source(src)
        {
            let body = self.priors(body)?;
            let new = self.prior_for(y, rel, &ctors, body, self.ty(t));
            map.insert(t, new);
            return Ok(());
        }
        if let Kind::Var(_) = self.kind(t)
            && let Some((rel, ctors, Origin::Prior)) = self.source(t)
        {
            let elem = match self.ty(t) {
                Ty::Set(e) | Ty::NonEmpty(e) => *e,
                other => return internal(format!("a prior relation is a set, not {other:?}")),
            };
            let ty = Ty::Set(Box::new(elem.clone()));
            let y = self.terms.var("%prior", elem.clone());
            let row = self.add(Kind::Var(y), elem);
            let body = self.add(Kind::Singleton(row), ty.clone());
            let new = self.prior_for(y, rel, &ctors, body, ty);
            map.insert(t, new);
            return Ok(());
        }
        for c in self.terms.children(t) {
            self.find_priors(c, seen, map)?;
        }
        Ok(())
    }

    /// Returns `for y in prior(ctors of rel) do body`, a set term of type `ty`.
    fn prior_for(&mut self, y: Var, rel: RelId, ctors: &CtorSet, body: Tm, ty: Ty) -> Tm {
        let elem = self.terms.var_info(y).ty.clone();
        let set_ty = Ty::Set(Box::new(elem.clone()));
        let plus = self.input(rel, ctors, Part::Inserted, &elem);
        let minus = self.input(rel, ctors, Part::Deleted, &elem);
        let key_ty = Ty::Key { rel, ctors: None };
        let z = self.terms.var("%inserted", elem.clone());
        let zv = self.add(Kind::Var(z), elem.clone());
        let yv = self.add(Kind::Var(y), elem.clone());
        let zk = self.add(Kind::Key(zv), key_ty.clone());
        let yk = self.add(Kind::Key(yv), key_ty);
        let same = self.add(Kind::Prim(Prim::Eq, vec![zk, yk]), self.bool_ty());
        let one = self.add(Kind::Singleton(zv), set_ty.clone());
        let none = self.add(Kind::EmptySet, set_ty.clone());
        let hit = self.add(Kind::If(same, one, none), set_ty.clone());
        let plus_set = self.add(Kind::Var(plus), set_ty.clone());
        let hits = self.add(Kind::For(z, plus_set, hit), set_ty.clone());
        let kept = self.add(Kind::IsEmpty(hits), self.bool_ty());
        let guarded = self.guard_inside(body, kept, &ty);
        let src = self.add(
            Kind::Rel(rel, (*ctors != self.all_ctors(rel)).then(|| ctors.clone())),
            set_ty.clone(),
        );
        let state = self.add(Kind::For(y, src, guarded), ty.clone());
        let minus_set = self.add(Kind::Var(minus), set_ty);
        let deleted = self.add(Kind::For(y, minus_set, body), ty.clone());
        self.add(Kind::Union(state, deleted), ty)
    }

    /// Returns `body` with `cond` guarding what its leading conditions guard.
    fn guard_inside(&mut self, body: Tm, cond: Tm, ty: &Ty) -> Tm {
        match self.kind(body) {
            Kind::If(c, a, b) if matches!(self.terms.kind(b), Kind::EmptySet) => {
                let a = self.guard_inside(a, cond, ty);
                self.add(Kind::If(c, a, b), self.ty(body))
            }
            _ => self.guard(&[cond], body, ty),
        }
    }

    /// Returns why the shapes of `t` are outside the bound: a `min` or `max` with no index on
    /// its field within its group, and an exclusion under `overlaps` with no interval index.
    fn shapes(&mut self, t: Tm) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        self.find_shapes(t, &mut seen, &mut out);
        out
    }

    fn find_shapes(&mut self, t: Tm, seen: &mut BTreeSet<Tm>, out: &mut Vec<String>) {
        if !seen.insert(t) {
            return;
        }
        match self.kind(t) {
            Kind::Agg {
                op: op @ (Agg::Min | Agg::Max),
                var,
                source,
                body,
            } => {
                if let Some(why) = self.extreme(op, var, source, body) {
                    out.push(why);
                }
            }
            Kind::For(y, src, body) => {
                if let Kind::Rel(rel, s) = self.kind(src)
                    && let Some(why) = self.overlap(y, rel, &self.ctors_or_all(rel, &s), body)
                {
                    out.push(why);
                }
            }
            _ => {}
        }
        for c in self.terms.children(t) {
            self.find_shapes(c, seen, out);
        }
    }

    /// Returns the conditions guarding `body` directly, and what they guard.
    fn peel(&mut self, mut body: Tm) -> (Vec<Tm>, Tm) {
        let mut conds = Vec::new();
        while let Kind::If(c, a, b) = self.terms.kind(body)
            && matches!(self.terms.kind(*b), Kind::EmptySet)
        {
            let (c, a) = (*c, *a);
            self.conjuncts(c, &mut conds);
            body = a;
        }
        (conds, body)
    }

    /// Returns the fields of row `y` of `rel` that the equalities among `conds` fix to values
    /// not mentioning `y`.
    fn fixed(&mut self, rel: RelId, y: Var, conds: &[Tm]) -> Vec<String> {
        let mut out = Vec::new();
        for c in conds {
            let Kind::Prim(Prim::Eq, args) = self.kind(*c) else {
                continue;
            };
            let [a, b] = args[..] else { continue };
            for (this, other) in [(a, b), (b, a)] {
                if self.facts.mentions(self.terms, other, &[y]) {
                    continue;
                }
                match self.kind(this) {
                    Kind::Field(row, f) if matches!(self.terms.kind(row), Kind::Var(v) if *v == y) =>
                    {
                        out.push(f);
                    }
                    Kind::Key(row) if matches!(self.terms.kind(row), Kind::Var(v) if *v == y) => {
                        out.extend(self.schema.relation(rel).key.iter().cloned());
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// Returns the declared indexes of `rel` that cover `ctors`, each with its fields and
    /// whether it is an interval index.
    fn indexes(&self, rel: RelId, ctors: &CtorSet) -> Vec<(Vec<String>, bool)> {
        let r = self.schema.relation(rel);
        let mut out = vec![(r.key.clone(), false)];
        for def in &r.indexes {
            let covers = def
                .scope
                .as_ref()
                .is_none_or(|s| ctors.iter().all(|c| s.contains(c)));
            if covers && let Some(fields) = indexed_fields(def) {
                out.push((fields, def.interval));
            }
        }
        out
    }

    /// Returns why `op` of a field over the rows sharing a group key has no index on the field
    /// within the group, when the aggregate has that shape.
    fn extreme(&mut self, op: Agg, var: Var, source: Tm, body: Tm) -> Option<String> {
        let Kind::Field(of, field) = self.kind(body) else {
            return None;
        };
        if !matches!(self.terms.kind(of), Kind::Var(v) if *v == var) {
            return None;
        }
        let Kind::For(y, src, inner) = self.kind(source) else {
            return None;
        };
        let (rel, ctors, _) = self.source(src)?;
        let (conds, _) = self.peel(inner);
        let group = self.fixed(rel, y, &conds);
        if group.is_empty() {
            return None;
        }
        let covered = self.indexes(rel, &ctors).into_iter().any(|(fs, _)| {
            fs.iter()
                .position(|f| *f == field)
                .is_some_and(|i| i > 0 && fs[..i].iter().all(|f| group.contains(f)))
        });
        if covered {
            return None;
        }
        let name = if op == Agg::Min { "min" } else { "max" };
        Some(format!(
            "`{name}` of {field} needs index ({}, {field}) in {}",
            group.join(", "),
            self.scope_name(rel, &ctors)
        ))
    }

    /// Returns why the generator `y` over `ctors` of `rel`, filtered by `overlaps` on a range
    /// field, has no interval index over that field after its equality fields.
    fn overlap(&mut self, y: Var, rel: RelId, ctors: &CtorSet, body: Tm) -> Option<String> {
        let (conds, _) = self.peel(body);
        let mut ranges = Vec::new();
        for c in &conds {
            let Kind::Prim(Prim::Overlaps, args) = self.kind(*c) else {
                continue;
            };
            let [a, b] = args[..] else { continue };
            for (this, other) in [(a, b), (b, a)] {
                if let Kind::Field(row, f) = self.kind(this)
                    && matches!(self.terms.kind(row), Kind::Var(v) if *v == y)
                    && !self.facts.mentions(self.terms, other, &[y])
                {
                    ranges.push(f);
                }
            }
        }
        if ranges.is_empty() {
            return None;
        }
        let equal = self.fixed(rel, y, &conds);
        let key = &self.schema.relation(rel).key;
        if key.iter().all(|k| equal.contains(k)) {
            return None;
        }
        let indexes = self.indexes(rel, ctors);
        let range = ranges.into_iter().find(|f| {
            !indexes.iter().any(|(fs, interval)| {
                *interval
                    && fs.last() == Some(f)
                    && fs[..fs.len() - 1].iter().all(|g| equal.contains(g))
            })
        })?;
        let mut fields = equal;
        fields.push(range.clone());
        Some(format!(
            "an exclusion under `overlaps` on {range} needs interval index ({}) in {}",
            fields.join(", "),
            self.scope_name(rel, ctors)
        ))
    }
}
