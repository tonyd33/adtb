use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::ast;
use crate::kernel::{Kind, Terms, Tm, Var};
use crate::resolve::{Scope, Visible};
use crate::schema::{
    DefaultValue, PRELUDE, Schema, TypeDefKind, TypeMode, TypeName, TypeScope, ValueDef,
};
use crate::span::{Diag, FileDiag, FileId, Span};
use crate::types::*;

mod expr;
mod pat;
mod prelude;
mod rel;
mod ty;
mod write;

#[cfg(test)]
mod tests;

pub use ty::{join, subtype};
pub use write::{Txn, check_txn};

type R<T> = Result<T, Diag>;

/// A checked expression: its kernel term and type, and the relation whose rows it holds.
#[derive(Clone, Debug)]
pub struct Out {
    pub tm: Tm,
    pub ty: Ty,
    /// For a row, the relation it is a row of; for a set, the relation its elements are rows of.
    pub rows: Option<RelId>,
}

/// The kernel terms of every body a schema declares.
#[derive(Debug)]
pub struct Program {
    pub terms: Terms,
    /// Each function's term, a `Lam` over its parameters; `None` for Wasm and host functions.
    pub functions: Vec<Option<Tm>>,
    pub constraints: Vec<Tm>,
    pub checks: Vec<Tm>,
    /// Each newtype's domain predicate over its `self` variable.
    pub domains: BTreeMap<TypeId, (Var, Tm)>,
    /// Each relation's `on delete retag` new value, a `Lam` over the referrer's row.
    pub retags: BTreeMap<RelId, Tm>,
}

/// What a body may refer to besides its locals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// A function or registered query: `caller` is in scope.
    Function,
    /// A constraint, check, default or other schema-level expression.
    Schema,
    /// A transition constraint: also `inserted`, `deleted` and `prior`.
    Constraint,
    /// Ad hoc text: `_` covers a sum's remaining constructors.
    Adhoc,
}

#[derive(Clone, Debug)]
struct Local {
    var: Var,
    ty: Ty,
    rows: Option<RelId>,
    group: Option<Rc<GroupInfo>>,
}

/// What a group binder `g` stands for after `into`.
#[derive(Debug)]
struct GroupInfo {
    /// The binders before `group`, each with its field in a row of the group.
    binders: Vec<(String, Var)>,
    /// The grouped value, over the binders' variables.
    value: Tm,
    value_ty: Ty,
    value_rows: Option<RelId>,
    rows_ty: Ty,
}

pub struct Checker<'a> {
    pub schema: &'a Schema,
    pub terms: Terms,
    vis: Visible,
    ctx: Context,
    scope: Scope<Local>,
    /// Type parameters of the function being checked, with their bounds.
    tparams: Vec<(String, Vec<Bound>)>,
    /// Relations whose rows each checked function returns, for `key` over a view's rows.
    fn_rows: Vec<Option<RelId>>,
    /// Ad hoc parameters, typed by their descriptors.
    params: BTreeMap<String, (Var, Ty)>,
    /// Whether `reject` may appear: in the source query of an insert.
    reject_ok: bool,
    /// Per enclosing write, the rows its nested `key (insert ...)` insert.
    hoist: Vec<Vec<write::Nested>>,
    /// Whether a nested `key (insert ...)` here is evaluated once per statement.
    once: bool,
    /// Variables bound to rows of the snapshot, which `update x` and `delete x` may name.
    snapshot: BTreeSet<Var>,
    /// Variables bound to rows a write proposes, which are never rows of the snapshot.
    proposed: BTreeSet<Var>,
    read_only: bool,
}

impl std::fmt::Debug for Checker<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Checker").field("ctx", &self.ctx).finish()
    }
}

fn diag(span: Span, msg: impl Into<String>) -> Diag {
    Diag::new(span, msg)
}

impl<'a> Checker<'a> {
    pub fn new(schema: &'a Schema, vis: Visible, ctx: Context) -> Self {
        Checker {
            schema,
            terms: Terms::default(),
            vis,
            ctx,
            scope: Scope::default(),
            tparams: Vec::new(),
            fn_rows: vec![None; schema.functions.len()],
            params: BTreeMap::new(),
            reject_ok: false,
            hoist: Vec::new(),
            once: true,
            snapshot: BTreeSet::new(),
            proposed: BTreeSet::new(),
            read_only: false,
        }
    }

