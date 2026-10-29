use super::*;

impl Builder<'_> {
    pub(super) fn relation_head(
        &mut self,
        file: FileId,
        module: ModuleId,
        id: RelId,
        r: &ast::RelationDecl,
    ) {
        let head = match &r.ty.kind {
            ast::TypeKind::Named {
                name, sel: None, ..
            } => self.schema.lookup_type(module, name),
            _ => Err("a relation's element type is a declared sum or record".to_string()),
        };
        match head {
            Ok(TypeName::Decl(t)) if matches!(self.schema.ty(t).kind, TypeDefKind::Sum { .. }) => {
                self.schema.relations[id.0 as usize].elem_def = t;
            }
            Ok(_) => self.err(
                file,
                r.ty.span,
                "a relation's element type is a declared sum or record",
            ),
            Err(e) => self.err(file, r.ty.span, e),
        }
    }

    pub(super) fn named_set(&mut self, file: FileId, module: ModuleId, s: &ast::SetDecl) {
        let qname = ast::QName {
            module: None,
            name: s.relation.clone(),
        };
        let rel = match self.schema.lookup_relation(module, &qname) {
            Ok(r) => r,
            Err(e) => return self.err(file, s.relation.span, e),
        };
        let elem_def = self.schema.relation(rel).elem_def;
        if elem_def.0 == u32::MAX {
            return;
        }
        let def = self.schema.ty(elem_def);
        let mut ctors = BTreeSet::new();
        let mut bad = Vec::new();
        for m in &s.members {
            match def.ctor_index(&m.text) {
                Some(i) => {
                    if !ctors.insert(i) {
                        bad.push((m.span, format!("`{}` is listed twice", m.text)));
                    }
                }
                None => bad.push((
                    m.span,
                    format!("`{}` is not a constructor of `{}`", m.text, def.name),
                )),
            }
        }
        let clash = def.ctor_index(&s.name.text).is_some();
        for (span, msg) in bad {
            self.err(file, span, msg);
        }
        if clash {
            self.err(
                file,
                s.name.span,
                format!("`{}` is already a constructor name", s.name.text),
            );
        }
        let r = &mut self.schema.relations[rel.0 as usize];
        if r.sets.iter().any(|x| x.name == s.name.text) {
            let msg = format!("`{}` declares set `{}` twice", r.name, s.name.text);
            return self.err(file, s.name.span, msg);
        }
        r.sets.push(NamedSet {
            name: s.name.text.clone(),
            ctors: ctors.into_iter().collect(),
        });
    }

    pub(super) fn check_set_consistency(&mut self) {
        let mut seen: HashMap<(TypeId, String), (CtorSet, String)> = HashMap::new();
        let mut errs = Vec::new();
        for r in &self.schema.relations {
            for s in &r.sets {
                let k = (r.elem_def, s.name.clone());
                match seen.get(&k) {
                    Some((cs, other)) if *cs != s.ctors => errs.push((
                        r.file,
                        r.span,
                        format!(
                            "named set `{}` of `{}` has other constructors than the set of that name of `{other}`",
                            s.name, r.name
                        ),
                    )),
                    Some(_) => {}
                    None => {
                        seen.insert(k, (s.ctors.clone(), r.name.clone()));
                    }
                }
            }
        }
        for (f, s, m) in errs {
            self.err(f, s, m);
        }
    }

    pub(super) fn relation_body(
        &mut self,
        file: FileId,
        module: ModuleId,
        id: RelId,
        r: &ast::RelationDecl,
    ) {
        let elem_def = self.schema.relation(id).elem_def;
        if elem_def.0 == u32::MAX {
            return;
        }
        let scope = TypeScope {
            module,
            params: &[],
            mode: TypeMode::Field,
        };
        if let Some(elem) = self.resolve_type(file, &scope, &r.ty) {
            self.schema.relations[id.0 as usize].elem = elem;
        }
        let def = self.schema.ty(elem_def).clone();
        let mut key: Option<Vec<String>> = None;
        let mut generate = None;
        let mut indexes = Vec::new();
        let mut defaults = Vec::new();
        let mut computed = Vec::new();
        let mut volatile = false;
        let scope_of = |b: &mut Self, s: &Option<ast::CtorScope>| -> Option<CtorSet> {
            let s = s.as_ref()?;
            let sel = match s {
                ast::CtorScope::One(c) => ast::CtorSel::One(c.clone()),
                ast::CtorScope::Many(cs) => ast::CtorSel::Many(cs.clone()),
            };
            match b.schema.relation_sel(id, &sel) {
                Ok(cs) => Some(cs),
                Err(e) => {
                    b.err(file, r.span, e);
                    None
                }
            }
        };
        for clause in &r.clauses {
            match clause {
                ast::RelationClause::Key(fields, gen_name) => {
                    if key.is_some() {
                        self.err(file, r.name.span(), "a relation declares one key");
                    }
                    let (names, g) = self.key_clause(file, id, &def, fields, gen_name.as_ref());
                    key = Some(names);
                    if g.is_some() {
                        generate = g;
                    }
                }
                ast::RelationClause::Volatile => {
                    volatile = true;
                    if !file.is_engine() {
                        self.err(
                            file,
                            r.name.span(),
                            "`volatile` appears only in the engine's own declarations",
                        );
                    }
                }
                ast::RelationClause::Unique(fields, s) => {
                    let scope = scope_of(self, s);
                    indexes.push(IndexDef {
                        fields: fields.clone(),
                        scope,
                        interval: false,
                        unique: true,
                    });
                }
                ast::RelationClause::Index {
                    interval,
                    fields,
                    scope: s,
                } => {
                    let scope = scope_of(self, s);
                    indexes.push(IndexDef {
                        fields: fields.clone(),
                        scope,
                        interval: *interval,
                        unique: false,
                    });
                }
                ast::RelationClause::Default { ctor, field, value } => {
                    let Some(d) = self.default_clause(file, &def, ctor.as_ref(), field, value)
                    else {
                        continue;
                    };
                    if defaults
                        .iter()
                        .any(|x: &DefaultDef| x.ctor == d.ctor && x.field == d.field)
                    {
                        self.err(file, field.span, "this default is declared twice");
                    }
                    defaults.push(d);
                }
                ast::RelationClause::Computed(name, ty, e) => {
                    let scope = TypeScope {
                        module,
                        params: &[],
                        mode: TypeMode::Field,
                    };
                    let ty = self.resolve_type(file, &scope, ty).unwrap_or(Ty::Unit);
                    if (0..def.ctors().len() as u32).any(|i| def.field(i, &name.text).is_some()) {
                        self.err(
                            file,
                            name.span,
                            format!("`{}` already has a field `{}`", def.name, name.text),
                        );
                    }
                    computed.push(ComputedDef {
                        name: name.text.clone(),
                        ty,
                        expr: e.clone(),
                    });
                }
            }
        }
        if key.is_none() {
            self.err(file, r.name.span(), "a relation declares exactly one key");
        }
        let rel = &mut self.schema.relations[id.0 as usize];
        rel.key = key.unwrap_or_default();
        rel.generate = generate;
        rel.indexes = indexes;
        rel.defaults = defaults;
        rel.computed = computed;
        rel.volatile = volatile;
        if let Ty::Data { args, .. } = &rel.elem
            && args.iter().any(has_param)
        {
            let span = rel.span;
            self.err(
                file,
                span,
                "a relation's element type has no free type parameters",
            );
        }
    }

    /// Returns the fields of a `key` clause and its generator.
    fn key_clause(
        &mut self,
        file: FileId,
        id: RelId,
        def: &TypeDef,
        fields: &[ast::Ident],
        gen_name: Option<&ast::Ident>,
    ) -> (Vec<String>, Option<Generator>) {
        let key_candidates: Vec<&FieldDef> = if def.ctors().len() == 1 {
            def.header()
                .iter()
                .chain(def.ctors()[0].fields.iter())
                .collect()
        } else {
            def.header().iter().collect()
        };
        let mut generate = None;
        let mut names = Vec::new();
        for f in fields {
            if !key_candidates.iter().any(|c| c.name == f.text) {
                self.err(
                    file,
                    f.span,
                    format!(
                        "key field `{}` is not a header field or a field of the only constructor of `{}`",
                        f.text, def.name
                    ),
                );
            } else if names.contains(&f.text) {
                self.err(
                    file,
                    f.span,
                    format!("key field `{}` is named twice", f.text),
                );
            }
            names.push(f.text.clone());
        }
        if let Some(g) = gen_name {
            let gen_kind = match g.text.as_str() {
                "sequence" => Some(Generator::Sequence),
                "uuidv7" => Some(Generator::Uuidv7),
                _ => None,
            };
            match gen_kind {
                None => self.err(
                    file,
                    g.span,
                    format!(
                        "`{}` is not a key generator; use `sequence` or `uuidv7`",
                        g.text
                    ),
                ),
                Some(k) => {
                    let self_key = names.len() == 1
                        && key_candidates.iter().any(|c| {
                            c.name == names[0] && matches!(c.ty, Ty::Key { rel, .. } if rel == id)
                        });
                    if !self_key {
                        self.err(
                            file,
                            g.span,
                            "a generated key is one field of type `Key` of its own relation",
                        );
                    }
                    generate = Some(k);
                }
            }
        }
        (names, generate)
    }

    /// Returns the default a `default` clause declares, `None` when it names an unknown constructor
    /// or no field.
    fn default_clause(
        &mut self,
        file: FileId,
        def: &TypeDef,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
        value: &ast::DefaultValue,
    ) -> Option<DefaultDef> {
        let ctor_idx = match ctor {
            None => None,
            Some(c) => match def.ctor_index(&c.text) {
                Some(i) => Some(i),
                None => {
                    self.err(
                        file,
                        c.span,
                        format!("`{}` is not a constructor of `{}`", c.text, def.name),
                    );
                    return None;
                }
            },
        };
        let field_ty = match ctor_idx {
            Some(i) => def.field(i, &field.text).map(|f| f.ty.clone()),
            None => (0..def.ctors().len() as u32)
                .find_map(|i| def.field(i, &field.text))
                .map(|f| f.ty.clone()),
        };
        let Some(field_ty) = field_ty else {
            self.err(
                file,
                field.span,
                format!("`{}` has no field `{}`", def.name, field.text),
            );
            return None;
        };
        let value = match value {
            ast::DefaultValue::Generate(g) => {
                if g.text != "clock" {
                    self.err(
                        file,
                        g.span,
                        "a default generates only `clock`; keys generate in the `key` clause",
                    );
                } else if field_ty != Ty::Base(Base::Instant) {
                    self.err(file, g.span, "`generate clock` fills an `Instant` field");
                }
                DefaultValue::Generate(Generator::Clock)
            }
            ast::DefaultValue::Expr(e) => DefaultValue::Expr(Box::new(e.clone())),
        };
        Some(DefaultDef {
            ctor: ctor_idx,
            field: field.text.clone(),
            value,
        })
    }
}
