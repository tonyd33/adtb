//! Apply and check schema migrations: state, renames, row maps, grants and reverts.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use crate::ast::{
    self, CtorPlace, CtorSel, CtorTarget, Decl, Expr, Fill, MigrationFile, Operator, OperatorKind,
    QName, RelationClause,
};
use crate::canon::Names;
use crate::check::{self, Program, Ref};
use crate::ids::{
    CatConstraintId, CatCtorId, CatFieldId, CatFunctionId, CatIndexId, CatRelId, CatSetId,
    CatTypeId, ObjectKey, held_id,
};
use crate::lex::{Tok, lex};
use crate::parse::{Options, parse_schema};
use crate::schema::{self, Committed, Schema, TypeDefKind, ValueDef};
use crate::span::{Diag, FileDiag, FileId, Span};
use crate::types::*;

mod carry;
mod ctor;
mod cx;
mod expected;
mod field;
mod grant;
mod grant_text;
mod ids;
mod keep;
#[cfg(test)]
pub(crate) mod meaning;
mod memo;
mod model;
mod names;
mod operands;
mod recorded;
mod resolved;
mod revert;
mod rowmap;
mod source;
mod text;

use field::path_text;
use text::*;

#[cfg(test)]
#[path = "../evolve_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../evolve_memo_tests.rs"]
mod memo_tests;

pub use crate::privilege::{Capability, Privilege};
pub use crate::schema::{Adder, Stage};
pub use expected::{Expected, Listed};
use grant::HeldGrant;
pub use grant::{
    CallSignatures, DeployedGrant, DeploymentGrant, DeploymentRevoke, GrantFields, GrantRow,
    GrantSel, GrantTarget, Granted, LogGrant, NoCalls, OWNER_ROLE, Stated, closes_option_cycle,
    deployment_grant, deployment_revoke, family_query, target_covers,
};
pub use keep::Kept;
pub use recorded::{Migrated, OpSteps, Recorded};
use resolved::Reached;
pub use resolved::{
    ArmTarget, Clause, Code, Function, IndexKind, Place, Resolved, ResolvedOp, Target,
};
use rowmap::Applied;
pub use rowmap::{RowFn, RowMap, Step};

/// A stage and the version whose migration set it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub stage: Stage,
    pub since: u64,
}

/// One declaration of a module, as source text without the module header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDecl {
    pub module: String,
    pub text: String,
}

/// A migration of the log, with the schema before it.
#[derive(Clone, Debug)]
pub struct Logged {
    pub number: u64,
    pub module: String,
    pub text: String,
    pub operators: Vec<Operator>,
    pub before: Arc<Source>,
}

/// Why a source gives no committed schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Uncommitted {
    /// The source does not build.
    Unbuilt(Vec<FileDiag>),
    /// A migration is being applied to the source.
    Migrating,
}

impl std::fmt::Display for Uncommitted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Uncommitted::Unbuilt(ds) => {
                let ms: Vec<&str> = ds.iter().map(|d| d.diag.message.as_str()).collect();
                f.write_str(&ms.join("\n"))
            }
            Uncommitted::Migrating => f.write_str("a migration is being applied to the source"),
        }
    }
}

/// A schema as the fold of its log: the declarations of every user module, the write states of
/// constructors, and the log itself.
///
/// Declaration `i` is file `FileId(i)` of [`Source::files`].
#[derive(Clone, Debug, Default)]
pub struct Source {
    pub decls: Vec<SourceDecl>,
    /// Every user module, those that hold nothing included.
    pub modules: BTreeSet<String>,
    /// The modules `install` created.
    pub installed: BTreeSet<String>,
    /// The declarations of each extension package `install` may read, by the file it names, as
    /// the text of a module file without its `module` line.
    pub packages: BTreeMap<String, String>,
    /// The privileges the log's `grant` and `revoke` operators leave each role holding, in the
    /// order they were first granted.
    pub grants: Vec<LogGrant>,
    /// The write state of each constructor relation, by the ids of the relation variable and the
    /// constructor.
    pub states: BTreeMap<(CatRelId, CatCtorId), State>,
    /// The write state of each value-level constructor, by its id.
    pub value_states: BTreeMap<CatCtorId, State>,
    pub version: u64,
    pub log: Vec<Logged>,
    /// The catalog id of each object of the user modules, by its key.
    pub ids: BTreeMap<ObjectKey, u64>,
    /// The next id of each space.
    pub next: BTreeMap<crate::ids::Space, u64>,
    /// The relation variables, by module and name, that the operators of the migration being
    /// applied add.
    pub ahead: Vec<Expected>,
    memo: memo::BuiltMemo,
    parsed: memo::ParsedMemo,
}

/// Text an operator writes into a declaration, with the ranges of the migration's text it copies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Written {
    text: String,
    /// Each copy's offset in `text`, and the range of the migration's text it copies.
    copies: Vec<(usize, Range<usize>)>,
}

