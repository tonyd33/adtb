use std::collections::BTreeSet;

use crate::ast::*;
use crate::schema::{Schema, TypeDefKind, TypeName, ValueDef};
use crate::span::{Diag, FileDiag, FileId, Span};
use crate::types::{FnId, ModuleId, PRELUDE_FUNCTIONS, RelId};

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;

/// Lexical scopes of local names, innermost last.
#[derive(Debug)]
pub struct Scope<T> {
    frames: Vec<Vec<(String, T)>>,
}

impl<T> Default for Scope<T> {
    fn default() -> Self {
        Scope {
            frames: vec![Vec::new()],
        }
    }
}

impl<T> Scope<T> {
    pub fn push(&mut self) {
        self.frames.push(Vec::new());
    }

    pub fn pop(&mut self) {
        self.frames.pop();
    }

    pub fn bind(&mut self, name: &str, value: T) {
        self.frames
            .last_mut()
            .expect("a scope frame")
            .push((name.to_string(), value));
    }

    pub fn lookup(&self, name: &str) -> Option<&T> {
        self.frames
            .iter()
            .rev()
            .flat_map(|f| f.iter().rev())
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }
}

/// The modules a body reads unqualified names from, searched before the prelude.
#[derive(Clone, Debug)]
pub struct Visible {
    pub modules: Vec<ModuleId>,
}

impl Visible {
    pub fn module(m: ModuleId) -> Visible {
        Visible { modules: vec![m] }
    }

    /// Every user module, as ad hoc text sees them.
    pub fn all(schema: &Schema) -> Visible {
        Visible {
            modules: (0..schema.modules.len() as u32)
                .map(ModuleId)
                .filter(|&m| !schema.module(m).engine)
                .collect(),
        }
    }

    pub fn first(&self) -> ModuleId {
        self.modules
            .first()
            .copied()
            .unwrap_or(crate::schema::PRELUDE)
    }
}

pub fn lookup_value(schema: &Schema, vis: &Visible, name: &QName) -> Result<ValueDef, String> {
    if name.module.is_some() {
        return schema.lookup_value(vis.first(), name);
    }
    let found: Vec<(ModuleId, ValueDef)> = vis
        .modules
        .iter()
        .filter_map(|&m| {
            schema
                .module(m)
                .values
                .get(&name.name.text)
                .map(|&v| (m, v))
        })
        .collect();
    match found.as_slice() {
        [(_, v)] => Ok(*v),
        [] => Err(format!("no `{}` in scope", name.name.text)),
        many => Err(format!(
            "`{}` is declared in modules {}; qualify it",
            name.name.text,
            many.iter()
                .map(|(m, _)| format!("`{}`", schema.module(*m).name))
                .collect::<Vec<_>>()
                .join(" and ")
        )),
    }
}

/// Returns the relation variable `name` denotes among the modules of `vis`.
///
/// Fails when `name` does not resolve or denotes something other than a relation.
pub fn lookup_relation(schema: &Schema, vis: &Visible, name: &QName) -> Result<RelId, String> {
    match lookup_value(schema, vis, name)? {
        ValueDef::Relation(rel) => Ok(rel),
        _ => Err(format!("`{}` is not a relation", name.name.text)),
    }
}

pub fn lookup_type(schema: &Schema, vis: &Visible, name: &QName) -> Result<TypeName, String> {
    if name.module.is_some() {
        return schema.lookup_type(vis.first(), name);
    }
    let found: Vec<ModuleId> = vis
        .modules
        .iter()
        .copied()
        .filter(|&m| schema.module(m).types.contains_key(&name.name.text))
        .collect();
    match found.as_slice() {
        [m] => schema.lookup_type(*m, name),
        [] => schema.lookup_type(crate::schema::PRELUDE, name),
        _ => Err(format!(
            "type `{}` is declared in several modules; qualify it",
            name.name.text
        )),
    }
}

/// What a lowercase name in an expression denotes, besides a local.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// A function body or a registered query: parameters are bound, `caller` is in scope.
    Function,
    /// A constraint, a default or another schema-level expression: no `caller`.
    Schema,
    /// Ad hoc text: a free name is a parameter.
    Adhoc,
}

struct Resolver<'a> {
    schema: &'a Schema,
    vis: Visible,
    ctx: Context,
    scope: Scope<()>,
    diags: Vec<Diag>,
    params: BTreeSet<String>,
    functions: BTreeSet<FnId>,
}

