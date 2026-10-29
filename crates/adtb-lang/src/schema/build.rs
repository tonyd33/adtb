use super::*;

impl Builder<'_> {
    pub(super) fn err(&mut self, file: FileId, span: Span, msg: impl Into<String>) {
        self.diags.push(FileDiag {
            file,
            diag: Diag::new(span, msg),
        });
    }

    fn module_id(&mut self, name: &str) -> ModuleId {
        if let Some(m) = self.schema.module_named(name) {
            return m;
        }
        self.schema.modules.push(ModuleDef {
            name: name.to_string(),
            engine: self.schema.modules.is_empty() || ENGINE_MODULES.contains(&name),
            types: BTreeMap::new(),
            values: BTreeMap::new(),
        });
        ModuleId(self.schema.modules.len() as u32 - 1)
    }

    pub(super) fn run(&mut self) {
        let d = self.collect();
        self.declare_ahead();
        self.schema.principal = match d.principal {
            Some((id, _, _)) => id,
            None => self.schema.prelude_type("Principal"),
        };
        for &(file, module, id, r) in &d.relations {
            self.relation_head(file, module, id, r);
        }
        for &(file, module, s) in &d.sets {
            self.named_set(file, module, s);
        }
        self.check_set_consistency();
        for &(file, module, id, t) in &d.types {
            self.type_body(file, module, id, t);
        }
        if let Some((_, file, span)) = d.principal {
            self.check_principal(file, span);
        }
        for &(file, module, id, r) in &d.relations {
            self.relation_body(file, module, id, r);
        }
        for &(file, module, id, f) in &d.fns {
            self.fn_signature(file, module, id, f);
        }
        for i in 0..self.schema.aggregates.len() {
            self.aggregate_signature(i);
        }
        for (file, module, s) in d.stores {
            self.store(file, module, s);
        }
        self.checks();
        self.check_recursion();
        self.check_references();
        self.check_ordered();
    }

    fn declare_ahead(&mut self) {
        for a in self.ahead {
            let Some(module) = self.schema.module_named(&a.module) else {
                continue;
            };
            if a.typed {
                self.schema.late_sels.push((module, a.relation.clone()));
            }
            if self.schema.module(module).values.contains_key(&a.relation) {
                continue;
            }
            self.schema.ahead.push((module, a.relation.clone()));
        }
    }

    fn check_type_name(&mut self, file: FileId, name: &ast::Ident, principal_seen: bool) {
        let text = name.text.as_str();
        if RESERVED_TYPE_NAMES.contains(&text) {
            self.err(file, name.span, format!("`{text}` is a reserved type name"));
        } else if text == "Principal" {
            if principal_seen {
                self.err(file, name.span, "the schema declares `Principal` once");
            }
        } else if is_prelude_type(text) {
            self.err(file, name.span, format!("`{text}` is a prelude type"));
        }
    }

    /// Refuse a declared `Principal` other than a sum of `EndUser`, `Service { name: Text }` and
    /// `Anonymous`, with no header fields.
    fn check_principal(&mut self, file: FileId, span: Span) {
        let def = self.schema.ty(self.schema.principal);
        let shaped = |name: &str, fields: Option<&[(&str, Ty)]>| {
            def.ctors().iter().any(|c| {
                c.name == name
                    && fields.is_none_or(|want| {
                        c.fields.len() == want.len()
                            && c.fields
                                .iter()
                                .zip(want)
                                .all(|(f, (n, t))| f.name == *n && f.ty == *t)
                    })
            })
        };
        let ok = matches!(def.kind, TypeDefKind::Sum { .. })
            && def.header().is_empty()
            && def.ctors().len() == 3
            && shaped("EndUser", None)
            && shaped("Service", Some(&[("name", Ty::Base(Base::Text))]))
            && shaped("Anonymous", Some(&[]));
        if !ok {
            self.err(
                file,
                span,
                "`Principal` has exactly the constructors `EndUser`, `Service { name: Text }` and \
                 `Anonymous`",
            );
        }
    }

    fn check_fn_name(&mut self, file: FileId, name: &ast::Ident) {
        if PRELUDE_FUNCTIONS.contains(&name.text.as_str()) {
            self.err(
                file,
                name.span,
                format!("`{}` is a prelude function", name.text),
            );
        }
    }

    fn declare_value(&mut self, file: FileId, module: ModuleId, name: &ast::Ident, v: ValueDef) {
        let m = &mut self.schema.modules[module.0 as usize];
        if m.values.insert(name.text.clone(), v).is_some() {
            let msg = format!("`{}` is declared twice in module `{}`", name.text, m.name);
            self.err(file, name.span, msg);
        }
    }

    fn aggregate_signature(&mut self, i: usize) {
        let (file, module, a) = {
            let d = &self.schema.aggregates[i];
            (d.file, d.module, d.decl.clone())
        };
        let scope = TypeScope {
            module,
            params: &[],
            mode: TypeMode::Signature,
        };
        let mut params: Vec<(String, Ty)> = Vec::new();
        for p in &a.params {
            if params.iter().any(|(n, _)| *n == p.name.text) {
                self.err(
                    file,
                    p.name.span,
                    format!("parameter `{}` is declared twice", p.name.text),
                );
            }
            let ty = self.resolve_type(file, &scope, &p.ty).unwrap_or(Ty::Unit);
            params.push((p.name.text.clone(), ty));
        }
        let result = self
            .resolve_type(file, &scope, &a.result)
            .unwrap_or(Ty::Unit);
        let mut fields: Vec<(String, Ty)> = Vec::new();
        for f in &a.state {
            if fields.iter().any(|(n, _)| *n == f.name.text) {
                self.err(
                    file,
                    f.name.span,
                    format!("state field `{}` is declared twice", f.name.text),
                );
            }
            let ty = self.resolve_type(file, &scope, &f.ty).unwrap_or(Ty::Unit);
            fields.push((f.name.text.clone(), ty));
        }
        fields.sort_by(|x, y| x.0.cmp(&y.0));
        let def = &mut self.schema.aggregates[i];
        def.params = params;
        def.result = result;
        def.state = Ty::Record(fields);
    }

    fn fn_signature(&mut self, file: FileId, module: ModuleId, id: FnId, f: &ast::FnDecl) {
        let mut tparams = Vec::new();
        let mut names = Vec::new();
        for tp in &f.type_params {
            if names.contains(&tp.name.text) {
                self.err(
                    file,
                    tp.name.span,
                    format!("type parameter `{}` is declared twice", tp.name.text),
                );
            }
            let mut bounds = Vec::new();
            for b in &tp.bounds {
                match Bound::from_name(&b.text) {
                    Some(k) => bounds.push(k),
                    None => self.err(
                        file,
                        b.span,
                        format!(
                            "`{}` is not a bound; use `Eq`, `Ord`, `Hash` or `Num`",
                            b.text
                        ),
                    ),
                }
            }
            names.push(tp.name.text.clone());
            tparams.push((tp.name.text.clone(), bounds));
        }
        let scope = TypeScope {
            module,
            params: &names,
            mode: TypeMode::Signature,
        };
        let mut params = Vec::new();
        for p in &f.params {
            if params.iter().any(|(n, _): &(String, Ty)| *n == p.name.text) {
                self.err(
                    file,
                    p.name.span,
                    format!("parameter `{}` is declared twice", p.name.text),
                );
            }
            let ty = self.resolve_type(file, &scope, &p.ty).unwrap_or(Ty::Unit);
            params.push((p.name.text.clone(), ty));
        }
        let result = self
            .resolve_type(file, &scope, &f.result)
            .unwrap_or(Ty::Unit);
        let def = &mut self.schema.functions[id.0 as usize];
        def.type_params = tparams;
        def.params = params;
        def.result = result;
    }

    fn store(&mut self, file: FileId, module: ModuleId, s: &ast::StoreDecl) {
        match self.schema.lookup_value(module, &s.view) {
            Ok(ValueDef::Function(f)) => {
                let def = &self.schema.functions[f.0 as usize];
                if !def.params.is_empty() || !def.type_params.is_empty() {
                    return self.err(
                        file,
                        s.view.span(),
                        "a stored view is a function with no parameters",
                    );
                }
                if def.stored.is_some() {
                    return self.err(file, s.view.span(), "this view is stored twice");
                }
                self.schema.functions[f.0 as usize].stored = Some(s.clone());
            }
            Ok(_) => self.err(
                file,
                s.view.span(),
                format!("`{}` is not a function", s.view.name.text),
            ),
            Err(e) => self.err(file, s.view.span(), e),
        }
    }

    fn checks(&mut self) {
        let files = self.files;
        let mut i = 0;
        for (file, sf) in files {
            let module = self.schema.module_named(&sf.module.text).unwrap_or(PRELUDE);
            for decl in &sf.decls {
                let ast::Decl::Check(c) = decl else { continue };
                let resolved = self
                    .schema
                    .lookup_relation(module, &c.on.relation)
                    .and_then(|rel| {
                        let ctors = match &c.on.sel {
                            None => None,
                            Some(sel) => {
                                let set = self.schema.relation_sel(rel, sel)?;
                                self.schema
                                    .normalize_ctors(self.schema.relation(rel).elem_def, set)
                            }
                        };
                        Ok((rel, ctors))
                    });
                match resolved {
                    Ok((rel, ctors)) => {
                        self.schema.checks[i].relation = rel;
                        self.schema.checks[i].ctors = ctors;
                    }
                    Err(e) => self.err(*file, c.on.span, e),
                }
                i += 1;
            }
        }
    }

    fn check_recursion(&mut self) {
        let n = self.schema.types.len();
        let mut state = vec![0u8; n];
        let mut errs = Vec::new();
        fn reached(ty: &Ty, out: &mut Vec<TypeId>) {
            match ty {
                Ty::Data { def, args, .. } => {
                    out.push(*def);
                    for a in args {
                        reached(a, out);
                    }
                }
                Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => reached(t, out),
                _ => {}
            }
        }
        fn visit(s: &Schema, t: usize, state: &mut [u8], errs: &mut Vec<usize>) {
            if state[t] == 2 {
                return;
            }
            if state[t] == 1 {
                errs.push(t);
                return;
            }
            state[t] = 1;
            let def = &s.types[t];
            let mut next = Vec::new();
            match &def.kind {
                TypeDefKind::Sum { header, ctors, .. } => {
                    for f in header
                        .iter()
                        .chain(ctors.iter().flat_map(|c| c.fields.iter()))
                    {
                        reached(&f.ty, &mut next);
                    }
                }
                TypeDefKind::Newtype { base, .. } => reached(base, &mut next),
            }
            for d in next {
                visit(s, d.0 as usize, state, errs);
            }
            state[t] = 2;
        }
        for t in 0..n {
            visit(&self.schema, t, &mut state, &mut errs);
        }
        errs.sort_unstable();
        errs.dedup();
        for t in errs {
            let def = &self.schema.types[t];
            let (file, span, name) = (def.file, def.span, def.name.clone());
            self.err(
                file,
                span,
                format!("type `{name}` reaches itself through its fields; recursion goes through references"),
            );
        }
    }

    /// References are direct fields, `Key r.S` or `Option (Key r.S)`, of types used only as relation element types.
    fn check_references(&mut self) {
        let mut errs = Vec::new();
        let has_refs: Vec<bool> = self
            .schema
            .types
            .iter()
            .map(|t| match &t.kind {
                TypeDefKind::Sum { header, ctors, .. } => header
                    .iter()
                    .chain(ctors.iter().flat_map(|c| c.fields.iter()))
                    .any(|f| mentions_key(&f.ty)),
                TypeDefKind::Newtype { .. } => false,
            })
            .collect();
        for t in &self.schema.types {
            let TypeDefKind::Sum { header, ctors, .. } = &t.kind else {
                continue;
            };
            for f in header
                .iter()
                .chain(ctors.iter().flat_map(|c| c.fields.iter()))
            {
                let direct = matches!(f.ty, Ty::Key { .. }) || self.is_option_key(&f.ty);
                if !direct && mentions_key(&f.ty) {
                    errs.push((
                        t.file,
                        f.span,
                        "a reference is a direct field of type `Key r.S` or `Option (Key r.S)`"
                            .to_string(),
                    ));
                }
                let mut inner = Vec::new();
                collect_defs(&f.ty, &mut inner);
                for d in inner {
                    if has_refs[d.0 as usize] {
                        errs.push((
                            t.file,
                            f.span,
                            format!(
                                "type `{}` holds references, so it is a relation's element type, never a field's type",
                                self.schema.ty(d).name
                            ),
                        ));
                    }
                }
            }
        }
        let mut owner: HashMap<TypeId, RelId> = HashMap::new();
        for (i, r) in self.schema.relations.iter().enumerate() {
            let id = RelId(i as u32);
            if r.elem_def.0 == u32::MAX || r.key.len() != 1 {
                continue;
            }
            let def = self.schema.ty(r.elem_def);
            if let Some(f) = def.field(0, &r.key[0])
                && matches!(f.ty, Ty::Key { rel, .. } if rel == id)
            {
                owner.insert(r.elem_def, id);
            }
        }
        for (i, r) in self.schema.relations.iter().enumerate() {
            if let Some(&o) = owner.get(&r.elem_def)
                && o.0 as usize != i
            {
                errs.push((
                    r.file,
                    r.span,
                    format!(
                        "type `{}` is keyed by `Key {}`, so it is the element type of `{}` only",
                        self.schema.ty(r.elem_def).name,
                        self.schema.relation(o).name,
                        self.schema.relation(o).name
                    ),
                ));
            }
        }
        for (f, s, m) in errs {
            self.err(f, s, m);
        }
    }

    fn check_ordered(&mut self) {
        let mut errs = Vec::new();
        for t in &self.schema.types {
            if !t.ordered {
                continue;
            }
            for f in t
                .header()
                .iter()
                .chain(t.ctors().iter().flat_map(|c| c.fields.iter()))
            {
                if !self.schema.is_ordered(&f.ty) {
                    errs.push((
                        t.file,
                        f.span,
                        format!(
                            "`{}` is declared ordered but field `{}` has an unordered type",
                            t.name, f.name
                        ),
                    ));
                }
            }
        }
        for (ty, file, span) in std::mem::take(&mut self.ordered_args) {
            if !self.schema.is_ordered(&ty) {
                errs.push((file, span, "`Span a` needs an ordered `a`".to_string()));
            }
        }
        for (f, s, m) in errs {
            self.err(f, s, m);
        }
    }
}

