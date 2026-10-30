use super::*;

impl Source {
    /// Returns the source of a schema declared directly, every constructor enabled at version 0.
    pub fn new(decls: Vec<SourceDecl>) -> Result<Source, Vec<FileDiag>> {
        let mut src = Source {
            modules: decls.iter().map(|d| d.module.clone()).collect(),
            decls,
            ..Source::default()
        };
        let schema = src.built()?;
        src.states = Source::all_states(&schema, Stage::Enabled);
        src.reconcile_ids();
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
            if schema.module(r.module).engine {
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

    /// Builds the schema, with the catalog ids and write states the source holds.
    pub fn schema(&self) -> Result<Schema, Vec<FileDiag>> {
        let mut schema = Schema::clone(&*self.built()?);
        self.annotate(&mut schema);
        Ok(schema)
    }

    /// Builds the schema the declarations state, its ids those of declaration order.
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
                files.push((FileId(u32::MAX - 2), empty));
            }
        }
        schema::build_ahead(&files, &self.ahead)
    }

    /// Prepares the source for the operators `ops` of a migration of `module`, applied one at a
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
        self.ahead = ops
            .iter()
            .filter_map(|op| match &op.kind {
                OperatorKind::AddRelation(r) => Some(schema::Ahead {
                    module: module.to_string(),
                    relation: r.name.name.text.clone(),
                    typed: matches!(
                        &r.ty.kind,
                        ast::TypeKind::Named { name, .. }
                            if name.module.as_ref().is_none_or(|m| m.text == module)
                                && types.contains(name.name.text.as_str())
                    ),
                }),
                _ => None,
            })
            .collect();
    }

    /// Ends the migration `expecting` prepared the source for.
    pub fn settled(&mut self) {
        self.ahead.clear();
    }

    /// Builds and type checks the schema.
    pub fn check(&self) -> Result<(Schema, Program), Vec<FileDiag>> {
        let schema = self.schema()?;
        let program = check::check_schema(&schema)?;
        Ok((schema, program))
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

    /// Replaces each range of declaration `i`'s text by its text.
    ///
    /// Precondition: the ranges do not overlap, except that equal edits count once.
    pub(super) fn edit<W: Into<Written>>(&mut self, i: usize, edits: Vec<(Range<usize>, W)>) {
        let mut edits: Vec<(Range<usize>, Written)> =
            edits.into_iter().map(|(r, w)| (r, w.into())).collect();
        edits.sort_by(|a, b| b.0.start.cmp(&a.0.start).then(b.0.end.cmp(&a.0.end)));
        edits.dedup();
        for (r, w) in edits {
            self.decls[i].text.replace_range(r.clone(), &w.text);
            let Some(marks) = &mut self.marks else {
                continue;
            };
            let grown = w.text.len() as isize - r.len() as isize;
            let moved = |at: usize| {
                at.checked_add_signed(grown)
                    .expect("a copy after an edit stays inside its declaration")
            };
            let mut kept = Vec::new();
            for m in std::mem::take(&mut marks.written) {
                let end = m.at + m.from.len();
                if m.decl != i || end <= r.start {
                    kept.push(m);
                } else if m.at >= r.end {
                    kept.push(Mark {
                        at: moved(m.at),
                        ..m
                    });
                } else {
                    if m.at < r.start {
                        kept.push(Mark {
                            from: m.from.start..m.from.start + (r.start - m.at),
                            ..m.clone()
                        });
                    }
                    if end > r.end {
                        kept.push(Mark {
                            from: m.from.end - (end - r.end)..m.from.end,
                            decl: i,
                            at: moved(r.end),
                        });
                    }
                }
            }
            marks.written = kept;
            marks.written.extend(
                w.copies
                    .into_iter()
                    .filter(|(_, from)| !from.is_empty())
                    .map(|(o, from)| Mark {
                        from,
                        decl: i,
                        at: r.start + o,
                    }),
            );
        }
    }

    pub(super) fn push(&mut self, module: &str, text: impl Into<Written>) -> usize {
        self.decls.push(SourceDecl {
            module: module.to_string(),
            text: String::new(),
        });
        let i = self.decls.len() - 1;
        self.edit(i, vec![(0..0, text)]);
        i
    }

    /// Replaces the whole text of declaration `i`.
    pub(super) fn rewrite(&mut self, i: usize, text: impl Into<Written>) {
        let end = self.decls[i].text.len();
        self.edit(i, vec![(0..end, text)]);
    }

    /// Removes every declaration `drop` holds.
    pub(super) fn remove_decls(&mut self, drop: impl Fn(usize, &SourceDecl) -> bool) {
        let mut kept = Vec::new();
        let mut decls = Vec::new();
        for (i, d) in std::mem::take(&mut self.decls).into_iter().enumerate() {
            if drop(i, &d) {
                kept.push(None);
            } else {
                kept.push(Some(decls.len()));
                decls.push(d);
            }
        }
        self.decls = decls;
        if let Some(marks) = &mut self.marks {
            for ms in [&mut marks.written, &mut marks.read] {
                ms.retain_mut(|m| match kept[m.decl] {
                    Some(k) => {
                        m.decl = k;
                        true
                    }
                    None => false,
                });
            }
        }
    }

    pub(super) fn remove_decl(&mut self, i: usize) {
        self.remove_decls(|j, _| j == i);
    }

    /// Records that the operator matched the text at `from` of the migration against the text at `at`
    /// of declaration `i`, as it is before the operator edits it.
    pub(super) fn read_mark(&mut self, from: Range<usize>, i: usize, at: usize) {
        if let Some(marks) = &mut self.marks {
            marks.read.push(Mark { from, decl: i, at });
        }
    }

    /// Applies a migration, numbered one past the current version, and rechecks the result.
    pub fn migrate(&self, m: &MigrationFile, text: &str) -> Result<Source, Refusal> {
        self.migrate_keeping(m, text, &[])
    }

    /// Apply a migration as [`Source::migrate`] does, putting back after each operator every
    /// declaration of `kept` it removes, as it was before that operator and with its catalog id.
    pub fn migrate_keeping(
        &self,
        m: &MigrationFile,
        text: &str,
        kept: &[Kept],
    ) -> Result<Source, Refusal> {
        let mut next = self.clone();
        next.version += 1;
        next.expecting(&m.module.text, &m.operators);
        next.unstated.clear();
        let mut refusal = Refusal::default();
        let mut rekeyed = Vec::new();
        let mut applied = Vec::with_capacity(m.operators.len());
        for op in &m.operators {
            if let OperatorKind::Rekey { relation, .. } = &op.kind {
                rekeyed.push((relation.clone(), op.span));
            }
            let prior = (!kept.is_empty()).then(|| next.clone());
            let ended = next.apply(&m.module.text, op, text);
            applied.push(ended.is_ok());
            if let Err(mut ds) = ended {
                refusal.operators.append(&mut ds);
            }
            if let Some(prior) = prior {
                next.keep(&prior, kept);
            }
        }
        refusal.operators.extend(
            std::mem::take(&mut next.unstated)
                .into_iter()
                .map(|u| Diag::new(u.span, u.message)),
        );
        for (relation, span) in rekeyed {
            if let Err(d) = next.rekey_leaves_no_old_reference(&m.module.text, &relation, span) {
                refusal.operators.push(d);
            }
        }
        next.settled();
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
            refusal.recheck_from = self.copied_from(m, text, &applied, &refusal.recheck);
        }
        if refusal.operators.is_empty() && refusal.recheck.is_empty() {
            next.log.push(Logged {
                number: next.version,
                module: m.module.text.clone(),
                text: text.to_string(),
                operators: m.operators.clone(),
                before: Arc::new(self.clone()),
            });
            Ok(next)
        } else {
            refusal.result = Some(Box::new(next));
            Err(refusal)
        }
    }

    /// Returns, for each diagnostic of the result of migration `m`, the span of `text` an operator
    /// copied the diagnosed text from, by applying `m` again recording its copies.
    ///
    /// Preconditions:
    /// - `applied` holds, for each operator of `m`, whether [`Source::migrate`] applied it.
    fn copied_from(
        &self,
        m: &MigrationFile,
        text: &str,
        applied: &[bool],
        diags: &[FileDiag],
    ) -> Vec<Option<Span>> {
        let mut replay = self.clone();
        replay.version += 1;
        replay.expecting(&m.module.text, &m.operators);
        replay.marks = Some(Marks::default());
        for (op, &was) in m.operators.iter().zip(applied) {
            let noted = RefCell::new(Names::default());
            let ended = replay.apply_noting(&m.module.text, op, text, Some(&noted));
            assert_eq!(
                was,
                ended.is_ok(),
                "a replay of a migration applies each operator the migration applied, and no other"
            );
        }
        let marks = replay.marks.take().expect("the replay records its copies");
        diags
            .iter()
            .map(|d| {
                let i = d.file.0 as usize;
                let h = header(replay.decls.get(i)?.module.as_str()).len();
                let start = (d.diag.span.start as usize).checked_sub(h)?;
                let end = d.diag.span.end as usize - h;
                marks
                    .written
                    .iter()
                    .find(|w| w.decl == i && w.at <= start && end <= w.at + w.from.len())
                    .map(|w| Span::new(w.from.start + start - w.at, w.from.start + end - w.at))
            })
            .collect()
    }

    /// Apply one operator of a migration of `module` whose text is `text`.
    ///
    /// On a refusal the source is left as it was.
    pub fn apply(&mut self, module: &str, op: &Operator, text: &str) -> Result<(), Vec<Diag>> {
        self.apply_noting(module, op, text, None)
    }

    pub(super) fn apply_noting(
        &mut self,
        module: &str,
        op: &Operator,
        text: &str,
        noted: Option<&RefCell<Names>>,
    ) -> Result<(), Vec<Diag>> {
        self.apply_recording(module, op, text, noted, None)
    }

    pub(super) fn apply_recording(
        &mut self,
        module: &str,
        op: &Operator,
        text: &str,
        noted: Option<&RefCell<Names>>,
        steps: Option<&RefCell<Vec<Step>>>,
    ) -> Result<(), Vec<Diag>> {
        let before = self.clone();
        let r = Cx {
            src: self,
            module: module.to_string(),
            text,
            span: op.span,
            noted,
            copying: noted.is_some(),
            steps,
        }
        .operator(&op.kind);
        match r {
            Ok(()) => {
                self.rename_ids(&before, module, &op.kind);
                self.reconcile_ids();
                self.carry_grants(&before, module, &op.kind);
            }
            Err(_) => *self = before,
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
