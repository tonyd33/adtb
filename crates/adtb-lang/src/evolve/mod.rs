use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::rc::Rc;

use crate::ast::{
    self, CtorPlace, CtorSel, CtorTarget, Decl, Expr, Fill, MigrationFile, Operator, OperatorKind,
    QName, RelationClause,
};
use crate::check::{self, Program, Ref, RowBody};
use crate::lex::{Tok, lex};
use crate::parse::{Options, parse_schema};
use crate::schema::{self, ENGINE_MODULES, Schema, TypeDefKind, ValueDef};
use crate::span::{Diag, FileDiag, FileId, Span};
use crate::types::*;

mod ctor;
mod field;
mod revert;
mod text;

use field::{Part, path_text};
use text::*;

#[cfg(test)]
#[path = "../evolve_tests.rs"]
mod tests;

/// The write state of a constructor in a relation variable, or of a value-level constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Declared,
    Enabled,
    Deprecated,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Declared => "declared",
            Stage::Enabled => "enabled",
            Stage::Deprecated => "deprecated",
        }
    }
}

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
    /// The write state of each constructor relation, keyed by the relation's module, the relation and the constructor.
    pub states: BTreeMap<StateKey, State>,
    /// The write state of each value-level constructor, keyed by the type's module, the type and the constructor.
    pub value_states: BTreeMap<StateKey, State>,
    pub version: u64,
    pub log: Vec<Logged>,
}

/// Why a migration is refused: the operators that do not apply, at their spans in the migration's text,
/// and the schema objects of the result that do not check, in the files of the result.
#[derive(Clone, Debug, Default)]
pub struct Refusal {
    pub operators: Vec<Diag>,
    pub recheck: Vec<FileDiag>,
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
            ast::TypeKind::Var(_) | ast::TypeKind::Key(_) | ast::TypeKind::Unit => {}
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

impl Source {
    /// Returns the source of a schema declared directly, every constructor enabled at version 0.
    pub fn new(decls: Vec<SourceDecl>) -> Result<Source, Vec<FileDiag>> {
        let mut src = Source {
            modules: decls.iter().map(|d| d.module.clone()).collect(),
            decls,
            ..Source::default()
        };
        let schema = src.schema()?;
        src.states = Source::all_states(&schema, Stage::Enabled);
        for t in &schema.types {
            if schema.module(t.module).engine {
                continue;
            }
            for c in t.ctors() {
                src.value_states.insert(
                    (
                        schema.module(t.module).name.clone(),
                        t.name.clone(),
                        c.name.clone(),
                    ),
                    State {
                        stage: Stage::Enabled,
                        since: 0,
                    },
                );
            }
        }
        Ok(src)
    }

    fn all_states(schema: &Schema, stage: Stage) -> BTreeMap<StateKey, State> {
        let mut out = BTreeMap::new();
        for r in &schema.relations {
            if schema.module(r.module).engine || r.elem_def.0 == u32::MAX {
                continue;
            }
            for c in schema.ty(r.elem_def).ctors() {
                out.insert(
                    (
                        schema.module(r.module).name.clone(),
                        r.name.clone(),
                        c.name.clone(),
                    ),
                    State { stage, since: 0 },
                );
            }
        }
        out
    }

    /// Returns the module files of the declarations, file `i` holding declaration `i`.
    pub fn files(&self) -> Result<Vec<(FileId, ast::SchemaFile)>, Vec<FileDiag>> {
        let mut out = Vec::new();
        let mut errs = Vec::new();
        for (i, d) in self.decls.iter().enumerate() {
            let text = format!("{}{}", header(&d.module), d.text);
            match parse_schema(&text, Options { elisions: true }).ok() {
                Ok(f) => out.push((FileId(i as u32), f)),
                Err(ds) => errs.extend(ds.into_iter().map(|diag| FileDiag {
                    file: FileId(i as u32),
                    diag,
                })),
            }
        }
        if errs.is_empty() { Ok(out) } else { Err(errs) }
    }

    /// The text of file `file` of [`Source::files`], module header included.
    pub fn file_text(&self, file: FileId) -> String {
        let d = &self.decls[file.0 as usize];
        format!("{}{}", header(&d.module), d.text)
    }

    pub fn schema(&self) -> Result<Schema, Vec<FileDiag>> {
        let mut files = self.files()?;
        for m in &self.modules {
            if !files.iter().any(|(_, f)| f.module.text == *m) {
                let empty = parse_schema(&header(m), Options::default())
                    .ok()
                    .expect("a module header parses");
                files.push((FileId(u32::MAX - 2), empty));
            }
        }
        schema::build(&files)
    }

    /// Builds and type checks the schema.
    pub fn check(&self) -> Result<(Schema, Program), Vec<FileDiag>> {
        let schema = self.schema()?;
        let program = check::check_schema(&schema)?;
        Ok((schema, program))
    }

    fn parsed(&self, i: usize) -> Option<Decl> {
        let text = self.file_text(FileId(i as u32));
        parse_schema(&text, Options { elisions: true })
            .ok()
            .ok()
            .and_then(|f| f.decls.into_iter().next())
    }

    fn find(&self, kind: &str, module: &str, name: &str) -> Option<usize> {
        (0..self.decls.len()).find(|&i| {
            self.decls[i].module == module
                && self.parsed(i).is_some_and(|d| {
                    let (k, n) = key_of(&d);
                    k == kind && n == name
                })
        })
    }

