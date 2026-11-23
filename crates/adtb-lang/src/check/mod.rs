//! Type check programs, ad hoc commands and transactions, elaborating them to kernel terms.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::ast;
use crate::capability::RootKind;
use crate::kernel::{Build, Frees, IndexField, Kind, PathStep, Terms, Tm, Var};
use crate::resolve::{Scope, Visible};
use crate::schema::prelude::ctor;
use crate::schema::{
    Committed, DefaultValue, FN_RELATION, FN_TYPE_ARGUMENT, FnTypes, PRELUDE, Schema, TypeDefKind,
    TypeMode, TypeName, TypeScope, ValueDef,
};
use crate::span::{Diag, DiagKind, FileDiag, FileId, Span};
use crate::types::*;

mod adhoc;
mod bodies;
mod body;
mod call;
mod compiled;
mod expr;
mod names;
mod order;
mod pat;
mod placed;
use placed::SlotBuild;
mod prelude;
pub(crate) mod prim_ty;
mod query;
mod record;
pub(crate) mod rel;
mod row_body;
mod rows;
pub(crate) mod sums;
mod txn;
pub(crate) mod ty;
mod write;

#[cfg(test)]
pub(crate) mod tests;

use bodies::{Bodies, Demand, Demanded, bodies};
use rows::RowOrigin;

pub use adhoc::{AdhocCommand, Bundles, Command, Explained, RestorePoint};
pub use body::ProgramBody;
pub use compiled::{AsCommitted, AsSchema, Compiled};
pub use order::{declaration_order, schema_migration};
pub use row_body::{RowBody, check_row, row_query};
pub use txn::{Txn, check_txn, check_txn_in};
use ty::admits_late_key;
pub use ty::{join, subtype};

type R<T> = Result<T, Diag>;

/// A checked expression: its kernel term and type, and the relation whose rows it holds.
#[derive(Clone, Debug)]
pub struct Out {
    pub tm: Tm,
    pub ty: Ty,
    /// For a row, the relation it is a row of; for a set, the relation its elements are rows of.
    pub rows: Option<RelId>,
    provenance: Provenance,
}

/// Where a value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provenance {
    /// Rows read from the statement's snapshot.
    Snapshot,
    /// Rows a write proposes, or a value built from them.
    Proposed,
    /// A reference field's value read from a row of the snapshot, which names a row.
    Reference,
    /// Any other value.
    Built,
}

impl Provenance {
    /// Returns the provenance of a value that is either `self` or `other`.
    fn join(self, other: Self) -> Self {
        match (self, other) {
            (a, b) if a == b => a,
            (Self::Proposed, _) | (_, Self::Proposed) => Self::Proposed,
            _ => Self::Built,
        }
    }
}

impl Out {
    fn of(tm: Tm, ty: Ty) -> Out {
        Out {
            tm,
            ty,
            rows: None,
            provenance: Provenance::Built,
        }
    }
}

/// The kernel terms of every body a schema declares.
#[derive(Debug)]
pub struct Program {
    pub terms: Terms,
    /// Each function's term, a `Lam` over its parameters; `None` for Wasm and host functions.
    pub functions: Vec<Option<Tm>>,
    constraints: BTreeMap<ConstraintIdx, Tm>,
    checks: BTreeMap<CheckIdx, Tm>,
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

impl Program {
    /// Returns each user-defined aggregate's init, step, merge and finish bodies; `None` for one
    /// that did not check.
    pub fn aggregate_parts(&self) -> impl Iterator<Item = Option<[Tm; 4]>> + '_ {
        self.aggregates
            .iter()
            .map(|a| a.as_ref().map(|a| [a.init, a.step, a.merge, a.finish]))
    }

    /// Returns the term of constraint `c`, a denial query; `None` when it did not check.
    pub fn constraint(&self, c: ConstraintIdx) -> Option<Tm> {
        self.constraints.get(&c).copied()
    }

