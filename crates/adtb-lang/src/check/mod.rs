use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::ast;
use crate::kernel::{IndexField, Kind, PathStep, Terms, Tm, Var};
use crate::resolve::{Scope, Visible};
use crate::schema::{
    DefaultValue, PRELUDE, Schema, TypeDefKind, TypeMode, TypeName, TypeScope, ValueDef,
};
use crate::span::{Diag, FileDiag, FileId, Span};
use crate::types::*;

mod adhoc;
mod expr;
mod pat;
mod prelude;
mod rel;
mod ty;
mod write;

#[cfg(test)]
mod tests;

pub use adhoc::{AdhocCommand, Bundles, Command, Explained, RestorePoint};
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
    /// Each default expression, keyed by relation and position among its defaults.
    pub defaults: BTreeMap<(RelId, usize), Tm>,
    /// Each computed field's expression over its `self` variable, keyed by relation and position.
    pub computed: BTreeMap<(RelId, usize), (Var, Tm)>,
    /// Each index field, its value a `Lam` over the row, keyed by relation, index position and field position.
    pub index_fields: BTreeMap<(RelId, usize, usize), IndexField>,
    /// Each user-defined aggregate's parts, by position among `Schema::aggregates`.
    pub aggregates: Vec<Option<AggregateTerms>>,
    /// The relation whose rows each function returns, for `key` over a view's rows.
    pub fn_rows: Vec<Option<RelId>>,
}

/// The parts of a user-defined aggregate: `init` a state, `step` a `Lam` over the state and an
/// input, `merge` a `Lam` over two states, `finish` a `Lam` over a state.
#[derive(Clone, Copy, Debug)]
pub struct AggregateTerms {
    pub init: Tm,
    pub step: Tm,
    pub merge: Tm,
    pub finish: Tm,
}