    fn module(&self) -> ModuleId {
        self.vis.first()
    }

    fn adhoc(&self) -> bool {
        self.ctx == Context::Adhoc
    }

    fn add(&mut self, kind: Kind, ty: Ty) -> Tm {
        self.terms.add(kind, ty)
    }

    fn out(&mut self, kind: Kind, ty: Ty) -> Out {
        let tm = self.add(kind, ty.clone());
        Out { tm, ty, rows: None }
    }

    fn show(&self, ty: &Ty) -> String {
        let names: Vec<String> = self.tparams.iter().map(|(n, _)| n.clone()).collect();
        self.schema.show_ty_with(ty, &names)
    }

    /// Binds `name` in the innermost scope and returns its variable.
    fn bind(&mut self, name: &str, ty: Ty, rows: Option<RelId>) -> Var {
        let var = self.terms.var(name, ty.clone());
        self.scope.bind(
            name,
            Local {
                var,
                ty,
                rows,
                group: None,
            },
        );
        var
    }

    fn fresh(&mut self, ty: Ty) -> Var {
        self.terms.var("ξ", ty)
    }

    fn var_tm(&mut self, v: Var) -> Tm {
        let ty = self.terms.var_info(v).ty.clone();
        self.add(Kind::Var(v), ty)
    }

    fn bool_ty(&self) -> Ty {
        Ty::Data {
            def: self.schema.prelude_type("Bool"),
            args: Vec::new(),
            ctors: None,
        }
    }

    fn bool_tm(&mut self, value: bool) -> Tm {
        let ty = self.bool_ty();
        self.add(
            Kind::Ctor {
                ctor: value as u32,
                fields: Vec::new(),
            },
            ty,
        )
    }

    fn option_ty(&self, t: Ty) -> Ty {
        Ty::Data {
            def: self.schema.prelude_type("Option"),
            args: vec![t],
            ctors: None,
        }
    }

    pub fn resolve_type(&self, t: &ast::Type) -> R<Ty> {
        let names: Vec<String> = self.tparams.iter().map(|(n, _)| n.clone()).collect();
        let scope = TypeScope {
            module: self.module(),
            params: &names,
            mode: TypeMode::Signature,
        };
        let mut ordered = Vec::new();
        let ty = crate::schema::resolve_type(self.schema, &scope, t, &mut ordered, FileId(0))
            .map_err(|(span, msg)| diag(span, msg))?;
        for (arg, _, span) in ordered {
            if !self.ordered(&arg) {
                return Err(diag(span, "`Span a` needs an ordered `a`"));
            }
        }
        Ok(ty)
    }

    fn lookup_type(&self, name: &ast::QName) -> Result<TypeName, String> {
        crate::resolve::lookup_type(self.schema, &self.vis, name)
    }

    fn lookup_value(&self, name: &ast::QName) -> Result<ValueDef, String> {
        crate::resolve::lookup_value(self.schema, &self.vis, name)
    }

    /// Every declared sum in scope with a constructor of this name, with its index.
    fn ctors_named(&self, name: &str) -> Vec<(TypeId, u32)> {
        let mut modules = self.vis.modules.clone();
        modules.push(PRELUDE);
        let mut out: Vec<(TypeId, u32)> = Vec::new();
        let mut push = |t: TypeId| {
            if let Some(i) = self.schema.ty(t).ctor_index(name)
                && !out.contains(&(t, i))
            {
                out.push((t, i));
            }
        };
        for m in modules {
            for (tname, &t) in &self.schema.module(m).types {
                if m == PRELUDE && tname == "Principal" {
                    continue;
                }
                push(t);
            }
        }
        push(self.schema.principal);
        out
    }

    fn is_num(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Base(b) => b.is_num(),
            Ty::Param(i) => self.param_has(*i, Bound::Num),
            _ => false,
        }
    }

    fn param_has(&self, i: u32, b: Bound) -> bool {
        self.tparams
            .get(i as usize)
            .is_some_and(|(_, bs)| bs.contains(&b))
    }

    pub fn ordered(&self, ty: &Ty) -> bool {
        self.schema
            .is_ordered_with(ty, &|i| self.param_has(i, Bound::Ord))
    }
}

