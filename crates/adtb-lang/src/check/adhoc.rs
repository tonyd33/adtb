use super::*;
use crate::ast::{
    AdhocItem, Command, CtorScope, CtorSel, ExplainTarget, Grant, PeerStmt, Privilege, RelationRef,
    RestorePoint, Revoke, RoleStmt, TempDecl,
};

impl Checker<'_> {
    /// Checks one ad hoc item; returns its term when it is a query, a write or a `transact` block.
    pub(super) fn adhoc_item(&mut self, item: &AdhocItem) -> R<Option<Tm>> {
        match item {
            AdhocItem::Expr(e) => self.statement(e).map(|o| Some(o.tm)),
            AdhocItem::Write(w) => self.write(w).map(Some),
            AdhocItem::Transact(b) => self.transact(b, None).map(|o| Some(o.tm)),
            AdhocItem::Command(c, span) => self.command(c, *span).map(|()| None),
            AdhocItem::Grant(Grant { privileges, .. })
            | AdhocItem::Revoke(Revoke { privileges, .. }) => {
                for p in privileges {
                    self.deployment_privilege(p)?;
                }
                Ok(None)
            }
            AdhocItem::Role(r, _) => self.role_stmt(r).map(|()| None),
            AdhocItem::Peer(PeerStmt::Map { .. } | PeerStmt::Unmap { .. }, _) => Ok(None),
            AdhocItem::Temp(_) => Ok(None),
        }
    }

    fn deployment_privilege(&mut self, p: &Privilege) -> R<()> {
        let operator = |span: Span, what: &str| {
            diag(
                span,
                format!(
                    "a grant of `{what}` on a relation variable of the schema is a schema operator; write it in a migration"
                ),
            )
        };
        let schema_relation = |c: &Self, r: &RelationRef, what: &str| -> R<()> {
            let span = c.relation_ref(r)?;
            let rel = c.relation_named(&r.relation)?;
            if c.schema.module(c.schema.relation(rel).module).engine {
                Ok(())
            } else {
                Err(operator(span, what))
            }
        };
        match p {
            Privilege::Read(r, _) => schema_relation(self, r, "read"),
            Privilege::Insert(r) => schema_relation(self, r, "insert"),
            Privilege::Update(r, _) => schema_relation(self, r, "update"),
            Privilege::Delete(r) => schema_relation(self, r, "delete"),
            Privilege::Retag(from, to) => {
                schema_relation(self, from, "retag")?;
                schema_relation(self, to, "retag")
            }
            Privilege::CallFn(f) => match self.lookup_value(f) {
                Ok(ValueDef::Function(_)) => Err(diag(
                    f.span(),
                    "`grant call fn` is a schema operator; write it in a migration",
                )),
                Ok(_) => Err(diag(
                    f.span(),
                    format!("`{}` is not a function", f.name.text),
                )),
                Err(e) => Err(diag(f.span(), e)),
            },
            Privilege::Call(..) | Privilege::Capability(_) => Ok(()),
        }
    }

    /// Resolves a relation and its constructor selection; returns the span of the reference.
    fn relation_ref(&self, r: &RelationRef) -> R<Span> {
        let rel = self.relation_named(&r.relation)?;
        if let Some(sel) = &r.sel {
            self.relation_sel(rel, sel).map_err(|e| diag(r.span, e))?;
        }
        Ok(r.span)
    }

    fn relation_named(&self, q: &ast::QName) -> R<RelId> {
        match self.lookup_value(q) {
            Ok(ValueDef::Relation(rel)) => Ok(rel),
            Ok(_) => Err(diag(
                q.span(),
                format!("`{}` is not a relation", q.name.text),
            )),
            Err(e) => Err(diag(q.span(), e)),
        }
    }

    fn role_stmt(&mut self, r: &RoleStmt) -> R<()> {
        match r {
            RoleStmt::Create(_) | RoleStmt::Drop(_) => Ok(()),
            RoleStmt::AlterPassword(_, e) => {
                if !matches!(&e.kind, ast::ExprKind::Name(q) if q.module.is_none()) {
                    return Err(diag(
                        e.span,
                        "the password is a parameter of the text, never written in it",
                    ));
                }
                self.check(e, &Ty::Base(Base::Text)).map(|_| ())
            }
        }
    }

    /// Returns the row type and key fields of a temp relation, `taken` holding the names of the
    /// temp relations declared before it.
    fn temp_decl(&self, t: &TempDecl, taken: &[String]) -> R<(Ty, Vec<String>)> {
        if taken.contains(&t.name.text) {
            return Err(diag(
                t.name.span,
                format!("temp relation `{}` is declared twice", t.name.text),
            ));
        }
        let q = ast::QName {
            module: None,
            name: t.name.clone(),
        };
        if let Ok(ValueDef::Relation(_)) = self.lookup_value(&q) {
            return Err(diag(
                t.name.span,
                format!(
                    "`{}` is a relation variable the text reads; a temp relation takes another name",
                    t.name.text
                ),
            ));
        }
        let ty = self.resolve_type(&t.ty)?;
        let candidates: Vec<String> = match &ty {
            Ty::Record(fs) => fs.iter().map(|(n, _)| n.clone()).collect(),
            Ty::Data { def, .. }
                if matches!(self.schema.ty(*def).kind, TypeDefKind::Sum { .. }) =>
            {
                let d = self.schema.ty(*def);
                let own = match d.ctors() {
                    [only] => only.fields.as_slice(),
                    _ => &[],
                };
                d.header()
                    .iter()
                    .chain(own)
                    .map(|f| f.name.clone())
                    .collect()
            }
            _ => {
                return Err(diag(
                    t.ty.span,
                    format!(
                        "a temp relation holds a declared sum or an anonymous record, found {}",
                        self.show(&ty)
                    ),
                ));
            }
        };
        let Some((fields, generate)) = &t.key else {
            if let Ty::Data { def, .. } = &ty
                && self.schema.ty(*def).ctors().len() > 1
            {
                return Err(diag(
                    t.span,
                    "a temp relation of a sum of several constructors keyed by its whole row is not checked yet",
                ));
            }
            return Ok((ty, candidates));
        };
        let mut seen: Vec<&str> = Vec::new();
        for f in fields {
            if !candidates.contains(&f.text) {
                return Err(diag(
                    f.span,
                    format!(
                        "key field `{}` is not a field of {}, or not one every constructor has",
                        f.text,
                        self.show(&ty)
                    ),
                ));
            }
            if seen.contains(&f.text.as_str()) {
                return Err(diag(
                    f.span,
                    format!("key field `{}` is named twice", f.text),
                ));
            }
            seen.push(&f.text);
        }
        if let Some(g) = generate {
            if !matches!(g.text.as_str(), "sequence" | "uuidv7") {
                return Err(diag(
                    g.span,
                    format!(
                        "`{}` is not a key generator; use `sequence` or `uuidv7`",
                        g.text
                    ),
                ));
            }
            return Err(diag(
                g.span,
                "a generated key is one field of type `Key` of its own relation, and no type names a temp relation",
            ));
        }
        Ok((ty, fields.iter().map(|f| f.text.clone()).collect()))
    }

    /// Returns the type of a key of the engine's relation `module::name`.
    fn engine_key(&self, module: &str, name: &str) -> Ty {
        let m = self.schema.module_named(module).expect("an engine module");
        let Some(&ValueDef::Relation(rel)) = self.schema.module(m).values.get(name) else {
            panic!("the engine declares `{module}::{name}`");
        };
        Ty::Key { rel, ctors: None }
    }

    fn set_setting(&mut self, name: &ast::Ident, value: &ast::Expr) -> R<()> {
        let m = self
            .schema
            .module_named("catalog")
            .expect("the catalog module");
        let def = self.schema.module(m).types["Setting"];
        let setting = self.schema.ty(def);
        let c = setting.ctor_index(&name.text).ok_or_else(|| {
            diag(
                name.span,
                format!("`{}` is not a constructor of `catalog::Setting`", name.text),
            )
        })?;
        let mut fields: Vec<(String, Ty)> = setting.ctors()[c as usize]
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        self.check(value, &Ty::Record(fields)).map(|_| ())
    }

    fn command(&mut self, c: &Command, span: Span) -> R<()> {
        match c {
            Command::Explain { target, .. } => match target {
                ExplainTarget::Registered { .. } => Ok(()),
                ExplainTarget::Expr(e) => self.statement(e).map(|_| ()),
                ExplainTarget::Write(w) => self.write(w).map(|_| ()),
                ExplainTarget::Transact(b) => self.transact(b, None).map(|_| ()),
            },
            Command::PinPlan(e) | Command::UnpinPlan(e) => {
                let plan = self.engine_key("catalog", "plans");
                self.check(e, &plan).map(|_| ())
            }
            Command::CancelCall(e) => {
                let call = self.engine_key("sys", "calls");
                self.check(e, &call).map(|_| ())
            }
            Command::TerminateSession(e) => {
                let session = self.engine_key("sys", "sessions");
                self.check(e, &session).map(|_| ())
            }
            Command::SetSetting(name, value) => self.set_setting(name, value),
            Command::Vacuum(r) | Command::Verify(Some(r)) => self.relation_named(r).map(|_| ()),
            Command::Reindex {
                relation, index, ..
            } => {
                let rel = self.relation_named(relation)?;
                if let Some((fields, scope)) = index {
                    let sel = match scope {
                        CtorScope::One(c) => CtorSel::One(c.clone()),
                        CtorScope::Many(cs) => CtorSel::Many(cs.clone()),
                    };
                    let set = self.relation_sel(rel, &sel).map_err(|e| diag(span, e))?;
                    let ctx = std::mem::replace(&mut self.ctx, Context::Schema);
                    let r = fields
                        .iter()
                        .try_for_each(|e| self.index_field(rel, Some(set.clone()), e).map(|_| ()));
                    self.ctx = ctx;
                    r?;
                }
                Ok(())
            }
            Command::Export { source, .. } => {
                let o = self.synth(source)?;
                if self.elem_ty(&o.ty).is_none() {
                    return Err(diag(
                        source.span,
                        format!("`export` takes a set, found {}", self.show(&o.ty)),
                    ));
                }
                Ok(())
            }
            Command::ArchiveAudit { through, .. } => {
                self.check(through, &Ty::Base(Base::I64)).map(|_| ())
            }
            Command::Dump { encrypt, .. } => match encrypt {
                Some(k) => self.synth(k).map(|_| ()),
                None => Ok(()),
            },
            Command::Restore { point, .. } => match point {
                Some(RestorePoint::AtInstant(e)) => {
                    self.check(e, &Ty::Base(Base::Instant)).map(|_| ())
                }
                Some(RestorePoint::AtPosition(e)) => {
                    let position = Ty::Data {
                        def: self.schema.prelude_type("Position"),
                        args: Vec::new(),
                        ctors: None,
                    };
                    self.check(e, &position).map(|_| ())
                }
                Some(RestorePoint::BeforeVersion(_)) | None => Ok(()),
            },
            Command::Verify(None)
            | Command::Compact
            | Command::FinalizeUpgrade
            | Command::RetireBundle(_)
            | Command::SupersedeBundle(..)
            | Command::RegisterBundle(_)
            | Command::ApplyMigrations(_)
            | Command::PrintSchema
            | Command::RotateKek(_)
            | Command::RotateDek
            | Command::DestroyDek(_)
            | Command::Backup { .. } => Ok(()),
        }
    }
}

