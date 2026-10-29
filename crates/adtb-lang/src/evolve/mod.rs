use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::rc::Rc;

use crate::ast::{
    self, CtorPlace, CtorSel, CtorTarget, Decl, Expr, Fill, MigrationFile, Operator, OperatorKind,
    QName, RelationClause,
};
use crate::canon::Names;
use crate::check::{self, Program, Ref, RowBody};
use crate::lex::{Tok, lex};
use crate::parse::{Options, parse_schema};
use crate::schema::{self, ENGINE_MODULES, Schema, TypeDefKind, ValueDef};
use crate::span::{Diag, FileDiag, FileId, Span};
use crate::types::*;

mod carry;
mod ctor;
mod cx;
mod field;
mod grant;
mod ids;
mod keep;
mod names;
mod resolved;
mod revert;
mod rowmap;
mod source;
mod text;

use field::{Part, path_text};
use text::*;

#[cfg(test)]
#[path = "../evolve_tests.rs"]
mod tests;

pub use crate::schema::Stage;
use grant::HeldGrant;
pub use grant::{GrantFields, GrantSel, GrantTarget, Granted, LogGrant};
pub use keep::Kept;
pub use resolved::{
    ArmTarget, Clause, Code, Function, IndexKind, Place, Resolved, ResolvedOp, Target,
};
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
    pub before: Rc<Source>,
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
    /// The write state of each constructor relation, keyed by the relation's module, the relation and the constructor.
    pub states: BTreeMap<StateKey, State>,
    /// The write state of each value-level constructor, keyed by the type's module, the type and the constructor.
    pub value_states: BTreeMap<StateKey, State>,
    pub version: u64,
    pub log: Vec<Logged>,
    /// The catalog id of each object of the user modules, by its path (`ids::Object::path`).
    pub ids: BTreeMap<String, u64>,
    /// The next id of each space.
    pub next: BTreeMap<crate::ids::Space, u64>,
    /// Where operators copied the text of the migration [`Logged::names`] replays; `None` otherwise.
    pub marks: Option<Marks>,
    /// The relation variables, by module and name, that the operators of the migration being
    /// applied add.
    pub ahead: Vec<schema::Ahead>,
    /// How the operator being applied carries grants, once the constructors it makes have ids.
    carry: Option<carry::Carry>,
    /// The grants the structural operators of the migration being applied could not carry and
    /// no later grant operator of it has stated.
    unstated: Vec<carry::Unstated>,
}

/// Text of the migration being replayed that an operator copied into a declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mark {
    /// The copied range of the migration's text.
    from: Range<usize>,
    decl: usize,
    /// The offset of the copy in the declaration's text, without the header.
    at: usize,
}

/// The copies of a replayed migration's text in the declarations of a source.
#[derive(Clone, Debug, Default)]
pub struct Marks {
    /// The copies in the declarations as they are now, each moved by every later edit, less the text a
    /// later edit replaces.
    written: Vec<Mark>,
    /// The text of the migration that the current operator matched against declarations as they were
    /// before it.
    read: Vec<Mark>,
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

    /// Returns `parts`, each after `before`.
    fn prefixed(parts: impl IntoIterator<Item = Written>, before: &str) -> Written {
        let mut out = Written::default();
        for p in parts {
            out.push(before);
            out.append(p);
        }
        out
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
}

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

fn scope_idents(s: &ast::CtorScope) -> Vec<&ast::Ident> {
    match s {
        ast::CtorScope::One(c) => vec![c],
        ast::CtorScope::Many(cs) => cs.iter().collect(),
    }
}

fn scope_sel(s: &ast::CtorScope) -> CtorSel {
    match s {
        ast::CtorScope::One(c) => CtorSel::One(c.clone()),
        ast::CtorScope::Many(cs) => CtorSel::Many(cs.clone()),
    }
}

/// The text of a constructor selection: `C` or `(C | D)`.
fn sel_text(names: &[String]) -> String {
    match names {
        [one] => one.clone(),
        many => format!("({})", many.join(" | ")),
    }
}

/// Calls `f` on every type expression of a declaration.
fn each_type<'a>(d: &'a Decl, f: &mut dyn FnMut(&'a ast::Type)) {
    fn walk<'a>(t: &'a ast::Type, f: &mut dyn FnMut(&'a ast::Type)) {
        f(t);
        match &t.kind {
            ast::TypeKind::Named { args, .. } => {
                for a in args {
                    if let ast::TypeArg::Type(t) = a {
                        walk(t, f);
                    }
                }
            }
            ast::TypeKind::Record(fs) => fs.iter().for_each(|x| walk(&x.ty, f)),
            ast::TypeKind::Sum(cs) => cs
                .iter()
                .flat_map(|c| c.record.iter().flatten())
                .for_each(|x| walk(&x.ty, f)),
            ast::TypeKind::Fn(a, b) => {
                walk(a, f);
                walk(b, f);
            }
            ast::TypeKind::Var(_) | ast::TypeKind::Key(..) | ast::TypeKind::Unit => {}
        }
    }
    match d {
        Decl::Type(t) => {
            for x in t.header.iter().flatten() {
                walk(&x.ty, f);
            }
            match &t.body {
                ast::TypeBody::New(base, _) => walk(base, f),
                ast::TypeBody::Record(fs) => fs.iter().for_each(|x| walk(&x.ty, f)),
                ast::TypeBody::Ctors(cs) => cs
                    .iter()
                    .flat_map(|c| c.record.iter().flatten())
                    .for_each(|x| walk(&x.ty, f)),
            }
        }
        Decl::Relation(r) => {
            walk(&r.ty, f);
            for c in &r.clauses {
                if let RelationClause::Computed(_, t, _) = c {
                    walk(t, f);
                }
            }
        }
        Decl::Fn(fd) => {
            fd.params.iter().for_each(|p| walk(&p.ty, f));
            walk(&fd.result, f);
        }
        Decl::Aggregate(a) => {
            a.params.iter().for_each(|p| walk(&p.ty, f));
            walk(&a.result, f);
            a.state.iter().for_each(|x| walk(&x.ty, f));
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

/// How a structural operator rewrites the selections over the constructors it replaces.
#[derive(Clone, Copy, Debug)]
struct Reselect {
    /// A selection must hold all or none of them.
    whole: bool,
    /// The scopes of relation clauses are rewritten too.
    clauses: bool,
    /// The scopes of checks are rewritten too.
    checks: bool,
    /// The operator, as a refusal names it.
    what: &'static str,
}

/// One operator being applied.
struct Cx<'a> {
    src: &'a mut Source,
    module: String,
    text: &'a str,
    span: Span,
    /// Where a fill or routing records the names it resolves, when replaying for them.
    noted: Option<&'a RefCell<Names>>,
    /// Whether the text the operator copies into declarations is recorded as copies of `text`.
    copying: bool,
    /// Where a revert records the steps of its inverses, when replaying for them.
    steps: Option<&'a RefCell<Vec<Step>>>,
}

type R<T> = Result<T, Vec<Diag>>;

/// Edits of declaration texts, by declaration.
type Edits = BTreeMap<usize, Vec<(Range<usize>, Written)>>;

/// A write state's key: the module, the relation variable or type, and the constructor.
type StateKey = (String, String, String);

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
        Ty::Key { rel: r, .. } => *r == rel,
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