impl Written {
    fn push(&mut self, s: &str) {
        self.text.push_str(s);
    }

    fn append(&mut self, w: Written) {
        let at = self.text.len();
        self.text.push_str(&w.text);
        self.copies
            .extend(w.copies.into_iter().map(|(o, r)| (at + o, r)));
    }

    /// Returns `parts` with `sep` between each two.
    fn joined(parts: impl IntoIterator<Item = Written>, sep: &str) -> Written {
        let mut out = Written::default();
        for (k, p) in parts.into_iter().enumerate() {
            if k > 0 {
                out.push(sep);
            }
            out.append(p);
        }
        out
    }

    /// Returns `pieces` concatenated.
    fn concat(pieces: impl IntoIterator<Item = Written>) -> Written {
        Written::joined(pieces, "")
    }
}

impl From<String> for Written {
    fn from(text: String) -> Written {
        Written {
            text,
            copies: Vec::new(),
        }
    }
}

impl From<&str> for Written {
    fn from(text: &str) -> Written {
        text.to_string().into()
    }
}

/// Why a migration is refused: the operators that do not apply, at their spans in the migration's text,
/// and the schema objects of the result that do not check, in the files of the result.
#[derive(Clone, Debug, Default)]
pub struct Refusal {
    pub operators: Vec<Diag>,
    pub recheck: Vec<FileDiag>,
    /// For each of `recheck`, in order, the span of the migration's text an operator copied the
    /// failing text from, when it copied it.
    pub recheck_from: Vec<Option<Span>>,
    /// The result, whose files `recheck` points into.
    pub result: Option<Box<Source>>,
    unstepped: Option<usize>,
}

impl Refusal {
    /// Returns the index of the operator whose steps over the rows fail, when one does.
    pub fn unstepped(&self) -> Option<usize> {
        self.unstepped
    }
}

/// Why a migration, or a migration of the log applied again, does not apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failed<R = Refusal> {
    Refused(R),
    /// The migration model fails on the migration: an internal error.
    Broken(Diag),
}

impl<R> Failed<R> {
    /// Returns this failure with `f` applied to its refusal.
    pub fn map_refused<S>(self, f: impl FnOnce(R) -> S) -> Failed<S> {
        match self {
            Failed::Refused(r) => Failed::Refused(f(r)),
            Failed::Broken(d) => Failed::Broken(d),
        }
    }
}

impl From<String> for Failed<String> {
    fn from(why: String) -> Failed<String> {
        Failed::Refused(why)
    }
}

impl From<Vec<Diag>> for Failed<Vec<Diag>> {
    fn from(ds: Vec<Diag>) -> Failed<Vec<Diag>> {
        Failed::Refused(ds)
    }
}

/// The result of applying a migration of the log again, or of what reads it.
pub type Replaying<T> = Result<T, Failed<Vec<Diag>>>;

fn header(module: &str) -> String {
    format!("module {module}\n")
}

fn key_of(d: &Decl) -> (&'static str, String) {
    match d {
        Decl::Type(t) => ("type", t.name.text.clone()),
        Decl::Relation(r) => ("relation", r.name.name.text.clone()),
        Decl::Set(s) => ("set", format!("{}.{}", s.relation.text, s.name.text)),
        Decl::Fn(f) => ("fn", f.name.text.clone()),
        Decl::Constraint(c) => ("constraint", c.name.text.clone()),
        Decl::Check(c) => ("check", c.name.text.clone()),
        Decl::Store(s) => ("store", s.view.name.text.clone()),
        Decl::Aggregate(a) => ("aggregate", a.name.text.clone()),
    }
}

/// The tokens of `text` with their byte ranges, the end of text left out.
fn tokens(text: &str) -> Vec<(Tok, Range<usize>)> {
    lex(text)
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| (t.tok, t.span.range()))
        .collect()
}

fn sel_idents(sel: &CtorSel) -> Vec<&ast::Ident> {
    match sel {
        CtorSel::One(c) => vec![c],
        CtorSel::Many(cs) => cs.iter().collect(),
    }
}

/// Call `f` on `t` and every type expression nested in it.
fn walk_type<'a>(t: &'a ast::Type, f: &mut dyn FnMut(&'a ast::Type)) {
    f(t);
    match &t.kind {
        ast::TypeKind::Named { args, .. } => {
            for a in args {
                if let ast::TypeArg::Type(t) = a {
                    walk_type(t, f);
                }
            }
        }
        ast::TypeKind::Record(fs) => fs.iter().for_each(|x| walk_type(&x.ty, f)),
        ast::TypeKind::Sum(cs) => cs
            .iter()
            .flat_map(|c| c.record.iter().flatten())
            .for_each(|x| walk_type(&x.ty, f)),
        ast::TypeKind::Fn(a, b) => {
            walk_type(a, f);
            walk_type(b, f);
        }
        ast::TypeKind::Var(_) | ast::TypeKind::Key(..) | ast::TypeKind::Unit => {}
    }
}