    /// The byte range of `span`, of declaration `i`'s file, in its text without the header.
    fn local(&self, i: usize, span: Span) -> Range<usize> {
        let h = header(&self.decls[i].module).len();
        span.start as usize - h..span.end as usize - h
    }

    /// Replaces each range of declaration `i`'s text by its text.
    fn edit(&mut self, i: usize, mut edits: Vec<(Range<usize>, String)>) {
        edits.sort_by(|a, b| b.0.start.cmp(&a.0.start).then(b.0.end.cmp(&a.0.end)));
        edits.dedup();
        let text = &mut self.decls[i].text;
        for (r, s) in edits {
            text.replace_range(r, &s);
        }
    }

    fn push(&mut self, module: &str, text: &str) -> usize {
        self.decls.push(SourceDecl {
            module: module.to_string(),
            text: text.to_string(),
        });
        self.decls.len() - 1
    }

    /// Applies a migration, numbered one past the current version, and rechecks the result.
    pub fn migrate(&self, m: &MigrationFile, text: &str) -> Result<Source, Refusal> {
        let mut next = self.clone();
        next.version += 1;
        let mut refusal = Refusal::default();
        let mut rekeyed = Vec::new();
        for op in &m.operators {
            if let OperatorKind::Rekey { relation, .. } = &op.kind {
                rekeyed.push((relation.clone(), op.span));
            }
            if let Err(mut ds) = next.apply(&m.module.text, op, text) {
                refusal.operators.append(&mut ds);
            }
        }
        for (relation, span) in rekeyed {
            if let Err(d) = next.rekey_leaves_no_old_reference(&m.module.text, &relation, span) {
                refusal.operators.push(d);
            }
        }
        if let Err(ds) = next.check() {
            let reported: BTreeSet<String> = refusal
                .operators
                .iter()
                .map(|d| d.message.clone())
                .collect();
            refusal.recheck = ds
                .into_iter()
                .filter(|d| !reported.contains(&d.diag.message))
                .collect();
        }
        if refusal.operators.is_empty() && refusal.recheck.is_empty() {
            next.log.push(Logged {
                number: next.version,
                module: m.module.text.clone(),
                text: text.to_string(),
                operators: m.operators.clone(),
                before: Rc::new(self.clone()),
            });
            Ok(next)
        } else {
            refusal.result = Some(Box::new(next));
            Err(refusal)
        }
    }

    /// Applies one operator of a migration of `module` whose text is `text`.
    ///
    /// On a refusal the source is left as it was.
    pub fn apply(&mut self, module: &str, op: &Operator, text: &str) -> Result<(), Vec<Diag>> {
        let before = self.clone();
        let r = Cx {
            src: self,
            module: module.to_string(),
            text,
            span: op.span,
        }
        .operator(&op.kind);
        if r.is_err() {
            *self = before;
        }
        r
    }

