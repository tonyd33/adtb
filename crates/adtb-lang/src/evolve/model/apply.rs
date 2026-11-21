use std::cell::RefCell;

use super::field::Maps;
use super::lift::Lifting;
use super::op::{Fail, Kind, Op, Res, Resolving, StateKey, broken, refuse, unmodelled};
use super::slots::{Found, default_refs, implied_key};
use super::*;
use crate::ast::{self, MigrationFile, OperatorKind};
use crate::canon::{self, Names, canonical_term};
use crate::check;
use crate::evolve::grant::IdKind;
use crate::evolve::source::rekey_leaves_no_old_reference;
use crate::evolve::{
    Clause, Resolved, ResolvedOp, SourceDecl, Target, each_written_type, mentions_def, mentions_rel,
};
use crate::ids::{CatalogIds, objects};
use crate::lex::{Tok, lex};
use crate::schema::{self, Adder, Ahead, AheadState, PRELUDE, Schema, Stage};
use crate::span::{FileDiag, FileId, Span};
use crate::types::ModuleId;

/// A migration applied to a model.
pub(in crate::evolve) struct Migrated {
    model: Model,
    /// For each operator applied, its resolved form, or why it was refused.
    ops: Vec<Result<ResolvedOp, String>>,
    /// For each operator applied, the row maps of its one step, or why they fail.
    maps: Vec<Maps>,
    /// For each operator applied, the model it leaves.
    afters: Vec<Model>,
    /// Whether the result checks; false when an operator stopped the run.
    checks: bool,
    /// The kind of the first operator the model does not apply, which stops the run there.
    stopped: Option<String>,
}

impl Migrated {
    pub(super) fn model(&self) -> &Model {
        &self.model
    }

    /// Returns, for each operator applied, its resolved form, or why it was refused.
    pub(super) fn ops(&self) -> &[Result<ResolvedOp, String>] {
        &self.ops
    }

    /// Returns, for each operator applied, the model it leaves.
    pub(super) fn afters(&self) -> &[Model] {
        &self.afters
    }

    /// Returns the kind of the operator that stopped the run, if one did.
    pub(super) fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// Returns, for each operator applied, the row maps of its one step, or why they fail.
    pub(super) fn maps(&self) -> &[Maps] {
        &self.maps
    }

    /// Returns whether the result checks.
    pub(super) fn checks(&self) -> bool {
        self.checks
    }
}

/// A relation variable an operator of the migration being applied adds.
struct Expected {
    module: String,
    relation: String,
    /// Whether the migration also adds its type.
    typed: bool,
    by: usize,
    added: Added,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Added {
    Not,
    Applying,
    As(u64),
}

/// The migration being applied.
pub(super) struct Run<'a> {
    module: &'a str,
    text: &'a str,
    expected: Vec<Expected>,
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
    /// A clause it added or changed, whose resolved form the model does not give late.
    Unresolved,
}