    /// Returns the term of row check `c`, a denial query; `None` when it did not check.
    pub fn check(&self, c: CheckIdx) -> Option<Tm> {
        self.checks.get(&c).copied()
    }

    pub fn constraints(&self) -> impl Iterator<Item = (ConstraintIdx, Tm)> + '_ {
        self.constraints.iter().map(|(&c, &t)| (c, t))
    }

    pub fn checks(&self) -> impl Iterator<Item = (CheckIdx, Tm)> + '_ {
        self.checks.iter().map(|(&c, &t)| (c, t))
    }
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
    /// A name of a type, relation variable, constructor or named set the schema does not declare.
    Unresolved,
}

/// What a body may refer to besides its locals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    /// A body a schema declares or a bundle registers.
    Schema,
    /// Ad hoc text: `_` covers a sum's remaining constructors.
    Adhoc,
}

#[derive(Clone, Debug)]
struct Local {
    var: Var,
    ty: Ty,
    rows: Option<RelId>,
    provenance: Provenance,
    group: Option<Rc<GroupInfo>>,
}

/// What a group binder `g` stands for after `into`.
#[derive(Debug)]
struct GroupInfo {
    /// The function from a row of the group to its grouped value.
    value: Var,
    value_ty: Ty,
    value_rows: Option<RelId>,
    value_provenance: Provenance,
    rows_ty: Ty,
}

