use super::*;

/// The names a checker resolves, each recorded by the resolver that resolved it.
#[derive(Default)]
pub(super) struct Refs(RefCell<Vec<(Span, Ref)>>);

impl Refs {
    pub(super) fn take(&self) -> Vec<(Span, Ref)> {
        self.0.take()
    }

    fn push(&self, span: Span, r: Ref) {
        self.0.borrow_mut().push((span, r));
    }
}

impl Checker<'_> {
    /// Returns every name resolved so far, leaving none.
    pub(crate) fn resolved(&self) -> Vec<(Span, Ref)> {
        self.refs.take()
    }

    pub fn resolve_type(&self, t: &ast::Type) -> R<Ty> {
        let names: Vec<String> = self.tparams.iter().map(|(n, _)| n.clone()).collect();
        let scope =
            TypeScope::new(self.module(), &names, TypeMode::Signature).naming(self.vis.scope());
        let mut ordered = Vec::new();
        let ty = crate::schema::resolve_type(self.schema, &scope, t, &mut ordered, FileId(0))
            .map_err(|(span, msg)| diag(span, msg))?;
        crate::canon::type_refs(self.schema, self.module(), t, &mut self.refs.0.borrow_mut());
        for (arg, _, span) in ordered {
            if !self.ordered(&arg, span)? {
                return Err(diag(span, "`Span a` needs an ordered `a`"));
            }
        }
        Ok(ty)
    }

    pub(super) fn lookup_type(&self, name: &ast::QName) -> Result<TypeName, String> {
        let t = crate::resolve::lookup_type(self.schema, &self.vis, name);
        match t {
            Ok(TypeName::Decl(d)) => self.refs.push(name.name.span, Ref::Type(d)),
            Ok(_) => self.refs.push(name.name.span, Ref::BuiltinType),
            Err(_) => {}
        }
        t
    }

    pub(crate) fn lookup_relation(&self, name: &ast::QName) -> Result<RelId, String> {
        match self.lookup_value(name)? {
            ValueDef::Relation(rel) => Ok(rel),
            _ => Err(format!("`{}` is not a relation", name.name.text)),
        }
    }

    pub(crate) fn lookup_value(&self, name: &ast::QName) -> Result<ValueDef, String> {
        let v = crate::resolve::lookup_value(self.schema, &self.vis, name);
        if let Ok(ValueDef::Relation(r)) = v
            && self.hidden.contains(&r)
        {
            return Err(format!("no `{}` in scope", name.name.text));
        }
        let at = name.name.span;
        match v {
            Ok(ValueDef::Relation(r)) => self.refs.push(at, Ref::Relation(r)),
            Ok(ValueDef::Function(f)) => self.refs.push(at, Ref::Function(f)),
            Ok(ValueDef::Aggregate(a)) => self.refs.push(at, Ref::Aggregate(a)),
            _ => {}
        }
        v
    }

    /// Returns the constructors a selection over `def` names.
    pub(super) fn type_sel(
        &self,
        def: TypeId,
        rel: Option<RelId>,
        sel: &ast::CtorSel,
    ) -> Result<CtorSet, String> {
        self.record_sel(def, rel, sel);
        self.schema.type_sel(def, sel)
    }

    /// Returns the constructors a relation's selection names.
    pub(crate) fn relation_sel(&self, rel: RelId, sel: &ast::CtorSel) -> Result<CtorSet, String> {
        self.record_sel(self.schema.relation(rel).elem_def, Some(rel), sel);
        self.schema.relation_sel(rel, sel)
    }

    fn record_sel(&self, def: TypeId, rel: Option<RelId>, sel: &ast::CtorSel) {
        for n in sel_names(sel) {
            if self.declared_ctor(n.span, def, &n.text).is_none() {
                self.set_named(rel, n);
            }
        }
    }

    /// Returns whether `name` names a named set of `rel`.
    pub(super) fn set_named(&self, rel: Option<RelId>, name: &ast::Ident) -> bool {
        let Some(r) = rel.filter(|&r| {
            self.schema
                .relation(r)
                .sets
                .iter()
                .any(|s| s.name == name.text)
        }) else {
            return false;
        };
        self.refs.push(name.span, Ref::Set(r, name.text.clone()));
        true
    }

    /// Returns the index of the constructor of `def` named `name`.
    pub(crate) fn declared_ctor(&self, at: Span, def: TypeId, name: &str) -> Option<u32> {
        let c = self.schema.ty(def).ctor_index(name)?;
        self.refs.push(at, Ref::Ctor(def, c));
        Some(c)
    }

    /// Record the constructor at `at` as one of an anonymous sum or a tag.
    pub(super) fn anonymous_ctor(&self, at: Span) {
        self.refs.push(at, Ref::Anonymous);
    }

    /// Record the `match` at `span` as examining a value of `matched`.
    pub(super) fn matching(&self, span: Span, matched: Ty) {
        self.refs.push(span, Ref::Match(matched));
    }

    /// Returns whether `name` is a field of the rows of `def` in `ctors`, every constructor when
    /// `None`.
    pub(crate) fn field_named(
        &self,
        span: Span,
        def: TypeId,
        ctors: Option<&[u32]>,
        name: &str,
        pun: bool,
    ) -> bool {
        let t = self.schema.ty(def);
        let at = |ctor: Option<u32>| Ref::Field {
            def,
            ctor,
            name: name.to_string(),
            pun,
        };
        if t.field(None, name).is_some() {
            self.refs.push(span, at(None));
            return true;
        }
        let all: Vec<u32> = (0..t.ctors().len() as u32).collect();
        let mut found = false;
        for &c in ctors.unwrap_or(&all) {
            if t.ctors()[c as usize].fields.iter().any(|f| f.name == name) {
                self.refs.push(span, at(Some(c)));
                found = true;
            }
        }
        found
    }
}

fn sel_names(sel: &ast::CtorSel) -> &[ast::Ident] {
    match sel {
        ast::CtorSel::One(c) => std::slice::from_ref(c),
        ast::CtorSel::Many(cs) => cs,
    }
}
