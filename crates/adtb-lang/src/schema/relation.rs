use super::*;
use crate::resolve::Unbound;

impl Builder<'_> {
    /// Returns the declared sum `r` ranges over, else reports why it names none.
    pub(super) fn relation_head(
        &mut self,
        file: FileId,
        module: ModuleId,
        r: &ast::RelationDecl,
    ) -> Option<TypeId> {
        let head = match &r.ty.kind {
            ast::TypeKind::Named {
                name, sel: None, ..
            } => self.schema.find_type_in(module, name, None),
            _ => Err(Unbound::Missing(
                "a relation's element type is a declared sum or record".to_string(),
            )),
        };
        match head {
            Ok(TypeName::Decl(t)) if matches!(self.schema.ty(t).kind, TypeDefKind::Sum { .. }) => {
                return Some(t);
            }
            Ok(_) => self.err(
                file,
                r.ty.span,
                "a relation's element type is a declared sum or record",
            ),
            Err(e) => self.diags.push(FileDiag {
                file,
                diag: e.at(r.ty.span),
            }),
        }
        None
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
        let def = self.schema.ty(self.schema.relation(rel).elem_def);
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
        let r = &mut self.schema.defs_mut().relations[rel.0 as usize];
        if r.sets.iter().any(|x| x.name == s.name.text) {
            let msg = format!("`{}` declares set `{}` twice", r.name, s.name.text);
            return self.err(file, s.name.span, msg);
        }
        r.sets.push(NamedSet {
            name: s.name.text.clone(),
            ctors: ctors.into_iter().collect(),
            file,
            span: s.span,
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
        let scope = TypeScope {
            module,
            params: &[],
            mode: TypeMode::Field,
            names: None,
        };
        if let Some(elem) = self.resolve_type(file, &scope, &r.ty, FnTypes::Refused(FN_RELATION)) {
            self.schema.defs_mut().relations[id.0 as usize].elem = elem;
        }
        let mut cx = Clauses {
            schema: &self.schema,
            diags: &mut self.diags,
            ordered_args: &mut self.ordered_args,
            file,
        };
        let def = cx.schema.ty(elem_def);
        let mut key: Option<Vec<String>> = None;
        let mut generate = None;
        let mut indexes = Vec::new();
        let mut defaults = Vec::new();
        let mut computed = Vec::new();
        let mut volatile = false;
        type Scoped = (Option<CtorSet>, Option<Vec<ScopePart>>);
        let scope_of = |cx: &mut Clauses, s: &Option<ast::CtorScope>| -> Scoped {
            let Some(s) = s.as_ref() else {
                return (None, None);
            };
            let sel = match s {
                ast::CtorScope::One(c) => ast::CtorSel::One(c.clone()),
                ast::CtorScope::Many(cs) => ast::CtorSel::Many(cs.clone()),
            };
            match (
                cx.schema.relation_sel(id, &sel),
                cx.schema.written_scope(id, Some(&sel)),
            ) {
                (Ok(cs), Ok(written)) => (Some(cs), written),
                (Err(e), _) | (_, Err(e)) => {
                    cx.err(r.span, e);
                    (None, None)
                }
            }
        };
        for clause in &r.clauses {
            match clause {
                ast::RelationClause::Key(fields, gen_name) => {
                    if key.is_some() {
                        cx.err(r.name.span(), "a relation declares one key");
                    }
                    let (names, g) = cx.key_clause(id, def, fields, gen_name.as_ref());
                    key = Some(names);
                    if g.is_some() {
                        generate = g;
                    }
                }
                ast::RelationClause::Volatile => {
                    volatile = true;
                    if !file.is_engine() {
                        cx.err(
                            r.name.span(),
                            "`volatile` appears only in the engine's own declarations",
                        );
                    }
                }
                ast::RelationClause::Unique(fields, s, texts) => {
                    let (scope, written) = scope_of(&mut cx, s);
                    indexes.push(IndexDef {
                        fields: fields.clone(),
                        texts: texts.clone(),
                        scope,
                        written,
                        interval: false,
                        unique: true,
                    });
                }
                ast::RelationClause::Index {
                    interval,
                    fields,
                    texts,
                    scope: s,
                } => {
                    let (scope, written) = scope_of(&mut cx, s);
                    indexes.push(IndexDef {
                        fields: fields.clone(),
                        texts: texts.clone(),
                        scope,
                        written,
                        interval: *interval,
                        unique: false,
                    });
                }
                ast::RelationClause::Default { ctor, field, value } => {
                    for d in cx.default_clause(def, ctor.as_ref(), field, value) {
                        if defaults.iter().any(|x: &DefaultDef| x.field == d.field) {
                            cx.err(field.span, "this default is declared twice");
                            break;
                        }
                        defaults.push(d);
                    }
                }
                ast::RelationClause::Computed(name, ty, e) => {
                    let scope = TypeScope {
                        module,
                        params: &[],
                        mode: TypeMode::Field,
                        names: None,
                    };
                    let ty = cx
                        .resolve_type(&scope, ty, FnTypes::Refused(FN_FIELD))
                        .unwrap_or(Ty::Unit);
                    if (0..def.ctors().len() as u32)
                        .any(|i| def.field_named(i, &name.text).is_some())
                    {
                        cx.err(
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
            cx.err(r.name.span(), "a relation declares exactly one key");
        }
        let rel = &mut self.schema.defs_mut().relations[id.0 as usize];
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
}

/// The parts of a builder a relation's clauses use while its element type is borrowed.
struct Clauses<'b> {
    schema: &'b Schema,
    diags: &'b mut Vec<FileDiag>,
    ordered_args: &'b mut Vec<(Ty, FileId, Span)>,
    file: FileId,
}

impl Clauses<'_> {
    fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(FileDiag {
            file: self.file,
            diag: Diag::new(span, msg),
        });
    }

    fn resolve_type(&mut self, scope: &TypeScope, t: &ast::Type, fns: FnTypes) -> Option<Ty> {
        match resolve_type(self.schema, scope, t, fns, self.ordered_args, self.file) {
            Ok(ty) => Some(ty),
            Err(diag) => {
                self.diags.push(FileDiag {
                    file: self.file,
                    diag,
                });
                None
            }
        }
    }

    /// Returns the fields of a `key` clause and its generator.
    fn key_clause(
        &mut self,
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
                    f.span,
                    format!(
                        "key field `{}` is not a header field or a field of the only constructor of `{}`",
                        f.text, def.name
                    ),
                );
            } else if names.contains(&f.text) {
                self.err(f.span, format!("key field `{}` is named twice", f.text));
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
                    g.span,
                    format!(
                        "`{}` is not a key generator; use `sequence` or `uuidv7`",
                        g.text
                    ),
                ),
                Some(k) => {
                    let self_key = names.len() == 1 && key_candidates.iter().any(|c| {
                        c.name == names[0]
                            && matches!(c.ty, Ty::Key { rel, .. } if rel == RelRef::Declared(id))
                    });
                    if !self_key {
                        self.err(
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

    /// Returns the default of each field a `default` clause fills, none when it names an unknown
    /// constructor or no field.
    fn default_clause(
        &mut self,
        def: &TypeDef,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
        value: &ast::DefaultValue,
    ) -> Vec<DefaultDef> {
        if let Some(c) = ctor
            && def.ctor_index(&c.text).is_none()
        {
            self.err(
                c.span,
                format!("`{}` is not a constructor of `{}`", c.text, def.name),
            );
            return Vec::new();
        }
        let fields = def.default_fields(ctor.map(|c| c.text.as_str()), &field.text);
        if fields.is_empty() {
            self.err(
                field.span,
                format!("`{}` has no field `{}`", def.name, field.text),
            );
        }
        let (value, written) = match value {
            ast::DefaultValue::Generate(g) => {
                if g.text != "clock" {
                    self.err(
                        g.span,
                        "a default generates only `clock`; keys generate in the `key` clause",
                    );
                } else if fields
                    .iter()
                    .any(|&f| def.field_def(f).ty != Ty::Base(Base::Instant))
                {
                    self.err(g.span, "`generate clock` fills an `Instant` field");
                }
                (DefaultValue::Generate(Generator::Clock), g.span)
            }
            ast::DefaultValue::Expr(e) => (DefaultValue::Expr(Box::new(e.clone())), e.span),
        };
        fields
            .into_iter()
            .map(|field| DefaultDef {
                field,
                value: value.clone(),
                written,
            })
            .collect()
    }
}

impl TypeDef {
    /// Returns the fields a default on `field`, of constructor `ctor` when one is named, fills:
    /// the header field of that name, else that field of each constructor it covers, in
    /// declaration order.
    pub fn default_fields(&self, ctor: Option<&str>, field: &str) -> Vec<FieldRef> {
        if let Some(f) = self.field(None, field) {
            return vec![f];
        }
        self.ctor_refs()
            .filter(|&c| ctor.is_none_or(|n| n == self.ctor_def(c).name))
            .filter_map(|c| self.field(Some(c), field))
            .collect()
    }
}

impl Schema {
    /// Returns what the `in` scope `sel` of relation `rel` names, sorted; `None` without `in`.
    pub fn written_scope(
        &self,
        rel: RelId,
        sel: Option<&ast::CtorSel>,
    ) -> Result<Option<Vec<ScopePart>>, String> {
        let Some(sel) = sel else {
            return Ok(None);
        };
        let mut parts = self.relation_scope(rel, sel)?;
        parts.sort_unstable();
        Ok(Some(parts))
    }

    /// Returns what the constructor selection `sel` after relation `rel` names, in written order.
    pub fn relation_scope(&self, rel: RelId, sel: &ast::CtorSel) -> Result<Vec<ScopePart>, String> {
        let r = self.relation(rel);
        let def = self.ty(r.elem_def);
        match sel {
            ast::CtorSel::One(c) => {
                if let Some(i) = def.ctor_index(&c.text) {
                    return Ok(vec![ScopePart::Ctor(i)]);
                }
                match r.sets.iter().position(|s| s.name == c.text) {
                    Some(s) => Ok(vec![ScopePart::Set(s)]),
                    None => Err(format!(
                        "`{}` is neither a constructor of `{}` nor a named set of `{}`",
                        c.text, def.name, r.name
                    )),
                }
            }
            ast::CtorSel::Many(cs) => {
                let mut out = Vec::new();
                for c in cs {
                    match def.ctor_index(&c.text) {
                        Some(i) if out.contains(&ScopePart::Ctor(i)) => {
                            return Err(format!("`{}` is listed twice", c.text));
                        }
                        Some(i) => out.push(ScopePart::Ctor(i)),
                        None => {
                            return Err(format!(
                                "`{}` is not a constructor of `{}`",
                                c.text, def.name
                            ));
                        }
                    }
                }
                Ok(out)
            }
        }
    }
}
