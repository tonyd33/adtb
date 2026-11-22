use std::cell::RefCell;

use super::lift::Lifting;
use super::op::{Res, broken, refuse};
use super::slots::{Found, default_refs, implied_key};
use super::*;
use crate::ast::{self, MigrationFile};
use crate::canon::{self, Names, canonical_term};
use crate::check;
use crate::evolve::{SourceDecl, decl_exprs, each_type, walk_type};
use crate::ids::{CatalogIds, objects, placing};
use crate::lex::{Tok, lex};
use crate::schema::{self, Adder, Ahead, AheadState, PRELUDE, Schema};
use crate::span::{FileDiag, FileId, Span};
use crate::types::ModuleId;

/// The migration being applied.
pub(super) struct Run<'a> {
    module: &'a str,
    text: &'a str,
    /// The operator being applied, and its span.
    at: usize,
    span: Span,
    /// Each declaration or field an operator wrote naming what only the migration's end
    /// resolves, by the operator.
    late: RefCell<Vec<(usize, Late)>>,
}

/// What an operator wrote naming what only the migration's end resolves.
#[derive(Clone, Copy, Debug)]
pub(super) enum Late {
    /// A declaration it added.
    Decl(Item),
    /// A field it added, or whose delete action it altered when `action_only`.
    Field { field: u64, action_only: bool },
    /// A constructor it added or made, from its name through its record when `record`.
    Ctor { ctor: u64, record: bool },
    /// A default it added, by its relation variable and id.
    Default { relation: u64, default: u64 },
    /// A computed field it added, by its relation variable and id.
    Computed { relation: u64, computed: u64 },
    /// A clause it added or changed, whose resolved form the model does not give late.
    Unresolved,
    /// A declaration it added while the declarations did not build, by its key in
    /// [`Model::held`].
    Held(u64),
}

impl<'a> Run<'a> {
    /// Returns the run of migration `m`, whose text is `text`, before its first operator.
    pub(super) fn new(m: &'a MigrationFile, text: &'a str) -> Run<'a> {
        Run {
            module: m.module.text.as_str(),
            text,
            at: 0,
            span: Span { start: 0, end: 0 },
            late: RefCell::default(),
        }
    }

    /// Note that the operator being applied wrote `late`. Returns the empty text its canonical
    /// form holds until the migration's end gives it.
    pub(super) fn late(&self, late: Late) -> String {
        self.late.borrow_mut().push((self.at, late));
        String::new()
    }

    /// Start applying operator `at`, whose span is `span`.
    pub(super) fn begin(&mut self, at: usize, span: Span) {
        self.at = at;
        self.span = span;
    }

    /// Forget what operator `at` wrote naming what only the migration's end resolves.
    pub(super) fn forget_late(&self, at: usize) {
        self.late.borrow_mut().retain(|(k, _)| *k != at);
    }

    /// Returns what the operators wrote naming what only the migration's end resolves.
    pub(super) fn take_late(&self) -> Vec<(usize, Late)> {
        self.late.take()
    }

    /// Returns the module of the migration.
    pub(super) fn module(&self) -> &'a str {
        self.module
    }

    /// Returns the text of the migration.
    pub(super) fn text(&self) -> &'a str {
        self.text
    }

    /// Returns the span of the operator being applied.
    pub(super) fn span(&self) -> Span {
        self.span
    }

    /// Returns the relation variables the builder takes as ones a later operator adds, from
    /// `m`'s.
    pub(super) fn ahead(&self, m: &Model) -> Vec<Ahead> {
        m.ahead
            .iter()
            .filter_map(|e| {
                let (module, relation, state) = match e.added {
                    Added::Not => (e.module.clone(), e.name.clone(), AheadState::Pending),
                    Added::Applying => {
                        (e.module.clone(), e.name.clone(), AheadState::UnlessDeclared)
                    }
                    Added::As(r) if e.typed => (
                        m.module_of((Space::Shared, r))?.to_string(),
                        m.relations.get(&r)?.name.clone(),
                        AheadState::Declared,
                    ),
                    Added::As(_) => return None,
                };
                Some(Ahead::new(module, relation, e.typed, state, e.by))
            })
            .collect()
    }
}