pub struct Checker<'a> {
    pub schema: &'a Schema,
    /// The `fix` expressions around the expression being checked.
    fixes: u32,
    /// The nodes of the type of every term added.
    type_nodes: usize,
    pub terms: Terms,
    /// The free variables of terms of `terms`.
    frees: Frees,
    vis: Visible,
    ctx: Context,
    scope: Scope<Local>,
    /// Type parameters of the function being checked, with their bounds.
    tparams: Vec<(String, Vec<Bound>)>,
    /// Relations whose rows each checked function returns, for `key` over a view's rows.
    fn_rows: Vec<Option<RelId>>,
    /// The functions and aggregates of the schema being checked whose checks began.
    demand: Option<Rc<Demand>>,
    /// Ad hoc parameters, typed by their descriptors.
    params: BTreeMap<String, (Var, Ty)>,
    /// Whether `reject` may appear: in the source query of an insert.
    reject_ok: bool,
    /// Per enclosing write, the rows its nested `key (insert ...)` insert.
    hoist: Vec<Vec<write::Nested>>,
    /// Whether a nested `key (insert ...)` here is evaluated once per statement.
    once: bool,
    empty_row_terms: BTreeSet<Tm>,
    empty_row_vars: BTreeSet<Var>,
    read_only: bool,
    /// The names this checker resolves.
    refs: names::Refs,
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
    bundles: &'a Bundles,
    /// The writer, `script`, `fill` or `routing`, that refuses a value given or matched where a key type
    /// with a late selection is expected.
    late_writer: Option<&'static str>,
    /// The `at` bodies around the expression being checked.
    at_bodies: u32,
    sums: sums::Sums,
    /// The arguments of the calls being solved already elaborated, each taken by its check.
    solved_parts: std::collections::HashMap<*const ast::Expr, Out>,
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
    /// Returns a checker of ad hoc text against `schema`.
    pub fn of_adhoc(schema: &'a Schema, vis: Visible) -> Self {
        Checker::with_rows(
            schema,
            vis,
            Context::Adhoc,
            vec![None; schema.functions.len()],
        )
    }

    /// Returns a checker that knows `fn_rows`, the relation whose rows each function returns.
    fn with_rows(
        schema: &'a Schema,
        vis: Visible,
        ctx: Context,
        fn_rows: Vec<Option<RelId>>,
    ) -> Self {
        Checker {
            fixes: 0,
            type_nodes: 0,
            schema,
            terms: Terms::default(),
            frees: Frees::default(),
            vis,
            ctx,
            scope: Scope::default(),
            tparams: Vec::new(),
            fn_rows,
            demand: None,
            params: BTreeMap::new(),
            reject_ok: false,
            hoist: Vec::new(),
            once: true,
            empty_row_terms: BTreeSet::new(),
            empty_row_vars: BTreeSet::new(),
            read_only: false,
            refs: names::Refs::default(),
            coverage: true,
            carry: None,
            row_fields: BTreeMap::new(),
            hidden: BTreeSet::new(),
            program: None,
            bundles: &adhoc::NO_BUNDLES,
            late_writer: None,
            at_bodies: 0,
            sums: sums::Sums::default(),
            solved_parts: std::collections::HashMap::new(),
        }
    }

    /// Returns a checker of a body against `c`.
    fn of_program(
        c: &'a Compiled<impl AsSchema>,
        vis: Visible,
        ctx: Context,
        bundles: &'a Bundles,
        late_writer: Option<&'static str>,
        hidden: BTreeSet<RelId>,
    ) -> Self {
        let program = c.program();
        let mut c = Checker::with_rows(c.schema(), vis, ctx, program.fn_rows.clone());
        c.program = Some(program);
        c.bundles = bundles;
        c.late_writer = late_writer;
        c.hidden = hidden;
        c
    }

    /// Returns a checker of a registered query or transaction against `c`.
    fn registered(c: &'a Compiled<impl AsSchema>, vis: Visible) -> Self {
        Checker::of_program(
            c,
            vis,
            Context::Schema,
            &adhoc::NO_BUNDLES,
            None,
            BTreeSet::new(),
        )
    }

    fn module(&self) -> ModuleId {
        self.vis.first()
    }

    fn adhoc(&self) -> bool {
        self.ctx == Context::Adhoc
    }

    fn b(&mut self) -> SlotBuild<'_> {
        SlotBuild::new(
            Build::new(self.schema, &mut self.terms)
                .counting(&mut self.type_nodes)
                .classes(self.sums.parents()),
        )
    }

    /// Returns `tm` at the type of its node.
    fn out(&self, tm: Tm) -> Out {
        Out::of(tm, self.terms.ty(tm).clone())
    }

    /// Returns `p` over `args` at the type the primitive's signature gives, `target` naming the
    /// result where its operands do not.
    fn prim(
        &mut self,
        p: crate::kernel::Prim,
        args: Vec<Tm>,
        target: Option<Ty>,
        span: Span,
    ) -> R<Out> {
        let bounds = prim_ty::Bounds::Declared(&self.tparams);
        let ty = prim_ty::prim_ty(self.schema, &self.terms, bounds, p, &args, target.as_ref())
            .map_err(|m| diag(span, m))?;
        let tm = self.b().prim(p, args, ty);
        Ok(self.out(tm))
    }

    fn constant(&mut self, c: crate::kernel::Const, ty: Ty) -> Out {
        let tm = self.b().constant(c, ty);
        self.out(tm)
    }

    fn show(&self, ty: &Ty) -> String {
        let names: Vec<String> = self.tparams.iter().map(|(n, _)| n.clone()).collect();
        self.schema.show_ty_with(ty, &names)
    }

    /// Bind `name` in the innermost scope. Returns its variable.
    fn bind(&mut self, name: &str, ty: Ty, rows: Option<RelId>, provenance: Provenance) -> Var {
        let var = self.terms.var(name, ty.clone());
        self.scope.bind(
            name,
            Local {
                var,
                ty,
                rows,
                provenance,
                group: None,
            },
        );
        var
    }

    fn fresh(&mut self, ty: Ty) -> Var {
        self.terms.var("ξ", ty)
    }

    fn var_tm(&mut self, v: Var) -> Tm {
        let tm = self.b().var(v);
        if self.empty_row_vars.contains(&v) {
            self.empty_row_terms.insert(tm);
        }
        tm
    }

    fn bool_ty(&self) -> Ty {
        Ty::Data {
            def: self.schema.prelude().bool(),
            args: Vec::new(),
            ctors: None,
        }
    }

    fn bool_tm(&mut self, value: bool) -> Tm {
        let ty = self.bool_ty();
        self.b().ctor(ctor::bool(value), Vec::new(), ty)
    }

    fn option_ty(&self, t: Ty) -> Ty {
        Ty::Data {
            def: self.schema.prelude().option(),
            args: vec![t],
            ctors: None,
        }
    }

    /// Returns whether more than one visible module declares a value named `name`.
    fn ambiguous(&self, name: &str) -> bool {
        let declaring = self.vis.modules.iter();
        declaring
            .filter(|&&m| self.vis.value_in(self.schema, m, name).is_some())
            .count()
            > 1
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
            for (_, t) in self.vis.types_of(self.schema, m) {
                if t != self.schema.prelude().principal() {
                    push(t);
                }
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

    /// Returns whether `ty` is ordered, refused at `span` when the answer needs a relation a later
    /// operator of the migration adds.
    pub fn ordered(&self, ty: &Ty, span: Span) -> R<bool> {
        self.schema
            .is_ordered_with(ty, &|i| self.param_has(i, Bound::Ord))
            .map_err(|e| diag(span, e))
    }
}

/// Type checks every body of `schema` and elaborates it to the kernel, returning the two together.
///
/// Fail with no diagnostic of a body that names a relation variable a refused operator left
/// poisoned.
pub fn check_schema<S: AsSchema>(schema: S) -> Result<Compiled<S>, Vec<FileDiag>> {
    let (program, mut diags) = bodies(schema.as_schema(), None);
    let poisoned = diags.iter().any(|d| d.diag.kind == DiagKind::Poisoned);
    diags.retain(|d| d.diag.kind != DiagKind::Poisoned);
    if !diags.is_empty() || poisoned {
        return Err(diags);
    }
    let c = Compiled::new(schema, program);
    let diags = crate::analysis::schema_diags(&c);
    if !diags.is_empty() {
        return Err(diags);
    }
    let (schema, program) = (c.schema(), c.program());
    for (root, t, free) in program.bodies() {
        crate::kernel::assert_valid_bounded(
            schema,
            &program.terms,
            &[(t, free)],
            root.bounds(schema),
        );
    }
    Ok(c)
}

/// Returns every schema object the bodies of a schema name, by file and span, with each `match` the bodies hold.
///
/// A body that does not check contributes the names resolved before its first error; a match that misses
/// constructors is not an error here.
pub fn schema_refs(schema: &Schema) -> Vec<(FileId, Span, Ref)> {
    let mut refs = Vec::new();
    bodies(schema, Some(&mut refs));
    refs
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
    pub temps: Option<Committed>,
    kind: RootKind,
}

impl Adhoc {
    /// Returns the kind of the text: ad hoc text or a migration script.
    pub fn kind(&self) -> RootKind {
        self.kind
    }

    /// Returns each item and each term of a command, its parameters free in it.
    pub fn roots(&self) -> Vec<(Tm, Vec<Var>)> {
        let free: Vec<Var> = self.params.values().copied().collect();
        self.items
            .iter()
            .copied()
            .chain(self.commands.iter().flat_map(|c| c.command.terms()))
            .map(|t| (t, free.clone()))
            .collect()
    }

    /// Returns the schema the terms name declarations of, `given` being the one the text was checked against.
    pub fn schema<'a>(&'a self, given: &'a Committed) -> &'a Committed {
        self.temps.as_ref().unwrap_or(given)
    }
}

/// The module that holds the temp relations of ad hoc text; no module can take its name.
pub const TEMP_MODULE: &str = "temp";

/// Type checks ad hoc text, its free names typed by `params`, against `program`, the checked `schema`,
/// knowing no bundle.
pub fn check_adhoc(
    c: &Compiled<impl AsCommitted>,
    items: &[ast::AdhocItem],
    params: &BTreeMap<String, Ty>,
) -> Result<Adhoc, Vec<Diag>> {
    check_adhoc_recording(c, items, params, &Bundles::default(), None)
}

/// Type checks ad hoc text as [`check_adhoc`] does, its `explain` of a registered call against
/// `bundles`. With `record`, appends the schema objects the text names, as [`schema_refs`] reports
/// them, those of items before a refused one included.
pub fn check_adhoc_recording(
    c: &Compiled<impl AsCommitted>,
    items: &[ast::AdhocItem],
    params: &BTreeMap<String, Ty>,
    bundles: &Bundles,
    record: Option<&mut Vec<(Span, Ref)>>,
) -> Result<Adhoc, Vec<Diag>> {
    check_adhoc_in(c, None, items, params, bundles, record)
}

/// Type checks ad hoc text as [`check_adhoc_recording`] does, its names finding only what `scope`
/// holds when given.
pub fn check_adhoc_in(
    c: &Compiled<impl AsCommitted>,
    scope: Option<&crate::schema::Scope>,
    items: &[ast::AdhocItem],
    params: &BTreeMap<String, Ty>,
    bundles: &Bundles,
    record: Option<&mut Vec<(Span, Ref)>>,
) -> Result<Adhoc, Vec<Diag>> {
    let schema = c.committed();
    let mut diags = Vec::new();
    let vis = Visible::all(schema).scoped(scope.cloned());
    let temps = adhoc::temp_relations(schema, &vis, items, &mut diags);
    let (checked, declared) = match &temps {
        Some((s, d)) => (c.sharing(s), d.clone()),
        None => (c.sharing(schema), Vec::new()),
    };
    let text = Text {
        scope,
        items,
        params,
        bundles,
        late_writer: None,
        kind: RootKind::Adhoc,
    };
    let mut adhoc = checked_items(&checked, &text, declared, diags, record)?;
    adhoc.temps = temps.map(|(s, _)| s);
    Ok(adhoc)
}

/// Type checks the write of a migration's `script` against `c`, the schema the operators before
/// it produce, as [`check_adhoc_recording`] does.
pub(crate) fn check_script(
    c: &Compiled<impl AsSchema>,
    write: &ast::Write,
    record: Option<&mut Vec<(Span, Ref)>>,
) -> Result<(), Vec<Diag>> {
    let text = Text {
        scope: None,
        items: &[ast::AdhocItem::Write(Box::new(write.clone()))],
        params: &BTreeMap::new(),
        bundles: &Bundles::default(),
        late_writer: Some("script"),
        kind: RootKind::Script,
    };
    checked_items(c, &text, Vec::new(), Vec::new(), record).map(drop)
}

/// Ad hoc text and what it is checked with.
struct Text<'a> {
    scope: Option<&'a crate::schema::Scope>,
    items: &'a [ast::AdhocItem],
    params: &'a BTreeMap<String, Ty>,
    bundles: &'a Bundles,
    late_writer: Option<&'static str>,
    kind: RootKind,
}