/// Returns the functions the body of `f` names, called or passed as values.
pub fn function_refs(schema: &Schema, f: FnId) -> BTreeSet<FnId> {
    let def = schema.function(f);
    let FnBody::Expr(body) = &def.body else {
        return BTreeSet::new();
    };
    let mut r = Resolver::new(schema, Visible::module(def.module), Context::Function);
    for (name, _) in &def.params {
        r.scope.bind(name, ());
    }
    r.expr(body);
    r.functions
}

/// Returns the functions the expressions of aggregate `a` name, called or passed as values.
pub fn aggregate_refs(schema: &Schema, a: usize) -> BTreeSet<FnId> {
    let def = &schema.aggregates[a];
    let params: Vec<&Ident> = def.decl.params.iter().map(|p| &p.name).collect();
    let (s, x, step) = &def.decl.step;
    let (l, r, merge) = &def.decl.merge;
    let (f, finish) = &def.decl.finish;
    let mut out = BTreeSet::new();
    for (extra, e) in [
        (vec![], &def.decl.init),
        (vec![s, x], step),
        (vec![l, r], merge),
        (vec![f], finish),
    ] {
        let mut res = Resolver::new(schema, Visible::module(def.module), Context::Function);
        for name in params.iter().chain(&extra) {
            res.scope.bind(&name.text, ());
        }
        res.expr(e);
        out.append(&mut res.functions);
    }
    out
}

/// Checks the names in every expression the schema declares.
pub fn check_schema(schema: &Schema) -> Vec<FileDiag> {
    let mut out = Vec::new();
    let mut run = |file: FileId, vis: Visible, ctx: Context, binds: &[String], e: &Expr| {
        let mut r = Resolver::new(schema, vis, ctx);
        for b in binds {
            r.scope.bind(b, ());
        }
        r.expr(e);
        out.extend(r.diags.into_iter().map(|diag| FileDiag { file, diag }));
    };
    for t in &schema.types {
        if let TypeDefKind::Newtype {
            domain: Some(d), ..
        } = &t.kind
        {
            run(
                t.file,
                Visible::module(t.module),
                Context::Schema,
                &["self".into()],
                d,
            );
        }
    }
    for f in &schema.functions {
        if let FnBody::Expr(e) = &f.body {
            let names: Vec<String> = f.params.iter().map(|(n, _)| n.clone()).collect();
            run(
                f.file,
                Visible::module(f.module),
                Context::Function,
                &names,
                e,
            );
        }
    }
    for c in &schema.constraints {
        run(
            c.file,
            Visible::module(c.module),
            Context::Schema,
            &[],
            &c.query,
        );
    }
    for c in &schema.checks {
        if c.relation.0 == u32::MAX {
            continue;
        }
        let mut names = row_fields(schema, c.relation, c.ctors.as_deref());
        names.push("self".into());
        run(
            c.file,
            Visible::module(c.module),
            Context::Schema,
            &names,
            &c.predicate,
        );
    }
    for (i, r) in schema.relations.iter().enumerate() {
        let vis = Visible::module(r.module);
        for ix in &r.indexes {
            let names = row_fields(schema, RelId(i as u32), ix.scope.as_deref());
            for e in &ix.fields {
                run(r.file, vis.clone(), Context::Schema, &names, e);
            }
        }
        for d in &r.defaults {
            if let crate::schema::DefaultValue::Expr(e) = &d.value {
                run(r.file, vis.clone(), Context::Schema, &[], e);
            }
        }
        for c in &r.computed {
            run(
                r.file,
                vis.clone(),
                Context::Schema,
                &["self".into()],
                &c.expr,
            );
        }
    }
    for a in &schema.aggregates {
        let vis = Visible::module(a.module);
        let params: Vec<String> = a.decl.params.iter().map(|p| p.name.text.clone()).collect();
        let with = |extra: &[&Ident]| -> Vec<String> {
            params
                .iter()
                .cloned()
                .chain(extra.iter().map(|i| i.text.clone()))
                .collect()
        };
        run(
            a.file,
            vis.clone(),
            Context::Function,
            &params,
            &a.decl.init,
        );
        let (s, x, e) = &a.decl.step;
        run(a.file, vis.clone(), Context::Function, &with(&[s, x]), e);
        let (l, r, e) = &a.decl.merge;
        run(a.file, vis.clone(), Context::Function, &with(&[l, r]), e);
        let (f, e) = &a.decl.finish;
        run(a.file, vis, Context::Function, &with(&[f]), e);
    }
    out
}