/// Call `f` on every type expression of a declaration.
fn each_type<'a>(d: &'a Decl, f: &mut dyn FnMut(&'a ast::Type)) {
    match d {
        Decl::Type(t) => {
            for x in t.header.iter().flatten() {
                walk_type(&x.ty, f);
            }
            match &t.body {
                ast::TypeBody::New(base, _) => walk_type(base, f),
                ast::TypeBody::Record(fs) => fs.iter().for_each(|x| walk_type(&x.ty, f)),
                ast::TypeBody::Ctors(cs) => cs
                    .iter()
                    .flat_map(|c| c.record.iter().flatten())
                    .for_each(|x| walk_type(&x.ty, f)),
            }
        }
        Decl::Relation(r) => {
            walk_type(&r.ty, f);
            for c in &r.clauses {
                if let RelationClause::Computed(_, t, _) = c {
                    walk_type(t, f);
                }
            }
        }
        Decl::Fn(fd) => {
            fd.params.iter().for_each(|p| walk_type(&p.ty, f));
            walk_type(&fd.result, f);
        }
        Decl::Aggregate(a) => {
            a.params.iter().for_each(|p| walk_type(&p.ty, f));
            walk_type(&a.result, f);
            a.state.iter().for_each(|x| walk_type(&x.ty, f));
        }
        Decl::Set(_) | Decl::Constraint(_) | Decl::Check(_) | Decl::Store(_) => {}
    }
}

/// The field declarations of a type declaration: the header's, then each constructor's, by constructor name.
fn decl_fields(t: &ast::TypeDecl) -> Vec<(Option<&str>, &ast::FieldType)> {
    let mut out: Vec<(Option<&str>, &ast::FieldType)> =
        t.header.iter().flatten().map(|f| (None, f)).collect();
    match &t.body {
        ast::TypeBody::Record(fs) => out.extend(fs.iter().map(|f| (Some(t.name.text.as_str()), f))),
        ast::TypeBody::Ctors(cs) => {
            for c in cs {
                out.extend(
                    c.record
                        .iter()
                        .flatten()
                        .map(|f| (Some(c.name.text.as_str()), f)),
                );
            }
        }
        ast::TypeBody::New(..) => {}
    }
    out
}

/// A `revert` whose inverse is being computed: the source it applies to, its migration's module
/// and text, and its span there.
struct Cx<'a> {
    src: &'a mut Source,
    module: String,
    text: &'a str,
    span: Span,
    /// Whether the text it copies into the inverse is recorded as copies of `text`.
    copying: bool,
    /// Whether the log does not yet hold the revert's migration; a fill no inverse takes is then
    /// refused.
    accepting: bool,
    /// Whether the revert is among the operators of a logged migration an inverse applies again;
    /// it then applies without its dependency gate.
    replaying: bool,
}

type R<T> = Result<T, Vec<Diag>>;

/// The write state of a constructor relation, by the ids of the relation variable and the
/// constructor, or of a value-level constructor, by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::evolve) enum StateOf {
    Relation(CatRelId, CatCtorId),
    Value(CatCtorId),
}

/// Returns the name of a field of type `def` whose delete action retags into constructor `c`.
fn retag_into(schema: &Schema, def: TypeId, c: u32) -> Option<String> {
    let t = schema.ty(def);
    t.header()
        .iter()
        .chain(t.ctors().iter().flat_map(|x| &x.fields))
        .find(|f| matches!(f.on_delete, schema::OnDelete::Retag(k, _) if k == c))
        .map(|f| f.name.clone())
}

fn mentions_def(t: &Ty, def: TypeId) -> bool {
    match t {
        Ty::Data { def: d, args, .. } => *d == def || args.iter().any(|a| mentions_def(a, def)),
        Ty::Set(x) | Ty::NonEmpty(x) | Ty::Tag(x) => mentions_def(x, def),
        Ty::Group(a, b) | Ty::Fn(a, b) => mentions_def(a, def) || mentions_def(b, def),
        Ty::Record(fs) => fs.iter().any(|(_, x)| mentions_def(x, def)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, x)| mentions_def(x, def))),
        Ty::Base(_) | Ty::Param(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => false,
    }
}

fn mentions_rel(t: &Ty, rel: RelId) -> bool {
    match t {
        Ty::Key { rel: r, .. } => *r == RelRef::Declared(rel),
        Ty::Data { args, .. } => args.iter().any(|a| mentions_rel(a, rel)),
        Ty::Set(x) | Ty::NonEmpty(x) | Ty::Tag(x) => mentions_rel(x, rel),
        Ty::Group(a, b) | Ty::Fn(a, b) => mentions_rel(a, rel) || mentions_rel(b, rel),
        Ty::Record(fs) => fs.iter().any(|(_, x)| mentions_rel(x, rel)),
        Ty::Sum { ctors, .. } => ctors
            .iter()
            .any(|(_, fs)| fs.iter().any(|(_, x)| mentions_rel(x, rel))),
        Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => false,
    }
}
