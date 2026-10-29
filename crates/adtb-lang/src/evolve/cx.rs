use super::*;

impl Cx<'_> {
    pub(super) fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(vec![Diag::new(self.span, msg)])
    }

    pub(super) fn err_at<T>(&self, span: Span, msg: impl Into<String>) -> R<T> {
        Err(vec![Diag::new(span, msg)])
    }

    pub(super) fn slice(&self, span: Span) -> &str {
        &self.text[span.range()]
    }

    /// Returns the text at `span` of the migration, as a copy of it.
    pub(super) fn copy(&self, span: Span) -> Written {
        self.copy_range(span.range())
    }

    pub(super) fn copy_range(&self, r: Range<usize>) -> Written {
        Written {
            text: self.text[r.clone()].to_string(),
            copies: if self.copying {
                vec![(0, r)]
            } else {
                Vec::new()
            },
        }
    }

    /// Records the names a fill or routing of this operator resolves, when replaying for them.
    pub(super) fn note(&self, schema: &Schema, refs: &[(Span, Ref)]) {
        if let Some(noted) = self.noted {
            let mut noted = noted.borrow_mut();
            for (span, r) in refs {
                noted.note(schema, *span, r);
            }
        }
    }

    pub(super) fn module_of(&self, q: &QName) -> String {
        q.module
            .as_ref()
            .map_or_else(|| self.module.clone(), |m| m.text.clone())
    }

    pub(super) fn schema(&self) -> R<Schema> {
        self.src.schema().map_err(|ds| {
            ds.into_iter()
                .map(|d| Diag::new(self.span, d.diag.message))
                .collect()
        })
    }

    pub(super) fn checked(&self) -> R<(Schema, Program)> {
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
    pub(super) fn type_decl(&self, q: &QName) -> R<(usize, ast::TypeDecl)> {
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

    pub(super) fn relation_decl(&self, q: &QName) -> R<(usize, ast::RelationDecl)> {
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

    pub(super) fn type_id(&self, schema: &Schema, q: &QName) -> R<TypeId> {
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

    pub(super) fn rel_id(&self, schema: &Schema, q: &QName) -> R<RelId> {
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
    pub(super) fn relations_of(schema: &Schema, def: TypeId) -> Vec<(RelId, String, String)> {
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
    pub(super) fn object_name(&self, file: FileId) -> String {
        match self.src.parsed(file.0 as usize) {
            Some(d) => {
                let (kind, name) = key_of(&d);
                format!("{kind} `{name}`")
            }
            None => "a declaration".to_string(),
        }
    }

    pub(super) fn operator(&mut self, op: &OperatorKind) -> R<()> {
        match op {
            OperatorKind::AddModule(m) => self.add_module(m),
            OperatorKind::DropModule(m) => self.drop_module(m),
            OperatorKind::Install { name, from, .. } => self.install(name, from),
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
            } => self.merge(ty, sel, into, arms.as_deref(), clauses, false),
            OperatorKind::NestConstructor {
                ty,
                sel,
                into,
                clauses,
            } => self.nest(ty, sel, into, clauses),
            OperatorKind::UnnestConstructor(path, clauses) => self.unnest(path, clauses),
            OperatorKind::AddField {
                path, decl, fill, ..
            } => self.add_field(path, *decl, fill),
            OperatorKind::AlterField { path, span, .. } => self.alter_field(path, *span),
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
            OperatorKind::Grant(g) => self.grant(g),
            OperatorKind::Revoke(r) => self.revoke(r),
        }
    }

    pub(super) fn reserved_module(&self, m: &ast::Ident) -> R<()> {
        if m.text == "prelude" || ENGINE_MODULES.contains(&m.text.as_str()) {
            return self.err_at(m.span, format!("no module may be named `{}`", m.text));
        }
        if self.src.modules.contains(&m.text) {
            return self.err_at(m.span, format!("module `{}` already exists", m.text));
        }
        Ok(())
    }

    pub(super) fn add_module(&mut self, m: &ast::Ident) -> R<()> {
        self.reserved_module(m)?;
        self.src.modules.insert(m.text.clone());
        Ok(())
    }

    pub(super) fn drop_module(&mut self, m: &ast::Ident) -> R<()> {
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

    pub(super) fn install(&mut self, m: &ast::Ident, from: &str) -> R<()> {
        self.reserved_module(m)?;
        let Some(package) = self.src.packages.get(from) else {
            return self.err(format!("the package `{from}` is not available"));
        };
        let file = format!("module {}\n", m.text);
        let text = format!("{file}{package}");
        let parsed = parse_schema(&text, Options::default()).ok().map_err(|ds| {
            ds.into_iter()
                .map(|d| {
                    Diag::new(
                        self.span,
                        format!("the package `{from}` does not parse: {}", d.message),
                    )
                })
                .collect::<Vec<_>>()
        })?;
        let first = self.src.decls.len();
        self.src.modules.insert(m.text.clone());
        self.src.installed.insert(m.text.clone());
        for d in &parsed.decls {
            self.src.push(&m.text, &text[d.span().range()]);
        }
        for i in first..self.src.decls.len() {
            self.checks_where_written(i)?;
        }
        Ok(())
    }

    pub(super) fn uninstall(&mut self, m: &ast::Ident) -> R<()> {
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
        self.src.remove_decls(|_, d| d.module == m.text);
        self.src.modules.remove(&m.text);
        self.src.installed.remove(&m.text);
        Ok(())
    }

    pub(super) fn known_module(&self) -> R<()> {
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
    pub(super) fn add_decl(&mut self, kind: &str, name: &str, span: Span) -> R<()> {
        self.known_module()?;
        if self.src.find(kind, &self.module, name).is_some() {
            return self.err_at(
                span,
                format!("module `{}` already declares {kind} `{name}`", self.module),
            );
        }
        let text = self.copy(span);
        let module = self.module.clone();
        let i = self.src.push(&module, text);
        self.checks_where_written(i)
    }

    /// Refuses the operator when declaration `i` fails to check against the source it is in.
    ///
    /// The failures of other declarations are left to the recheck at the end of the migration.
    pub(super) fn checks_where_written(&self, i: usize) -> R<()> {
        let Err(ds) = self.src.check() else {
            return Ok(());
        };
        let own: Vec<Diag> = ds
            .into_iter()
            .filter(|d| d.file == FileId(i as u32))
            .map(|d| Diag::new(self.span, d.diag.message))
            .collect();
        if own.is_empty() { Ok(()) } else { Err(own) }
    }

    pub(super) fn redefine(&mut self, kind: &str, name: &ast::Ident, span: Span) -> R<()> {
        let Some(i) = self.src.find(kind, &self.module, &name.text) else {
            return self.err_at(
                name.span,
                format!(
                    "module `{}` declares no {kind} `{}`",
                    self.module, name.text
                ),
            );
        };
        let text = self.copy(span);
        self.src.rewrite(i, text);
        self.checks_where_written(i)
    }

    pub(super) fn drop_decl(&mut self, kind: &str, q: &QName) -> R<()> {
        let m = self.module_of(q);
        let Some(i) = self.src.find(kind, &m, &q.name.text) else {
            return self.err_at(
                q.span(),
                format!("module `{m}` declares no {kind} `{}`", q.name.text),
            );
        };
        self.src.remove_decl(i);
        Ok(())
    }

    pub(super) fn add_type(&mut self, t: &ast::TypeDecl) -> R<()> {
        self.add_decl("type", &t.name.text, t.span)?;
        let names: Vec<String> = match &t.body {
            ast::TypeBody::Record(_) | ast::TypeBody::New(..) => vec![t.name.text.clone()],
            ast::TypeBody::Ctors(cs) => cs.iter().map(|c| c.name.text.clone()).collect(),
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

    pub(super) fn drop_type(&mut self, q: &QName) -> R<()> {
        let (i, decl) = self.type_decl(q)?;
        let schema = match self.schema() {
            Ok(s) => s,
            Err(ds) => {
                let mut without = self.src.clone();
                without.remove_decl(i);
                if without.schema().is_err() {
                    return Err(ds);
                }
                self.remove_type(i, &decl);
                return Ok(());
            }
        };
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
        self.remove_type(i, &decl);
        Ok(())
    }

    /// Removes type declaration `i`, `decl`, and its value-level states.
    pub(super) fn remove_type(&mut self, i: usize, decl: &ast::TypeDecl) {
        let module = self.src.decls[i].module.clone();
        self.src.remove_decl(i);
        self.src
            .value_states
            .retain(|(m, t, _), _| !(*m == module && *t == decl.name.text));
    }

    pub(super) fn rename_type(&mut self, q: &QName, to: &ast::Ident) -> R<()> {
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

    pub(super) fn add_relation(&mut self, r: &ast::RelationDecl) -> R<()> {
        if r.name.module.is_some() {
            return self.err_at(
                r.name.span(),
                "a relation name is qualified only in the engine's own declarations",
            );
        }
        self.add_decl("relation", &r.name.name.text, r.span)?;
        let schema = self.schema()?;
        let rel = self.rel_id(&schema, &r.name)?;
        let t = schema.ty(schema.relation(rel).elem_def);
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

    pub(super) fn drop_relation(&mut self, q: &QName) -> R<()> {
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
                if mentions_rel(&f.ty, rel) {
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
        drop.sort_unstable();
        self.src.remove_decls(|j, _| drop.contains(&j));
        self.src
            .states
            .retain(|(m, r, _), _| !(*m == module && r == name));
        Ok(())
    }

    pub(super) fn rename_relation(&mut self, q: &QName, to: &ast::Ident) -> R<()> {
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
        edits.entry(i).or_default().push((
            self.src.local(i, decl.name.name.span),
            to.text.clone().into(),
        ));
        for (file, span, r) in check::schema_refs(&schema) {
            if matches!(r, Ref::Relation(x) if x == rel) {
                let j = file.0 as usize;
                edits
                    .entry(j)
                    .or_default()
                    .push((self.src.local(j, span), to.text.clone().into()));
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
                if let ast::TypeKind::Key(_, rr) = &t.kind
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
                    .push((self.src.local(j, s), to.text.clone().into()));
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
    pub(super) fn set_decl(&self, name: &ast::Ident, rel: &QName) -> R<(usize, ast::SetDecl)> {
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

    pub(super) fn drop_set(&mut self, name: &ast::Ident, rel: &QName) -> R<()> {
        let (i, _) = self.set_decl(name, rel)?;
        self.src.remove_decl(i);
        Ok(())
    }

    pub(super) fn rename_set(&mut self, name: &ast::Ident, rel: &QName, to: &ast::Ident) -> R<()> {
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
            .push((self.src.local(i, decl.name.span), to.text.clone().into()));
        for (file, span, x) in check::schema_refs(&schema) {
            if matches!(&x, Ref::Set(y, s) if *y == r && *s == name.text) {
                let j = file.0 as usize;
                edits
                    .entry(j)
                    .or_default()
                    .push((self.src.local(j, span), to.text.clone().into()));
            }
        }
        self.each_selection(&schema, r, &module, &mut |j, ident| {
            if ident.text == name.text {
                edits
                    .entry(j)
                    .or_default()
                    .push((ident.span.range(), to.text.clone().into()));
            }
        });
        self.apply_edits(edits);
        Ok(())
    }

    /// Calls `f` with each constructor or named set name that a declaration selects from relation `r`:
    /// in `Key r.S` types, the scopes of `r`'s own clauses, and checks on `r`. Spans are local to the declaration's text.
    pub(super) fn each_selection(
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
                if let ast::TypeKind::Key(_, rr) = &t.kind
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

    pub(super) fn script(&mut self, w: &ast::Write) -> R<()> {
        let (schema, program) = self.checked()?;
        let mut refs = Vec::new();
        check::check_adhoc_recording(
            &schema,
            &program,
            &[ast::AdhocItem::Write(Box::new(w.clone()))],
            &BTreeMap::new(),
            &check::Bundles::default(),
            self.noted.is_some().then_some(&mut refs),
        )?;
        self.note(&schema, &refs);
        Ok(())
    }

    /// The state a lifecycle operator names, and how a refusal names it.
    pub(super) fn stage_key(&self, t: &CtorTarget) -> R<(bool, StateKey, String)> {
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

    pub(super) fn stage(&mut self, t: &CtorTarget, verb: &str) -> R<()> {
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
        if relation
            && next != Stage::Enabled
            && let CtorTarget::Relation(rr) = t
        {
            let schema = self.schema()?;
            let def = schema
                .relation(self.rel_id(&schema, &rr.relation)?)
                .elem_def;
            let c = schema
                .ty(def)
                .ctor_index(&key.2)
                .expect("a lifecycle target names a constructor of its relation");
            if let Some(f) = retag_into(&schema, def, c) {
                return self.err(format!(
                    "`{verb} {shown}` is refused: the delete action of field `{f}` retags into it"
                ));
            }
        }
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
