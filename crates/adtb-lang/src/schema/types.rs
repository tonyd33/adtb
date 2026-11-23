use super::*;

/// Whether a written type may hold a function type, at any depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FnTypes {
    /// A function type is allowed at the top and on either side of an arrow; inside a field, a
    /// set element or a type argument it is refused.
    Allowed,
    /// Refused with this reason wherever a function type occurs in it.
    Refused(&'static str),
}

pub(crate) const FN_FIELD: &str = "a field never has a function type";
pub(crate) const FN_TYPE_ARGUMENT: &str = "a type argument is a value type, never a function type";
pub(crate) const FN_SET_ELEMENT: &str = "a set element never has a function type";
pub(crate) const FN_QUERY_RESULT: &str = "a query result never has a function type";
pub(crate) const FN_RELATION: &str = "a relation never holds a function";
pub(crate) const FN_AGGREGATE: &str = "an aggregate's input and result never have a function type";

impl FnTypes {
    /// Returns the rule for a part of the type other than a side of its arrow: refused with
    /// `reason` unless refused already.
    fn under(self, reason: &'static str) -> FnTypes {
        match self {
            FnTypes::Allowed => FnTypes::Refused(reason),
            FnTypes::Refused(_) => self,
        }
    }
}

impl Builder<'_> {
    /// The constructor and header names of a type, with field types left for later.
    pub(super) fn type_skeleton(
        &mut self,
        id: TypeId,
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
            id,
            name: t.name.text.clone(),
            module,
            params,
            ordered: t.ordered,
            kind,
            file,
            span: t.span,
        }
    }

    pub fn resolve_type(
        &mut self,
        file: FileId,
        scope: &TypeScope,
        t: &ast::Type,
        fns: FnTypes,
    ) -> Option<Ty> {
        match resolve_type(&self.schema, scope, t, fns, &mut self.ordered_args, file) {
            Ok(ty) => Some(ty),
            Err(d) => {
                self.diags.push(FileDiag { file, diag: d });
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
            names: None,
        };
        let mut out = Vec::new();
        for f in fields {
            let ty = self
                .resolve_type(file, &scope, &f.ty, FnTypes::Refused(FN_FIELD))
                .unwrap_or(Ty::Unit);
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
                *def == self.schema.prelude().option()
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
                    names: None,
                };
                let ty = self
                    .resolve_type(file, &scope, base, FnTypes::Refused(FN_FIELD))
                    .unwrap_or(Ty::Unit);
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
        self.schema.defs_mut().types[id.0 as usize].kind = kind;
    }
}

/// Resolve a written type in `scope`, where `fns` rules on a function type, recording `Span`
/// arguments whose order is checked later.
pub fn resolve_type(
    schema: &Schema,
    scope: &TypeScope,
    t: &ast::Type,
    fns: FnTypes,
    ordered_args: &mut Vec<(Ty, FileId, Span)>,
    file: FileId,
) -> Result<Ty, Diag> {
    Resolver {
        schema,
        scope,
        file,
    }
    .ty(t, fns, ordered_args)
}