/// Checks the names of ad hoc text, returning its free names, which are its parameters.
pub fn check_adhoc(schema: &Schema, items: &[AdhocItem]) -> Result<BTreeSet<String>, Vec<Diag>> {
    let mut r = Resolver::new(schema, Visible::all(schema), Context::Adhoc);
    for item in items {
        match item {
            AdhocItem::Expr(e) => r.expr(e),
            AdhocItem::Write(w) => r.write(w),
            AdhocItem::Transact(b) => r.transact(b),
            AdhocItem::Temp(_)
            | AdhocItem::Command(..)
            | AdhocItem::Grant(_)
            | AdhocItem::Revoke(_)
            | AdhocItem::Role(..)
            | AdhocItem::Peer(..) => {}
        }
    }
    if r.diags.is_empty() {
        Ok(r.params)
    } else {
        Err(r.diags)
    }
}

/// The fields every constructor of the set has, and the computed fields, which a check or an index names bare.
fn row_fields(schema: &Schema, rel: RelId, ctors: Option<&[u32]>) -> Vec<String> {
    let def = schema.ty(schema.relation(rel).elem_def);
    let all: Vec<u32> = (0..def.ctors().len() as u32).collect();
    let set = ctors.unwrap_or(&all);
    let mut names: Vec<String> = def.header().iter().map(|f| f.name.clone()).collect();
    if let Some(&first) = set.first() {
        for f in &def.ctors()[first as usize].fields {
            if set.iter().all(|&c| {
                def.ctors()[c as usize]
                    .fields
                    .iter()
                    .any(|g| g.name == f.name)
            }) {
                names.push(f.name.clone());
            }
        }
    }
    names.extend(schema.relation(rel).computed.iter().map(|c| c.name.clone()));
    names
}

/// Every field name of any constructor of a relation's element type.
fn any_fields(schema: &Schema, rel: RelId) -> Vec<String> {
    let def = schema.ty(schema.relation(rel).elem_def);
    def.header()
        .iter()
        .chain(def.ctors().iter().flat_map(|c| c.fields.iter()))
        .map(|f| f.name.clone())
        .collect()
}

impl<'a> Resolver<'a> {
    fn new(schema: &'a Schema, vis: Visible, ctx: Context) -> Self {
        Resolver {
            schema,
            vis,
            ctx,
            scope: Scope::default(),
            diags: Vec::new(),
            params: BTreeSet::new(),
            functions: BTreeSet::new(),
        }
    }

    fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }

    fn lower_name(&mut self, q: &QName, as_call: bool) {
        if q.module.is_none() {
            let text = q.name.text.as_str();
            if self.scope.lookup(text).is_some() {
                return;
            }
            if text == "caller" {
                if self.ctx == Context::Schema {
                    self.err(q.name.span, "`caller` is not in scope here");
                }
                return;
            }
            if as_call && PRELUDE_FUNCTIONS.contains(&text) {
                return;
            }
        }
        match lookup_value(self.schema, &self.vis, q) {
            Ok(ValueDef::Relation(_)) => {}
            Ok(ValueDef::Function(f)) => {
                self.functions.insert(f);
            }
            Ok(ValueDef::Aggregate(_)) if as_call => {}
            Ok(_) => self.err(
                q.span(),
                format!(
                    "`{}` is not a value; it names a constraint, check or aggregate",
                    q.name.text
                ),
            ),
            Err(e) => {
                if self.ctx == Context::Adhoc && q.module.is_none() && !as_call {
                    self.params.insert(q.name.text.clone());
                } else {
                    self.err(q.span(), e);
                }
            }
        }
    }

    fn relation(&mut self, r: &RelationRef) -> Option<RelId> {
        let rel = match lookup_relation(self.schema, &self.vis, &r.relation) {
            Ok(rel) => rel,
            Err(e) => {
                self.err(r.span, e);
                return None;
            }
        };
        if let Some(sel) = &r.sel
            && let Err(e) = self.schema.relation_sel(rel, sel)
        {
            self.err(r.span, e);
        }
        Some(rel)
    }

    fn qualified_ctor(&mut self, path: &QName, ctor: &Ident) {
        match lookup_type(self.schema, &self.vis, path) {
            Ok(TypeName::Decl(t)) => {
                if self.schema.ty(t).ctor_index(&ctor.text).is_none() {
                    let msg = format!(
                        "`{}` is not a constructor of `{}`",
                        ctor.text,
                        self.schema.ty(t).name
                    );
                    self.err(ctor.span, msg);
                }
            }
            Ok(_) => self.err(
                path.span(),
                format!("`{}` has no constructors", path.name.text),
            ),
            Err(e) => self.err(path.span(), e),
        }
    }

    fn type_name(&mut self, ty: &Type) {
        if let TypeKind::Named { name, .. } = &ty.kind
            && let Err(e) = lookup_type(self.schema, &self.vis, name)
        {
            self.err(name.span(), e);
        }
    }

    fn call_args(&mut self, args: &CallArgs) {
        match args {
            CallArgs::List(es) => {
                for e in es {
                    self.expr(e);
                }
            }
            CallArgs::Aggregate {
                expr,
                generators,
                filter,
            } => {
                self.scope.push();
                self.generators(generators);
                if let Some(f) = filter {
                    self.expr(f);
                }
                self.expr(expr);
                self.scope.pop();
            }
        }
    }

    fn generators(&mut self, gens: &[Generator]) {
        for g in gens {
            self.expr(&g.source);
            self.scope.bind(&g.binder.text, ());
        }
    }

    fn record(&mut self, r: &RecordExpr) {
        match r {
            RecordExpr::Fields(fs, _) => {
                for (_, e) in fs {
                    self.expr(e);
                }
            }
            RecordExpr::With(base, fs, _) => {
                self.expr(base);
                for (_, e) in fs {
                    self.expr(e);
                }
            }
            RecordExpr::Elided(_) => {}
        }
    }

    fn clauses(&mut self, clauses: &[Clause]) {
        for c in clauses {
            match c {
                Clause::From(gens) => self.generators(gens),
                Clause::Where(e) => self.expr(e),
            }
        }
    }

    /// Resolves a comprehension, then `then` in the scope of its `select`.
    fn comprehension(&mut self, c: &Comprehension, then: impl FnOnce(&mut Self)) {
        self.scope.push();
        self.clauses(&c.clauses);
        if let Some((g, after)) = &c.group {
            self.expr(&g.value);
            self.expr(&g.by);
            self.scope.pop();
            self.scope.push();
            self.scope.bind(&g.into.text, ());
            self.clauses(after);
        }
        self.expr(&c.select);
        if let Some(o) = &c.order {
            for k in &o.keys {
                self.expr(&k.expr);
            }
        }
        then(self);
        self.scope.pop();
        if let Some(o) = &c.order {
            if let Some(t) = &o.take {
                self.expr(t);
            }
            if let Some(a) = &o.after {
                self.expr(a);
            }
        }
    }

    fn pattern_binders(&mut self, p: &Pattern, out: &mut BTreeSet<String>) {
        match &p.kind {
            PatternKind::Wildcard | PatternKind::Lit { .. } => {}
            PatternKind::Bind(x) => {
                if !out.insert(x.text.clone()) {
                    self.err(
                        x.span,
                        format!("`{}` is bound twice in one pattern", x.text),
                    );
                }
            }
            PatternKind::Ctor {
                path,
                ctor,
                payload,
            } => {
                if let Some(c) = ctor {
                    self.qualified_ctor(path, c);
                }
                match payload {
                    None | Some(Payload::Wildcard) => {}
                    Some(Payload::Bind(x)) => {
                        if !out.insert(x.text.clone()) {
                            self.err(
                                x.span,
                                format!("`{}` is bound twice in one pattern", x.text),
                            );
                        }
                    }
                    Some(Payload::Record(fs)) => {
                        for f in fs {
                            match &f.pattern {
                                None => {
                                    if !out.insert(f.field.text.clone()) {
                                        self.err(
                                            f.field.span,
                                            format!(
                                                "`{}` is bound twice in one pattern",
                                                f.field.text
                                            ),
                                        );
                                    }
                                }
                                Some(p) => self.pattern_binders(p, out),
                            }
                        }
                    }
                }
            }
        }
    }

    /// Binds an arm's patterns, which must all bind the same names.
    fn arm_patterns(&mut self, patterns: &[Pattern]) {
        let mut first: Option<BTreeSet<String>> = None;
        for p in patterns {
            let mut names = BTreeSet::new();
            self.pattern_binders(p, &mut names);
            match &first {
                None => first = Some(names),
                Some(f) if *f != names => self.err(
                    p.span,
                    "every alternative of an or-pattern binds the same names",
                ),
                Some(_) => {}
            }
        }
        for n in first.unwrap_or_default() {
            self.scope.bind(&n, ());
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per expression kind; the larger arms would have to be functions"
    )]
    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Lit(_) | ExprKind::Unit | ExprKind::Elided => {}
            ExprKind::Name(q) => self.lower_name(q, false),
            ExprKind::Call { callee, args, .. } => {
                if callee
                    .name
                    .text
                    .starts_with(|c: char| c.is_ascii_uppercase())
                {
                    match lookup_type(self.schema, &self.vis, callee) {
                        Ok(TypeName::Decl(t))
                            if matches!(self.schema.ty(t).kind, TypeDefKind::Newtype { .. }) => {}
                        Ok(_) => self.err(
                            callee.span(),
                            format!(
                                "`{}` is not a newtype; build a constructor with `{{ ... }}`",
                                callee.name.text
                            ),
                        ),
                        Err(err) => self.err(callee.span(), err),
                    }
                } else {
                    self.lower_name(callee, true);
                }
                self.call_args(args);
            }
            ExprKind::TypeCall { ty, args, .. } => {
                self.type_name(ty);
                self.call_args(args);
            }
            ExprKind::Ctor { path, ctor, record } => {
                if let Some(c) = ctor {
                    self.qualified_ctor(path, c);
                }
                if let Some(r) = record {
                    self.record(r);
                }
            }
            ExprKind::Record(r) => self.record(r),
            ExprKind::Set(es) => {
                for x in es {
                    self.expr(x);
                }
            }
            ExprKind::Unary(_, x)
            | ExprKind::Reject(x)
            | ExprKind::Field(x, _)
            | ExprKind::Select(x, _) => self.expr(x),
            ExprKind::Binary(_, a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Backlink(x, rel, field) => {
                self.expr(x);
                if let Some(r) = self.relation(rel) {
                    let ctors = rel
                        .sel
                        .as_ref()
                        .and_then(|s| self.schema.relation_sel(r, s).ok());
                    if !row_fields(self.schema, r, ctors.as_deref()).contains(&field.text) {
                        self.err(
                            field.span,
                            format!(
                                "`{}` is not a field of every row of `{}`",
                                field.text, rel.relation.name.text
                            ),
                        );
                    }
                }
            }
            ExprKind::KeyCall(x, args) => {
                match &x.kind {
                    ExprKind::Name(q)
                        if self.scope.lookup(&q.name.text).is_none() || q.module.is_some() =>
                    {
                        match lookup_value(self.schema, &self.vis, q) {
                            Ok(ValueDef::Relation(_)) => {}
                            Ok(_) => self.err(x.span, "`.key(...)` follows a relation name"),
                            Err(err) => self.err(x.span, err),
                        }
                    }
                    _ => self.err(x.span, "`.key(...)` follows a relation name"),
                }
                match args {
                    KeyArgs::Value(v) => self.expr(v),
                    KeyArgs::Named(ns) => {
                        for (_, v) in ns {
                            self.expr(v);
                        }
                    }
                }
            }
            ExprKind::Comprehension(c) => self.comprehension(c, |_| {}),
            ExprKind::Match(scrutinee, arms) => {
                if let Some(s) = scrutinee {
                    self.expr(s);
                }
                self.arms(arms);
            }
            ExprKind::Fix(binds, body) => {
                self.scope.push();
                for (n, _) in binds {
                    self.scope.bind(&n.text, ());
                }
                for (_, b) in binds {
                    self.expr(b);
                }
                self.expr(body);
                self.scope.pop();
            }
            ExprKind::Lambda(params, body) => {
                self.scope.push();
                match params {
                    LambdaParams::One(p) => self.scope.bind(&p.text, ()),
                    LambdaParams::List(ps) => {
                        for p in ps {
                            self.type_name(&p.ty);
                            self.scope.bind(&p.name.text, ());
                        }
                    }
                }
                self.expr(body);
                self.scope.pop();
            }
            ExprKind::Exists(gens, filter) => {
                self.scope.push();
                self.generators(gens);
                if let Some(f) = filter {
                    self.expr(f);
                }
                self.scope.pop();
            }
            ExprKind::Top { count, keys, body } => {
                self.expr(count);
                self.comprehension(body, |r| {
                    for k in keys {
                        r.expr(&k.expr);
                    }
                });
            }
            ExprKind::If(c, a, b) => {
                self.expr(c);
                self.expr(a);
                self.expr(b);
            }
            ExprKind::At { point, body, .. } => {
                self.expr(point);
                self.expr(body);
            }
            ExprKind::Write(w) => self.write(w),
        }
    }

    fn arms(&mut self, arms: &[Arm]) {
        for arm in arms {
            self.scope.push();
            self.arm_patterns(&arm.patterns);
            if let Some(g) = &arm.guard {
                self.expr(g);
            }
            self.expr(&arm.body);
            self.scope.pop();
        }
    }

    fn bound_binder(&mut self, binder: &Ident) {
        if self.scope.lookup(&binder.text).is_none() {
            self.err(
                binder.span,
                format!(
                    "`{}` names no row bound here; write `in` and its source",
                    binder.text
                ),
            );
        }
    }

    fn write(&mut self, w: &Write) {
        match &w.kind {
            WriteKind::Insert {
                target,
                source,
                upsert,
                rejects,
                ..
            } => {
                self.relation(target);
                self.expr(source);
                if let Some(u) = upsert {
                    if let ConflictTarget::Fields(fields, _) = &u.on {
                        let rel = lookup_value(self.schema, &self.vis, &target.relation);
                        if let Ok(ValueDef::Relation(r)) = rel {
                            let names = any_fields(self.schema, r);
                            self.scope.push();
                            for n in &names {
                                self.scope.bind(n, ());
                            }
                            for f in fields {
                                self.expr(f);
                            }
                            self.scope.pop();
                        }
                    }
                    self.scope.push();
                    self.scope.bind("old", ());
                    self.scope.bind("new", ());
                    self.write(&u.otherwise);
                    self.scope.pop();
                }
                if let Some(r) = rejects {
                    match lookup_value(self.schema, &self.vis, r) {
                        Ok(ValueDef::Relation(_)) => {}
                        Ok(_) => self.err(r.span(), "rejects go into a relation"),
                        Err(e) => self.err(r.span(), e),
                    }
                }
            }
            WriteKind::Update {
                binder,
                source,
                filter,
                value,
            } => {
                self.scope.push();
                match source {
                    Some(s) => {
                        self.expr(s);
                        self.scope.bind(&binder.text, ());
                    }
                    None => self.bound_binder(binder),
                }
                if let Some(f) = filter {
                    self.expr(f);
                }
                self.expr(value);
                self.scope.pop();
            }
            WriteKind::Delete {
                binder,
                source,
                filter,
            } => {
                self.scope.push();
                match source {
                    Some(s) => {
                        self.expr(s);
                        self.scope.bind(&binder.text, ());
                    }
                    None => self.bound_binder(binder),
                }
                if let Some(f) = filter {
                    self.expr(f);
                }
                self.scope.pop();
            }
            WriteKind::Fail(e) => self.expr(e),
            WriteKind::Skip => {}
            WriteKind::Seq(ws) => {
                for w in ws {
                    self.write(w);
                }
            }
            WriteKind::If(c, a, b) => {
                self.expr(c);
                self.write(a);
                self.write(b);
            }
            WriteKind::Match(s, arms) => {
                self.expr(s);
                for arm in arms {
                    self.scope.push();
                    self.arm_patterns(&arm.patterns);
                    if let Some(g) = &arm.guard {
                        self.expr(g);
                    }
                    self.write(&arm.body);
                    self.scope.pop();
                }
            }
        }
    }

    fn transact(&mut self, b: &TransactBlock) {
        self.scope.push();
        for item in &b.items {
            match item {
                BlockItem::Let(name, v) => {
                    match v {
                        LetValue::Expr(e) => self.expr(e),
                        LetValue::Write(w) => self.write(w),
                    }
                    self.scope.bind(&name.text, ());
                }
                BlockItem::Write(w) => self.write(w),
            }
        }
        if let Some(r) = &b.ret {
            self.expr(r);
        }
        self.scope.pop();
    }
}