/// Returns whether a value of `from` is numbered differently at `to`: an anonymous sum, or a set of one, with other constructors.
fn renumbers(from: &Ty, to: &Ty) -> bool {
    match (from, to) {
        (Ty::Sum { ctors: a, .. }, Ty::Sum { ctors: b, .. }) => {
            a.len() != b.len() || a.iter().zip(b).any(|(x, y)| x.0 != y.0)
        }
        (Ty::Set(x) | Ty::NonEmpty(x), Ty::Set(y) | Ty::NonEmpty(y)) => renumbers(x, y),
        _ => false,
    }
}

fn index_in(ctors: &[(String, Vec<(String, Ty)>)], name: &str) -> u32 {
    ctors
        .iter()
        .position(|(c, _)| c == name)
        .expect("a joined sum has every constructor of its operands") as u32
}

impl Checker<'_> {
    /// Returns `o` at `to`, a join of its type, with its anonymous constructors numbered in `to`.
    pub(super) fn widen(&mut self, o: Out, to: &Ty) -> Out {
        if !renumbers(&o.ty, to) {
            return o;
        }
        let tm = self.renumber(o.tm, to);
        Out {
            tm,
            ty: to.clone(),
            rows: o.rows,
        }
    }

    fn renumber(&mut self, t: Tm, to: &Ty) -> Tm {
        let from = self.terms.ty(t).clone();
        if !renumbers(&from, to) {
            return t;
        }
        let kind = match self.terms.kind(t).clone() {
            Kind::If(c, a, b) => Kind::If(c, self.renumber(a, to), self.renumber(b, to)),
            Kind::Let(x, e, b) => Kind::Let(x, e, self.renumber(b, to)),
            Kind::Case { scrutinee, arms } => Kind::Case {
                scrutinee,
                arms: arms
                    .into_iter()
                    .map(|(c, x, b)| (c, x, self.renumber(b, to)))
                    .collect(),
            },
            Kind::Unreachable => Kind::Unreachable,
            kind => match (&from, to) {
                (Ty::Sum { ctors, .. }, Ty::Sum { ctors: into, .. }) => match kind {
                    Kind::Ctor { ctor, fields } => {
                        let i = index_in(into, &ctors[ctor as usize].0);
                        let row = ty::restrict(self.schema, to, vec![i]);
                        return self.add(Kind::Ctor { ctor: i, fields }, row);
                    }
                    _ => return self.retag_anon(t, &from, to),
                },
                (_, Ty::Set(elem) | Ty::NonEmpty(elem)) => match kind {
                    Kind::EmptySet => Kind::EmptySet,
                    Kind::Singleton(e) => Kind::Singleton(self.renumber(e, elem)),
                    Kind::Union(a, b) => Kind::Union(self.renumber(a, to), self.renumber(b, to)),
                    Kind::For(x, src, b) => Kind::For(x, src, self.renumber(b, to)),
                    _ => {
                        let x = self.fresh(self.elem_ty(&from).expect("a set"));
                        let xt = self.var_tm(x);
                        let one = self.renumber(xt, elem);
                        let one = self.add(Kind::Singleton(one), to.clone());
                        Kind::For(x, t, one)
                    }
                },
                _ => unreachable!("only sums and sets are renumbered"),
            },
        };
        self.add(kind, to.clone())
    }

    /// A `case` over `t`, of anonymous sum `from`, rebuilding each constructor's value at `to`.
    fn retag_anon(&mut self, t: Tm, from: &Ty, to: &Ty) -> Tm {
        let (Ty::Sum { ctors, sel }, Ty::Sum { ctors: into, .. }) = (from, to) else {
            unreachable!("both are anonymous sums")
        };
        let mut arms = Vec::new();
        for (c, (name, fields)) in ctors.iter().enumerate() {
            let c = c as u32;
            if sel.as_ref().is_some_and(|s| !s.contains(&c)) {
                continue;
            }
            let y = self.fresh(ty::restrict(self.schema, from, vec![c]));
            let mut given = Vec::new();
            for (f, fty) in fields {
                let yt = self.var_tm(y);
                given.push((f.clone(), self.add(Kind::Field(yt, f.clone()), fty.clone())));
            }
            let i = index_in(into, name);
            let row = ty::restrict(self.schema, to, vec![i]);
            let body = self.add(
                Kind::Ctor {
                    ctor: i,
                    fields: given,
                },
                row,
            );
            arms.push((c, y, body));
        }
        self.add(Kind::Case { scrutinee: t, arms }, to.clone())
    }
}