/// Function ids in an order where each function follows the functions it calls.
fn call_order(schema: &Schema) -> Result<Vec<FnId>, FileDiag> {
    let n = schema.functions.len();
    let calls: Vec<BTreeSet<FnId>> = (0..n as u32)
        .map(|i| crate::resolve::function_refs(schema, FnId(i)))
        .collect();
    let mut order = Vec::new();
    let mut state = vec![0u8; n];
    fn visit(
        i: usize,
        calls: &[BTreeSet<FnId>],
        state: &mut [u8],
        order: &mut Vec<FnId>,
    ) -> Result<(), usize> {
        match state[i] {
            2 => return Ok(()),
            1 => return Err(i),
            _ => {}
        }
        state[i] = 1;
        for g in &calls[i] {
            visit(g.0 as usize, calls, state, order)?;
        }
        state[i] = 2;
        order.push(FnId(i as u32));
        Ok(())
    }
    for i in 0..n {
        if let Err(j) = visit(i, &calls, &mut state, &mut order) {
            let f = &schema.functions[j];
            return Err(FileDiag {
                file: f.file,
                diag: diag(
                    f.span,
                    format!(
                        "function `{}` calls itself; recursion is written with `fix`",
                        f.name
                    ),
                ),
            });
        }
    }
    Ok(order)
}

/// Type checks every body of a schema and elaborates it to the kernel.
pub fn check_schema(schema: &Schema) -> Result<Program, Vec<FileDiag>> {
    let order = call_order(schema).map_err(|d| vec![d])?;
    let mut program = Program {
        terms: Terms::default(),
        functions: vec![None; schema.functions.len()],
        constraints: Vec::new(),
        checks: Vec::new(),
        domains: BTreeMap::new(),
        retags: BTreeMap::new(),
    };
    let mut diags = Vec::new();
    let mut fn_rows = vec![None; schema.functions.len()];
    let mut terms = Terms::default();
    let mut run = |module: ModuleId,
                   ctx: Context,
                   file: FileId,
                   terms: &mut Terms,
                   fn_rows: &[Option<RelId>],
                   f: &mut dyn FnMut(&mut Checker) -> R<Tm>|
     -> Option<Tm> {
        let mut c = Checker::new(schema, Visible::module(module), ctx);
        c.terms = std::mem::take(terms);
        c.fn_rows = fn_rows.to_vec();
        let r = f(&mut c);
        *terms = std::mem::take(&mut c.terms);
        match r {
            Ok(t) => Some(t),
            Err(d) => {
                diags.push(FileDiag { file, diag: d });
                None
            }
        }
    };
    for id in order {
        let def = schema.function(id);
        let ast::FnBody::Expr(body) = &def.body else {
            continue;
        };
        let mut rows = None;
        let tm = run(
            def.module,
            Context::Function,
            def.file,
            &mut terms,
            &fn_rows,
            &mut |c| {
                let (tm, r) = c.function(def, body)?;
                rows = r;
                Ok(tm)
            },
        );
        fn_rows[id.0 as usize] = rows;
        program.functions[id.0 as usize] = tm;
    }
    for con in &schema.constraints {
        if let Some(tm) = run(
            con.module,
            Context::Constraint,
            con.file,
            &mut terms,
            &fn_rows,
            &mut |c| c.constraint(&con.query),
        ) {
            program.constraints.push(tm);
        }
    }
    for chk in &schema.checks {
        if chk.relation.0 == u32::MAX {
            continue;
        }
        if let Some(tm) = run(
            chk.module,
            Context::Schema,
            chk.file,
            &mut terms,
            &fn_rows,
            &mut |c| c.check_clause(chk.relation, chk.ctors.clone(), &chk.predicate),
        ) {
            program.checks.push(tm);
        }
    }
    for (i, t) in schema.types.iter().enumerate() {
        if let TypeDefKind::Newtype {
            base,
            domain: Some(d),
        } = &t.kind
        {
            let mut var = None;
            if let Some(tm) = run(
                t.module,
                Context::Schema,
                t.file,
                &mut terms,
                &fn_rows,
                &mut |c| {
                    c.scope.push();
                    let v = c.bind("self", base.clone(), None);
                    var = Some(v);
                    let ty = c.bool_ty();
                    let out = c.check(d, &ty)?;
                    Ok(out.tm)
                },
            ) {
                program
                    .domains
                    .insert(TypeId(i as u32), (var.expect("bound"), tm));
            }
        }
    }
    for (i, r) in schema.relations.iter().enumerate() {
        if r.elem_def.0 == u32::MAX {
            continue;
        }
        let rel = RelId(i as u32);
        for d in &r.defaults {
            if let DefaultValue::Expr(e) = &d.value {
                run(
                    r.module,
                    Context::Schema,
                    r.file,
                    &mut terms,
                    &fn_rows,
                    &mut |c| c.default_clause(rel, d.ctor, &d.field, e),
                );
            }
        }
        for comp in &r.computed {
            run(
                r.module,
                Context::Schema,
                r.file,
                &mut terms,
                &fn_rows,
                &mut |c| {
                    c.scope.push();
                    c.bind("self", r.elem.clone(), Some(rel));
                    Ok(c.check(&comp.expr, &comp.ty)?.tm)
                },
            );
        }
        for ix in &r.indexes {
            for e in &ix.fields {
                run(
                    r.module,
                    Context::Schema,
                    r.file,
                    &mut terms,
                    &fn_rows,
                    &mut |c| c.index_field(rel, ix.scope.clone(), e),
                );
            }
        }
    }
    program.retags = write::delete_actions(schema, &mut terms, &mut diags);
    program.terms = terms;
    if diags.is_empty() {
        Ok(program)
    } else {
        Err(diags)
    }
}