type Resolved<T> = Result<T, Diag>;

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

    fn ty(&self, t: &ast::Type, fns: FnTypes, ordered_args: &mut Ordered) -> Resolved<Ty> {
        let field = self.field();
        match &t.kind {
            ast::TypeKind::Var(v) => self
                .scope
                .params
                .iter()
                .position(|p| *p == v.text)
                .map(|i| Ty::Param(i as u32))
                .ok_or_else(|| Diag::new(v.span, format!("no type parameter `{}`", v.text))),
            ast::TypeKind::Unit => {
                if field {
                    Err(Diag::new(t.span, "a field has no unit type".to_string()))
                } else {
                    Ok(Ty::Unit)
                }
            }
            ast::TypeKind::Fn(a, b) => match fns {
                FnTypes::Refused(reason) => Err(Diag::new(t.span, reason.to_string())),
                FnTypes::Allowed => Ok(Ty::Fn(
                    Box::new(self.ty(a, fns, ordered_args)?),
                    Box::new(self.ty(b, fns, ordered_args)?),
                )),
            },
            ast::TypeKind::Record(fs) => {
                if field {
                    return Err(Diag::new(
                        t.span,
                        "a declared field's type is a named type; declare a record type for it"
                            .to_string(),
                    ));
                }
                let out = self.fields(fs, fns, ordered_args)?;
                Ok(Ty::Record(Fields::new(out)))
            }
            ast::TypeKind::Sum(cs) => self.sum(t, cs, fns, ordered_args),
            ast::TypeKind::Key(_, r) => self.key(r),
            ast::TypeKind::Named { name, sel, args } => {
                self.named(t, name, sel.as_ref(), args, fns, ordered_args)
            }
        }
    }

    /// Returns the fields written, in order, refusing a name written twice.
    fn fields(
        &self,
        fs: &[ast::FieldType],
        fns: FnTypes,
        ordered_args: &mut Ordered,
    ) -> Resolved<Vec<(String, Ty)>> {
        let mut out: Vec<(String, Ty)> = Vec::new();
        for f in fs {
            if out.iter().any(|(n, _)| *n == f.name.text) {
                return Err(Diag::new(
                    f.name.span,
                    format!("field `{}` is written twice", f.name.text),
                ));
            }
            out.push((
                f.name.text.clone(),
                self.ty(&f.ty, fns.under(FN_FIELD), ordered_args)?,
            ));
        }
        Ok(out)
    }

    fn sum(
        &self,
        t: &ast::Type,
        cs: &[ast::AnonCtor],
        fns: FnTypes,
        ordered_args: &mut Ordered,
    ) -> Resolved<Ty> {
        if self.field() {
            return Err(Diag::new(
                t.span,
                "a declared field's type is a named type; declare a sum type for it".to_string(),
            ));
        }
        let mut out: Vec<(String, Fields)> = Vec::new();
        for c in cs {
            if out.iter().any(|(n, _)| *n == c.name.text) {
                return Err(Diag::new(
                    c.name.span,
                    format!("constructor `{}` is written twice", c.name.text),
                ));
            }
            let fields = self.fields(c.record.as_deref().unwrap_or_default(), fns, ordered_args)?;
            out.push((c.name.text.clone(), Fields::new(fields)));
        }
        Ok(Ty::Sum {
            ctors: Ctors::new(out),
            sel: None,
        })
    }

    fn key(&self, r: &ast::RelationRef) -> Resolved<Ty> {
        let schema = self.schema;
        let module = self.scope.module;
        let rel = schema
            .lookup_relation_in(module, &r.relation, self.scope.names)
            .map(RelRef::Declared)
            .or_else(|e| {
                schema
                    .relation_ahead(module, &r.relation)
                    .map(RelRef::Ahead)
                    .ok_or(e)
            })
            .map_err(|e| Diag::new(r.span, e))?;
        let late = schema.selects_late(rel);
        let written = |sel: &ast::CtorSel| match sel {
            ast::CtorSel::One(c) => c.text.clone(),
            ast::CtorSel::Many(cs) => format!(
                "({})",
                cs.iter()
                    .map(|c| c.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
        };
        let ctors = match (&r.sel, rel) {
            (None, _) => KeyCtors::All,
            (Some(sel), RelRef::Ahead(_)) if late => KeyCtors::Late(written(sel)),
            (Some(_), RelRef::Ahead(_)) => {
                return Err(Diag::new(
                    r.span,
                    format!(
                        "`{}` is added by a later operator and its type is not added by this migration, so only `Key {}` may name it",
                        r.relation.name.text, r.relation.name.text
                    ),
                ));
            }
            (Some(sel), RelRef::Declared(rel)) => match schema.relation_sel(rel, sel) {
                Ok(set) => {
                    KeyCtors::of_row(schema.normalize_ctors(schema.relation(rel).elem_def, set))
                }
                Err(_) if late => KeyCtors::Late(written(sel)),
                Err(e) => return Err(Diag::new(r.span, e)),
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
        fns: FnTypes,
        ordered_args: &mut Ordered,
    ) -> Resolved<Ty> {
        let schema = self.schema;
        let found = schema
            .find_type_in(self.scope.module, name, self.scope.names)
            .map_err(|e| e.at(name.span()))?;
        let arity = |n: usize| -> Resolved<()> {
            if args.len() == n {
                Ok(())
            } else {
                Err(Diag::new(
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
                Some(_) => Err(Diag::new(
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
                            Err(Diag::new(*span, "a decimal scale is at most 38".to_string()))
                        } else {
                            Ok(Ty::Base(Base::Decimal(*s as u8)))
                        }
                    }
                    _ => Err(Diag::new(
                        t.span,
                        "`Decimal` takes its scale, a number: `Decimal 2`".to_string(),
                    )),
                }
            }
            TypeName::Set => {
                if self.field() {
                    return Err(Diag::new(
                        t.span,
                        "a field never holds a set; a many-valued fact is a relation".to_string(),
                    ));
                }
                arity(1)?;
                no_sel()?;
                Ok(Ty::Set(Box::new(
                    self.type_args(args, fns.under(FN_SET_ELEMENT), ordered_args)?
                        .remove(0),
                )))
            }
            TypeName::Group => Err(Diag::new(
                t.span,
                "`Group K T` is the type of a group binder and is never written; a group passed on is a `Set T`"
                    .to_string(),
            )),
            TypeName::Tag => {
                arity(1)?;
                no_sel()?;
                let arg = self
                    .type_args(args, fns.under(FN_TYPE_ARGUMENT), ordered_args)?
                    .remove(0);
                match &arg {
                    Ty::Data { def, .. }
                        if matches!(schema.ty(*def).kind, TypeDefKind::Sum { .. }) =>
                    {
                        Ok(Ty::Tag(Box::new(arg)))
                    }
                    _ => Err(Diag::new(t.span, "`Tag T` takes a declared sum".to_string())),
                }
            }
            TypeName::Decl(def) => {
                arity(schema.ty(def).params.len())?;
                let targs = self.type_args(args, fns.under(FN_TYPE_ARGUMENT), ordered_args)?;
                if def == schema.prelude().span() {
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

    fn type_args(
        &self,
        args: &[ast::TypeArg],
        fns: FnTypes,
        ordered_args: &mut Ordered,
    ) -> Resolved<Vec<Ty>> {
        args.iter()
            .map(|a| match a {
                ast::TypeArg::Type(t) => self.ty(t, fns, ordered_args),
                ast::TypeArg::Int(_, span) => Err(Diag::new(
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
            return Err(Diag::new(
                t.span,
                format!("newtype `{}` has no constructors", d.name),
            ));
        }
        if self.field() {
            return Err(Diag::new(
                t.span,
                "a declared field's type is a whole sum, never a constructor row type".to_string(),
            ));
        }
        let set = self
            .schema
            .type_sel(def, sel)
            .map_err(|e| Diag::new(t.span, e))?;
        Ok(self.schema.normalize_ctors(def, set))
    }
}