/// Returns `schema` with the temp relations of `items` added, and the relation each temp
/// declaration declares, in order, or `None` when the text declares none. A declaration that is
/// refused adds its diagnostic to `diags` and declares nothing.
pub(super) fn temp_relations(
    schema: &Schema,
    items: &[AdhocItem],
    diags: &mut Vec<Diag>,
) -> Option<(Schema, Vec<Option<RelId>>)> {
    let decls: Vec<&TempDecl> = items
        .iter()
        .filter_map(|i| match i {
            AdhocItem::Temp(t) => Some(t),
            _ => None,
        })
        .collect();
    if decls.is_empty() {
        return None;
    }
    let mut out = schema.clone();
    let module = match out.module_named(TEMP_MODULE) {
        Some(m) => m,
        None => {
            out.modules.push(crate::schema::ModuleDef {
                name: TEMP_MODULE.to_string(),
                engine: false,
                types: BTreeMap::new(),
                values: BTreeMap::new(),
            });
            ModuleId(out.modules.len() as u32 - 1)
        }
    };
    let mut taken: Vec<String> = out.module(module).values.keys().cloned().collect();
    let mut declared = Vec::new();
    for t in decls {
        let c = Checker::new(schema, Visible::all(schema), Context::Adhoc);
        let (ty, key) = match c.temp_decl(t, &taken) {
            Ok(x) => x,
            Err(d) => {
                diags.push(d);
                declared.push(None);
                continue;
            }
        };
        let (elem, elem_def) = match ty {
            Ty::Record(fs) => {
                let def = TypeId(out.types.len() as u32);
                let fields = fs
                    .into_iter()
                    .map(|(name, ty)| crate::schema::FieldDef {
                        name,
                        ty,
                        on_delete: crate::schema::OnDelete::Restrict,
                        span: t.ty.span,
                    })
                    .collect();
                out.types.push(crate::schema::TypeDef {
                    name: t.name.text.clone(),
                    module,
                    params: Vec::new(),
                    ordered: false,
                    kind: TypeDefKind::Sum {
                        header: Vec::new(),
                        ctors: vec![crate::schema::CtorDef {
                            name: t.name.text.clone(),
                            fields,
                            span: t.ty.span,
                        }],
                        record: true,
                    },
                    file: FileId(0),
                    span: t.span,
                });
                let elem = Ty::Data {
                    def,
                    args: Vec::new(),
                    ctors: None,
                };
                (elem, def)
            }
            Ty::Data { def, args, .. } => (
                Ty::Data {
                    def,
                    args,
                    ctors: None,
                },
                def,
            ),
            _ => unreachable!("a temp relation holds a declared sum or a record"),
        };
        let rel = RelId(out.relations.len() as u32);
        out.relations.push(crate::schema::RelationDef {
            name: t.name.text.clone(),
            module,
            elem,
            elem_def,
            key,
            generate: None,
            indexes: Vec::new(),
            defaults: Vec::new(),
            computed: Vec::new(),
            sets: Vec::new(),
            volatile: false,
            file: FileId(0),
            span: t.span,
        });
        out.modules[module.0 as usize]
            .values
            .insert(t.name.text.clone(), ValueDef::Relation(rel));
        taken.push(t.name.text.clone());
        declared.push(Some(rel));
    }
    Some((out, declared))
}