/// A schema object a body names, as [`schema_refs`] reports it at the span that names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ref {
    Type(TypeId),
    /// A type of the prelude with no declaration: a base type, `Key`, `Decimal`, `Set`, `Group` or `Tag`.
    BuiltinType,
    /// A constructor of an anonymous sum or of a tag type, which no declaration names.
    Anonymous,
    Relation(RelId),
    Function(FnId),
    Aggregate(u32),
    Ctor(TypeId, u32),
    Set(RelId, String),
    /// A field of constructor `ctor`, or of the header when `ctor` is `None`; `pun` when the name is also the bound or given value.
    Field {
        def: TypeId,
        ctor: Option<u32>,
        name: String,
        pun: bool,
    },
    /// A `match`, at its span, over a value of this type.
    Match(Ty),
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
    /// The names this checker resolves, when recording.
    refs: Option<RefCell<Vec<(Span, Ref)>>>,
    /// Whether a match that misses a constructor is refused.
    coverage: bool,
    /// A row whose header fields a constructor of its type built here takes when it omits them.
    carry: Option<(Var, TypeId)>,
    /// The locals that stand for a field of a row, which a bare field name in an index or check binds.
    row_fields: BTreeMap<Var, (TypeId, Option<CtorSet>, String)>,
    /// The temp relations of ad hoc text not yet declared at the item being checked.
    hidden: BTreeSet<RelId>,
    /// The checked schema, which ad hoc text names the index fields of.
    program: Option<&'a Program>,
    /// The registered queries ad hoc text may explain.
    bundles: Option<&'a Bundles>,
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
            refs: None,
            coverage: true,
            carry: None,
            row_fields: BTreeMap::new(),
            hidden: BTreeSet::new(),
            program: None,
            bundles: None,
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
                ctor: u32::from(value),
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
        if let Some(refs) = &self.refs {
            crate::canon::type_refs(self.schema, self.module(), t, &mut refs.borrow_mut());
        }
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

    /// Returns whether more than one visible module declares a value named `name`.
    fn ambiguous(&self, name: &str) -> bool {
        let declaring = self.vis.modules.iter();
        declaring
            .filter(|&&m| self.schema.module(m).values.contains_key(name))
            .count()
            > 1
    }

    fn lookup_value(&self, name: &ast::QName) -> Result<ValueDef, String> {
        let v = crate::resolve::lookup_value(self.schema, &self.vis, name);
        if let Ok(ValueDef::Relation(r)) = v
            && self.hidden.contains(&r)
        {
            return Err(format!("no `{}` in scope", name.name.text));
        }
        match v {
            Ok(ValueDef::Relation(r)) => self.note(name.name.span, Ref::Relation(r)),
            Ok(ValueDef::Function(f)) => self.note(name.name.span, Ref::Function(f)),
            Ok(ValueDef::Aggregate(a)) => self.note(name.name.span, Ref::Aggregate(a)),
            _ => {}
        }
        v
    }

    fn note(&self, span: Span, r: Ref) {
        if let Some(refs) = &self.refs {
            refs.borrow_mut().push((span, r));
        }
    }

    /// Notes each constructor or named set a selection names, of type `def` and, for a relation's selection, of `rel`.
    fn note_sel(&self, def: TypeId, rel: Option<RelId>, sel: &ast::CtorSel) {
        if self.refs.is_none() {
            return;
        }
        let names: Vec<&ast::Ident> = match sel {
            ast::CtorSel::One(c) => vec![c],
            ast::CtorSel::Many(cs) => cs.iter().collect(),
        };
        for n in names {
            match (self.schema.ty(def).ctor_index(&n.text), rel) {
                (Some(i), _) => self.note(n.span, Ref::Ctor(def, i)),
                (None, Some(r)) => self.note(n.span, Ref::Set(r, n.text.clone())),
                (None, None) => {}
            }
        }
    }

    /// Notes field `name` of the rows of `def` in `ctors`, every constructor when `None`.
    fn note_field(&self, span: Span, def: TypeId, ctors: Option<&[u32]>, name: &str, pun: bool) {
        if self.refs.is_none() {
            return;
        }
        let t = self.schema.ty(def);
        if t.header().iter().any(|f| f.name == name) {
            return self.note(
                span,
                Ref::Field {
                    def,
                    ctor: None,
                    name: name.to_string(),
                    pun,
                },
            );
        }
        let all: Vec<u32> = (0..t.ctors().len() as u32).collect();
        for &c in ctors.unwrap_or(&all) {
            if t.ctors()[c as usize].fields.iter().any(|f| f.name == name) {
                self.note(
                    span,
                    Ref::Field {
                        def,
                        ctor: Some(c),
                        name: name.to_string(),
                        pun,
                    },
                );
            }
        }
    }

    /// Resolves a relation's constructor selection, noting what it names.
    fn relation_sel(&self, rel: RelId, sel: &ast::CtorSel) -> Result<CtorSet, String> {
        self.note_sel(self.schema.relation(rel).elem_def, Some(rel), sel);
        self.schema.relation_sel(rel, sel)
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
    let (program, mut diags) = bodies(schema, &order, None);
    if diags.is_empty() {
        diags = crate::analysis::schema_diags(schema, &program);
    }
    if diags.is_empty() {
        Ok(program)
    } else {
        Err(diags)
    }
}

/// Returns every schema object the bodies of a schema name, by file and span, with each `match` the bodies hold.
///
/// A body that does not check contributes the names resolved before its first error; a match that misses
/// constructors is not an error here.
pub fn schema_refs(schema: &Schema) -> Vec<(FileId, Span, Ref)> {
    let order = call_order(schema)
        .unwrap_or_else(|_| (0..schema.functions.len() as u32).map(FnId).collect());
    let mut refs = Vec::new();
    bodies(schema, &order, Some(&mut refs));
    refs
}

