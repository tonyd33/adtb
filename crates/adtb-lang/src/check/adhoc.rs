use super::*;
use crate::ast::{
    AdhocItem, BundleName, CallArgs, CtorScope, CtorSel, ExplainTarget, Grant, PeerStmt, Privilege,
    RelationRef, Revoke, RoleStmt, TempDecl,
};
use crate::kernel::alpha_equal;

/// The registered queries of the live bundles, by bundle and query name.
#[derive(Clone, Debug, Default)]
pub struct Bundles {
    queries: BTreeMap<(String, String), BTreeMap<String, Vec<Ty>>>,
}

/// The bundles of a checker that knows none.
pub(super) static NO_BUNDLES: Bundles = Bundles {
    queries: BTreeMap::new(),
};

impl Bundles {
    /// Records that bundle `family-version` registers query `name` over parameters of types `params`.
    pub fn insert(&mut self, family: &str, version: &str, name: &str, params: Vec<Ty>) {
        self.queries
            .entry((family.to_string(), version.to_string()))
            .or_default()
            .insert(name.to_string(), params);
    }

    /// Returns the parameter types of registered query `query` of `bundle`.
    fn params(&self, bundle: &BundleName, query: &str) -> Result<&[Ty], String> {
        let name = format!("{}-{}", bundle.family, bundle.version);
        let queries = self
            .queries
            .get(&(bundle.family.clone(), bundle.version.clone()))
            .ok_or_else(|| format!("bundle `{name}` is not registered"))?;
        queries
            .get(query)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("bundle `{name}` registers no query `{query}`"))
    }
}

/// A checked command; each of its terms is in the arena of the text it was checked in.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Explain {
        analyze: bool,
        target: Explained,
    },
    /// A key of `catalog::plans`.
    PinPlan(Tm),
    /// A key of `catalog::plans`.
    UnpinPlan(Tm),
    /// A key of `sys::calls`.
    CancelCall(Tm),
    /// A key of `sys::sessions`.
    TerminateSession(Tm),
    Vacuum(RelId),
    Verify(Option<RelId>),
    /// `index` is the position of the one index rebuilt among the relation's indexes, `None` for every index.
    Reindex {
        relation: RelId,
        index: Option<usize>,
        online: bool,
    },
    Compact,
    FinalizeUpgrade,
    RetireBundle(BundleName),
    SupersedeBundle(BundleName, BundleName),
    /// `value` is a record of the fields of constructor `setting` of `catalog::Setting`.
    SetSetting {
        setting: u32,
        value: Tm,
    },
    /// `source` is a set.
    Export {
        source: Tm,
        path: String,
        sums_text: bool,
    },
    RegisterBundle(String),
    ApplyMigrations(Vec<String>),
    PrintSchema,
    /// `through` is an `I64`.
    ArchiveAudit {
        through: Tm,
        path: String,
    },
    RotateKek(String),
    RotateDek,
    DestroyDek(u64),
    Dump {
        path: String,
        encrypt: Option<Tm>,
    },
    Restore {
        from: String,
        to: String,
        point: Option<RestorePoint>,
    },
    Backup {
        path: String,
        incremental: bool,
    },
}