/// A schema built from a model's declarations and the files and texts it was built from.
pub(super) struct Built {
    schema: Schema,
    files: Vec<(FileId, ast::SchemaFile)>,
    texts: Vec<String>,
}

impl Built {
    pub(super) fn schema(&self) -> &Schema {
        &self.schema
    }

    pub(super) fn into_schema(self) -> Schema {
        self.schema
    }

    /// Returns every object the built declarations declare, in creation order.
    pub(super) fn objects(&self) -> Vec<crate::ids::Object> {
        objects(&self.schema, &self.files, &|f, s| {
            span_text(&self.texts[f.0 as usize], s)
        })
    }

    /// Returns the text of file `file`, its module header included.
    pub(super) fn text(&self, file: FileId) -> &str {
        &self.texts[file.0 as usize]
    }

    /// Returns the span of the relation variable name of each `Key` type of file `file` that
    /// names one a later operator adds, with the module of that relation variable and what adds
    /// it.
    fn ahead(&self, file: FileId) -> Vec<(Span, ModuleId, Adder)> {
        let (Some(d), Some(module)) = (
            self.decl(file),
            self.files
                .iter()
                .find(|(f, _)| *f == file)
                .and_then(|(_, sf)| self.schema.module_named(&sf.module.text)),
        ) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        each_written_type(d, &mut |t| {
            let ast::TypeKind::Key(_, r) = &t.kind else {
                return;
            };
            let m = match &r.relation.module {
                Some(q) => self.schema.module_named(&q.text),
                None => Some(module),
            };
            let Some(m) = m else { return };
            let name = &r.relation.name;
            let found = self
                .schema
                .ahead
                .iter()
                .find(|(am, n, _)| *am == m && *n == name.text);
            if let Some((_, _, by)) = found
                && !self
                    .schema
                    .relations
                    .iter()
                    .any(|x| x.module == m && x.name == name.text)
            {
                out.push((name.span, m, *by));
            }
        });
        out
    }

    /// Returns the names of file `file`, as [`Source::names`] gives them.
    pub(super) fn names(&self, file: FileId) -> Names {
        let mut names = Names::default();
        for (f, sf) in &self.files {
            if *f != file {
                continue;
            }
            let module = self.schema.module_named(&sf.module.text).unwrap_or(PRELUDE);
            for d in &sf.decls {
                canon::decl_names(&self.schema, module, d, &mut names);
            }
        }
        for (f, span, r) in check::schema_refs(&self.schema) {
            if f == file {
                names.note(&self.schema, span, &r);
            }
        }
        for (span, m, _) in self.ahead(file) {
            names.qualify(&self.schema, span, m);
        }
        names
    }

    pub(super) fn ahead_ranges(&self, file: FileId) -> Vec<(std::ops::Range<usize>, Adder)> {
        self.ahead(file)
            .into_iter()
            .map(|(span, _, k)| (span.range(), k))
            .collect()
    }

    pub(super) fn decl(&self, file: FileId) -> Option<&ast::Decl> {
        self.files
            .iter()
            .find(|(f, _)| *f == file)
            .and_then(|(_, sf)| sf.decls.first())
    }

    /// Returns the canonical form of file `file`'s declaration, names as they resolve there,
    /// `None` when a name of it does not resolve there.
    pub(super) fn canonical(&self, file: FileId) -> Res<Option<String>> {
        let Some(d) = self.decl(file) else {
            return broken("a declaration the model printed does not parse");
        };
        Ok(canonical_term(self.text(file), d.span(), &self.names(file)).ok())
    }

    /// Returns the errors of checking the schema in file `file`.
    pub(super) fn errors_in(&self, file: FileId) -> Vec<String> {
        match check::check_schema(&self.schema) {
            Ok(_) => Vec::new(),
            Err(ds) => ds
                .into_iter()
                .filter(|d| d.file == file)
                .map(|d| d.diag.message)
                .collect(),
        }
    }
}