/// Returns `text` checked against `c`, each relation of `declared` hidden until its
/// declaration, failing with `diags` and every diagnostic of its items.
fn checked_items(
    c: &Compiled<impl AsSchema>,
    text: &Text<'_>,
    declared: Vec<Option<RelId>>,
    mut diags: Vec<Diag>,
    record: Option<&mut Vec<(Span, Ref)>>,
) -> Result<Adhoc, Vec<Diag>> {
    let Text {
        scope,
        items,
        params,
        bundles,
        late_writer,
        kind,
    } = *text;
    let compiled = c;
    let checked = compiled.schema();
    let vis = Visible::all(checked).scoped(scope.cloned());
    let hidden = declared.iter().flatten().copied().collect();
    let mut c = Checker::of_program(compiled, vis, Context::Adhoc, bundles, late_writer, hidden);
    for (name, ty) in params {
        let v = c.terms.var(name, ty.clone());
        c.params.insert(name.clone(), (v, ty.clone()));
    }
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
    if let Some(out) = record {
        out.extend(c.refs.take_checked());
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    let adhoc = Adhoc {
        params: c.params.iter().map(|(n, (v, _))| (n.clone(), *v)).collect(),
        terms: c.terms,
        items: out,
        commands,
        temps: None,
        kind,
    };
    let spans: Vec<Span> = items
        .iter()
        .filter_map(|i| match i {
            ast::AdhocItem::Expr(e) => Some(e.span),
            ast::AdhocItem::Write(w) => Some(w.span),
            ast::AdhocItem::Transact(b) => Some(b.span),
            _ => None,
        })
        .collect();
    let diags = crate::analysis::adhoc_diags(compiled, &adhoc, &spans);
    if !diags.is_empty() {
        return Err(diags);
    }
    crate::kernel::assert_valid(checked, &adhoc.terms, &adhoc.roots());
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
    c: &Compiled<impl AsSchema>,
    module: ModuleId,
    q: &ast::QueryDecl,
) -> Result<Query, Diag> {
    check_query_in(c, Visible::module(module), q)
}

/// Type checks a registered query of a bundle that reads the modules of `vis`, as [`check_query`] does.
pub fn check_query_in(
    c: &Compiled<impl AsSchema>,
    vis: Visible,
    q: &ast::QueryDecl,
) -> Result<Query, Diag> {
    let compiled = c;
    let schema = compiled.schema();
    let mut c = Checker::registered(compiled, vis);
    let term = c.query(q)?;
    let query = Query {
        terms: c.terms,
        term,
    };
    crate::analysis::query_diag(compiled, &query, q.span)?;
    crate::kernel::assert_valid(schema, &query.terms, &[(query.term, Vec::new())]);
    Ok(query)
}

/// Returns the relation variable a feed of a bundle of `module` names and the constructors it streams.
///
/// Fail when the relation does not resolve, a selected name is no constructor or named set of it,
/// or the relation is volatile.
pub fn check_feed(
    schema: &Schema,
    module: ModuleId,
    f: &ast::FeedDecl,
) -> Result<(RelId, CtorSet), Diag> {
    check_feed_in(schema, Visible::module(module), f)
}

/// Returns the relation variable a feed of a bundle that reads the modules of `vis` names, as
/// [`check_feed`] does.
pub fn check_feed_in(
    schema: &Schema,
    vis: Visible,
    f: &ast::FeedDecl,
) -> Result<(RelId, CtorSet), Diag> {
    let of = &f.of;
    let rel =
        crate::resolve::lookup_relation(schema, &vis, &of.relation).map_err(|e| e.at(of.span))?;
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
        let result = self.resolve_type(
            &q.result,
            FnTypes::Refused("a registered query never returns a function"),
        )?;
        self.scope.push();
        let mut vars = Vec::new();
        for p in &q.params {
            let pty = self.resolve_type(
                &p.ty,
                FnTypes::Refused("a parameter of a registered query never has a function type"),
            )?;
            vars.push(self.bind(&p.name.text, pty, None, Provenance::Built));
        }
        let body = self.top_query(&q.body, Some(&result));
        self.scope.pop();
        let body = body?;
        Ok(self.lams(vars, body.tm, result))
    }

    /// Wrap `body`, of type `result`, in one `Lam` per variable of `vars`, or one over unit when there are none.
    fn lams(&mut self, mut vars: Vec<Var>, body: Tm, result: Ty) -> Tm {
        if vars.is_empty() {
            vars.push(self.terms.var("ξ", Ty::Unit));
        }
        let mut tm = body;
        let mut ty = result;
        for v in vars.into_iter().rev() {
            let pty = self.terms.var_info(v).ty.clone();
            ty = Ty::Fn(Box::new(pty), Box::new(ty));
            tm = self.b().lam(v, tm, ty.clone());
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
            vars.push(self.bind(name, ty.clone(), None, Provenance::Built));
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

    /// Check `body` against `ty` with `binds` in scope. Returns it as a `Lam` over them.
    fn lam_over(&mut self, binds: &[(&ast::Ident, &Ty)], body: &ast::Expr, ty: &Ty) -> R<Tm> {
        self.scope.push();
        let vars: Vec<Var> = binds
            .iter()
            .map(|(x, t)| self.bind(&x.text, (*t).clone(), None, Provenance::Built))
            .collect();
        let out = self.check(body, ty);
        self.scope.pop();
        Ok(self.lams(vars, out?.tm, ty.clone()))
    }

    /// Check a top-level query of ad hoc text, where `order by` is allowed.
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

    /// Check `f` with `self` and each field of a row of `elem` bound by name, wrapping its term in those bindings.
    fn in_row(
        &mut self,
        elem: &Ty,
        rel: RelId,
        f: &mut dyn FnMut(&mut Self) -> R<Out>,
    ) -> R<(Var, Out)> {
        let fields = self.fields_of(elem);
        self.in_row_with(elem, fields, rel, f)
    }

    /// Check `f` as [`Self::in_row`] does, binding `fields` in place of the fields of `elem`.
    fn in_row_with(
        &mut self,
        elem: &Ty,
        fields: Vec<(String, Ty)>,
        rel: RelId,
        f: &mut dyn FnMut(&mut Self) -> R<Out>,
    ) -> R<(Var, Out)> {
        self.scope.push();
        let x = self.bind("self", elem.clone(), Some(rel), Provenance::Snapshot);
        let mut lets = Vec::new();
        for (name, fty) in &fields {
            let v = self.bind(name, fty.clone(), None, Provenance::Built);
            if let Ty::Data { def, ctors, .. } = elem {
                self.row_fields
                    .insert(v, (*def, ctors.clone(), name.clone()));
            }
            let xt = self.var_tm(x);
            let ft = self.b().field(xt, name.clone());
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
                provenance: Provenance::Snapshot,
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
            let v = self.bind(&c.name, o.ty.clone(), None, Provenance::Built);
            lets.push((v, o.tm));
        }
        let r = f(self);
        self.scope.pop();
        let mut out = r?;
        for (v, ft) in lets.into_iter().rev() {
            if self.frees.of(&self.terms, out.tm).contains(&v) {
                out.tm = self.b().let_(v, ft, out.tm, out.ty.clone());
            }
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
        let empty = self.b().empty_set(set_ty.clone());
        let xt = self.var_tm(x);
        let one = self.singleton_checked(xt, set_ty.clone());
        let body = self.b().if_(pred.tm, empty, one, set_ty.clone());
        Ok(self.b().for_(x, source.tm, body, set_ty))
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
            Some(c) => def.field_named(c, field),
            None => (0..def.ctors().len() as u32).find_map(|c| def.field_named(c, field)),
        }
        .map(|f| f.ty.subst(r.elem_args()))
        .ok_or_else(|| diag(e.span, format!("`{}` has no field `{field}`", def.name)))?;
        Ok(self.check(e, &fty)?.tm)
    }

    /// Returns the term of index element `e` of `rel` and its type.
    fn index_field(
        &mut self,
        rel: RelId,
        ctors: Option<CtorSet>,
        e: &ast::Expr,
    ) -> R<(IndexField, Ty)> {
        let source = self.rel_out(rel, ctors);
        let elem = self.elem_ty(&source.ty).expect("a relation is a set");
        self.declared_by_every_member(&elem, e)?;
        let fields = ty::held_fields(self.schema, &elem);
        let segs = ctor_path(e).filter(|(root, _)| fields.iter().any(|(n, _)| *n == root.text));
        let Some((root, segs)) = segs else {
            let (x, out) = self.in_row_with(&elem, fields, rel, &mut |c| c.synth(e))?;
            let value = self.lams(vec![x], out.tm, out.ty.clone());
            let field = IndexField {
                value,
                path: Vec::new(),
            };
            return Ok((field, out.ty));
        };
        let mut path = Vec::new();
        let x = self.fresh(elem.clone());
        let xt = self.var_tm(x);
        let row = Out {
            tm: xt,
            ty: elem,
            rows: None,
            provenance: Provenance::Built,
        };
        let out = self.along(row, &[Seg::Field(root)], &mut path)?;
        let out = self.along(out, &segs, &mut path)?;
        let value = self.lams(vec![x], out.tm, out.ty.clone());
        Ok((IndexField { value, path }, out.ty))
    }

    /// Refuse index element `e` when its root names a field some constructor of `elem` declares
    /// and another lacks or declares at another type than the first, naming that constructor.
    fn declared_by_every_member(&self, elem: &Ty, e: &ast::Expr) -> R<()> {
        let (Some(root), Ty::Data { def, args, ctors }) = (path_root(e), elem) else {
            return Ok(());
        };
        if ty::held_fields(self.schema, elem)
            .iter()
            .any(|(n, _)| *n == root.text)
        {
            return Ok(());
        }
        let t = self.schema.ty(*def);
        let set = ctors
            .clone()
            .unwrap_or_else(|| (0..t.ctors().len() as u32).collect());
        let declared: Vec<(u32, Option<Ty>)> = set
            .iter()
            .map(|&c| (c, t.field_named(c, &root.text).map(|f| f.ty.subst(args))))
            .collect();
        let Some(first) = declared.iter().find_map(|(_, ty)| ty.clone()) else {
            return Ok(());
        };
        let Some((c, ty)) = declared
            .iter()
            .find(|(_, ty)| !ty.as_ref().is_some_and(|ty| ty.alike(&first)))
        else {
            return Ok(());
        };
        let (ctor, u) = (&t.ctors()[*c as usize].name, &root.text);
        let why = match ty {
            Some(ty) => format!(
                "member `{ctor}` declares `{u}` as `{}`, not `{}`",
                self.show(ty),
                self.show(&first)
            ),
            None => format!("member `{ctor}` of `{}` has no field `{u}`", t.name),
        };
        Err(diag(root.span, why))
    }

    /// Refuse element `k` of index `ix` of `rel`, of type `ty`, when the type has no key form, or
    /// is a set and `ix` is a unique or interval index.
    fn keyable(&self, rel: RelId, ix: &crate::schema::IndexDef, k: usize, ty: &Ty) -> R<()> {
        let why = match ty.key_form() {
            Some(KeyForm::Value) => return Ok(()),
            Some(KeyForm::Entries) if !ix.unique && !ix.interval => return Ok(()),
            Some(KeyForm::Entries) => "a unique or interval index element is not a set",
            None => "its type contains a set or function type",
        };
        Err(diag(
            ix.fields[k].span,
            format!(
                "index element `{}` of `{}` has type {}, which has no key form: {why}",
                ix.text(k),
                self.schema.relation(rel).name,
                self.show(ty)
            ),
        ))
    }

    /// Follow `segs` from `o`, recording each step in `path`; a constructor step is a `case` whose other arms are unreachable.
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
                    self.field_named(f.span, *def, ctors.as_deref(), &f.text, false);
                }
                path.push(PathStep::Field(f.text.clone()));
                let next = Out::of(self.b().field(o.tm, f.text.clone()), fty);
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
                    self.declared_ctor(c.span, *def, &c.text);
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
                        provenance: Provenance::Built,
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
                        let none = self.b().unreachable(body.ty.clone());
                        arms.push((d, z, none));
                    }
                }
                let tm = self.b().case(o.tm, arms, body.ty.clone());
                Ok(Out::of(tm, body.ty))
            }
        }
    }
}

enum Seg<'e> {
    Field(&'e ast::Ident),
    Ctor(&'e ast::Ident),
}

/// Returns the root field and the steps after it of `e` when it is a path of fields that selects a constructor.
/// Returns the name a path of field and constructor steps starts from.
fn path_root(e: &ast::Expr) -> Option<&ast::Ident> {
    match &e.kind {
        ast::ExprKind::Field(x, _) | ast::ExprKind::Select(x, ast::CtorSel::One(_)) => path_root(x),
        ast::ExprKind::Name(q) if q.module.is_none() => Some(&q.name),
        _ => None,
    }
}

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
