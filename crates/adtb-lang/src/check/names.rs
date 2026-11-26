use super::*;
use crate::resolve::Unbound;

/// The text a recorded name's span indexes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Origin {
    /// The text being checked.
    #[default]
    Checked,
    /// A schema file, such as the one declaring a default or computed field checked here.
    File(FileId),
}

/// The names a checker resolves, each recorded by the resolver that resolved it with the text
/// its span indexes.
#[derive(Default)]
pub(super) struct Refs {
    at: Cell<Origin>,
    names: RefCell<Vec<(Origin, Span, Ref)>>,
}

impl Refs {
    /// Returns every name recorded, each with the file its span indexes, `checked` being the
    /// text checked; leaves none.
    pub(super) fn take_in(&self, checked: FileId) -> Vec<(FileId, Span, Ref)> {
        self.names
            .take()
            .into_iter()
            .map(|(o, s, r)| match o {
                Origin::Checked => (checked, s, r),
                Origin::File(f) => (f, s, r),
            })
            .collect()
    }

    /// Returns the names recorded in the text being checked; leaves none.
    pub(super) fn take_checked(&self) -> Vec<(Span, Ref)> {
        self.names
            .take()
            .into_iter()
            .filter(|(o, ..)| *o == Origin::Checked)
            .map(|(_, s, r)| (s, r))
            .collect()
    }

    /// Returns every name recorded so far in the text being checked, leaving them.
    pub(super) fn seen(&self) -> Vec<(Span, Ref)> {
        self.names
            .borrow()
            .iter()
            .filter(|(o, ..)| *o == Origin::Checked)
            .map(|(_, s, r)| (*s, r.clone()))
            .collect()
    }

    fn push(&self, span: Span, r: Ref) {
        self.names.borrow_mut().push((self.at.get(), span, r));
    }
}

impl Checker<'_> {
    /// Run `f` over an expression relation `rel` declares, such as a default or a computed
    /// field: names resolve in `rel`'s module and are recorded in its file. Fail at `site`, the
    /// text that reached the expression: a schema body poisons it, since the expression's own
    /// check reports its failure.
    pub(super) fn in_relation<T>(
        &mut self,
        rel: RelId,
        site: Span,
        f: impl FnOnce(&mut Self) -> R<T>,
    ) -> R<T> {
        let decl = self.schema.relation(rel);
        let home = Visible::module(decl.module).scoped(self.vis.scope().cloned());
        let vis = std::mem::replace(&mut self.vis, home);
        let was = self.refs.at.replace(Origin::File(decl.file));
        let out = f(self);
        self.refs.at.set(was);
        self.vis = vis;
        let schema = matches!(self.ctx, Context::Schema);
        out.map_err(|d| match d.kind {
            DiagKind::Invalid | DiagKind::Poisoned if schema => Diag::poisoned(site),
            _ => Diag { span: site, ..d },
        })
    }

    /// Returns every name resolved so far in the text being checked, leaving none.
    pub(crate) fn resolved(&self) -> Vec<(Span, Ref)> {
        self.refs.take_checked()
    }

    pub fn resolve_type(&self, t: &ast::Type, fns: FnTypes) -> R<Ty> {
        let names: Vec<String> = self.tparams.iter().map(|(n, _)| n.clone()).collect();
        let scope =
            TypeScope::new(self.module(), &names, TypeMode::Signature).naming(self.vis.scope());
        let mut ordered = Vec::new();
        let ty = crate::schema::resolve_type(self.schema, &scope, t, fns, &mut ordered, FileId(0))?;
        let mut named = Vec::new();
        crate::canon::type_refs(self.schema, self.module(), t, &mut named);
        for (span, r) in named {
            self.refs.push(span, r);
        }
        for (arg, _, span) in ordered {
            if !self.ordered(&arg, span)? {
                return Err(diag(span, "`Span a` needs an ordered `a`"));
            }
        }
        Ok(ty)
    }

    pub(super) fn lookup_type(&self, name: &ast::QName) -> Result<TypeName, Unbound> {
        let t = crate::resolve::lookup_type(self.schema, &self.vis, name);
        match t {
            Ok(TypeName::Decl(d)) => self.refs.push(name.name.span, Ref::Type(d)),
            Ok(_) => self.refs.push(name.name.span, Ref::BuiltinType),
            Err(_) => {}
        }
        t
    }

    pub(crate) fn lookup_relation(&self, name: &ast::QName) -> Result<RelId, Unbound> {
        match self.lookup_value(name)? {
            ValueDef::Relation(rel) => Ok(rel),
            _ => Err(Unbound::Missing(format!(
                "`{}` is not a relation",
                name.name.text
            ))),
        }
    }

    pub(crate) fn lookup_value(&self, name: &ast::QName) -> Result<ValueDef, Unbound> {
        let v = self.find_value(name)?;
        self.record_value(name, v);
        Ok(v)
    }

    /// Returns the declaration `name` denotes, recording nothing.
    pub(super) fn find_value(&self, name: &ast::QName) -> Result<ValueDef, Unbound> {
        match crate::resolve::lookup_value(self.schema, &self.vis, name) {
            Ok(ValueDef::Relation(r)) if self.hidden.contains(&r) => Err(Unbound::Missing(
                format!("no `{}` in scope", name.name.text),
            )),
            v => v,
        }
    }

    /// Record that `name` denotes `v`.
    pub(super) fn record_value(&self, name: &ast::QName, v: ValueDef) {
        let at = name.name.span;
        if let Some(d) = &self.demand {
            match v {
                ValueDef::Function(f) => d.named(Demanded::Function(f)),
                ValueDef::Aggregate(a) => d.named(Demanded::Aggregate(a)),
                _ => {}
            }
        }
        match v {
            ValueDef::Relation(r) => self.refs.push(at, Ref::Relation(r)),
            ValueDef::Function(f) => self.refs.push(at, Ref::Function(f)),
            ValueDef::Aggregate(a) => self.refs.push(at, Ref::Aggregate(a)),
            _ => {}
        }
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

    pub(super) fn record_sel(&self, def: TypeId, rel: Option<RelId>, sel: &ast::CtorSel) {
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
