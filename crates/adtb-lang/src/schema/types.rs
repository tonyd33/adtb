use super::*;

impl Builder<'_> {
    /// The constructor and header names of a type, with field types left for later.
    pub(super) fn type_skeleton(
        &mut self,
        module: ModuleId,
        file: FileId,
        t: &ast::TypeDecl,
    ) -> TypeDef {
        let mut params = Vec::new();
        for p in &t.params {
            if params.contains(&p.text) {
                self.err(
                    file,
                    p.span,
                    format!("type parameter `{}` is declared twice", p.text),
                );
            }
            params.push(p.text.clone());
        }
        let field_names = |fs: &[ast::FieldType]| -> Vec<FieldDef> {
            fs.iter()
                .map(|f| FieldDef {
                    name: f.name.text.clone(),
                    ty: Ty::Unit,
                    on_delete: OnDelete::Restrict,
                    span: f.name.span,
                })
                .collect()
        };
        let kind = match &t.body {
            ast::TypeBody::New(..) => {
                if t.header.is_some() {
                    self.err(file, t.name.span, "a newtype has no header");
                }
                TypeDefKind::Newtype {
                    base: Ty::Unit,
                    domain: None,
                }
            }
            ast::TypeBody::Record(fs) => {
                if t.header.is_some() {
                    self.err(
                        file,
                        t.name.span,
                        "a header is allowed only when the body lists constructors",
                    );
                }
                TypeDefKind::Sum {
                    header: Vec::new(),
                    ctors: vec![CtorDef {
                        name: t.name.text.clone(),
                        fields: field_names(fs),
                        span: t.name.span,
                    }],
                    record: true,
                }
            }
            ast::TypeBody::Ctors(cs) => {
                let mut ctors: Vec<CtorDef> = Vec::new();
                for c in cs {
                    if ctors.iter().any(|d| d.name == c.name.text) {
                        self.err(
                            file,
                            c.name.span,
                            format!("constructor `{}` is listed twice", c.name.text),
                        );
                    }
                    ctors.push(CtorDef {
                        name: c.name.text.clone(),
                        fields: c.record.as_deref().map(field_names).unwrap_or_default(),
                        span: c.name.span,
                    });
                }
                TypeDefKind::Sum {
                    header: t.header.as_deref().map(field_names).unwrap_or_default(),
                    ctors,
                    record: false,
                }
            }
        };
        if let TypeDefKind::Sum { header, ctors, .. } = &kind {
            let mut seen = BTreeSet::new();
            for f in header {
                if !seen.insert(f.name.clone()) {
                    self.err(
                        file,
                        f.span,
                        format!("field `{}` is declared twice", f.name),
                    );
                }
            }
            for c in ctors {
                let mut own = BTreeSet::new();
                for f in &c.fields {
                    if !own.insert(f.name.clone()) {
                        self.err(
                            file,
                            f.span,
                            format!("field `{}` is declared twice", f.name),
                        );
                    } else if seen.contains(&f.name) {
                        self.err(
                            file,
                            f.span,
                            format!("field `{}` of `{}` is also a header field", f.name, c.name),
                        );
                    }
                }
            }
        }
        TypeDef {
            name: t.name.text.clone(),
            module,
            params,
            ordered: t.ordered,
            kind,
            file,
            span: t.span,
        }
    }

    pub fn resolve_type(&mut self, file: FileId, scope: &TypeScope, t: &ast::Type) -> Option<Ty> {
        match resolve_type(&self.schema, scope, t, &mut self.ordered_args, file) {
            Ok(ty) => Some(ty),
            Err((span, msg)) => {
                self.err(file, span, msg);
                None
            }
        }
    }

    fn resolve_fields(
        &mut self,
        file: FileId,
        module: ModuleId,
        params: &[String],
        fields: &[ast::FieldType],
        own: TypeId,
    ) -> Vec<FieldDef> {
        let scope = TypeScope {
            module,
            params,
            mode: TypeMode::Field,
        };
        let mut out = Vec::new();
        for f in fields {
            let ty = self.resolve_type(file, &scope, &f.ty).unwrap_or(Ty::Unit);
            let on_delete = match &f.on_delete {
                None => OnDelete::Restrict,
                Some(a) => {
                    let is_key = matches!(ty, Ty::Key { .. });
                    let is_opt_key = self.is_option_key(&ty);
                    if !is_key && !is_opt_key {
                        self.err(
                            file,
                            f.name.span,
                            "`on delete` is allowed only on a `Key` field or an `Option` of one",
                        );
                    }
                    match &a.action {
                        ast::RefAction::Restrict => OnDelete::Restrict,
                        ast::RefAction::Cascade => OnDelete::Cascade,
                        ast::RefAction::Clear => {
                            if !is_opt_key {
                                self.err(
                                    file,
                                    f.name.span,
                                    "`on delete clear` is allowed only on an `Option (Key ...)` field",
                                );
                            }
                            OnDelete::Clear
                        }
                        ast::RefAction::Retag(c, rec) => {
                            match self.schema.ty(own).ctor_index(&c.text) {
                                Some(i) => OnDelete::Retag(i, rec.clone()),
                                None => {
                                    let msg = format!(
                                        "`{}` is not a constructor of `{}`",
                                        c.text,
                                        self.schema.ty(own).name
                                    );
                                    self.err(file, c.span, msg);
                                    OnDelete::Restrict
                                }
                            }
                        }
                    }
                }
            };
            out.push(FieldDef {
                name: f.name.text.clone(),
                ty,
                on_delete,
                span: f.name.span,
            });
        }
        out
    }

    pub(super) fn is_option_key(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Data { def, args, .. } => {
                *def == self.schema.prelude_type("Option")
                    && matches!(args.as_slice(), [Ty::Key { .. }])
            }
            _ => false,
        }
    }

    pub(super) fn type_body(
        &mut self,
        file: FileId,
        module: ModuleId,
        id: TypeId,
        t: &ast::TypeDecl,
    ) {
        let params = self.schema.ty(id).params.clone();
        let kind = match &t.body {
            ast::TypeBody::New(base, domain) => {
                let scope = TypeScope {
                    module,
                    params: &params,
                    mode: TypeMode::Field,
                };
                let ty = self.resolve_type(file, &scope, base).unwrap_or(Ty::Unit);
                if !matches!(ty, Ty::Base(_)) && ty != Ty::Unit {
                    self.err(file, base.span, "a newtype is over a base type");
                }
                TypeDefKind::Newtype {
                    base: ty,
                    domain: domain.clone(),
                }
            }
            ast::TypeBody::Record(fs) => {
                let fields = self.resolve_fields(file, module, &params, fs, id);
                TypeDefKind::Sum {
                    header: Vec::new(),
                    ctors: vec![CtorDef {
                        name: t.name.text.clone(),
                        fields,
                        span: t.name.span,
                    }],
                    record: true,
                }
            }
            ast::TypeBody::Ctors(cs) => {
                let header = t
                    .header
                    .as_deref()
                    .map(|h| self.resolve_fields(file, module, &params, h, id))
                    .unwrap_or_default();
                let ctors = cs
                    .iter()
                    .map(|c| CtorDef {
                        name: c.name.text.clone(),
                        fields: c
                            .record
                            .as_deref()
                            .map(|fs| self.resolve_fields(file, module, &params, fs, id))
                            .unwrap_or_default(),
                        span: c.name.span,
                    })
                    .collect();
                TypeDefKind::Sum {
                    header,
                    ctors,
                    record: false,
                }
            }
        };
        self.schema.types[id.0 as usize].kind = kind;
    }
}