/// What `explain` explains.
#[derive(Clone, Debug, PartialEq)]
pub enum Explained {
    /// A query, a write or a `transact` block, as the term the same text gives as a statement.
    Statement(Tm),
    /// A call of a registered query, each argument at its parameter's type.
    Registered {
        bundle: BundleName,
        query: String,
        args: Vec<Tm>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum RestorePoint {
    BeforeVersion(u64),
    /// An `Instant`.
    AtInstant(Tm),
    /// A `Position`.
    AtPosition(Tm),
}

impl Command {
    /// Returns the terms of the command, in field order.
    pub fn terms(&self) -> Vec<Tm> {
        self.clone().terms_mut().into_iter().map(|t| *t).collect()
    }

    /// Returns this command with its terms replaced by `ts`, given in the order of [`Command::terms`].
    pub fn with_terms(&self, ts: &[Tm]) -> Command {
        let mut c = self.clone();
        let slots = c.terms_mut();
        assert_eq!(slots.len(), ts.len(), "one term per term position");
        for (slot, t) in slots.into_iter().zip(ts) {
            *slot = *t;
        }
        c
    }

    fn terms_mut(&mut self) -> Vec<&mut Tm> {
        match self {
            Command::Explain { target, .. } => match target {
                Explained::Statement(t) => vec![t],
                Explained::Registered { args, .. } => args.iter_mut().collect(),
            },
            Command::PinPlan(t)
            | Command::UnpinPlan(t)
            | Command::CancelCall(t)
            | Command::TerminateSession(t)
            | Command::SetSetting { value: t, .. }
            | Command::Export { source: t, .. }
            | Command::ArchiveAudit { through: t, .. }
            | Command::Dump {
                encrypt: Some(t), ..
            }
            | Command::Restore {
                point: Some(RestorePoint::AtInstant(t) | RestorePoint::AtPosition(t)),
                ..
            } => vec![t],
            Command::Vacuum(_)
            | Command::Verify(_)
            | Command::Reindex { .. }
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
            | Command::Dump { encrypt: None, .. }
            | Command::Restore {
                point: Some(RestorePoint::BeforeVersion(_)) | None,
                ..
            }
            | Command::Backup { .. } => Vec::new(),
        }
    }
}

/// A checked command of ad hoc text and its place among the text's statements.
#[derive(Clone, Debug, PartialEq)]
pub struct AdhocCommand {
    /// The number of terms of [`Adhoc::items`] the text holds before the command.
    pub at: usize,
    pub command: Command,
    pub span: Span,
}

/// A checked ad hoc item.
pub(super) enum Checked {
    Statement(Tm),
    Command(Command, Span),
    Nothing,
}

impl Checker<'_> {
    /// Checks one ad hoc item; returns its term when it is a query, a write or a `transact` block,
    /// and its checked form when it is a command.
    pub(super) fn adhoc_item(&mut self, item: &AdhocItem) -> R<Checked> {
        match item {
            AdhocItem::Expr(e) => self.statement(e).map(|o| Checked::Statement(o.tm)),
            AdhocItem::Write(w) => self.write(w).map(Checked::Statement),
            AdhocItem::Transact(b) => self.transact(b, None).map(|o| Checked::Statement(o.tm)),
            AdhocItem::Command(c, span) => {
                self.command(c, *span).map(|c| Checked::Command(c, *span))
            }
            AdhocItem::Grant(Grant { privileges, .. })
            | AdhocItem::Revoke(Revoke { privileges, .. }) => {
                for p in privileges {
                    self.deployment_privilege(p)?;
                }
                Ok(Checked::Nothing)
            }
            AdhocItem::Role(r, _) => self.role_stmt(r).map(|()| Checked::Nothing),
            AdhocItem::Peer(PeerStmt::Map { .. } | PeerStmt::Unmap { .. }, _)
            | AdhocItem::Temp(_) => Ok(Checked::Nothing),
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
            Privilege::Retag(from, to, _) => {
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
            Privilege::Call(..) | Privilege::CallAt(_) | Privilege::Capability(_) => Ok(()),
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

    /// Returns the stored relation `q` names for the command `command`, refusing a temp relation.
    fn stored_relation_named(&self, q: &ast::QName, command: &str) -> R<RelId> {
        let rel = self.relation_named(q)?;
        if self.schema.module_named(TEMP_MODULE) == Some(self.schema.relation(rel).module) {
            return Err(diag(
                q.span(),
                format!(
                    "`{command}` takes a stored relation; `{}` is a temp relation",
                    q.name.text
                ),
            ));
        }
        Ok(rel)
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

    /// Returns a module the text reads that declares `name` as a relation, function, other value,
    /// type or constructor.
    fn declaring_module(&self, name: &str) -> Option<ModuleId> {
        self.vis.modules.iter().copied().find(|&m| {
            let def = self.schema.module(m);
            def.values.contains_key(name)
                || def.types.contains_key(name)
                || def
                    .types
                    .values()
                    .any(|&t| self.schema.ty(t).ctor_index(name).is_some())
        })
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
        if let Some(m) = self.declaring_module(&t.name.text) {
            return Err(diag(
                t.name.span,
                format!(
                    "module `{}` declares `{}`; a temp relation takes a name no module the text reads declares",
                    self.schema.module(m).name,
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
                    "a temp relation of a sum of several constructors declares its key",
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

    fn set_setting(&mut self, name: &ast::Ident, value: &ast::Expr) -> R<Command> {
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
        let value = self.check(value, &Ty::Record(fields))?.tm;
        Ok(Command::SetSetting { setting: c, value })
    }

    fn explained(&mut self, target: &ExplainTarget) -> R<Explained> {
        let statement = match target {
            ExplainTarget::Registered {
                bundle,
                query,
                args,
            } => return self.registered_call(bundle, query, args),
            ExplainTarget::Expr(e) => self.statement(e)?.tm,
            ExplainTarget::Write(w) => self.write(w)?,
            ExplainTarget::Transact(b) => self.transact(b, None)?.tm,
        };
        Ok(Explained::Statement(statement))
    }

    fn registered_call(
        &mut self,
        bundle: &BundleName,
        query: &ast::Ident,
        args: &CallArgs,
    ) -> R<Explained> {
        let params = self
            .bundles
            .params(bundle, &query.text)
            .map_err(|e| diag(bundle.span.to(query.span), e))?;
        let CallArgs::List(es) = args else {
            return Err(diag(
                query.span,
                "a registered query takes its arguments as a list",
            ));
        };
        if es.len() != params.len() {
            return Err(diag(
                query.span,
                format!(
                    "`{}-{}::{}` takes {} argument{}, given {}",
                    bundle.family,
                    bundle.version,
                    query.text,
                    params.len(),
                    if params.len() == 1 { "" } else { "s" },
                    es.len()
                ),
            ));
        }
        let args = es
            .iter()
            .zip(params)
            .map(|(e, ty)| self.check(e, ty).map(|o| o.tm))
            .collect::<R<Vec<Tm>>>()?;
        Ok(Explained::Registered {
            bundle: bundle.clone(),
            query: query.text.clone(),
            args,
        })
    }

    /// Returns the position among `rel`'s indexes of the one whose fields are `fields` over `scope`.
    fn reindexed(
        &mut self,
        rel: RelId,
        fields: &[ast::Expr],
        scope: &CtorScope,
        span: Span,
    ) -> R<usize> {
        let sel = match scope {
            CtorScope::One(c) => CtorSel::One(c.clone()),
            CtorScope::Many(cs) => CtorSel::Many(cs.clone()),
        };
        let set = self.relation_sel(rel, &sel).map_err(|e| diag(span, e))?;
        let schema = self.schema;
        let r = schema.relation(rel);
        let normal = |s: Option<CtorSet>| s.and_then(|s| schema.normalize_ctors(r.elem_def, s));
        let want = normal(Some(set.clone()));
        let ctx = std::mem::replace(&mut self.ctx, Context::Schema);
        let found = self.declared_position(rel, Some(set), fields, |ix| {
            ix.fields.len() == fields.len() && normal(ix.scope.clone()) == want
        });
        self.ctx = ctx;
        found?.ok_or_else(|| {
            diag(
                span,
                format!(
                    "`{}` declares no index or unique with these fields in this constructor set",
                    r.name
                ),
            )
        })
    }

    /// Checks `fields` at `scope`; returns the position of the first index of `rel` that `candidate`
    /// admits and whose fields check to the same terms as `fields`.
    fn declared_position(
        &mut self,
        rel: RelId,
        scope: Option<CtorSet>,
        fields: &[ast::Expr],
        candidate: impl Fn(&crate::schema::IndexDef) -> bool,
    ) -> R<Option<usize>> {
        self.index_terms(rel, scope, fields)?;
        for (i, ix) in self.schema.relation(rel).indexes.iter().enumerate() {
            if candidate(ix) {
                let ours = self.index_terms(rel, ix.scope.clone(), fields)?;
                if self.declared_index(rel, i, &ours) {
                    return Ok(Some(i));
                }
            }
        }
        Ok(None)
    }

    fn index_terms(
        &mut self,
        rel: RelId,
        scope: Option<CtorSet>,
        fields: &[ast::Expr],
    ) -> R<Vec<IndexField>> {
        fields
            .iter()
            .map(|e| self.index_field(rel, scope.clone(), e))
            .collect()
    }

    /// Returns whether `fields`, checked in this text, are the fields of `rel`'s `i`-th index.
    fn declared_index(&self, rel: RelId, i: usize, fields: &[IndexField]) -> bool {
        let program = self
            .program
            .expect("ad hoc text is checked against the program of its schema");
        fields.iter().enumerate().all(|(k, f)| {
            program.index_fields.get(&(rel, i, k)).is_some_and(|d| {
                d.path == f.path && alpha_equal(&self.terms, f.value, &program.terms, d.value)
            })
        })
    }

    fn command(&mut self, c: &ast::Command, span: Span) -> R<Command> {
        let key = |c: &mut Self, e: &ast::Expr, module: &str, name: &str| -> R<Tm> {
            let ty = c.engine_key(module, name);
            c.check(e, &ty).map(|o| o.tm)
        };
        Ok(match c {
            ast::Command::Explain { analyze, target } => Command::Explain {
                analyze: *analyze,
                target: self.explained(target)?,
            },
            ast::Command::PinPlan(e) => Command::PinPlan(key(self, e, "catalog", "plans")?),
            ast::Command::UnpinPlan(e) => Command::UnpinPlan(key(self, e, "catalog", "plans")?),
            ast::Command::CancelCall(e) => Command::CancelCall(key(self, e, "sys", "calls")?),
            ast::Command::TerminateSession(e) => {
                Command::TerminateSession(key(self, e, "sys", "sessions")?)
            }
            ast::Command::SetSetting(name, value) => self.set_setting(name, value)?,
            ast::Command::Vacuum(r) => Command::Vacuum(self.stored_relation_named(r, "vacuum")?),
            ast::Command::Verify(r) => Command::Verify(match r {
                Some(r) => Some(self.relation_named(r)?),
                None => None,
            }),
            ast::Command::Reindex {
                relation,
                index,
                online,
            } => {
                let rel = self.stored_relation_named(relation, "reindex")?;
                let index = match index {
                    Some((fields, scope)) => Some(self.reindexed(rel, fields, scope, span)?),
                    None => None,
                };
                Command::Reindex {
                    relation: rel,
                    index,
                    online: online.unwrap_or(true),
                }
            }
            ast::Command::Export {
                source,
                path,
                sums_text,
            } => {
                let o = self.synth(source)?;
                if self.elem_ty(&o.ty).is_none() {
                    return Err(diag(
                        source.span,
                        format!("`export` takes a set, found {}", self.show(&o.ty)),
                    ));
                }
                Command::Export {
                    source: o.tm,
                    path: path.clone(),
                    sums_text: *sums_text,
                }
            }
            ast::Command::ArchiveAudit { through, path } => Command::ArchiveAudit {
                through: self.check(through, &Ty::Base(Base::I64))?.tm,
                path: path.clone(),
            },
            ast::Command::Dump { path, encrypt } => Command::Dump {
                path: path.clone(),
                encrypt: match encrypt {
                    Some(k) => Some(self.synth(k)?.tm),
                    None => None,
                },
            },
            ast::Command::Restore { from, to, point } => Command::Restore {
                from: from.clone(),
                to: to.clone(),
                point: match point {
                    Some(p) => Some(self.restore_point(p)?),
                    None => None,
                },
            },
            ast::Command::Compact => Command::Compact,
            ast::Command::FinalizeUpgrade => Command::FinalizeUpgrade,
            ast::Command::RetireBundle(b) => Command::RetireBundle(b.clone()),
            ast::Command::SupersedeBundle(b, by) => Command::SupersedeBundle(b.clone(), by.clone()),
            ast::Command::RegisterBundle(f) => Command::RegisterBundle(f.clone()),
            ast::Command::ApplyMigrations(fs) => Command::ApplyMigrations(fs.clone()),
            ast::Command::PrintSchema => Command::PrintSchema,
            ast::Command::RotateKek(uri) => Command::RotateKek(uri.clone()),
            ast::Command::RotateDek => Command::RotateDek,
            ast::Command::DestroyDek(n) => Command::DestroyDek(*n),
            ast::Command::Backup { path, incremental } => Command::Backup {
                path: path.clone(),
                incremental: *incremental,
            },
        })
    }

    fn restore_point(&mut self, p: &ast::RestorePoint) -> R<RestorePoint> {
        Ok(match p {
            ast::RestorePoint::BeforeVersion(v) => RestorePoint::BeforeVersion(*v),
            ast::RestorePoint::AtInstant(e) => {
                RestorePoint::AtInstant(self.check(e, &Ty::Base(Base::Instant))?.tm)
            }
            ast::RestorePoint::AtPosition(e) => {
                let position = Ty::Data {
                    def: self.schema.prelude_type("Position"),
                    args: Vec::new(),
                    ctors: None,
                };
                RestorePoint::AtPosition(self.check(e, &position)?.tm)
            }
        })
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
        let origin = self.row_origin(&o);
        self.with_row_origin(
            Out {
                tm,
                ty: to.clone(),
                rows: None,
            },
            origin,
        )
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
                let ft = self.terms.transport_field(yt, f.clone(), fty.clone());
                given.push((f.clone(), ft));
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