impl<'a> Run<'a> {
    /// Note that the operator being applied wrote `late`. Returns the empty text its canonical
    /// form holds until the migration's end gives it.
    pub(super) fn late(&self, late: Late) -> String {
        self.late.borrow_mut().push((self.at, late));
        String::new()
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

    /// Returns the relation variables the builder takes as ones a later operator adds.
    pub(super) fn ahead(&self, m: &Model) -> Vec<Ahead> {
        self.expected
            .iter()
            .filter_map(|e| {
                let (module, relation, state) = match e.added {
                    Added::Not => (e.module.clone(), e.relation.clone(), AheadState::Pending),
                    Added::Applying => (
                        e.module.clone(),
                        e.relation.clone(),
                        AheadState::UnlessDeclared,
                    ),
                    Added::As(r) if e.typed => (
                        m.module_of((Space::Shared, r))?.to_string(),
                        m.relations.get(&r)?.name.clone(),
                        AheadState::Declared,
                    ),
                    Added::As(_) => return None,
                };
                Some(Ahead::new(
                    module,
                    relation,
                    e.typed,
                    state,
                    Adder::Operator(e.by),
                ))
            })
            .collect()
    }

    fn mark(&mut self, k: usize, added: Added) {
        for e in self.expected.iter_mut().filter(|e| e.by == k) {
            e.added = added;
        }
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

    /// Returns the text of file `file`, its module header included.
    pub(super) fn text(&self, file: FileId) -> &str {
        &self.texts[file.0 as usize]
    }

    /// Returns the span of the relation variable name of each `Key` type of file `file` that
    /// names one a later operator adds, with the module of that relation variable and the
    /// operator.
    fn ahead(&self, file: FileId) -> Vec<(Span, ModuleId, usize)> {
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
            if let Some((_, _, Adder::Operator(k))) = found
                && !self
                    .schema
                    .relations
                    .iter()
                    .any(|x| x.module == m && x.name == name.text)
            {
                out.push((name.span, m, *k));
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

    pub(super) fn ahead_ranges(&self, file: FileId) -> Vec<(std::ops::Range<usize>, usize)> {
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
    fn canonical(&self, file: FileId) -> Res<Option<String>> {
        let Some(d) = self.decl(file) else {
            return broken("a declaration the model printed does not parse");
        };
        Ok(canonical_term(self.text(file), d.span(), &self.names(file)).ok())
    }

    /// Returns the errors of checking the schema in file `file`.
    fn errors_in(&self, file: FileId) -> Vec<String> {
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
        for o in &objs {
            out.forget(o.slot());
        }
        for o in &objs {
            let id = match placed.get(o.slot()) {
                Some(&id) => id,
                None => {
                    let n = next.entry(o.space()).or_insert(FIRST_LOG_ID);
                    *n += 1;
                    *n - 1
                }
            };
            out.set(o.slot(), id);
        }
        out
    }

    /// Build the schema of the declarations and `more` after them. Returns it with the id
    /// counters after the objects no declaration holds an id for took theirs.
    fn built(
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
        self.adopt(&built.schema);
        Ok(built)
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
        self.built(&[], ahead).map(|(b, _)| b.schema)
    }

    /// Returns whether the declarations check: they name no object that is gone, and build and
    /// check.
    fn checks(&self, ahead: &[Ahead]) -> Result<bool, String> {
        if self.dropped_names() {
            return Ok(false);
        }
        Ok(self
            .built(&[], ahead)
            .is_ok_and(|(b, _)| check::check_schema(&b.schema).is_ok()))
    }

    /// Apply migration `m`, whose text is `text`, numbered one past the model's version.
    ///
    /// Fail with why the model does not apply it.
    pub(in crate::evolve) fn migrate(
        &self,
        m: &MigrationFile,
        text: &str,
    ) -> Result<Migrated, String> {
        let module = m.module.text.as_str();
        let types: BTreeSet<&str> = m
            .operators
            .iter()
            .filter_map(|op| match &op.kind {
                OperatorKind::AddType(t) => Some(t.name.text.as_str()),
                _ => None,
            })
            .collect();
        let expected = m
            .operators
            .iter()
            .enumerate()
            .filter_map(|(k, op)| match &op.kind {
                OperatorKind::AddRelation(r) => Some(Expected {
                    module: module.to_string(),
                    relation: r.name.name.text.clone(),
                    typed: matches!(
                        &r.ty.kind,
                        ast::TypeKind::Named { name, .. }
                            if name.module.as_ref().is_none_or(|m| m.text == module)
                                && types.contains(name.name.text.as_str())
                    ),
                    by: k,
                    added: Added::Not,
                }),
                _ => None,
            })
            .collect();
        let mut run = Run {
            module,
            text,
            expected,
            at: 0,
            span: Span { start: 0, end: 0 },
            late: RefCell::default(),
        };
        let mut model = self.clone();
        model.version += 1;
        let mut ops = Vec::new();
        let mut maps = Vec::new();
        let mut afters = Vec::new();
        let mut stopped = None;
        let mut rekeyed = Vec::new();
        for (k, op) in m.operators.iter().enumerate() {
            run.mark(k, Added::Applying);
            run.at = k;
            run.span = op.span;
            let mut next = model.clone();
            match next.operator(&run, &op.kind) {
                Ok((r, m)) => {
                    if let OperatorKind::Rekey { relation, .. } = &op.kind {
                        rekeyed.push((k, relation.clone(), op.span));
                    }
                    maps.push(m);
                    if let Resolved::AddRelation { relation, .. } = r.op {
                        run.mark(k, Added::As(relation));
                        next.terms_mut(&mut |t| t.added(k, relation));
                    }
                    next.rebind(&model);
                    model = next;
                    ops.push(Ok(r));
                }
                Err(Fail::Refused(why)) => {
                    maps.push(Ok(Vec::new()));
                    run.late.borrow_mut().retain(|(at, _)| *at != k);
                    run.mark(k, Added::Not);
                    ops.push(Err(why));
                }
                Err(Fail::Unmodelled(kind)) => {
                    stopped = Some(kind);
                    break;
                }
                Err(Fail::Broken(why)) => return Err(why),
            }
            afters.push(model.clone());
        }
        let late = run.late.take();
        let mut checks = false;
        if stopped.is_none() {
            checks =
                (late.is_empty() || model.resolve_late(&late, &mut ops)?) && model.checks(&[])?;
            if checks && !rekeyed.is_empty() {
                match model.built(&[], &[]) {
                    Ok((built, _)) => {
                        let module = m.module.text.as_str();
                        for (k, relation, span) in &rekeyed {
                            if let Err(d) = rekey_leaves_no_old_reference(
                                &built.schema,
                                module,
                                relation,
                                *span,
                            ) {
                                ops[*k] = Err(d.message);
                            }
                        }
                    }
                    Err(_) => {
                        stopped =
                            Some("a rekey's end check over declarations that do not build".into());
                    }
                }
            }
            if checks && late.iter().any(|(_, l)| matches!(l, Late::Unresolved)) {
                stopped = Some("a clause naming what only the migration's end resolves".into());
            }
        }
        Ok(Migrated {
            model,
            ops,
            maps,
            afters,
            checks,
            stopped,
        })
    }

    /// Lift again each declaration of `late` from the declarations as the migration leaves them,
    /// and give the operator that wrote it, or a field of it, the canonical form it has there.
    /// Returns false when the declarations do not build or a late name does not resolve.
    fn resolve_late(
        &mut self,
        late: &[(usize, Late)],
        ops: &mut [Result<ResolvedOp, String>],
    ) -> Result<bool, String> {
        let Ok(built) = self.build_adopting(&[], &[]) else {
            return Ok(false);
        };
        let broken = |f: Fail| match f {
            Fail::Broken(why) | Fail::Refused(why) | Fail::Unmodelled(why) => why,
        };
        for &(k, late) in late {
            let canonical = match late {
                Late::Decl(item) => {
                    let Some(i) = self.decl_at(item) else {
                        continue;
                    };
                    let file = FileId(i as u32);
                    let d = built
                        .decl(file)
                        .ok_or("a declaration the model printed does not parse")?;
                    let names = built.names(file);
                    Lifting::new(
                        &built.schema,
                        file,
                        built.text(file),
                        &names,
                        &[],
                        &self.decls[i].module.clone(),
                    )
                    .decl(self, d)?;
                    self.decls.pop();
                    built.canonical(file).map_err(broken)?
                }
                Late::Field { field, action_only } => {
                    let Some(t) = self.field_type(field) else {
                        continue;
                    };
                    let owner = self.fields[&field].owner;
                    let ctor = self.ctors.get(&owner).map(|c| c.name.clone());
                    let i = self.decl_at(Item::Type(t)).expect("a declared type");
                    Self::field_canonical(
                        &built,
                        FileId(i as u32),
                        ctor.as_deref(),
                        &self.fields[&field].name,
                        action_only,
                    )
                    .map_err(broken)?
                }
                Late::Unresolved => continue,
            };
            let Some(canonical) = canonical else {
                return Ok(false);
            };
            if let Some(Ok(op)) = ops.get_mut(k) {
                set_decl(&mut op.op, canonical);
            }
        }
        Ok(true)
    }

    /// Apply one operator. Returns it resolved.
    fn operator(&mut self, run: &Run, op: &OperatorKind) -> Res<(ResolvedOp, Maps)> {
        let structural = match op {
            OperatorKind::AddField {
                path,
                ty,
                on_delete,
                decl,
                fill,
            } => {
                let action = on_delete
                    .as_ref()
                    .map(|_| action_span(run.text, ty.span, *decl));
                Some(self.add_field(run, path, ty.span, action, fill)?)
            }
            OperatorKind::DropField(path) => Some(self.drop_field(run, path)?),
            OperatorKind::AlterField { path, span, .. } => {
                Some(self.alter_field(run, path, *span)?)
            }
            OperatorKind::AddUnique(fields, on) => {
                Some(self.add_index(run, IndexKind::Unique, fields, on)?)
            }
            OperatorKind::AddIndex {
                interval,
                fields,
                on,
            } => Some(self.add_index(run, index_kind(*interval), fields, on)?),
            OperatorKind::DropUnique(fields, on) => {
                Some(self.drop_index(run, IndexKind::Unique, fields, on)?)
            }
            OperatorKind::DropIndex {
                interval,
                fields,
                on,
            } => Some(self.drop_index(run, index_kind(*interval), fields, on)?),
            OperatorKind::AddDefault {
                relation,
                ctor,
                field,
                value,
            } => Some(self.add_default(run, relation, ctor.as_ref(), field, value)?),
            OperatorKind::DropDefault {
                relation,
                ctor,
                field,
            } => Some(self.drop_default(run, relation, ctor.as_ref(), field)?),
            OperatorKind::AddComputed {
                relation,
                field,
                ty,
                expr,
            } => Some(self.add_computed(run, relation, field, ty, expr)?),
            OperatorKind::RedefineComputed {
                relation,
                field,
                ty,
                expr,
            } => Some(self.redefine_computed(run, relation, field, ty, expr)?),
            OperatorKind::DropComputed(relation, field) => {
                Some(self.drop_computed(run, relation, field)?)
            }
            OperatorKind::AddConstructor {
                path,
                record,
                place,
                joins,
                arms,
            } => Some(self.add_constructor(
                run,
                path,
                record.as_deref(),
                place.as_ref(),
                joins,
                arms,
            )?),
            OperatorKind::MoveField {
                path,
                to,
                sel,
                fill,
            } => Some(self.move_field(run, path, to, sel.as_ref(), fill.as_ref())?),
            OperatorKind::Rekey {
                relation,
                key,
                carry,
            } => Some(self.rekey(run, relation, key, carry)?),
            OperatorKind::RemoveConstructor(p) => Some(self.remove_constructor(run, p)?),
            OperatorKind::Grant(g) => Some(self.grant(run, g)?),
            OperatorKind::Revoke(r) => Some(self.revoke(run, r)?),
            OperatorKind::Script(w) => Some(self.script(run, w)?),
            OperatorKind::Install {
                name,
                version,
                from,
            } => Some(self.install(run, name, version, from)?),
            OperatorKind::Uninstall(m) => Some(self.uninstall(&m.text)?),
            OperatorKind::AdoptTzdb { release, from } => Some(self.adopt_tzdb(release, from)?),
            _ => None,
        };
        if let Some((op, clauses, maps)) = structural {
            self.settle();
            return Ok((ResolvedOp { op, clauses }, maps));
        }
        if matches!(
            op,
            OperatorKind::RenameConstructor(..)
                | OperatorKind::Enable(_)
                | OperatorKind::Disable(_)
                | OperatorKind::Deprecate(_)
        ) {
            self.schema_now(run)?;
        }
        let resolved = Resolving::new(self, run.module).op(op)?;
        let out = match resolved {
            Op::AddModule(m) => self.add_module(m)?,
            Op::DropModule(m) => self.drop_module(m)?,
            Op::Add { kind, name, span } => self.add(run, kind, &name, span, op)?,
            Op::RedefineFn { function, span } => self.redefine_fn(run, function, span, op)?,
            Op::Drop(item) => self.drop(run, item)?,
            Op::Rename { obj, to } => self.rename(run, obj, to)?,
            Op::Stage {
                key,
                to,
                verb,
                shown,
            } => self.stage(run, key, to, verb, &shown)?,
        };
        self.settle();
        Ok((
            ResolvedOp {
                op: out.0,
                clauses: out.1,
            },
            Ok(Vec::new()),
        ))
    }

    /// Forget the write states and grants of objects that are gone.
    fn settle(&mut self) {
        let ctors = &self.ctors;
        let rels = &self.relations;
        let stores = &self.stores;
        self.states.retain(|(r, c), _| {
            (rels.contains_key(r) || stores.contains_key(r))
                && (ctors.contains_key(c) || *c < FIRST_LOG_ID)
        });
        self.value_states.retain(|c, _| ctors.contains_key(c));
        let live = |k: IdKind, x: u64| match k {
            IdKind::Relation => self.relations.contains_key(&x) || self.stores.contains_key(&x),
            IdKind::Ctor => self.ctors.contains_key(&x),
            IdKind::Set => self.sets.contains_key(&x),
            IdKind::Field => self.fields.contains_key(&x),
            IdKind::Function => self.functions.contains_key(&x),
        };
        self.grants = crate::evolve::carry::pruned(&self.grants, &live);
    }

    /// Refuse a module named `m` added: a built-in module's name, or one taken.
    pub(super) fn reserved(&self, m: &str) -> Res<()> {
        let Ok(schema) = schema::build(&[]) else {
            return broken("the built-in modules do not build");
        };
        let built_in = schema
            .modules
            .iter()
            .enumerate()
            .any(|(i, d)| (i == PRELUDE.0 as usize || d.engine) && d.name == m);
        if built_in {
            return refuse(format!("no module may be named `{m}`"));
        }
        if self.modules.contains(m) {
            return refuse(format!("module `{m}` already exists"));
        }
        Ok(())
    }

    fn add_module(&mut self, m: String) -> Res<(Resolved, Vec<Clause>)> {
        self.reserved(&m)?;
        self.modules.insert(m.clone());
        Ok((Resolved::AddModule { module: m }, Vec::new()))
    }

    fn drop_module(&mut self, m: String) -> Res<(Resolved, Vec<Clause>)> {
        if !self.modules.contains(&m) {
            return refuse(format!("no module `{m}`"));
        }
        if let Some(i) = self.decls.iter().position(|d| d.module == m) {
            return refuse(format!(
                "`drop module {m}` is refused while the module holds {}",
                self.shown_decl(i)
            ));
        }
        self.modules.remove(&m);
        self.installed.remove(&m);
        Ok((Resolved::DropModule { module: m }, Vec::new()))
    }

    fn add(
        &mut self,
        run: &Run,
        kind: Kind,
        name: &str,
        span: Span,
        op: &OperatorKind,
    ) -> Res<(Resolved, Vec<Clause>)> {
        if !self.modules.contains(run.module) {
            return refuse(format!(
                "no module `{0}`; a migration of a new module starts with `add module {0}`",
                run.module
            ));
        }
        if self.find(run.module, kind, name).is_some() {
            return refuse(format!(
                "module `{}` already declares {} `{name}`",
                run.module,
                kind.word()
            ));
        }
        let at = FileId(self.decls.len() as u32);
        let decl = SourceDecl {
            module: run.module.to_string(),
            text: run.text[span.range()].to_string(),
        };
        let built = match self.build_adopting(std::slice::from_ref(&decl), &run.ahead(self)) {
            Ok(b) => b,
            Err(ds) => {
                if let Some(d) = ds.iter().find(|d| d.file == at) {
                    return refuse(d.diag.message.clone());
                }
                if kind == Kind::Relation {
                    return unbuilt(&ds);
                }
                return unmodelled("an added declaration in a source that does not build");
            }
        };
        if let Some(e) = built.errors_in(at).into_iter().next() {
            return refuse(e);
        }
        let Some(d) = built.decl(at) else {
            return broken("an added declaration that does not parse alone");
        };
        let names = built.names(at);
        let ahead = built.ahead_ranges(at);
        Lifting::new(
            &built.schema,
            at,
            built.text(at),
            &names,
            &ahead,
            run.module,
        )
        .decl(self, d)
        .map_err(Fail::Broken)?;
        let item = self.decls.last().expect("the added declaration").item;
        let decl = match built.canonical(at)? {
            Some(decl) => decl,
            None => run.late(Late::Decl(item)),
        };
        let since = self.version;
        let enabled = State {
            stage: Stage::Enabled,
            since,
        };
        let mut clauses = Vec::new();
        let resolved = match (item, op) {
            (Item::Type(t), _) => {
                if !matches!(self.types[&t].body, TypeBody::New { .. }) {
                    for c in self.ctors_of(t) {
                        self.value_states.insert(c, enabled);
                    }
                }
                Resolved::AddType { sum: t, decl }
            }
            (Item::Relation(r), _) => {
                let schema = &built.schema;
                if let Some(rel) = Found::new(schema, at).relation() {
                    let td = schema.ty(schema.relation(rel).elem_def);
                    if matches!(td.kind, schema::TypeDefKind::Sum { .. }) {
                        for c in td.ctor_refs() {
                            let Some(ctor) = schema.ids().ctor_id(c) else {
                                continue;
                            };
                            let stage = self
                                .value_states
                                .get(&ctor.raw())
                                .map_or(Stage::Enabled, |s| s.stage);
                            self.states.insert((r, ctor.raw()), State { stage, since });
                        }
                    }
                }
                Resolved::AddRelation { relation: r, decl }
            }
            (Item::Set(s), _) => {
                let set = &self.sets[&s];
                for m in &set.members {
                    if let Some((Space::Shared, c)) = m.objects().next() {
                        clauses.push(Clause::Member { ctor: c });
                    }
                }
                Resolved::AddSet {
                    set: s,
                    relation: set.relation,
                    name: set.name.clone(),
                }
            }
            (Item::Fn(f), _) => Resolved::DefineFn { function: f, decl },
            (Item::Aggregate(f), _) => Resolved::DefineAggregate { function: f, decl },
            (Item::Constraint(c), _) => Resolved::AddConstraint {
                constraint: c,
                body: decl,
            },
            (Item::Check(c), _) => Resolved::AddCheck {
                constraint: c,
                body: decl,
            },
            (Item::Store(v), _) => Resolved::Store {
                view: v,
                function: self.stores[&v].function,
                decl,
            },
        };
        Ok((resolved, clauses))
    }

    fn redefine_fn(
        &mut self,
        run: &Run,
        function: u64,
        span: Span,
        op: &OperatorKind,
    ) -> Res<(Resolved, Vec<Clause>)> {
        let OperatorKind::RedefineFn(f) = op else {
            return broken("a redefinition of no function");
        };
        let at = self
            .decl_at(Item::Fn(function))
            .expect("a function the operator resolved");
        let rest = &run.text[f.name.span.end as usize..span.end as usize];
        self.functions
            .get_mut(&function)
            .expect("the function")
            .rest = Term::text(rest);
        let built = match self.build_adopting(&[], &run.ahead(self)) {
            Ok(b) => b,
            Err(ds) => {
                return match ds.iter().find(|d| d.file.0 as usize == at) {
                    Some(d) => refuse(d.diag.message.clone()),
                    None => unmodelled("a redefinition in a source that does not build"),
                };
            }
        };
        let file = FileId(at as u32);
        if let Some(e) = built.errors_in(file).into_iter().next() {
            return refuse(e);
        }
        let Some(ast::Decl::Fn(d)) = built.decl(file) else {
            return broken("a redefined function that does not parse alone");
        };
        let names = built.names(file);
        let ahead = built.ahead_ranges(file);
        let lifting = Lifting::new(
            &built.schema,
            file,
            built.text(file),
            &names,
            &ahead,
            run.module,
        );
        let body = lifting.body(&d.name, d.span);
        self.functions.insert(function, body);
        Ok((
            Resolved::RedefineFn {
                function,
                decl: built
                    .canonical(file)?
                    .ok_or(Fail::Broken("a redefined function does not resolve".into()))?,
            },
            Vec::new(),
        ))
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

    fn drop(&mut self, run: &Run, item: Item) -> Res<(Resolved, Vec<Clause>)> {
        let resolved = match item {
            Item::Type(t) => {
                self.drop_type_allowed(run, t)?;
                Resolved::DropType { sum: t }
            }
            Item::Relation(r) => {
                self.drop_relation_allowed(run, r)?;
                Resolved::DropRelation { relation: r }
            }
            Item::Set(s) => Resolved::DropSet { set: s },
            Item::Fn(f) => Resolved::DropFn { function: f },
            Item::Aggregate(f) => Resolved::DropAggregate { function: f },
            Item::Constraint(c) => Resolved::DropConstraint { constraint: c },
            Item::Check(c) => Resolved::DropCheck { constraint: c },
            Item::Store(v) => Resolved::Unstore { view: v },
        };
        let mut gone = vec![item];
        if let Item::Relation(r) = item {
            gone.extend(self.sets_of(r).into_iter().map(Item::Set));
        }
        for g in gone {
            if let Item::Set(s) = g {
                self.unscope(s);
            }
            self.remove(g);
        }
        Ok((resolved, Vec::new()))
    }

    /// Leave each unique and index whose scope names set `s` without an id, so that the next
    /// build gives it a new one.
    fn unscope(&mut self, s: u64) {
        for r in self.relations.values_mut() {
            for c in &mut r.clauses {
                if let ClauseM::Index {
                    id,
                    scope: Some(scope),
                    ..
                } = c
                    && scope
                        .parts
                        .iter()
                        .any(|p| p.objects().any(|o| o == (Space::Set, s)))
                {
                    *id = None;
                }
            }
        }
    }

    /// Remove declaration `item` and every object it declares.
    pub(super) fn remove(&mut self, item: Item) {
        self.decls.retain(|d| d.item != item);
        match item {
            Item::Type(t) => {
                for f in self.fields_of(t) {
                    self.fields.remove(&f);
                }
                for c in self.ctors_of(t) {
                    self.ctors.remove(&c);
                }
                self.types.remove(&t);
            }
            Item::Relation(r) => {
                self.relations.remove(&r);
            }
            Item::Set(s) => {
                self.sets.remove(&s);
            }
            Item::Fn(f) | Item::Aggregate(f) => {
                self.functions.remove(&f);
            }
            Item::Constraint(c) | Item::Check(c) => {
                self.denials.remove(&c);
            }
            Item::Store(v) => {
                self.stores.remove(&v);
            }
        }
    }

    /// Refuse dropping type `t` while a declaration uses it. While the declarations do not
    /// build, allow it only when they build without it.
    fn drop_type_allowed(&self, run: &Run, t: u64) -> Res<()> {
        let built = match self.schema_now(run) {
            Ok(b) => b,
            Err(e) => {
                let mut without = self.clone();
                without.remove(Item::Type(t));
                return match without.built(&[], &run.ahead(&without)) {
                    Ok(_) => Ok(()),
                    Err(_) => Err(e),
                };
            }
        };
        let schema = &built.schema;
        let Some(def) = schema.ids().type_of(crate::ids::CatTypeId::from_raw(t)) else {
            return broken("a dropped type the schema does not hold");
        };
        let name = &self.types[&t].name;
        let refused = |what: String| {
            refuse(format!(
                "`drop type {name}` is refused while {what} uses it"
            ))
        };
        for r in &schema.relations {
            if r.elem_def == def {
                return refused(format!("relation `{}`", r.name));
            }
        }
        for (j, td) in schema.types.iter().enumerate() {
            if j == def.0 as usize || schema.module(td.module).engine {
                continue;
            }
            for f in td
                .header()
                .iter()
                .chain(td.ctors().iter().flat_map(|c| c.fields.iter()))
            {
                if mentions_def(&f.ty, def) {
                    return refused(format!("field `{}.{}`", td.name, f.name));
                }
            }
        }
        for f in &schema.functions {
            if f.params.iter().any(|(_, x)| mentions_def(x, def)) || mentions_def(&f.result, def) {
                return refused(format!("function `{}`", f.name));
            }
        }
        let own = FileId(self.decl_at(Item::Type(t)).expect("a declared type") as u32);
        for (file, _, r) in check::schema_refs(schema) {
            if file != own
                && matches!(r, check::Ref::Ctor(d, _) | check::Ref::Field { def: d, .. } if d == def)
            {
                return refused(self.shown_decl(file.0 as usize));
            }
        }
        Ok(())
    }

    fn drop_relation_allowed(&self, run: &Run, r: u64) -> Res<()> {
        let built = self.schema_now(run)?;
        let schema = &built.schema;
        let Some(rel) = schema.ids().relation_of(crate::ids::CatRelId::from_raw(r)) else {
            return broken("a dropped relation variable the schema does not hold");
        };
        let name = &self.relations[&r].name;
        let own = schema.relation(rel).elem_def;
        for (j, td) in schema.types.iter().enumerate() {
            if j == own.0 as usize {
                continue;
            }
            for f in td
                .header()
                .iter()
                .chain(td.ctors().iter().flat_map(|c| c.fields.iter()))
            {
                if mentions_rel(&f.ty, rel) {
                    return refuse(format!(
                        "`drop relation {name}` is refused while field `{}.{}` references it",
                        td.name, f.name
                    ));
                }
            }
        }
        let file = FileId(
            self.decl_at(Item::Relation(r))
                .expect("a declared relation") as u32,
        );
        for (f, _, x) in check::schema_refs(schema) {
            if f != file && matches!(x, check::Ref::Relation(y) if y == rel) {
                return refuse(format!(
                    "`drop relation {name}` is refused while {} names it",
                    self.shown_decl(f.0 as usize)
                ));
            }
        }
        Ok(())
    }

    /// Rename object `(space, id)` to `to`, refusing a name taken and, as the operator reads
    /// them, declarations that do not build.
    fn rename(&mut self, run: &Run, (space, id): Obj, to: String) -> Res<(Resolved, Vec<Clause>)> {
        let resolved = match space {
            Space::Shared if self.types.contains_key(&id) => {
                let module = self
                    .module_of((space, id))
                    .expect("a declared type")
                    .to_string();
                if self.type_named(&module, &to).is_some() {
                    return refuse(format!("module `{module}` already declares type `{to}`"));
                }
                self.schema_now(run)?;
                let ty = self.types.get_mut(&id).expect("the type");
                ty.name = to.clone();
                if let TypeBody::Record { ctor } | TypeBody::New { ctor, .. } = ty.body {
                    self.ctors.get_mut(&ctor).expect("the constructor").name = to.clone();
                }
                Resolved::RenameType { sum: id, name: to }
            }
            Space::Shared if self.relations.contains_key(&id) => {
                let module = self
                    .module_of((space, id))
                    .expect("a declared relation")
                    .to_string();
                self.schema_now(run)?;
                let taken = self.decls.iter().any(|d| {
                    d.module == module
                        && match d.item {
                            Item::Relation(r) => self.relations[&r].name == to,
                            Item::Fn(f) | Item::Aggregate(f) => self.functions[&f].name == to,
                            Item::Constraint(c) | Item::Check(c) => self.denials[&c].name == to,
                            _ => false,
                        }
                });
                if taken {
                    return refuse(format!("module `{module}` already declares `{to}`"));
                }
                self.relations.get_mut(&id).expect("the relation").name = to.clone();
                Resolved::RenameRelation {
                    relation: id,
                    name: to,
                }
            }
            Space::Shared => {
                let ty = self.ctors[&id].ty;
                if self.ctor_named(ty, &to).is_some() {
                    return refuse(format!(
                        "`{to}` is already a constructor of `{}`",
                        self.types[&ty].name
                    ));
                }
                for r in self.relations_of(ty) {
                    if self
                        .sets_of(r)
                        .into_iter()
                        .any(|s| self.sets[&s].name == to)
                    {
                        return refuse(format!(
                            "`{to}` is already a named set of `{}`",
                            self.relations[&r].name
                        ));
                    }
                }
                self.ctors.get_mut(&id).expect("the constructor").name = to.clone();
                Resolved::RenameConstructor { ctor: id, name: to }
            }
            Space::Set => {
                self.schema_now(run)?;
                let r = self.sets[&id].relation;
                let clash = self
                    .sets_of(r)
                    .into_iter()
                    .any(|s| self.sets[&s].name == to)
                    || self
                        .relation_type(r)
                        .is_some_and(|t| self.ctor_named(t, &to).is_some());
                if clash {
                    return refuse(format!(
                        "`{}` already has a constructor or named set `{to}`",
                        self.relations[&r].name
                    ));
                }
                self.sets.get_mut(&id).expect("the set").name = to.clone();
                Resolved::RenameSet { set: id, name: to }
            }
            Space::Field => {
                let owner = self.fields[&id].owner;
                let t = self.field_type(id).expect("a field's type");
                let header = self.types.contains_key(&owner);
                let clash = self.fields_of(t).into_iter().any(|f| {
                    let x = &self.fields[&f];
                    x.name == to
                        && (header || self.types.contains_key(&x.owner) || x.owner == owner)
                });
                if clash {
                    let ctor = (!header).then_some(owner);
                    return refuse(format!(
                        "`{}` already has a field `{to}`",
                        self.shown_part(t, ctor)
                    ));
                }
                self.schema_now(run)?;
                self.fields.get_mut(&id).expect("the field").name = to.clone();
                Resolved::RenameField {
                    field: id,
                    name: to,
                }
            }
            _ => return broken("a rename of this kind of object"),
        };
        Ok((resolved, Vec::new()))
    }

    fn stage(
        &mut self,
        run: &Run,
        key: StateKey,
        to: Stage,
        verb: &str,
        shown: &str,
    ) -> Res<(Resolved, Vec<Clause>)> {
        let held = match key {
            StateKey::Relation(r, c) => self.states.get(&(r, c)),
            StateKey::Value(c) => self.value_states.get(&c),
        };
        let current = held.map_or(Stage::Enabled, |s| s.stage);
        match (verb, current) {
            ("enable", Stage::Enabled) => return refuse(format!("`{shown}` is already enabled")),
            ("enable", _) | ("disable" | "deprecate", Stage::Enabled) => {}
            ("disable", s) => {
                return refuse(format!(
                    "`disable` moves an enabled constructor back to declared; `{shown}` is {}",
                    s.name()
                ));
            }
            (_, s) => {
                return refuse(format!(
                    "`{verb}` applies only to an enabled constructor; `{shown}` is {}",
                    s.name()
                ));
            }
        }
        if to != Stage::Enabled
            && let StateKey::Relation(r, c) = key
        {
            let built = self.schema_now(run)?;
            let schema = &built.schema;
            let rel = schema
                .ids()
                .relation_of(crate::ids::CatRelId::from_raw(r))
                .expect("a relation variable the model holds");
            let def = schema.relation(rel).elem_def;
            let k = schema
                .ids()
                .ctor_of(crate::ids::CatCtorId::from_raw(c))
                .expect("a constructor the model holds")
                .index();
            if let Some(f) = crate::evolve::retag_into(schema, def, k) {
                return refuse(format!(
                    "`{verb} {shown}` is refused: the delete action of field `{f}` retags into it"
                ));
            }
        }
        let state = State {
            stage: to,
            since: self.version,
        };
        let target = match key {
            StateKey::Relation(r, c) => {
                self.states.insert((r, c), state);
                Target::InRelation {
                    relation: r,
                    ctor: c,
                }
            }
            StateKey::Value(c) => {
                self.value_states.insert(c, state);
                Target::OfType { ctor: c }
            }
        };
        let resolved = match verb {
            "enable" => Resolved::Enable { target },
            "disable" => Resolved::Disable { target },
            _ => Resolved::Deprecate { target },
        };
        Ok((resolved, Vec::new()))
    }
}

fn index_kind(interval: bool) -> IndexKind {
    if interval {
        IndexKind::Interval
    } else {
        IndexKind::Plain
    }
}

/// Give `op`, an operator adding a declaration, the canonical form `decl`.
fn set_decl(op: &mut Resolved, decl: String) {
    match op {
        Resolved::AddType { decl: d, .. }
        | Resolved::AddRelation { decl: d, .. }
        | Resolved::DefineFn { decl: d, .. }
        | Resolved::DefineAggregate { decl: d, .. }
        | Resolved::Store { decl: d, .. }
        | Resolved::AddConstraint { body: d, .. }
        | Resolved::AddCheck { body: d, .. }
        | Resolved::AddField { decl: d, .. }
        | Resolved::AlterField { action: d, .. } => *d = decl,
        _ => {}
    }
}

/// Returns the span of the delete action of a field written at `decl` whose type is at `ty`: from
/// the token after `on delete` to the end.
fn action_span(text: &str, ty: Span, decl: Span) -> Span {
    let from = ty.end as usize;
    let toks: Vec<_> = lex(&text[from..decl.end as usize])
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .collect();
    let start = toks
        .get(2)
        .map_or(decl.end as usize, |t| from + t.span.start as usize);
    Span::new(start, decl.end as usize)
}

/// Refuse an operator that reads declarations failing with `ds`, by the first failure.
fn unbuilt<T>(ds: &[FileDiag]) -> Res<T> {
    refuse(ds[0].diag.message.clone())
}