#[expect(
    clippy::too_many_lines,
    reason = "checking function bodies, relation defaults, computed fields and index fields would each have to be a pass"
)]
fn bodies(
    schema: &Schema,
    order: &[FnId],
    mut record: Option<&mut Vec<(FileId, Span, Ref)>>,
) -> (Program, Vec<FileDiag>) {
    let mut program = Program {
        terms: Terms::default(),
        functions: vec![None; schema.functions.len()],
        constraints: Vec::new(),
        checks: Vec::new(),
        domains: BTreeMap::new(),
        retags: BTreeMap::new(),
        defaults: BTreeMap::new(),
        computed: BTreeMap::new(),
        index_fields: BTreeMap::new(),
        aggregates: vec![None; schema.aggregates.len()],
        fn_rows: Vec::new(),
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
        if record.is_some() {
            c.refs = Some(RefCell::new(Vec::new()));
            c.coverage = false;
        }
        let r = f(&mut c);
        *terms = std::mem::take(&mut c.terms);
        if let (Some(out), Some(refs)) = (record.as_deref_mut(), c.refs.take()) {
            out.extend(refs.into_inner().into_iter().map(|(s, r)| (file, s, r)));
        }
        match r {
            Ok(t) => Some(t),
            Err(d) => {
                diags.push(FileDiag { file, diag: d });
                None
            }
        }
    };
    for &id in order {
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
    for (i, def) in schema.aggregates.iter().enumerate() {
        let mut parts = None;
        run(
            def.module,
            Context::Function,
            def.file,
            &mut terms,
            &fn_rows,
            &mut |c| {
                let p = c.aggregate_decl(def)?;
                parts = Some(p);
                Ok(p.init)
            },
        );
        program.aggregates[i] = parts;
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
        let rel = RelId(i as u32);
        for (j, d) in r.defaults.iter().enumerate() {
            if let DefaultValue::Expr(e) = &d.value
                && let Some(tm) = run(
                    r.module,
                    Context::Schema,
                    r.file,
                    &mut terms,
                    &fn_rows,
                    &mut |c| c.default_clause(rel, d.ctor, &d.field, e),
                )
            {
                program.defaults.insert((rel, j), tm);
            }
        }
        for (j, comp) in r.computed.iter().enumerate() {
            let mut var = None;
            if let Some(tm) = run(
                r.module,
                Context::Schema,
                r.file,
                &mut terms,
                &fn_rows,
                &mut |c| {
                    c.scope.push();
                    var = Some(c.bind("self", r.elem.clone(), Some(rel)));
                    Ok(c.check(&comp.expr, &comp.ty)?.tm)
                },
            ) {
                program.computed.insert((rel, j), (var.expect("bound"), tm));
            }
        }
        for (j, ix) in r.indexes.iter().enumerate() {
            for (k, e) in ix.fields.iter().enumerate() {
                let mut path = Vec::new();
                if let Some(value) = run(
                    r.module,
                    Context::Schema,
                    r.file,
                    &mut terms,
                    &fn_rows,
                    &mut |c| {
                        let f = c.index_field(rel, ix.scope.clone(), e)?;
                        path = f.path;
                        Ok(f.value)
                    },
                ) {
                    program
                        .index_fields
                        .insert((rel, j, k), IndexField { value, path });
                }
            }
        }
    }
    program.retags = write::delete_actions(schema, &mut terms, &mut diags);
    program.terms = terms;
    program.fn_rows = fn_rows;
    (program, diags)
}

/// A body over one row, as a fill or a routing gives it.
#[derive(Clone, Copy, Debug)]
pub enum RowBody<'e> {
    /// A `match` with no scrutinee, at its span: it matches the row.
    Match(&'e [ast::Arm], Span),
    Expr(&'e ast::Expr),
}

/// Type checks `body` over a row of type `row`, seen from `module`, at `expected` when given.
///
/// A constructor of the row's type that a `match` body builds takes the row's header fields it omits.
/// Returns its terms, its term and its type. Refuses a body that reads anything but the row: a relation,
/// a reference, a generated key, `caller`, or a function that reads one of them. With `record`, appends
/// the schema objects the body names, as [`schema_refs`] reports them.
pub fn check_row(
    schema: &Schema,
    program: &Program,
    module: ModuleId,
    row: &Ty,
    body: RowBody,
    expected: Option<&Ty>,
    record: Option<&mut Vec<(Span, Ref)>>,
) -> Result<(Terms, Tm, Ty), Diag> {
    let mut c = Checker::new(schema, Visible::module(module), Context::Schema);
    if record.is_some() {
        c.refs = Some(RefCell::new(Vec::new()));
    }
    let checked = row_body(&mut c, program, row, body, expected);
    if let (Some(out), Some(refs)) = (record, c.refs.take()) {
        out.extend(refs.into_inner());
    }
    checked.map(|(_, tm, ty)| (c.terms, tm, ty))
}

/// Type checks `body` as [`check_row`] does, as a query whose one parameter is the row.
pub fn row_query(
    schema: &Schema,
    program: &Program,
    module: ModuleId,
    row: &Ty,
    body: RowBody,
    expected: Option<&Ty>,
) -> Result<Query, Diag> {
    let mut c = Checker::new(schema, Visible::module(module), Context::Schema);
    let (v, tm, ty) = row_body(&mut c, program, row, body, expected)?;
    let term = c.terms.add(
        Kind::Lam(v, tm),
        Ty::Fn(Box::new(row.clone()), Box::new(ty)),
    );
    Ok(Query {
        terms: c.terms,
        term,
    })
}

fn row_body(
    c: &mut Checker<'_>,
    program: &Program,
    row: &Ty,
    body: RowBody,
    expected: Option<&Ty>,
) -> Result<(Var, Tm, Ty), Diag> {
    c.scope.push();
    let v = c.bind(ROW, row.clone(), None);
    let (out, span) = match body {
        RowBody::Match(arms, span) => {
            if let Ty::Data { def, .. } = row {
                c.carry = Some((v, *def));
            }
            let scrutinee = ast::Expr {
                kind: ast::ExprKind::Name(ast::QName {
                    module: None,
                    name: ast::Ident {
                        text: ROW.to_string(),
                        span,
                    },
                }),
                span,
            };
            (c.match_expr(&scrutinee, arms, expected, span)?, span)
        }
        RowBody::Expr(e) => {
            let out = match expected {
                Some(t) => c.check(e, t)?,
                None => c.synth(e)?,
            };
            (out, e.span)
        }
    };
    if !reads_only_row(&c.terms, out.tm, program, &mut BTreeSet::new()) {
        return Err(diag(
            span,
            "a fill or a routing reads only its row; this reads a relation, a reference, a generated key, `caller` or a function that does",
        ));
    }
    Ok((v, out.tm, out.ty))
}

/// The name a row body binds its row to, which no source text can spell.
const ROW: &str = "row·";

fn reads_only_row(terms: &Terms, t: Tm, program: &Program, seen: &mut BTreeSet<FnId>) -> bool {
    match terms.kind(t) {
        Kind::Rel(..)
        | Kind::Transition(..)
        | Kind::Deref { .. }
        | Kind::Fresh(..)
        | Kind::Caller => {
            return false;
        }
        Kind::Call { func, .. } => {
            if seen.insert(*func)
                && let Some(Some(body)) = program.functions.get(func.0 as usize)
                && !reads_only_row(&program.terms, *body, program, seen)
            {
                return false;
            }
        }
        _ => {}
    }
    terms
        .children(t)
        .into_iter()
        .all(|c| reads_only_row(terms, c, program, seen))
}

/// Checked ad hoc text: one kernel term per statement and one checked form per command.
#[derive(Debug)]
pub struct Adhoc {
    pub terms: Terms,
    /// The term of each query, write and `transact` block, in the order of the text.
    pub items: Vec<Tm>,
    /// Each command, in the order of the text.
    pub commands: Vec<AdhocCommand>,
    pub params: BTreeMap<String, Var>,
    /// The schema the terms were checked against when the text declares temp relations: the given
    /// schema with each temp relation appended to its relations in module [`TEMP_MODULE`].
    pub temps: Option<Schema>,
}

impl Adhoc {
    /// Returns the schema the terms name declarations of, `given` being the one the text was checked against.
    pub fn schema<'a>(&'a self, given: &'a Schema) -> &'a Schema {
        self.temps.as_ref().unwrap_or(given)
    }
}

/// The module that holds the temp relations of ad hoc text; no module can take its name.
pub const TEMP_MODULE: &str = "temp";

/// Type checks ad hoc text, its free names typed by `params`, against `program`, the checked `schema`,
/// knowing no bundle.
pub fn check_adhoc(
    schema: &Schema,
    program: &Program,
    items: &[ast::AdhocItem],
    params: &BTreeMap<String, Ty>,
) -> Result<Adhoc, Vec<Diag>> {
    check_adhoc_recording(schema, program, items, params, &Bundles::default(), None)
}

/// Type checks ad hoc text as [`check_adhoc`] does, its `explain` of a registered call against
/// `bundles`. With `record`, appends the schema objects the text names, as [`schema_refs`] reports
/// them, those of items before a refused one included.
pub fn check_adhoc_recording(
    schema: &Schema,
    program: &Program,
    items: &[ast::AdhocItem],
    params: &BTreeMap<String, Ty>,
    bundles: &Bundles,
    record: Option<&mut Vec<(Span, Ref)>>,
) -> Result<Adhoc, Vec<Diag>> {
    let mut diags = Vec::new();
    let temps = adhoc::temp_relations(schema, items, &mut diags);
    let checked = temps.as_ref().map_or(schema, |(s, _)| s);
    let mut c = Checker::new(checked, Visible::all(checked), Context::Adhoc);
    c.program = Some(program);
    c.bundles = Some(bundles);
    if record.is_some() {
        c.refs = Some(RefCell::new(Vec::new()));
    }
    c.fn_rows.clone_from(&program.fn_rows);
    for (name, ty) in params {
        let v = c.terms.var(name, ty.clone());
        c.params.insert(name.clone(), (v, ty.clone()));
    }
    let declared: Vec<Option<RelId>> = temps.as_ref().map_or(Vec::new(), |(_, d)| d.clone());
    c.hidden = declared.iter().flatten().copied().collect();
    let mut declared = declared.into_iter();
    let mut out = Vec::new();
    let mut commands = Vec::new();
    for item in items {
        if let ast::AdhocItem::Temp(_) = item {
            if let Some(Some(r)) = declared.next() {
                c.hidden.remove(&r);
            }
            continue;
        }
        match c.adhoc_item(item) {
            Ok(adhoc::Checked::Statement(tm)) => out.push(tm),
            Ok(adhoc::Checked::Command(command, span)) => commands.push(AdhocCommand {
                at: out.len(),
                command,
                span,
            }),
            Ok(adhoc::Checked::Nothing) => {}
            Err(d) => diags.push(d),
        }
    }
    if let (Some(out), Some(refs)) = (record, c.refs.take()) {
        out.extend(refs.into_inner());
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    let adhoc = Adhoc {
        params: c.params.iter().map(|(n, (v, _))| (n.clone(), *v)).collect(),
        terms: c.terms,
        items: out,
        commands,
        temps: temps.map(|(s, _)| s),
    };
    let schema = adhoc.schema(schema);
    let spans: Vec<Span> = items
        .iter()
        .filter_map(|i| match i {
            ast::AdhocItem::Expr(e) => Some(e.span),
            ast::AdhocItem::Write(w) => Some(w.span),
            ast::AdhocItem::Transact(b) => Some(b.span),
            _ => None,
        })
        .collect();
    let diags = crate::analysis::adhoc_diags(schema, program, &adhoc, &spans);
    if !diags.is_empty() {
        return Err(diags);
    }
    Ok(adhoc)
}

/// A checked registered query: its term, a `Lam` over its parameters.
#[derive(Debug)]
pub struct Query {
    pub terms: Terms,
    pub term: Tm,
}

/// Type checks a registered query of a bundle that reads `module`, against `program`, the checked `schema`.
pub fn check_query(
    schema: &Schema,
    program: &Program,
    module: ModuleId,
    q: &ast::QueryDecl,
) -> Result<Query, Diag> {
    let mut c = Checker::new(schema, Visible::module(module), Context::Function);
    let term = c.query(q)?;
    let query = Query {
        terms: c.terms,
        term,
    };
    crate::analysis::query_diag(schema, program, &query, q.span)?;
    Ok(query)
}

/// Returns the relation variable a feed of a bundle of `module` names and the constructors it streams.
///
/// Fails when the relation does not resolve, a selected name is no constructor or named set of it,
/// or the relation is volatile.
pub fn check_feed(
    schema: &Schema,
    module: ModuleId,
    f: &ast::FeedDecl,
) -> Result<(RelId, CtorSet), Diag> {
    let of = &f.of;
    let rel = match crate::resolve::lookup_value(schema, &Visible::module(module), &of.relation) {
        Ok(ValueDef::Relation(rel)) => rel,
        Ok(_) => {
            return Err(Diag::new(
                of.span,
                format!("`{}` is not a relation", of.relation.name.text),
            ));
        }
        Err(e) => return Err(Diag::new(of.span, e)),
    };
    if schema.relation(rel).volatile {
        return Err(Diag::new(
            of.span,
            "a feed must not name a volatile relation",
        ));
    }
    let ctors = match &of.sel {
        None => (0..schema.ty(schema.relation(rel).elem_def).ctors().len() as u32).collect(),
        Some(sel) => schema
            .relation_sel(rel, sel)
            .map_err(|e| Diag::new(of.span, e))?,
    };
    Ok((rel, ctors))
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

    fn aggregate_decl(&mut self, def: &crate::schema::AggregateDef) -> R<AggregateTerms> {
        let a = &def.decl;
        let [(_, input)] = def.params.as_slice() else {
            return Err(diag(
                a.span,
                "an aggregate takes exactly one parameter; pass several values as one record",
            ));
        };
        let state = &def.state;
        let init = self.check(&a.init, state)?.tm;
        let (s, x, e) = &a.step;
        let step = self.lam_over(&[(s, state), (x, input)], e, state)?;
        let (l, r, e) = &a.merge;
        let merge = self.lam_over(&[(l, state), (r, state)], e, state)?;
        let (f, e) = &a.finish;
        let finish = self.lam_over(&[(f, state)], e, &def.result)?;
        Ok(AggregateTerms {
            init,
            step,
            merge,
            finish,
        })
    }

    /// Checks `body` against `ty` with `binds` in scope; returns it as a `Lam` over them.
    fn lam_over(&mut self, binds: &[(&ast::Ident, &Ty)], body: &ast::Expr, ty: &Ty) -> R<Tm> {
        self.scope.push();
        let vars: Vec<Var> = binds
            .iter()
            .map(|(x, t)| self.bind(&x.text, (*t).clone(), None))
            .collect();
        let out = self.check(body, ty);
        self.scope.pop();
        Ok(self.lams(vars, out?.tm, ty.clone()))
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
        let fields = self.fields_of(elem);
        for (name, fty) in &fields {
            let v = self.bind(name, fty.clone(), None);
            if let Ty::Data { def, ctors, .. } = elem {
                self.row_fields
                    .insert(v, (*def, ctors.clone(), name.clone()));
            }
            let xt = self.var_tm(x);
            let ft = self.add(Kind::Field(xt, name.clone()), fty.clone());
            lets.push((v, ft));
        }
        for c in self.schema.relation(rel).computed.clone() {
            if fields.iter().any(|(n, _)| *n == c.name) {
                continue;
            }
            let xt = self.var_tm(x);
            let row = Out {
                tm: xt,
                ty: elem.clone(),
                rows: Some(rel),
            };
            let name = ast::Ident {
                text: c.name.clone(),
                span: Span::default(),
            };
            let o = match self.field_of(row, &name) {
                Ok(o) => o,
                Err(e) => {
                    self.scope.pop();
                    return Err(e);
                }
            };
            let v = self.bind(&c.name, o.ty.clone(), None);
            lets.push((v, o.tm));
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

    fn index_field(&mut self, rel: RelId, ctors: Option<CtorSet>, e: &ast::Expr) -> R<IndexField> {
        let source = self.rel_out(rel, ctors);
        let elem = self.elem_ty(&source.ty).expect("a relation is a set");
        let segs = ctor_path(e)
            .filter(|(root, _)| self.fields_of(&elem).iter().any(|(n, _)| *n == root.text));
        let Some((root, segs)) = segs else {
            let (x, out) = self.in_row(&elem, rel, &mut |c| c.synth(e))?;
            let value = self.lams(vec![x], out.tm, out.ty);
            return Ok(IndexField {
                value,
                path: Vec::new(),
            });
        };
        let mut path = Vec::new();
        let x = self.fresh(elem.clone());
        let xt = self.var_tm(x);
        let row = Out {
            tm: xt,
            ty: elem,
            rows: None,
        };
        let out = self.along(row, &[Seg::Field(root)], &mut path)?;
        let out = self.along(out, &segs, &mut path)?;
        let value = self.lams(vec![x], out.tm, out.ty);
        Ok(IndexField { value, path })
    }

    /// Follows `segs` from `o`, recording each step in `path`; a constructor step is a `case` whose other arms are unreachable.
    fn along(&mut self, o: Out, segs: &[Seg], path: &mut Vec<PathStep>) -> R<Out> {
        let Some((seg, rest)) = segs.split_first() else {
            return Ok(o);
        };
        match seg {
            Seg::Field(f) => {
                let fty = self
                    .fields_of(&o.ty)
                    .into_iter()
                    .find(|(n, _)| *n == f.text)
                    .map(|(_, t)| t)
                    .ok_or_else(|| {
                        diag(
                            f.span,
                            format!("{} has no field `{}`", self.show(&o.ty), f.text),
                        )
                    })?;
                if let Ty::Data { def, ctors, .. } = &o.ty {
                    self.note_field(f.span, *def, ctors.as_deref(), &f.text, false);
                }
                path.push(PathStep::Field(f.text.clone()));
                let next = self.out(Kind::Field(o.tm, f.text.clone()), fty);
                self.along(next, rest, path)
            }
            Seg::Ctor(c) => {
                let sig = ty::ctor_names(self.schema, &o.ty)
                    .filter(|_| matches!(o.ty, Ty::Data { .. } | Ty::Sum { .. }))
                    .ok_or_else(|| {
                        diag(
                            c.span,
                            format!("{} has no constructors to select", self.show(&o.ty)),
                        )
                    })?;
                let Some(&(want, _)) = sig.iter().find(|(_, n)| *n == c.text) else {
                    return Err(diag(
                        c.span,
                        format!("`{}` is not a constructor of {}", c.text, self.show(&o.ty)),
                    ));
                };
                if let Ty::Data { def, .. } = &o.ty {
                    self.note(c.span, Ref::Ctor(*def, want));
                }
                path.push(PathStep::Ctor(want));
                let row = ty::restrict(self.schema, &o.ty, vec![want]);
                let y = self.fresh(row.clone());
                let yt = self.var_tm(y);
                let body = self.along(
                    Out {
                        tm: yt,
                        ty: row,
                        rows: None,
                    },
                    rest,
                    path,
                )?;
                let mut arms = Vec::with_capacity(sig.len());
                for (d, _) in sig {
                    if d == want {
                        arms.push((d, y, body.tm));
                    } else {
                        let z = self.fresh(ty::restrict(self.schema, &o.ty, vec![d]));
                        let none = self.add(Kind::Unreachable, body.ty.clone());
                        arms.push((d, z, none));
                    }
                }
                Ok(self.out(
                    Kind::Case {
                        scrutinee: o.tm,
                        arms,
                    },
                    body.ty,
                ))
            }
        }
    }
}

enum Seg<'e> {
    Field(&'e ast::Ident),
    Ctor(&'e ast::Ident),
}

/// Returns the root field and the steps after it of `e` when it is a path of fields that selects a constructor.
fn ctor_path(e: &ast::Expr) -> Option<(&ast::Ident, Vec<Seg<'_>>)> {
    let mut segs = Vec::new();
    let mut at = e;
    let root = loop {
        match &at.kind {
            ast::ExprKind::Field(x, f) => {
                segs.push(Seg::Field(f));
                at = x;
            }
            ast::ExprKind::Select(x, ast::CtorSel::One(c)) => {
                segs.push(Seg::Ctor(c));
                at = x;
            }
            ast::ExprKind::Name(q) if q.module.is_none() => break &q.name,
            _ => return None,
        }
    };
    segs.reverse();
    segs.iter()
        .any(|s| matches!(s, Seg::Ctor(_)))
        .then_some((root, segs))
}