fn span_text(text: &str, span: Span) -> String {
    let piece = &text[span.range()];
    lex(piece)
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| piece[t.span.range()].to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Model {
    /// Returns the catalog ids of `schema`, built from `files` whose texts are `texts`: each
    /// object a declaration of the model holds the model's id, and every other object of a user
    /// module the next id of its space `next` gives, in creation order.
    fn assigned(
        &self,
        schema: &Schema,
        files: &[(FileId, ast::SchemaFile)],
        texts: &[String],
        next: &mut BTreeMap<Space, u64>,
    ) -> CatalogIds {
        let mut out = schema.ids().clone();
        let text = |f: FileId, s: Span| span_text(&texts[f.0 as usize], s);
        let objs = objects(schema, files, &text);
        let placed: BTreeMap<crate::ids::Slot, u64> = self.placed(schema).into_iter().collect();
        let ids = placing(&objs, &|s| placed.get(s).copied(), &self.held_ids, next);
        for o in &objs {
            out.forget(o.slot());
        }
        for (o, id) in objs.iter().zip(ids) {
            out.set(o.slot(), id);
        }
        out
    }

    /// Build the schema of the declarations and `more` after them. Returns it with the id
    /// counters after the objects no declaration holds an id for took theirs.
    pub(super) fn built(
        &self,
        more: &[SourceDecl],
        ahead: &[Ahead],
    ) -> Result<(Built, BTreeMap<Space, u64>), Vec<FileDiag>> {
        let files = self.files(more)?;
        let held = self.printed().expect("the files printed");
        let texts: Vec<String> = held
            .iter()
            .chain(more)
            .map(|d| format!("module {}\n{}", d.module, d.text))
            .collect();
        let next = RefCell::new(self.next.clone());
        let schema = schema::build_with(
            &files,
            ahead,
            &|s| self.assigned(s, &files, &texts, &mut next.borrow_mut()),
            &self.schema_states(),
        )?;
        Ok((
            Built {
                schema,
                files,
                texts,
            },
            next.into_inner(),
        ))
    }

    /// Build as `built` does, the objects no declaration holds an id for taking theirs, and
    /// give every clause of the model the id the schema gives it.
    pub(super) fn build_adopting(
        &mut self,
        more: &[SourceDecl],
        ahead: &[Ahead],
    ) -> Result<Built, Vec<FileDiag>> {
        let (built, next) = self.built(more, ahead)?;
        self.next = next;
        self.lift_held(&built);
        self.adopt(built.schema());
        Ok(built)
    }

    /// Lift each held declaration in its place from `built`, which the declarations built, and lift
    /// again each declaration whose terms an operator wrote as text.
    ///
    /// Panics when a declaration the model printed does not lift from its own build.
    fn lift_held(&mut self, built: &Built) {
        for i in 0..self.decls.len() {
            let Item::Held(_, key) = self.decls[i].item else {
                continue;
            };
            let file = FileId(i as u32);
            let d = built
                .decl(file)
                .expect("a held declaration the declarations built parses");
            let names = built.names(file);
            let ahead = built.ahead_ranges(file);
            let module = self.decls[i].module.clone();
            Lifting::new(
                built.schema(),
                file,
                built.text(file),
                &names,
                &ahead,
                &module,
            )
            .decl(self, d)
            .unwrap_or_else(|e| panic!("a held declaration lifts once it builds: {e}"));
            let lifted = self.decls.pop().expect("the lifted declaration");
            self.decls[i] = lifted.clone();
            self.held.remove(&key);
            self.lifted.insert(key, lifted.item);
            self.enable(lifted.item, built, file);
        }
        self.held_ids.clear();
        for item in std::mem::take(&mut self.unlifted) {
            if self.decl_at(item).is_some() {
                self.relift(built, item).unwrap_or_else(|e| {
                    panic!("a declaration written as text lifts once it builds: {e:?}")
                });
            }
        }
    }

    /// Give each clause of each relation variable, and each index its reference fields imply,
    /// the id `schema` gives it.
    fn adopt(&mut self, schema: &Schema) {
        let ids = schema.ids();
        for i in 0..self.decls.len() {
            let Item::Relation(r) = self.decls[i].item else {
                continue;
            };
            let Some(rel) = Found::new(schema, FileId(i as u32)).relation() else {
                continue;
            };
            let field_id = |f| ids.field_id(f).map(u64::from);
            let m = self
                .relations
                .get_mut(&r)
                .expect("a declared relation variable");
            let (mut ix, mut dx, mut cx) = (0, 0, 0);
            for c in &mut m.clauses {
                match c {
                    ClauseM::Key { id, generate, .. } => {
                        *id = ids.key_id(rel).map(u64::from);
                        if let Some((_, g)) = generate {
                            *g = ids.generated_id(rel).map(u64::from);
                        }
                    }
                    ClauseM::Index { id, .. } => {
                        *id = ids.index_id(rel, ix).map(u64::from);
                        ix += 1;
                    }
                    ClauseM::Default { ids: held, .. } => {
                        *held = default_refs(schema, rel, dx)
                            .into_iter()
                            .filter_map(|f| {
                                let d = ids.clause_default_ids(rel, dx).find(|(x, _)| *x == f)?.1;
                                Some((field_id(f)?, d.raw()))
                            })
                            .collect();
                        dx += 1;
                    }
                    ClauseM::Computed { id, .. } => {
                        *id = ids.computed_id(rel, cx).map(u64::from);
                        cx += 1;
                    }
                    ClauseM::Volatile => {}
                }
            }
            m.implied = schema
                .implied_indexes(rel)
                .into_iter()
                .filter_map(|x| {
                    let f = x.field_ref();
                    Some((
                        implied_key(schema, &x, |f| field_id(*f))?,
                        ids.implied_id(rel, f)?.raw(),
                    ))
                })
                .collect();
        }
    }

    /// Returns the schema of the declarations.
    pub(super) fn schema(&self, ahead: &[Ahead]) -> Result<Schema, Vec<FileDiag>> {
        self.built(&[], ahead).map(|(b, _)| b.into_schema())
    }

    /// Returns whether the declarations check: they name no object that is gone, and build and
    /// check.
    pub(super) fn checks(&self, ahead: &[Ahead]) -> Result<bool, String> {
        if self.dropped_names() {
            return Ok(false);
        }
        Ok(self
            .built(&[], ahead)
            .is_ok_and(|(b, _)| check::check_schema(&b.schema).is_ok()))
    }

    /// Returns why the declarations do not check, empty when they do.
    pub(super) fn unchecked(&self) -> String {
        if self.dropped_names() {
            return "a declaration names a dropped object".into();
        }
        match self.built(&[], &[]) {
            Err(ds) => ds[0].diag.message.clone(),
            Ok((b, _)) => match check::check_schema(&b.schema) {
                Err(ds) => ds[0].diag.message.clone(),
                Ok(_) => String::new(),
            },
        }
    }

    /// Build as `build_adopting` does, refusing the operator when the declarations do not build.
    pub(super) fn built_for(&mut self, run: &Run) -> Res<Built> {
        let ahead = run.ahead(self);
        self.build_adopting(&[], &ahead).or_else(|ds| unbuilt(&ds))
    }

    /// Returns the schema the model builds now, for an operator's preconditions, refusing the
    /// operator when the declarations do not build.
    pub(super) fn schema_now(&self, run: &Run) -> Res<Built> {
        match self.built(&[], &run.ahead(self)) {
            Ok((b, _)) => Ok(b),
            Err(ds) => unbuilt(&ds),
        }
    }
}

/// Refuse an operator that reads declarations failing with `ds`, by the first failure.
pub(super) fn unbuilt<T>(ds: &[FileDiag]) -> Res<T> {
    refuse(ds[0].diag.message.clone())
}

/// Call `f` on every type expression of a declaration and of the expressions it holds: lambda
/// parameters and type calls.
pub(super) fn each_written_type<'a>(d: &'a ast::Decl, f: &mut dyn FnMut(&'a ast::Type)) {
    each_type(d, f);
    for e in decl_exprs(d) {
        crate::visit::each_expr(e, &mut |x| match &x.kind {
            ast::ExprKind::TypeCall { ty, .. } => walk_type(ty, f),
            ast::ExprKind::Lambda(ast::LambdaParams::List(ps), _) => {
                ps.iter().for_each(|p| walk_type(&p.ty, f));
            }
            _ => {}
        });
    }
}
