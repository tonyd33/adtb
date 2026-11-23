use super::*;
use crate::span::DiagKind;

impl Source {
    /// Returns the source of a schema declared directly, every constructor enabled at version 0.
    pub fn new(decls: Vec<SourceDecl>) -> Result<Source, Vec<FileDiag>> {
        let mut src = Source {
            modules: decls.iter().map(|d| d.module.clone()).collect(),
            decls,
            ..Source::default()
        };
        src.settle_ids();
        let schema = src.schema()?;
        let enabled = State {
            stage: Stage::Enabled,
            since: 0,
        };
        for (r, rel) in schema.ids().relation_ids() {
            if schema.module(schema.relation(r).module).engine {
                continue;
            }
            for c in stated_ctors(schema.ty(schema.relation(r).elem_def)) {
                if let Some(ctor) = schema.ids().ctor_id(c) {
                    src.states.insert((rel, ctor), enabled);
                }
            }
        }
        for t in &schema.types {
            if schema.module(t.module).engine {
                continue;
            }
            for c in stated_ctors(t) {
                if let Some(ctor) = schema.ids().ctor_id(c) {
                    src.value_states.insert(ctor, enabled);
                }
            }
        }
        Ok(src)
    }

    pub(super) fn state(&self, s: StateOf) -> Option<State> {
        match s {
            StateOf::Relation(r, c) => self.states.get(&(r, c)),
            StateOf::Value(c) => self.value_states.get(&c),
        }
        .copied()
    }