/// Checked ad hoc text: one kernel term per item.
#[derive(Debug)]
pub struct Adhoc {
    pub terms: Terms,
    pub items: Vec<Tm>,
    pub params: BTreeMap<String, Var>,
}

/// Type checks ad hoc text, its free names typed by `params`.
pub fn check_adhoc(
    schema: &Schema,
    items: &[ast::AdhocItem],
    params: &BTreeMap<String, Ty>,
) -> Result<Adhoc, Vec<Diag>> {
    let mut c = Checker::new(schema, Visible::all(schema), Context::Adhoc);
    for (name, ty) in params {
        let v = c.terms.var(name, ty.clone());
        c.params.insert(name.clone(), (v, ty.clone()));
    }
    let mut out = Vec::new();
    let mut diags = Vec::new();
    for item in items {
        let r = match item {
            ast::AdhocItem::Expr(e) => c.statement(e).map(|o| o.tm),
            ast::AdhocItem::Write(w) => c.write(w),
            ast::AdhocItem::Transact(b) => c.transact(b, None).map(|o| o.tm),
            _ => continue,
        };
        match r {
            Ok(tm) => out.push(tm),
            Err(d) => diags.push(d),
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    Ok(Adhoc {
        params: c.params.iter().map(|(n, (v, _))| (n.clone(), *v)).collect(),
        terms: c.terms,
        items: out,
    })
}

/// A checked registered query: its term, a `Lam` over its parameters.
#[derive(Debug)]
pub struct Query {
    pub terms: Terms,
    pub term: Tm,
}

/// Type checks a registered query of a bundle that reads `module`.
pub fn check_query(schema: &Schema, module: ModuleId, q: &ast::QueryDecl) -> Result<Query, Diag> {
    let mut c = Checker::new(schema, Visible::module(module), Context::Function);
    let term = c.query(q)?;
    Ok(Query {
        terms: c.terms,
        term,
    })
}

impl Checker<'_> {
    fn query(&mut self, q: &ast::QueryDecl) -> R<Tm> {
        if let Some(l) = &q.limit {
            self.check(l, &Ty::Base(Base::Duration))?;
        }
        let result = self.resolve_type(&q.result)?;
        if result.mentions_fn() {
            return Err(diag(
                q.result.span,
                "a registered query never returns a function",
            ));
        }
        self.scope.push();
        let mut vars = Vec::new();
        for p in &q.params {
            let pty = self.resolve_type(&p.ty)?;
            if pty.mentions_fn() {
                return Err(diag(
                    p.ty.span,
                    "a parameter of a registered query never has a function type",
                ));
            }
            vars.push(self.bind(&p.name.text, pty, None));
        }
        let body = self.top_query(&q.body, Some(&result));
        self.scope.pop();
        let body = body?;
        Ok(self.lams(vars, body.tm, result))
    }

    /// Wraps `body`, of type `result`, in one `Lam` per variable of `vars`, or one over unit when there are none.
    fn lams(&mut self, mut vars: Vec<Var>, body: Tm, result: Ty) -> Tm {
        if vars.is_empty() {
            vars.push(self.terms.var("ξ", Ty::Unit));
        }
        let mut tm = body;
        let mut ty = result;
        for v in vars.into_iter().rev() {
            let pty = self.terms.var_info(v).ty.clone();
            ty = Ty::Fn(Box::new(pty), Box::new(ty));
            tm = self.add(Kind::Lam(v, tm), ty.clone());
        }
        tm
    }

    fn function(
        &mut self,
        def: &crate::schema::FunctionDef,
        body: &ast::Expr,
    ) -> R<(Tm, Option<RelId>)> {
        self.tparams = def.type_params.clone();
        self.scope.push();
        let mut vars = Vec::new();
        for (name, ty) in &def.params {
            vars.push(self.bind(name, ty.clone(), None));
        }
        let out = self.check(body, &def.result)?;
        self.scope.pop();
        let rows = out
            .rows
            .filter(|_| matches!(def.result, Ty::Set(_) | Ty::NonEmpty(_)));
        Ok((self.lams(vars, out.tm, def.result.clone()), rows))
    }

    /// Checks a top-level query of ad hoc text, where `order by` is allowed.
    pub fn statement(&mut self, e: &ast::Expr) -> R<Out> {
        self.top_query(e, None)
    }

    fn constraint(&mut self, q: &ast::Expr) -> R<Tm> {
        let out = self.synth(q)?;
        if !matches!(out.ty, Ty::Set(_) | Ty::NonEmpty(_)) {
            return Err(diag(
                q.span,
                format!(
                    "a constraint is a query returning a set, here of type {}",
                    self.show(&out.ty)
                ),
            ));
        }
        Ok(out.tm)
    }

    /// Checks `f` with `self` and each field of a row of `elem` bound by name, wrapping its term in those bindings.
    fn in_row(
        &mut self,
        elem: &Ty,
        rel: RelId,
        f: &mut dyn FnMut(&mut Self) -> R<Out>,
    ) -> R<(Var, Out)> {
        self.scope.push();
        let x = self.bind("self", elem.clone(), Some(rel));
        let mut lets = Vec::new();
        for (name, fty) in self.fields_of(elem) {
            let v = self.bind(&name, fty.clone(), None);
            let xt = self.var_tm(x);
            let ft = self.add(Kind::Field(xt, name), fty);
            lets.push((v, ft));
        }
        let r = f(self);
        self.scope.pop();
        let mut out = r?;
        for (v, ft) in lets.into_iter().rev() {
            out.tm = self.add(Kind::Let(v, ft, out.tm), out.ty.clone());
        }
        Ok((x, out))
    }

    /// `for x in R do if p then {} else {x}`.
    fn check_clause(&mut self, rel: RelId, ctors: Option<CtorSet>, p: &ast::Expr) -> R<Tm> {
        let source = self.rel_out(rel, ctors);
        let elem = self.elem_ty(&source.ty).expect("a relation is a set");
        let b = self.bool_ty();
        let (x, pred) = self.in_row(&elem, rel, &mut |c| c.check(p, &b))?;
        let set_ty = Ty::Set(Box::new(elem));
        let empty = self.add(Kind::EmptySet, set_ty.clone());
        let xt = self.var_tm(x);
        let one = self.add(Kind::Singleton(xt), set_ty.clone());
        let body = self.add(Kind::If(pred.tm, empty, one), set_ty.clone());
        Ok(self.add(Kind::For(x, source.tm, body), set_ty))
    }

    fn default_clause(
        &mut self,
        rel: RelId,
        ctor: Option<u32>,
        field: &str,
        e: &ast::Expr,
    ) -> R<Tm> {
        let r = self.schema.relation(rel);
        let def = self.schema.ty(r.elem_def);
        let fty = match ctor {
            Some(c) => def.field(c, field),
            None => (0..def.ctors().len() as u32).find_map(|c| def.field(c, field)),
        }
        .map(|f| f.ty.subst(r.elem_args()))
        .ok_or_else(|| diag(e.span, format!("`{}` has no field `{field}`", def.name)))?;
        Ok(self.check(e, &fty)?.tm)
    }

    /// An index field as a function of the row.
    fn index_field(&mut self, rel: RelId, ctors: Option<CtorSet>, e: &ast::Expr) -> R<Tm> {
        let source = self.rel_out(rel, ctors);
        let elem = self.elem_ty(&source.ty).expect("a relation is a set");
        let (x, out) = self.in_row(&elem, rel, &mut |c| c.synth(e))?;
        Ok(self.lams(vec![x], out.tm, out.ty))
    }
}