/// Resolves a written type in `scope`, recording `Span` arguments whose order is checked later.
pub fn resolve_type(
    schema: &Schema,
    scope: &TypeScope,
    t: &ast::Type,
    ordered_args: &mut Vec<(Ty, FileId, Span)>,
    file: FileId,
) -> Result<Ty, (Span, String)> {
    Resolver {
        schema,
        scope,
        file,
    }
    .ty(t, ordered_args)
}

type Resolved<T> = Result<T, (Span, String)>;

type Ordered = Vec<(Ty, FileId, Span)>;

struct Resolver<'a> {
    schema: &'a Schema,
    scope: &'a TypeScope<'a>,
    file: FileId,
}

impl Resolver<'_> {
    fn field(&self) -> bool {
        self.scope.mode == TypeMode::Field
    }

    fn ty(&self, t: &ast::Type, ordered_args: &mut Ordered) -> Resolved<Ty> {
        let field = self.field();
        match &t.kind {
            ast::TypeKind::Var(v) => self
                .scope
                .params
                .iter()
                .position(|p| *p == v.text)
                .map(|i| Ty::Param(i as u32))
                .ok_or_else(|| (v.span, format!("no type parameter `{}`", v.text))),
            ast::TypeKind::Unit => {
                if field {
                    Err((t.span, "a field has no unit type".to_string()))
                } else {
                    Ok(Ty::Unit)
                }
            }
            ast::TypeKind::Fn(a, b) => {
                if field {
                    return Err((t.span, "a field never has a function type".to_string()));
                }
                Ok(Ty::Fn(
                    Box::new(self.ty(a, ordered_args)?),
                    Box::new(self.ty(b, ordered_args)?),
                ))
            }
            ast::TypeKind::Record(fs) => {
                if field {
                    return Err((
                        t.span,
                        "a declared field's type is a named type; declare a record type for it"
                            .to_string(),
                    ));
                }
                let mut out = self.fields(fs, ordered_args)?;
                out.sort_by(|a, b| a.0.cmp(&b.0));
                Ok(Ty::Record(out))
            }
            ast::TypeKind::Sum(cs) => self.sum(t, cs, ordered_args),
            ast::TypeKind::Key(_, r) => self.key(r),
            ast::TypeKind::Named { name, sel, args } => {
                self.named(t, name, sel.as_ref(), args, ordered_args)
            }
        }
    }

    /// Returns the fields written, in order, refusing a name written twice.
    fn fields(
        &self,
        fs: &[ast::FieldType],
        ordered_args: &mut Ordered,
    ) -> Resolved<Vec<(String, Ty)>> {
        let mut out: Vec<(String, Ty)> = Vec::new();
        for f in fs {
            if out.iter().any(|(n, _)| *n == f.name.text) {
                return Err((
                    f.name.span,
                    format!("field `{}` is written twice", f.name.text),
                ));
            }
            out.push((f.name.text.clone(), self.ty(&f.ty, ordered_args)?));
        }
        Ok(out)
    }

    fn sum(&self, t: &ast::Type, cs: &[ast::AnonCtor], ordered_args: &mut Ordered) -> Resolved<Ty> {
        if self.field() {
            return Err((
                t.span,
                "a declared field's type is a named type; declare a sum type for it".to_string(),
            ));
        }
        let mut out: Vec<(String, Vec<(String, Ty)>)> = Vec::new();
        for c in cs {
            if out.iter().any(|(n, _)| *n == c.name.text) {
                return Err((
                    c.name.span,
                    format!("constructor `{}` is written twice", c.name.text),
                ));
            }
            let mut fields = self.fields(c.record.as_deref().unwrap_or_default(), ordered_args)?;
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            out.push((c.name.text.clone(), fields));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Ty::Sum {
            ctors: out,
            sel: None,
        })
    }

    fn key(&self, r: &ast::RelationRef) -> Resolved<Ty> {
        let schema = self.schema;
        let module = self.scope.module;
        let rel = schema
            .lookup_relation(module, &r.relation)
            .or_else(|e| schema.relation_ahead(module, &r.relation).ok_or(e))
            .map_err(|e| (r.span, e))?;
        let late = schema.selects_late(rel);
        let ctors = match &r.sel {
            None => None,
            Some(_) if late && schema.ahead_relation(rel).is_some() => None,
            Some(_) if schema.ahead_relation(rel).is_some() => {
                return Err((
                    r.span,
                    format!(
                        "`{}` is added by a later operator and its type is not added by this migration, so only `Key {}` may name it",
                        r.relation.name.text, r.relation.name.text
                    ),
                ));
            }
            Some(sel) => match schema.relation_sel(rel, sel) {
                Ok(set) => schema.normalize_ctors(schema.relation(rel).elem_def, set),
                Err(_) if late => None,
                Err(e) => return Err((r.span, e)),
            },
        };
        Ok(Ty::Key { rel, ctors })
    }

    fn named(
        &self,
        t: &ast::Type,
        name: &ast::QName,
        sel: Option<&ast::CtorSel>,
        args: &[ast::TypeArg],
        ordered_args: &mut Ordered,
    ) -> Resolved<Ty> {
        let schema = self.schema;
        let found = schema
            .lookup_type(self.scope.module, name)
            .map_err(|e| (name.span(), e))?;
        let arity = |n: usize| -> Resolved<()> {
            if args.len() == n {
                Ok(())
            } else {
                Err((
                    t.span,
                    format!(
                        "`{}` takes {n} type argument{}, given {}",
                        name.name.text,
                        if n == 1 { "" } else { "s" },
                        args.len()
                    ),
                ))
            }
        };
        let no_sel = || -> Resolved<()> {
            match sel {
                None => Ok(()),
                Some(_) => Err((
                    t.span,
                    format!("`{}` has no constructors to select", name.name.text),
                )),
            }
        };
        match found {
            TypeName::Base(b) => {
                arity(0)?;
                no_sel()?;
                Ok(Ty::Base(b))
            }
            TypeName::Decimal => {
                no_sel()?;
                match args {
                    [ast::TypeArg::Int(s, span)] => {
                        if *s > 38 {
                            Err((*span, "a decimal scale is at most 38".to_string()))
                        } else {
                            Ok(Ty::Base(Base::Decimal(*s as u8)))
                        }
                    }
                    _ => Err((
                        t.span,
                        "`Decimal` takes its scale, a number: `Decimal 2`".to_string(),
                    )),
                }
            }
            TypeName::Set => {
                if self.field() {
                    return Err((
                        t.span,
                        "a field never holds a set; a many-valued fact is a relation".to_string(),
                    ));
                }
                arity(1)?;
                no_sel()?;
                Ok(Ty::Set(Box::new(self.type_args(args, ordered_args)?.remove(0))))
            }
            TypeName::Group => Err((
                t.span,
                "`Group K T` is the type of a group binder and is never written; a group passed on is a `Set T`"
                    .to_string(),
            )),
            TypeName::Tag => {
                arity(1)?;
                no_sel()?;
                let arg = self.type_args(args, ordered_args)?.remove(0);
                match &arg {
                    Ty::Data { def, .. }
                        if matches!(schema.ty(*def).kind, TypeDefKind::Sum { .. }) =>
                    {
                        Ok(Ty::Tag(Box::new(arg)))
                    }
                    _ => Err((t.span, "`Tag T` takes a declared sum".to_string())),
                }
            }
            TypeName::Decl(def) => {
                arity(schema.ty(def).params.len())?;
                let targs = self.type_args(args, ordered_args)?;
                if def == schema.prelude_type("Span") {
                    ordered_args.push((targs[0].clone(), self.file, t.span));
                }
                Ok(Ty::Data {
                    def,
                    args: targs,
                    ctors: self.declared_sel(t, def, sel)?,
                })
            }
        }
    }

    fn type_args(&self, args: &[ast::TypeArg], ordered_args: &mut Ordered) -> Resolved<Vec<Ty>> {
        args.iter()
            .map(|a| match a {
                ast::TypeArg::Type(t) => self.ty(t, ordered_args),
                ast::TypeArg::Int(_, span) => Err((
                    *span,
                    "a number is a type argument only as the scale of `Decimal`".to_string(),
                )),
            })
            .collect()
    }

    /// Returns the constructors `sel` selects of declared type `def`.
    fn declared_sel(
        &self,
        t: &ast::Type,
        def: TypeId,
        sel: Option<&ast::CtorSel>,
    ) -> Resolved<Option<CtorSet>> {
        let Some(sel) = sel else { return Ok(None) };
        let d = self.schema.ty(def);
        if matches!(d.kind, TypeDefKind::Newtype { .. }) {
            return Err((t.span, format!("newtype `{}` has no constructors", d.name)));
        }
        if self.field() {
            return Err((
                t.span,
                "a declared field's type is a whole sum, never a constructor row type".to_string(),
            ));
        }
        let set = self.schema.type_sel(def, sel).map_err(|e| (t.span, e))?;
        Ok(self.schema.normalize_ctors(def, set))
    }
}