    pub(super) fn every_state(&self) -> impl Iterator<Item = (StateOf, State)> + '_ {
        let relations = self
            .states
            .iter()
            .map(|(&(r, c), &v)| (StateOf::Relation(r, c), v));
        let values = self
            .value_states
            .iter()
            .map(|(&c, &v)| (StateOf::Value(c), v));
        relations.chain(values)
    }

    /// Returns the module files of the declarations, file `i` holding declaration `i`.
    pub fn files(&self) -> Result<Vec<(FileId, ast::SchemaFile)>, Vec<FileDiag>> {
        let mut out = Vec::new();
        let mut errs = Vec::new();
        for (i, d) in self.decls.iter().enumerate() {
            let text = format!("{}{}", header(&d.module), d.text);
            match self.parsed.parsed(&text) {
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

    /// Build the schema, with the catalog ids and write states the source holds.
    pub fn schema(&self) -> Result<Schema, Vec<FileDiag>> {
        let built = self.built()?;
        let mut states = schema::States::default();
        for (&(r, c), s) in &self.states {
            states.set_relation(r, c, s.stage);
        }
        for (&c, s) in &self.value_states {
            states.set_value(c, s.stage);
        }
        Ok(built.relabelled(&|s| self.assigned_ids(s), &states))
    }

    /// Returns the schema [`Source::schema`] builds, committed.
    ///
    /// Fail with `Unbuilt` when the source does not build, and with `Migrating` when a migration
    /// is being applied to it: `expecting` was called and `settled` was not.
    pub fn committed(&self) -> Result<Committed, Uncommitted> {
        let schema = self.schema().map_err(Uncommitted::Unbuilt)?;
        Committed::new(schema).ok_or(Uncommitted::Migrating)
    }

    /// Build the schema the declarations state, its ids those of declaration order.
    pub(super) fn built(&self) -> Result<Arc<Schema>, Vec<FileDiag>> {
        if let Some(schema) = self.memo.get(self) {
            return Ok(schema);
        }
        let schema = Arc::new(self.built_uncached()?);
        self.memo.put(self, schema.clone());
        Ok(schema)
    }

    pub(super) fn built_uncached(&self) -> Result<Schema, Vec<FileDiag>> {
        let mut files = self.files()?;
        for m in &self.modules {
            if !files.iter().any(|(_, f)| f.module.text == *m) {
                let empty = parse_schema(&header(m), Options::default())
                    .ok()
                    .expect("a module header parses");
                files.push((FileId::EMPTY_MODULE, empty));
            }
        }
        schema::build_ahead(&files, &self.ahead_inputs())
    }

    /// Prepare the source for the operators `ops` of a migration of `module`, applied one at a
    /// time: a `Key r` type may name a relation variable that a later one of them adds.
    ///
    /// Call `settled` after the last of them.
    pub fn expecting(&mut self, module: &str, ops: &[Operator]) {
        let types: BTreeSet<&str> = ops
            .iter()
            .filter_map(|op| match &op.kind {
                OperatorKind::AddType(t) => Some(t.name.text.as_str()),
                _ => None,
            })
            .collect();
        self.ahead = Listed::all(ops)
            .filter_map(|op| match &op.kind {
                OperatorKind::AddRelation(r) => Some(Expected::new(
                    module.to_string(),
                    r.name.name.text.clone(),
                    matches!(
                        &r.ty.kind,
                        ast::TypeKind::Named { name, .. }
                            if name.module.as_ref().is_none_or(|m| m.text == module)
                                && types.contains(name.name.text.as_str())
                    ),
                    Adder::Operator(op.at()?),
                )),
                _ => None,
            })
            .collect();
    }

    /// End the migration `expecting` prepared the source for, keeping the name of each relation
    /// variable a refused operator would have added bound, poisoned.
    pub fn settled(&mut self) {
        self.keep_refused();
    }

    /// Build and type check the schema.
    pub fn check(&self) -> Result<Compiled<Schema>, Vec<FileDiag>> {
        check::check_schema(self.schema()?)
    }

    pub(super) fn parsed(&self, i: usize) -> Option<Decl> {
        let text = self.file_text(FileId(i as u32));
        parse_schema(&text, Options { elisions: true })
            .ok()
            .ok()
            .and_then(|f| f.decls.into_iter().next())
    }

    pub(super) fn find(&self, kind: &str, module: &str, name: &str) -> Option<usize> {
        (0..self.decls.len()).find(|&i| {
            self.decls[i].module == module
                && self.parsed(i).is_some_and(|d| {
                    let (k, n) = key_of(&d);
                    k == kind && n == name
                })
        })
    }

    /// The byte range of `span`, of declaration `i`'s file, in its text without the header.
    pub(super) fn local(&self, i: usize, span: Span) -> Range<usize> {
        let h = header(&self.decls[i].module).len();
        span.start as usize - h..span.end as usize - h
    }

    /// Apply a migration, numbered one past the current version, and recheck the result.
    /// Refuse an operator whose steps over the rows fail.
    pub fn migrate(&self, m: &MigrationFile, text: &str) -> Result<Source, Failed> {
        self.migrate_keeping(m, text, &[])
    }

    /// Apply migration text the log does not hold yet as [`Source::migrate`] does, and refuse
    /// what only new text may not state: a `revert` fill no inverse takes.
    pub fn accept(&self, m: &MigrationFile, text: &str) -> Result<Source, Failed> {
        self.migrating(m, text, &[], true, false).map(|(s, ..)| s)
    }

    /// Apply a migration as [`Source::migrate`] does, putting back after each operator every
    /// declaration of `kept` it removes, as it was before that operator and with its catalog id.
    pub fn migrate_keeping(
        &self,
        m: &MigrationFile,
        text: &str,
        kept: &[Kept],
    ) -> Result<Source, Failed> {
        self.migrating(m, text, kept, false, false).map(|(s, ..)| s)
    }

    /// Apply migration `m` as [`Source::migrate_keeping`] does, refusing as [`Source::accept`]
    /// does when `accepting`, naming its text when `naming`. Returns the source it leaves,
    /// checked, and the migration as the model applied it.
    pub(super) fn migrating(
        &self,
        m: &MigrationFile,
        text: &str,
        kept: &[Kept],
        accepting: bool,
        naming: bool,
    ) -> Result<(Source, Compiled<Schema>, model::Migrated), Failed> {
        let got =
            model::migrated(self, m, text, kept, accepting, naming).map_err(Failed::Broken)?;
        let refusals = got.refusals(m);
        let mut next = got.source();
        next.settled();
        let checked = next.check();
        let failing = got.applies().then(|| got.failing_steps(m)).flatten();
        if got.applies()
            && failing.is_none()
            && let Ok(checked) = checked
        {
            next.log.push(Logged {
                number: next.version,
                module: m.module.text.clone(),
                text: text.to_string(),
                operators: m.operators.clone(),
                before: Arc::new(self.clone()),
            });
            return Ok((next, checked, got));
        }
        let ds: Vec<FileDiag> = checked
            .err()
            .into_iter()
            .flatten()
            .filter(|d| d.diag.kind != DiagKind::Poisoned)
            .collect();
        let (blame, rechecked) = got.ledger(&refusals, &ds);
        let mut refusal = Refusal {
            operators: refusals.into_iter().map(|(_, d)| d).collect(),
            blame,
            ..Refusal::default()
        };
        if let Some((k, why)) = failing {
            refusal.unstepped = Some(k);
            refusal.operators.push(why);
            refusal.blame.push(k);
        }
        for (d, by) in ds.iter().zip(rechecked) {
            if let Some(k) = by {
                refusal.recheck.push(d.clone());
                refusal.blame.push(k);
            }
        }
        refusal.recheck_from = got.copied_from(text, &refusal.recheck);
        if refusal.operators.is_empty() && refusal.recheck.is_empty() {
            let last = m.operators.len().saturating_sub(1);
            let at = m.operators.last().map_or(Span::default(), |op| op.span);
            refusal.operators.push(got.unchecked(at));
            refusal.blame.push(last);
        }
        refusal.result = Some(Box::new(next));
        Err(Failed::Refused(refusal))
    }
}

/// Refuse a rekey of `relation` whose relation's own type keeps a field typed by its key outside
/// the new key.
pub(super) fn rekey_leaves_no_old_reference(
    schema: &Schema,
    module: &str,
    relation: &QName,
    span: Span,
) -> Result<(), Diag> {
    let m = relation.module.as_ref().map_or(module, |m| m.text.as_str());
    let Some(mid) = schema.module_named(m) else {
        return Ok(());
    };
    let Some(&ValueDef::Relation(rel)) = schema.module(mid).values.get(&relation.name.text) else {
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
        if matches!(f.ty, Ty::Key { rel: own, .. } if own == RelRef::Declared(rel))
            && !r.key.contains(&f.name)
        {
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

/// Returns the constructors of `t` that hold write states: none of a newtype.
pub(super) fn stated_ctors(t: &schema::TypeDef) -> Vec<schema::CtorRef> {
    match t.kind {
        TypeDefKind::Newtype { .. } => Vec::new(),
        TypeDefKind::Sum { .. } => t.ctor_refs().collect(),
    }
}