    fn rekey_leaves_no_old_reference(
        &self,
        module: &str,
        relation: &QName,
        span: Span,
    ) -> Result<(), Diag> {
        let Ok(schema) = self.schema() else {
            return Ok(());
        };
        let m = relation.module.as_ref().map_or(module, |m| m.text.as_str());
        let Some(mid) = schema.module_named(m) else {
            return Ok(());
        };
        let Some(&ValueDef::Relation(rel)) = schema.module(mid).values.get(&relation.name.text)
        else {
            return Ok(());
        };
        let r = schema.relation(rel);
        let t = schema.ty(r.elem_def);
        for (c, f) in t
            .ctors()
            .iter()
            .flat_map(|c| c.fields.iter().map(move |f| (Some(&c.name), f)))
            .chain(t.header().iter().map(|f| (None, f)))
        {
            if matches!(f.ty, Ty::Key { rel: own, .. } if own == rel) && !r.key.contains(&f.name) {
                let path = match c {
                    Some(c) if t.ctors().len() > 1 => format!("{}.{c}.{}", t.name, f.name),
                    _ => format!("{}.{}", t.name, f.name),
                };
                return Err(Diag::new(
                    span,
                    format!(
                        "field `{path}` of `{}`'s own type is typed `Key {}` and is outside the new key; drop it in the same migration",
                        r.name, r.name
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// How a structural operator rewrites the selections over the constructors it replaces.
#[derive(Clone, Copy, Debug)]
struct Reselect {
    /// A selection must hold all or none of them.
    whole: bool,
    /// The scopes of relation clauses are rewritten too.
    clauses: bool,
    /// The operator, as a refusal names it.
    what: &'static str,
}

/// One operator being applied.
struct Cx<'a> {
    src: &'a mut Source,
    module: String,
    text: &'a str,
    span: Span,
}

type R<T> = Result<T, Vec<Diag>>;

/// Edits of declaration texts, by declaration.
type Edits = BTreeMap<usize, Vec<(Range<usize>, String)>>;

/// A write state's key: the module, the relation variable or type, and the constructor.
type StateKey = (String, String, String);

impl Cx<'_> {
    fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(vec![Diag::new(self.span, msg)])
    }

    fn err_at<T>(&self, span: Span, msg: impl Into<String>) -> R<T> {
        Err(vec![Diag::new(span, msg)])
    }

    fn slice(&self, span: Span) -> &str {
        &self.text[span.range()]
    }

    fn module_of(&self, q: &QName) -> String {
        q.module
            .as_ref()
            .map_or_else(|| self.module.clone(), |m| m.text.clone())
    }

    fn schema(&self) -> R<Schema> {
        self.src.schema().map_err(|ds| {
            ds.into_iter()
                .map(|d| Diag::new(self.span, d.diag.message))
                .collect()
        })
    }

    fn checked(&self) -> R<(Schema, Program)> {
        self.src.check().map_err(|ds| {
            ds.into_iter()
                .map(|d| {
                    Diag::new(
                        self.span,
                        format!(
                            "the operator checks against the schema the operators before it produce, which does not check: {}",
                            d.diag.message
                        ),
                    )
                })
                .collect()
        })
    }

    /// The declaration of type `q`.
    fn type_decl(&self, q: &QName) -> R<(usize, ast::TypeDecl)> {
        let m = self.module_of(q);
        match self.src.find("type", &m, &q.name.text) {
            Some(i) => match self.src.parsed(i) {
                Some(Decl::Type(t)) => Ok((i, t)),
                _ => unreachable!("a type key names a type declaration"),
            },
            None => self.err_at(
                q.span(),
                format!("module `{m}` declares no type `{}`", q.name.text),
            ),
        }
    }

    fn relation_decl(&self, q: &QName) -> R<(usize, ast::RelationDecl)> {
        let m = self.module_of(q);
        match self.src.find("relation", &m, &q.name.text) {
            Some(i) => match self.src.parsed(i) {
                Some(Decl::Relation(r)) => Ok((i, r)),
                _ => unreachable!("a relation key names a relation declaration"),
            },
            None => self.err_at(
                q.span(),
                format!("module `{m}` declares no relation `{}`", q.name.text),
            ),
        }
    }

    fn type_id(&self, schema: &Schema, q: &QName) -> R<TypeId> {
        let m = self.module_of(q);
        schema
            .module_named(&m)
            .and_then(|mid| schema.module(mid).types.get(&q.name.text).copied())
            .map_or_else(
                || {
                    self.err_at(
                        q.span(),
                        format!("module `{m}` declares no type `{}`", q.name.text),
                    )
                },
                Ok,
            )
    }

    fn rel_id(&self, schema: &Schema, q: &QName) -> R<RelId> {
        let m = self.module_of(q);
        match schema
            .module_named(&m)
            .and_then(|mid| schema.module(mid).values.get(&q.name.text))
        {
            Some(ValueDef::Relation(r)) => Ok(*r),
            _ => self.err_at(
                q.span(),
                format!("module `{m}` declares no relation `{}`", q.name.text),
            ),
        }
    }

    /// The relation variables of type `def`, by module and name.
    fn relations_of(schema: &Schema, def: TypeId) -> Vec<(RelId, String, String)> {
        schema
            .relations
            .iter()
            .enumerate()
            .filter(|(_, r)| r.elem_def == def)
            .map(|(i, r)| {
                (
                    RelId(i as u32),
                    schema.module(r.module).name.clone(),
                    r.name.clone(),
                )
            })
            .collect()
    }

    /// The schema object a file of the source declares, named for a refusal.
    fn object_name(&self, file: FileId) -> String {
        match self.src.parsed(file.0 as usize) {
            Some(d) => {
                let (kind, name) = key_of(&d);
                format!("{kind} `{name}`")
            }
            None => "a declaration".to_string(),
        }
    }

    fn operator(&mut self, op: &OperatorKind) -> R<()> {
        match op {
            OperatorKind::AddModule(m) => self.add_module(m),
            OperatorKind::DropModule(m) => self.drop_module(m),
            OperatorKind::Install { name, .. } => self.install(name),
            OperatorKind::Uninstall(m) => self.uninstall(m),
            OperatorKind::AddType(t) => self.add_type(t),
            OperatorKind::DropType(q) => self.drop_type(q),
            OperatorKind::RenameType(q, to) => self.rename_type(q, to),
            OperatorKind::AddRelation(r) => self.add_relation(r),
            OperatorKind::DropRelation(q) => self.drop_relation(q),
            OperatorKind::RenameRelation(q, to) => self.rename_relation(q, to),
            OperatorKind::AddSet(s) => self.add_decl(
                "set",
                &format!("{}.{}", s.relation.text, s.name.text),
                s.span,
            ),
            OperatorKind::DropSet(name, rel) => self.drop_set(name, rel),
            OperatorKind::RenameSet(name, rel, to) => self.rename_set(name, rel, to),
            OperatorKind::AddConstructor {
                path,
                record,
                place,
                joins,
                arms,
            } => self.add_constructor(path, record.is_some(), place.as_ref(), joins, arms),
            OperatorKind::RemoveConstructor(path) => self.remove_constructor(path),
            OperatorKind::RenameConstructor(path, to) => self.rename_constructor(path, to),
            OperatorKind::SplitConstructor {
                path,
                into,
                arms,
                clauses,
            } => self.split(path, into, arms, clauses),
            OperatorKind::MergeConstructor {
                ty,
                sel,
                into,
                arms,
                clauses,
            } => self.merge(ty, sel, into, arms.as_deref(), clauses),
            OperatorKind::NestConstructor {
                ty,
                sel,
                into,
                clauses,
            } => self.nest(ty, sel, into, clauses),
            OperatorKind::UnnestConstructor(path, clauses) => self.unnest(path, clauses),
            OperatorKind::AddField { path, ty, fill } => self.add_field(path, ty, fill),
            OperatorKind::DropField(path) => self.drop_field(path),
            OperatorKind::RenameField(path, to) => self.rename_field(path, to),
            OperatorKind::MoveField {
                path,
                to,
                sel,
                fill,
            } => self.move_field(path, to, sel.as_ref(), fill.as_ref()),
            OperatorKind::AddUnique(fields, on) => self.add_index("unique", fields, on),
            OperatorKind::AddIndex {
                interval,
                fields,
                on,
            } => self.add_index(
                if *interval { "interval index" } else { "index" },
                fields,
                on,
            ),
            OperatorKind::DropUnique(fields, on) => self.drop_index(true, false, fields, on),
            OperatorKind::DropIndex {
                interval,
                fields,
                on,
            } => self.drop_index(false, *interval, fields, on),
            OperatorKind::Rekey {
                relation,
                key,
                carry,
            } => self.rekey(relation, key, carry),
            OperatorKind::AddDefault {
                relation,
                ctor,
                field,
                value,
            } => self.add_default(relation, ctor.as_ref(), field, value),
            OperatorKind::DropDefault {
                relation,
                ctor,
                field,
            } => self.drop_default(relation, ctor.as_ref(), field),
            OperatorKind::DefineFn(f) => self.add_decl("fn", &f.name.text, f.span),
            OperatorKind::RedefineFn(f) => self.redefine("fn", &f.name, f.span),
            OperatorKind::DropFn(q) => self.drop_decl("fn", q),
            OperatorKind::DefineAggregate(a) => self.add_decl("aggregate", &a.name.text, a.span),
            OperatorKind::DropAggregate(q) => self.drop_decl("aggregate", q),
            OperatorKind::AddConstraint(c) => self.add_decl("constraint", &c.name.text, c.span),
            OperatorKind::DropConstraint(q) => self.drop_decl("constraint", q),
            OperatorKind::AddCheck(c) => self.add_decl("check", &c.name.text, c.span),
            OperatorKind::DropCheck(q) => self.drop_decl("check", q),
            OperatorKind::Store(s) => self.add_decl("store", &s.view.name.text, s.span),
            OperatorKind::Unstore(q) => self.drop_decl("store", q),
            OperatorKind::AddComputed {
                relation,
                field,
                ty,
                expr,
            } => self.add_computed(relation, field, ty, expr),
            OperatorKind::RedefineComputed {
                relation,
                field,
                ty,
                expr,
            } => self.redefine_computed(relation, field, ty, expr),
            OperatorKind::DropComputed(relation, field) => self.drop_computed(relation, field),
            OperatorKind::Enable(t) => self.stage(t, "enable"),
            OperatorKind::Disable(t) => self.stage(t, "disable"),
            OperatorKind::Deprecate(t) => self.stage(t, "deprecate"),
            OperatorKind::Revert { migration, fills } => self.revert(migration, fills),
            OperatorKind::Script(w) => self.script(w),
            OperatorKind::AdoptTzdb { .. } => Ok(()),
            OperatorKind::Grant(g) => self.log_grant(&g.privileges),
            OperatorKind::Revoke(r) => self.log_grant(&r.privileges),
        }
    }

    fn reserved_module(&self, m: &ast::Ident) -> R<()> {
        if m.text == "prelude" || ENGINE_MODULES.contains(&m.text.as_str()) {
            return self.err_at(m.span, format!("no module may be named `{}`", m.text));
        }
        if self.src.modules.contains(&m.text) {
            return self.err_at(m.span, format!("module `{}` already exists", m.text));
        }
        Ok(())
    }

    fn add_module(&mut self, m: &ast::Ident) -> R<()> {
        self.reserved_module(m)?;
        self.src.modules.insert(m.text.clone());
        Ok(())
    }

    fn drop_module(&mut self, m: &ast::Ident) -> R<()> {
        if !self.src.modules.contains(&m.text) {
            return self.err_at(m.span, format!("no module `{}`", m.text));
        }
        if let Some(i) = self.src.decls.iter().position(|d| d.module == m.text) {
            let name = self.object_name(FileId(i as u32));
            return self.err_at(
                m.span,
                format!(
                    "`drop module {}` is refused while the module holds {name}",
                    m.text
                ),
            );
        }
        self.src.modules.remove(&m.text);
        self.src.installed.remove(&m.text);
        Ok(())
    }

    fn install(&mut self, m: &ast::Ident) -> R<()> {
        self.reserved_module(m)?;
        self.src.modules.insert(m.text.clone());
        self.src.installed.insert(m.text.clone());
        Ok(())
    }

    fn uninstall(&mut self, m: &ast::Ident) -> R<()> {
        if !self.src.installed.contains(&m.text) {
            return self.err_at(
                m.span,
                format!("`{}` is not an installed extension", m.text),
            );
        }
        for (i, d) in self.src.decls.iter().enumerate() {
            if d.module == m.text {
                continue;
            }
            let toks = tokens(&d.text);
            let names = toks.windows(2).any(|w| {
                w[0].0 == Tok::Lower
                    && d.text[w[0].1.clone()] == m.text
                    && w[1].0 == Tok::ColonColon
            });
            if names {
                let name = self.object_name(FileId(i as u32));
                return self.err_at(
                    m.span,
                    format!("`uninstall {}` is refused while {name} names it", m.text),
                );
            }
        }
        self.src.decls.retain(|d| d.module != m.text);
        self.src.modules.remove(&m.text);
        self.src.installed.remove(&m.text);
        Ok(())
    }

    fn known_module(&self) -> R<()> {
        if self.src.modules.contains(&self.module) {
            Ok(())
        } else {
            self.err(format!(
                "no module `{}`; a migration of a new module starts with `add module {}`",
                self.module, self.module
            ))
        }
    }

    /// Adds the declaration written at `span` of the migration, after its verb.
    fn add_decl(&mut self, kind: &str, name: &str, span: Span) -> R<()> {
        self.known_module()?;
        if self.src.find(kind, &self.module, name).is_some() {
            return self.err_at(
                span,
                format!("module `{}` already declares {kind} `{name}`", self.module),
            );
        }
        let text = self.slice(span).to_string();
        let module = self.module.clone();
        self.src.push(&module, &text);
        Ok(())
    }

    fn redefine(&mut self, kind: &str, name: &ast::Ident, span: Span) -> R<()> {
        let Some(i) = self.src.find(kind, &self.module, &name.text) else {
            return self.err_at(
                name.span,
                format!(
                    "module `{}` declares no {kind} `{}`",
                    self.module, name.text
                ),
            );
        };
        self.src.decls[i].text = self.slice(span).to_string();
        Ok(())
    }

    fn drop_decl(&mut self, kind: &str, q: &QName) -> R<()> {
        let m = self.module_of(q);
        let Some(i) = self.src.find(kind, &m, &q.name.text) else {
            return self.err_at(
                q.span(),
                format!("module `{m}` declares no {kind} `{}`", q.name.text),
            );
        };
        self.src.decls.remove(i);
        Ok(())
    }

    fn add_type(&mut self, t: &ast::TypeDecl) -> R<()> {
        self.add_decl("type", &t.name.text, t.span)?;
        let names: Vec<String> = match &t.body {
            ast::TypeBody::Record(_) => vec![t.name.text.clone()],
            ast::TypeBody::Ctors(cs) => cs.iter().map(|c| c.name.text.clone()).collect(),
            ast::TypeBody::New(..) => vec![t.name.text.clone()],
        };
        let since = self.src.version;
        for c in names {
            self.src.value_states.insert(
                (self.module.clone(), t.name.text.clone(), c),
                State {
                    stage: Stage::Enabled,
                    since,
                },
            );
        }
        Ok(())
    }

    fn drop_type(&mut self, q: &QName) -> R<()> {
        let (i, decl) = self.type_decl(q)?;
        let schema = self.schema()?;
        let def = self.type_id(&schema, q)?;
        let name = &q.name.text;
        let refused = |what: String| -> R<()> {
            self.err(format!(
                "`drop type {name}` is refused while {what} uses it"
            ))
        };
        for r in &schema.relations {
            if r.elem_def == def {
                return refused(format!("relation `{}`", r.name));
            }
        }
        for (j, t) in schema.types.iter().enumerate() {
            if j == def.0 as usize || schema.module(t.module).engine {
                continue;
            }
            for f in t
                .header()
                .iter()
                .chain(t.ctors().iter().flat_map(|c| c.fields.iter()))
            {
                if mentions_def(&f.ty, def) {
                    return refused(format!("field `{}.{}`", t.name, f.name));
                }
            }
        }
        for f in &schema.functions {
            if f.params.iter().any(|(_, t)| mentions_def(t, def)) || mentions_def(&f.result, def) {
                return refused(format!("function `{}`", f.name));
            }
        }
        for (file, _, r) in check::schema_refs(&schema) {
            if file.0 as usize != i
                && matches!(r, Ref::Ctor(d, _) | Ref::Field { def: d, .. } if d == def)
            {
                return refused(self.object_name(file));
            }
        }
        let module = self.module_of(q);
        self.src.decls.remove(i);
        self.src
            .value_states
            .retain(|(m, t, _), _| !(*m == module && *t == decl.name.text));
        Ok(())
    }

    fn rename_type(&mut self, q: &QName, to: &ast::Ident) -> R<()> {
        let (_, decl) = self.type_decl(q)?;
        let module = self.module_of(q);
        if self.src.find("type", &module, &to.text).is_some() {
            return self.err_at(
                to.span,
                format!("module `{module}` already declares type `{}`", to.text),
            );
        }
        let schema = self.schema()?;
        let def = self.type_id(&schema, q)?;
        let record = matches!(decl.body, ast::TypeBody::Record(_));
        let refs = check::schema_refs(&schema);
        let from = &q.name.text;
        for i in 0..self.src.decls.len() {
            let text = self.src.decls[i].text.clone();
            let h = header(&self.src.decls[i].module).len();
            let other_ctors: BTreeSet<usize> = refs
                .iter()
                .filter(|(f, _, r)| {
                    f.0 as usize == i && matches!(r, Ref::Ctor(d, _) if *d != def || !record)
                })
                .map(|(_, s, _)| s.start as usize - h)
                .collect();
            let toks = tokens(&text);
            let mut edits = Vec::new();
            for (k, (tok, range)) in toks.iter().enumerate() {
                if *tok != Tok::Upper || text[range.clone()] != *from {
                    continue;
                }
                if k > 0 && toks[k - 1].0 == Tok::Dot {
                    continue;
                }
                let qualifier = (k >= 2 && toks[k - 1].0 == Tok::ColonColon)
                    .then(|| &text[toks[k - 2].1.clone()]);
                let resolves = match qualifier {
                    Some(m) => m == module,
                    None => self.src.decls[i].module == module,
                };
                if resolves && !other_ctors.contains(&range.start) {
                    edits.push((range.clone(), to.text.clone()));
                }
            }
            self.src.edit(i, edits);
        }
        let rename = |map: &mut BTreeMap<StateKey, State>, ty_key: bool| {
            let old: Vec<_> = map.keys().cloned().collect();
            for k in old {
                let (m, t, c) = k.clone();
                let new_t = if ty_key && m == module && t == *from {
                    to.text.clone()
                } else {
                    t.clone()
                };
                let new_c = if record && c == *from {
                    to.text.clone()
                } else {
                    c.clone()
                };
                if (new_t.clone(), new_c.clone()) != (t, c) {
                    let v = map.remove(&k).expect("a key of the map");
                    map.insert((m, new_t, new_c), v);
                }
            }
        };
        rename(&mut self.src.value_states, true);
        if record {
            let rels: BTreeSet<(String, String)> = Self::relations_of(&schema, def)
                .into_iter()
                .map(|(_, m, r)| (m, r))
                .collect();
            let old: Vec<_> = self.src.states.keys().cloned().collect();
            for k in old {
                if rels.contains(&(k.0.clone(), k.1.clone())) && k.2 == *from {
                    let v = self.src.states.remove(&k).expect("a key");
                    self.src.states.insert((k.0, k.1, to.text.clone()), v);
                }
            }
        }
        Ok(())
    }

    fn add_relation(&mut self, r: &ast::RelationDecl) -> R<()> {
        if r.name.module.is_some() {
            return self.err_at(
                r.name.span(),
                "a relation name is qualified only in the engine's own declarations",
            );
        }
        self.add_decl("relation", &r.name.name.text, r.span)?;
        let schema = self.schema()?;
        let rel = self.rel_id(&schema, &r.name)?;
        let def = schema.relation(rel).elem_def;
        if def.0 == u32::MAX {
            return Ok(());
        }
        let t = schema.ty(def);
        let tm = schema.module(t.module).name.clone();
        let since = self.src.version;
        for c in t.ctors() {
            let stage = self
                .src
                .value_states
                .get(&(tm.clone(), t.name.clone(), c.name.clone()))
                .map_or(Stage::Enabled, |s| s.stage);
            self.src.states.insert(
                (
                    self.module.clone(),
                    r.name.name.text.clone(),
                    c.name.clone(),
                ),
                State { stage, since },
            );
        }
        Ok(())
    }

    fn drop_relation(&mut self, q: &QName) -> R<()> {
        let (i, _) = self.relation_decl(q)?;
        let schema = self.schema()?;
        let rel = self.rel_id(&schema, q)?;
        let name = &q.name.text;
        for t in &schema.types {
            for f in t
                .header()
                .iter()
                .chain(t.ctors().iter().flat_map(|c| c.fields.iter()))
            {
                if mentions_rel(&f.ty, rel) && schema.relation(rel).elem_def != TypeId(u32::MAX) {
                    let own = schema.relation(rel).elem_def;
                    if schema.ty(own).name == t.name && schema.ty(own).module == t.module {
                        continue;
                    }
                    return self.err(format!(
                        "`drop relation {name}` is refused while field `{}.{}` references it",
                        t.name, f.name
                    ));
                }
            }
        }
        for (file, _, r) in check::schema_refs(&schema) {
            if file.0 as usize != i && matches!(r, Ref::Relation(x) if x == rel) {
                return self.err(format!(
                    "`drop relation {name}` is refused while {} names it",
                    self.object_name(file)
                ));
            }
        }
        let module = self.module_of(q);
        let sets: Vec<usize> = (0..self.src.decls.len())
            .filter(|&j| {
                self.src.decls[j].module == module
                    && matches!(self.src.parsed(j), Some(Decl::Set(s)) if s.relation.text == *name)
            })
            .collect();
        let mut drop = sets;
        drop.push(i);
        drop.sort();
        for j in drop.into_iter().rev() {
            self.src.decls.remove(j);
        }
        self.src
            .states
            .retain(|(m, r, _), _| !(*m == module && r == name));
        Ok(())
    }

    fn rename_relation(&mut self, q: &QName, to: &ast::Ident) -> R<()> {
        let (i, decl) = self.relation_decl(q)?;
        let module = self.module_of(q);
        let schema = self.schema()?;
        let rel = self.rel_id(&schema, q)?;
        if schema
            .module_named(&module)
            .is_some_and(|m| schema.module(m).values.contains_key(&to.text))
        {
            return self.err_at(
                to.span,
                format!("module `{module}` already declares `{}`", to.text),
            );
        }
        let from = q.name.text.clone();
        let mut edits: Edits = BTreeMap::new();
        edits
            .entry(i)
            .or_default()
            .push((self.src.local(i, decl.name.name.span), to.text.clone()));
        for (file, span, r) in check::schema_refs(&schema) {
            if matches!(r, Ref::Relation(x) if x == rel) {
                let j = file.0 as usize;
                edits
                    .entry(j)
                    .or_default()
                    .push((self.src.local(j, span), to.text.clone()));
            }
        }
        for j in 0..self.src.decls.len() {
            let Some(d) = self.src.parsed(j) else {
                continue;
            };
            let home = self.src.decls[j].module.clone();
            let names = |qn: &QName| {
                qn.name.text == from
                    && qn
                        .module
                        .as_ref()
                        .map_or(home == module, |m| m.text == module)
            };
            let mut here = Vec::new();
            each_type(&d, &mut |t| {
                if let ast::TypeKind::Key(rr) = &t.kind
                    && names(&rr.relation)
                {
                    here.push(rr.relation.name.span);
                }
            });
            match &d {
                Decl::Set(s) if home == module && s.relation.text == from => {
                    here.push(s.relation.span);
                }
                Decl::Check(c) if names(&c.on.relation) => here.push(c.on.relation.name.span),
                _ => {}
            }
            for s in here {
                edits
                    .entry(j)
                    .or_default()
                    .push((self.src.local(j, s), to.text.clone()));
            }
        }
        for (j, es) in edits {
            self.src.edit(j, es);
        }
        let old: Vec<_> = self.src.states.keys().cloned().collect();
        for k in old {
            if k.0 == module && k.1 == from {
                let v = self.src.states.remove(&k).expect("a key");
                self.src.states.insert((k.0, to.text.clone(), k.2), v);
            }
        }
        Ok(())
    }

    /// The declaration of named set `name` of relation `rel`.
    fn set_decl(&self, name: &ast::Ident, rel: &QName) -> R<(usize, ast::SetDecl)> {
        let m = self.module_of(rel);
        let key = format!("{}.{}", rel.name.text, name.text);
        match self.src.find("set", &m, &key) {
            Some(i) => match self.src.parsed(i) {
                Some(Decl::Set(s)) => Ok((i, s)),
                _ => unreachable!("a set key names a set declaration"),
            },
            None => self.err_at(
                name.span,
                format!("`{}` declares no named set `{}`", rel.name.text, name.text),
            ),
        }
    }

    fn drop_set(&mut self, name: &ast::Ident, rel: &QName) -> R<()> {
        let (i, _) = self.set_decl(name, rel)?;
        self.src.decls.remove(i);
        Ok(())
    }

    fn rename_set(&mut self, name: &ast::Ident, rel: &QName, to: &ast::Ident) -> R<()> {
        let (i, decl) = self.set_decl(name, rel)?;
        let schema = self.schema()?;
        let r = self.rel_id(&schema, rel)?;
        let rd = schema.relation(r);
        if rd.sets.iter().any(|s| s.name == to.text)
            || schema.ty(rd.elem_def).ctor_index(&to.text).is_some()
        {
            return self.err_at(
                to.span,
                format!(
                    "`{}` already has a constructor or named set `{}`",
                    rd.name, to.text
                ),
            );
        }
        let module = self.module_of(rel);
        let mut edits: Edits = BTreeMap::new();
        edits
            .entry(i)
            .or_default()
            .push((self.src.local(i, decl.name.span), to.text.clone()));
        for (file, span, x) in check::schema_refs(&schema) {
            if matches!(&x, Ref::Set(y, s) if *y == r && *s == name.text) {
                let j = file.0 as usize;
                edits
                    .entry(j)
                    .or_default()
                    .push((self.src.local(j, span), to.text.clone()));
            }
        }
        self.each_selection(&schema, r, &module, &mut |j, ident| {
            if ident.text == name.text {
                edits
                    .entry(j)
                    .or_default()
                    .push((ident.span.range(), to.text.clone()));
            }
        });
        self.apply_edits(edits);
        Ok(())
    }

    /// Calls `f` with each constructor or named set name that a declaration selects from relation `r`:
    /// in `Key r.S` types, the scopes of `r`'s own clauses, and checks on `r`. Spans are local to the declaration's text.
    fn each_selection(
        &self,
        schema: &Schema,
        r: RelId,
        module: &str,
        f: &mut dyn FnMut(usize, ast::Ident),
    ) {
        let name = schema.relation(r).name.clone();
        for j in 0..self.src.decls.len() {
            let Some(d) = self.src.parsed(j) else {
                continue;
            };
            let home = self.src.decls[j].module.clone();
            let names = |qn: &QName| {
                qn.name.text == name
                    && qn
                        .module
                        .as_ref()
                        .map_or(home == module, |m| m.text == module)
            };
            let mut found: Vec<ast::Ident> = Vec::new();
            each_type(&d, &mut |t| {
                if let ast::TypeKind::Key(rr) = &t.kind
                    && names(&rr.relation)
                    && let Some(sel) = &rr.sel
                {
                    found.extend(sel_idents(sel).into_iter().cloned());
                }
            });
            match &d {
                Decl::Relation(rd) if home == module && rd.name.name.text == name => {
                    for c in &rd.clauses {
                        match c {
                            RelationClause::Unique(_, Some(s))
                            | RelationClause::Index { scope: Some(s), .. } => {
                                found.extend(scope_idents(s).into_iter().cloned());
                            }
                            _ => {}
                        }
                    }
                }
                Decl::Check(c) if names(&c.on.relation) => {
                    if let Some(sel) = &c.on.sel {
                        found.extend(sel_idents(sel).into_iter().cloned());
                    }
                }
                _ => {}
            }
            for mut id in found {
                let range = self.src.local(j, id.span);
                id.span = Span::new(range.start, range.end);
                f(j, id);
            }
        }
    }

    fn script(&mut self, w: &ast::Write) -> R<()> {
        let (schema, program) = self.checked()?;
        check::check_adhoc(
            &schema,
            &program,
            &[ast::AdhocItem::Write(w.clone())],
            &BTreeMap::new(),
        )
        .map(|_| ())
    }

    fn log_grant(&mut self, privileges: &[ast::Privilege]) -> R<()> {
        let schema = self.schema()?;
        let vis = crate::resolve::Visible::module(
            schema.module_named(&self.module).unwrap_or(schema::PRELUDE),
        );
        let relation = |r: &ast::RelationRef| -> R<()> {
            let rel = match crate::resolve::lookup_value(&schema, &vis, &r.relation) {
                Ok(ValueDef::Relation(rel)) => rel,
                Ok(_) => {
                    return self.err_at(
                        r.span,
                        format!("`{}` is not a relation", r.relation.name.text),
                    );
                }
                Err(e) => return self.err_at(r.span, e),
            };
            if schema.module(schema.relation(rel).module).engine {
                return self.err_at(
                    r.span,
                    "a grant on a `catalog::` or `sys::` relation is deployment state, an ad hoc statement, never an operator in a migration",
                );
            }
            if let Some(sel) = &r.sel {
                schema
                    .relation_sel(rel, sel)
                    .map_err(|e| vec![Diag::new(r.span, e)])?;
            }
            Ok(())
        };
        for p in privileges {
            match p {
                ast::Privilege::Read(r, _)
                | ast::Privilege::Insert(r)
                | ast::Privilege::Update(r, _)
                | ast::Privilege::Delete(r) => relation(r)?,
                ast::Privilege::Retag(a, b) => {
                    relation(a)?;
                    relation(b)?;
                }
                ast::Privilege::CallFn(f) => match crate::resolve::lookup_value(&schema, &vis, f) {
                    Ok(ValueDef::Function(_)) => {}
                    Ok(_) => {
                        return self
                            .err_at(f.span(), format!("`{}` is not a function", f.name.text));
                    }
                    Err(e) => return self.err_at(f.span(), e),
                },
                ast::Privilege::Call(..) => {
                    return self.err(
                        "a bundle call grant is deployment state, an ad hoc statement, never an operator in a migration",
                    );
                }
                ast::Privilege::Capability(c) => {
                    return self.err_at(
                        c.span,
                        format!(
                            "the capability grant `{}` is deployment state, an ad hoc statement, never an operator in a migration",
                            c.text
                        ),
                    );
                }
            }
        }
        Ok(())
    }

    /// The state a lifecycle operator names, and how a refusal names it.
    fn stage_key(&self, t: &CtorTarget) -> R<(bool, StateKey, String)> {
        let schema = self.schema()?;
        match t {
            CtorTarget::Relation(rr) => {
                let rel = self.rel_id(&schema, &rr.relation)?;
                let r = schema.relation(rel);
                let def = schema.ty(r.elem_def);
                let c = match &rr.sel {
                    Some(CtorSel::One(c)) if def.ctor_index(&c.text).is_some() => c,
                    _ => {
                        return self.err_at(
                            rr.span,
                            "a lifecycle operator names one constructor: `r.C` or `T.C`",
                        );
                    }
                };
                Ok((
                    true,
                    (self.module_of(&rr.relation), r.name.clone(), c.text.clone()),
                    format!("{}.{}", r.name, c.text),
                ))
            }
            CtorTarget::Type(p) => {
                let def = self.type_id(&schema, &p.ty)?;
                let t = schema.ty(def);
                if t.ctor_index(&p.ctor.text).is_none() {
                    return self.err_at(
                        p.ctor.span,
                        format!("`{}` is not a constructor of `{}`", p.ctor.text, t.name),
                    );
                }
                Ok((
                    false,
                    (self.module_of(&p.ty), t.name.clone(), p.ctor.text.clone()),
                    format!("{}.{}", t.name, p.ctor.text),
                ))
            }
        }
    }

    fn stage(&mut self, t: &CtorTarget, verb: &str) -> R<()> {
        let (relation, key, shown) = self.stage_key(t)?;
        let map = if relation {
            &mut self.src.states
        } else {
            &mut self.src.value_states
        };
        let current = map.get(&key).map_or(Stage::Enabled, |s| s.stage);
        let next = match (verb, current) {
            ("enable", Stage::Enabled) => {
                return self.err(format!("`{shown}` is already enabled"));
            }
            ("enable", _) => Stage::Enabled,
            ("disable", Stage::Enabled) => Stage::Declared,
            ("disable", s) => {
                return self.err(format!(
                    "`disable` moves an enabled constructor back to declared; `{shown}` is {}",
                    s.name()
                ));
            }
            ("deprecate", Stage::Enabled) => Stage::Deprecated,
            (_, s) => {
                return self.err(format!(
                    "`deprecate` applies only to an enabled constructor; `{shown}` is {}",
                    s.name()
                ));
            }
        };
        let since = self.src.version;
        let map = if relation {
            &mut self.src.states
        } else {
            &mut self.src.value_states
        };
        map.insert(key, State { stage: next, since });
        Ok(())
    }
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