/// The declarations of the files, by kind, collected before the passes that resolve them.
#[derive(Default)]
struct Declared<'a> {
    types: Vec<(FileId, ModuleId, TypeId, &'a ast::TypeDecl)>,
    relations: Vec<(FileId, ModuleId, RelId, &'a ast::RelationDecl)>,
    sets: Vec<(FileId, ModuleId, &'a ast::SetDecl)>,
    fns: Vec<(FileId, ModuleId, FnId, &'a ast::FnDecl)>,
    stores: Vec<(FileId, ModuleId, &'a ast::StoreDecl)>,
    /// The user's `Principal` type and where it is declared.
    principal: Option<(TypeId, FileId, Span)>,
}

impl<'a> Builder<'a> {
    /// Declare every name of the files, so later passes may refer forward.
    fn collect(&mut self) -> Declared<'a> {
        let mut d = Declared::default();
        for (file, sf) in self.files {
            let file = *file;
            let engine = file.is_engine();
            let module = self.module_id(&sf.module.text);
            if !engine && self.schema.module(module).engine {
                let msg = format!("no module may be named `{}`", sf.module.text);
                self.err(file, sf.module.span, msg);
            }
            for decl in &sf.decls {
                self.declare(file, module, decl, &mut d);
            }
        }
        d
    }

    fn declare(
        &mut self,
        file: FileId,
        module: ModuleId,
        decl: &'a ast::Decl,
        d: &mut Declared<'a>,
    ) {
        match decl {
            ast::Decl::Type(t) => self.declare_type(file, module, t, d),
            ast::Decl::Relation(r) => self.declare_relation(file, module, r, d),
            ast::Decl::Set(s) => d.sets.push((file, module, s)),
            ast::Decl::Fn(f) => {
                let id = FnId(self.schema.functions.len() as u32);
                self.check_fn_name(file, &f.name);
                self.declare_value(file, module, &f.name, ValueDef::Function(id));
                self.schema.functions.push(FunctionDef {
                    name: f.name.text.clone(),
                    module,
                    type_params: Vec::new(),
                    params: Vec::new(),
                    result: Ty::Unit,
                    body: f.body.clone(),
                    stored: None,
                    file,
                    span: f.span,
                });
                d.fns.push((file, module, id, f));
            }
            ast::Decl::Constraint(c) => {
                let id = self.schema.constraints.len() as u32;
                self.declare_value(file, module, &c.name, ValueDef::Constraint(id));
                self.schema.constraints.push(ConstraintDef {
                    name: c.name.text.clone(),
                    module,
                    query: c.query.clone(),
                    file,
                });
            }
            ast::Decl::Check(c) => {
                let id = self.schema.checks.len() as u32;
                self.declare_value(file, module, &c.name, ValueDef::Check(id));
                self.schema.checks.push(CheckDef {
                    name: c.name.text.clone(),
                    module,
                    relation: RelId(u32::MAX),
                    ctors: None,
                    predicate: c.predicate.clone(),
                    file,
                });
            }
            ast::Decl::Aggregate(a) => {
                let id = self.schema.aggregates.len() as u32;
                self.check_fn_name(file, &a.name);
                self.declare_value(file, module, &a.name, ValueDef::Aggregate(id));
                self.schema.aggregates.push(AggregateDef {
                    name: a.name.text.clone(),
                    module,
                    decl: (**a).clone(),
                    params: Vec::new(),
                    result: Ty::Unit,
                    state: Ty::Unit,
                    file,
                });
            }
            ast::Decl::Store(s) => d.stores.push((file, module, s)),
        }
    }

    fn declare_type(
        &mut self,
        file: FileId,
        module: ModuleId,
        t: &'a ast::TypeDecl,
        d: &mut Declared<'a>,
    ) {
        let prelude = file == FileId::PRELUDE;
        let id = TypeId(self.schema.types.len() as u32);
        if !prelude {
            self.check_type_name(file, &t.name, d.principal.is_some());
        }
        if !prelude && t.name.text == "Principal" {
            d.principal = Some((id, file, t.name.span));
        } else if self.schema.modules[module.0 as usize]
            .types
            .insert(t.name.text.clone(), id)
            .is_some()
        {
            self.err(
                file,
                t.name.span,
                format!("type `{}` is declared twice", t.name.text),
            );
        }
        let skeleton = self.type_skeleton(module, file, t);
        self.schema.types.push(skeleton);
        d.types.push((file, module, id, t));
    }

    fn declare_relation(
        &mut self,
        file: FileId,
        module: ModuleId,
        r: &'a ast::RelationDecl,
        d: &mut Declared<'a>,
    ) {
        let engine = file.is_engine();
        let id = RelId(self.schema.relations.len() as u32);
        let home = match &r.name.module {
            Some(m) if engine => self.module_id(&m.text),
            Some(_) => {
                self.err(
                    file,
                    r.name.span(),
                    "a relation name is qualified only in the engine's own declarations",
                );
                module
            }
            None => module,
        };
        self.declare_value(file, home, &r.name.name, ValueDef::Relation(id));
        self.schema.relations.push(RelationDef {
            name: r.name.name.text.clone(),
            module: home,
            elem: Ty::Unit,
            elem_def: TypeId(u32::MAX),
            key: Vec::new(),
            generate: None,
            indexes: Vec::new(),
            defaults: Vec::new(),
            computed: Vec::new(),
            sets: Vec::new(),
            volatile: false,
            file,
            span: r.span,
        });
        d.relations.push((file, module, id, r));
    }
}
