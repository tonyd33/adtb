use super::*;

/// An operator of migration `logged` being inverted against `before`, the source before it.
#[derive(Clone, Copy)]
struct Inverting<'a> {
    before: &'a Source,
    logged: &'a Logged,
    op: &'a Operator,
    fills: &'a [(ast::FieldPath, Fill)],
}

impl Inverting<'_> {
    /// Returns the module `n` names, else the migration's.
    fn module_of(&self, n: &QName) -> String {
        n.module
            .as_ref()
            .map_or(self.logged.module.clone(), |m| m.text.clone())
    }

    fn text(&self, s: Span) -> String {
        self.logged.text[s.range()].to_string()
    }

    fn decl_text(&self, kind: &str, m: &str, name: &str) -> Option<String> {
        self.before
            .find(kind, m, name)
            .map(|i| self.before.decls[i].text.clone())
    }
}

fn decl(kind: &'static str, m: String, name: String, text: Option<String>) -> Inverse {
    Inverse::Decl {
        module: m,
        kind,
        name,
        text,
        states: Vec::new(),
    }
}

fn ops(s: String) -> R<Vec<Inverse>> {
    Ok(vec![Inverse::Ops(s.into())])
}

impl Cx<'_> {
    /// The inverse of `op` of migration `logged`, computed against `before`, the schema before it.
    pub(super) fn inverse(
        &self,
        before: &Source,
        logged: &Logged,
        op: &Operator,
        fills: &[(ast::FieldPath, Fill)],
    ) -> R<Vec<Inverse>> {
        let at = Inverting {
            before,
            logged,
            op,
            fills,
        };
        match &op.kind {
            OperatorKind::AddModule(..)
            | OperatorKind::Install { .. }
            | OperatorKind::DropModule(..)
            | OperatorKind::Uninstall(..) => self.module_inverse(&at),
            OperatorKind::AddType(..)
            | OperatorKind::DropType(..)
            | OperatorKind::RenameType(..) => self.type_inverse(&at),
            OperatorKind::AddRelation(..)
            | OperatorKind::DropRelation(..)
            | OperatorKind::RenameRelation(..)
            | OperatorKind::AddSet(..)
            | OperatorKind::DropSet(..)
            | OperatorKind::RenameSet(..)
            | OperatorKind::AddUnique(..)
            | OperatorKind::AddIndex { .. }
            | OperatorKind::DropUnique(..)
            | OperatorKind::DropIndex { .. }
            | OperatorKind::Rekey { .. }
            | OperatorKind::AddDefault { .. }
            | OperatorKind::DropDefault { .. }
            | OperatorKind::AddComputed { .. }
            | OperatorKind::RedefineComputed { .. }
            | OperatorKind::DropComputed(..) => self.relation_inverse(&at),
            OperatorKind::AddConstructor { .. }
            | OperatorKind::RemoveConstructor(..)
            | OperatorKind::RenameConstructor(..)
            | OperatorKind::Enable(..)
            | OperatorKind::Disable(..)
            | OperatorKind::Deprecate(..)
            | OperatorKind::NestConstructor { .. }
            | OperatorKind::UnnestConstructor(..)
            | OperatorKind::SplitConstructor { .. }
            | OperatorKind::MergeConstructor { .. } => self.ctor_inverse(&at),
            OperatorKind::AddField { .. }
            | OperatorKind::AlterField { .. }
            | OperatorKind::DropField(..)
            | OperatorKind::RenameField(..)
            | OperatorKind::MoveField { .. } => self.field_inverse(&at),
            OperatorKind::DefineFn(..)
            | OperatorKind::RedefineFn(..)
            | OperatorKind::DropFn(..)
            | OperatorKind::DefineAggregate(..)
            | OperatorKind::DropAggregate(..)
            | OperatorKind::AddConstraint(..)
            | OperatorKind::DropConstraint(..)
            | OperatorKind::AddCheck(..)
            | OperatorKind::DropCheck(..)
            | OperatorKind::Store(..)
            | OperatorKind::Unstore(..) => self.body_inverse(&at),
            OperatorKind::Grant(_) | OperatorKind::Revoke(_) | OperatorKind::AdoptTzdb { .. } => {
                Ok(Vec::new())
            }
            OperatorKind::Script(_) => self.err("a `script` has no inverse"),
            OperatorKind::Revert { migration, .. } => {
                let n: u64 = migration.parse().unwrap_or(u64::MAX);
                match self.src.log.iter().find(|l| l.number == n) {
                    Some(l) => Ok(vec![Inverse::Replay(Box::new(l.clone()))]),
                    None => self.err(format!("migration {migration} is not in the log")),
                }
            }
        }
    }

    fn module_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting { before, op, .. } = *at;
        match &op.kind {
            OperatorKind::AddModule(m) => Ok(vec![Inverse::Module {
                name: m.text.clone(),
                present: false,
                installed: false,
                decls: Vec::new(),
            }]),
            OperatorKind::Install { name, .. } => Ok(vec![Inverse::Module {
                name: name.text.clone(),
                present: false,
                installed: true,
                decls: Vec::new(),
            }]),
            OperatorKind::DropModule(m) | OperatorKind::Uninstall(m) => Ok(vec![Inverse::Module {
                name: m.text.clone(),
                present: true,
                installed: before.installed.contains(&m.text),
                decls: before
                    .decls
                    .iter()
                    .filter(|d| d.module == m.text)
                    .map(|d| d.text.clone())
                    .collect(),
            }]),
            _ => unreachable!("module_inverse is given a module operator"),
        }
    }

    fn type_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting { logged, op, .. } = *at;
        match &op.kind {
            OperatorKind::AddType(t) => Ok(vec![decl(
                "type",
                logged.module.clone(),
                t.name.text.clone(),
                None,
            )]),
            OperatorKind::DropType(t) => Ok(vec![decl(
                "type",
                at.module_of(t),
                t.name.text.clone(),
                at.decl_text("type", &at.module_of(t), &t.name.text),
            )]),
            OperatorKind::RenameType(t, to) => ops(format!(
                "rename type {}::{} to {}",
                at.module_of(t),
                to.text,
                t.name.text
            )),
            _ => unreachable!("type_inverse is given a type operator"),
        }
    }

    fn relation_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting {
            before, logged, op, ..
        } = *at;
        match &op.kind {
            OperatorKind::AddRelation(r) => Ok(vec![decl(
                "relation",
                logged.module.clone(),
                r.name.name.text.clone(),
                None,
            )]),
            OperatorKind::DropRelation(r) => {
                let m = at.module_of(r);
                let mut out = Vec::new();
                let states: Vec<(StateKey, State)> = before
                    .states
                    .iter()
                    .filter(|((sm, sr, _), _)| *sm == m && *sr == r.name.text)
                    .map(|(k, v)| (k.clone(), *v))
                    .collect();
                out.push(Inverse::Decl {
                    module: m.clone(),
                    kind: "relation",
                    name: r.name.text.clone(),
                    text: at.decl_text("relation", &m, &r.name.text),
                    states,
                });
                for (i, d) in before.decls.iter().enumerate() {
                    if d.module == m
                        && let Some(Decl::Set(s)) = before.parsed(i)
                        && s.relation.text == r.name.text
                    {
                        out.push(decl(
                            "set",
                            m.clone(),
                            format!("{}.{}", s.relation.text, s.name.text),
                            Some(d.text.clone()),
                        ));
                    }
                }
                Ok(out)
            }
            OperatorKind::RenameRelation(r, to) => ops(format!(
                "rename relation {}::{} to {}",
                at.module_of(r),
                to.text,
                r.name.text
            )),
            OperatorKind::AddSet(s) => Ok(vec![decl(
                "set",
                logged.module.clone(),
                format!("{}.{}", s.relation.text, s.name.text),
                None,
            )]),
            OperatorKind::DropSet(s, r) => {
                let name = format!("{}.{}", r.name.text, s.text);
                Ok(vec![decl(
                    "set",
                    at.module_of(r),
                    name.clone(),
                    at.decl_text("set", &at.module_of(r), &name),
                )])
            }
            OperatorKind::RenameSet(s, r, to) => ops(format!(
                "rename set {} of {}::{} to {}",
                to.text,
                at.module_of(r),
                r.name.text,
                s.text
            )),
            OperatorKind::AddUnique(..) | OperatorKind::AddIndex { .. } => ops(format!(
                "drop {}",
                at.text(op.span).trim_start_matches("add").trim_start()
            )),
            OperatorKind::DropUnique(..) | OperatorKind::DropIndex { .. } => ops(format!(
                "add {}",
                at.text(op.span).trim_start_matches("drop").trim_start()
            )),
            OperatorKind::Rekey {
                relation, carry, ..
            } => self.rekey_inverse(at, relation, carry),
            OperatorKind::AddDefault {
                relation,
                ctor,
                field,
                ..
            }
            | OperatorKind::DropDefault {
                relation,
                ctor,
                field,
            } => self.default_inverse(at, relation, ctor.as_ref(), field),
            OperatorKind::AddComputed {
                relation, field, ..
            } => ops(format!(
                "drop computed {}::{}.{}",
                at.module_of(relation),
                relation.name.text,
                field.text
            )),
            OperatorKind::RedefineComputed {
                relation, field, ..
            }
            | OperatorKind::DropComputed(relation, field) => {
                self.computed_inverse(at, relation, field)
            }
            _ => unreachable!("relation_inverse is given a relation operator"),
        }
    }

    fn ctor_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting {
            before, logged, op, ..
        } = *at;
        match &op.kind {
            OperatorKind::AddConstructor { path, .. } => {
                let record = before
                    .find("type", &at.module_of(&path.ty), &path.ty.name.text)
                    .filter(|&i| {
                        matches!(
                            before.parsed(i),
                            Some(Decl::Type(ast::TypeDecl {
                                body: ast::TypeBody::Record(_),
                                ..
                            }))
                        )
                    })
                    .map(|i| before.decls[i].text.clone());
                Ok(vec![Inverse::Unadd {
                    path: path.clone(),
                    record,
                }])
            }
            OperatorKind::RemoveConstructor(path) => self.removed_ctor_inverse(at, path),
            OperatorKind::RenameConstructor(path, to) => ops(format!(
                "rename constructor {}::{}.{} to {}",
                at.module_of(&path.ty),
                path.ty.name.text,
                to.text,
                path.ctor.text
            )),
            OperatorKind::Enable(t) | OperatorKind::Disable(t) | OperatorKind::Deprecate(t) => {
                let cx = Cx {
                    src: &mut before.clone(),
                    module: logged.module.clone(),
                    text: &logged.text,
                    span: op.span,
                    noted: None,
                    copying: false,
                    steps: None,
                };
                let (relation, key, _) = cx.stage_key(t)?;
                let prior = if relation {
                    before.states.get(&key)
                } else {
                    before.value_states.get(&key)
                };
                Ok(vec![Inverse::States(vec![(relation, key, prior.copied())])])
            }
            OperatorKind::NestConstructor {
                ty,
                sel,
                into,
                clauses,
            } => {
                let m = at.module_of(ty);
                let mut s = format!(
                    "unnest constructor {m}::{}.{}",
                    ty.name.text, into.name.text
                );
                let names: Vec<String> = sel_idents(sel).iter().map(|x| x.text.clone()).collect();
                s.push_str(&self.restored_arms(before, clauses, &names));
                ops(s)
            }
            OperatorKind::UnnestConstructor(path, clauses) => {
                self.unnest_inverse(at, path, clauses)
            }
            OperatorKind::SplitConstructor {
                path,
                into,
                arms,
                clauses,
            } => {
                let arms_of: Vec<ast::ArmClause> = clauses
                    .iter()
                    .filter_map(|c| match c {
                        ast::SplitClause::Arm(a) => Some((**a).clone()),
                        _ => None,
                    })
                    .collect();
                self.split_inverse(before, logged, path, into, arms, &arms_of)
            }
            OperatorKind::MergeConstructor {
                ty,
                sel,
                into,
                arms,
                clauses,
            } => self.merge_inverse(before, ty, sel, into, arms.as_deref(), clauses),
            _ => unreachable!("ctor_inverse is given a constructor operator"),
        }
    }

    fn field_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting {
            before, op, fills, ..
        } = *at;
        match &op.kind {
            OperatorKind::AddField { path, .. } => ops(format!(
                "drop field {}",
                qualified_path(path, &at.module_of(&path.ty))
            )),
            OperatorKind::AlterField { path, .. } => {
                let m = at.module_of(&path.ty);
                let Some((_, action)) = field_texts(before, &m, path) else {
                    return self.err("the log does not hold the altered field");
                };
                ops(format!(
                    "alter field {} on delete {}",
                    qualified_path(path, &m),
                    action.as_deref().unwrap_or("restrict")
                ))
            }
            OperatorKind::DropField(path) => {
                let m = at.module_of(&path.ty);
                let Some(fill) = self.fill_for(fills, &path_text(path)) else {
                    return self.err(format!(
                        "the inverse of `drop field {}` needs a fill: `fill {} = e`",
                        path_text(path),
                        path_text(path)
                    ));
                };
                let ty = field_type_text(before, &m, path).ok_or_else(|| {
                    vec![Diag::new(
                        self.span,
                        "the log does not hold the dropped field",
                    )]
                })?;
                Ok(vec![
                    Inverse::Ops(Written::concat([
                        format!("add field {}: {ty} fill ", qualified_path(path, &m)).into(),
                        fill,
                    ])),
                    self.reorder(before, path, path.ctor.as_ref().map(|c| c.text.clone())),
                ])
            }
            OperatorKind::RenameField(path, to) => {
                let renamed = ast::FieldPath {
                    field: to.clone(),
                    ..path.clone()
                };
                ops(format!(
                    "rename field {} to {}",
                    qualified_path(&renamed, &at.module_of(&path.ty)),
                    path.field.text
                ))
            }
            OperatorKind::MoveField { path, to, sel, .. } => {
                let m = at.module_of(&path.ty);
                match (&path.ctor, sel) {
                    (Some(c), None) => Ok(vec![
                        Inverse::Ops(
                            format!(
                                "move field {m}::{}.{} to {m}::{}.{}",
                                to.name.text, path.field.text, to.name.text, c.text
                            )
                            .into(),
                        ),
                        self.reorder(before, path, Some(c.text.clone())),
                    ]),
                    (None, Some(sel)) => {
                        let names: Vec<String> =
                            sel_idents(sel).iter().map(|x| x.text.clone()).collect();
                        let first = &names[0];
                        let key = format!("{}.{first}.{}", path.ty.name.text, path.field.text);
                        let schema = before
                            .schema()
                            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
                        let def = self.type_id(&schema, &path.ty)?;
                        let mut s = Written::from(format!(
                            "move field {m}::{}.{first}.{} to {m}::{}",
                            path.ty.name.text, path.field.text, path.ty.name.text
                        ));
                        if schema.ty(def).ctors().len() > names.len() {
                            let Some(fill) = self.fill_for(fills, &key) else {
                                return self.err(format!(
                                    "the inverse of moving `{}` down needs a fill: `fill {key} = e`",
                                    path_text(path)
                                ));
                            };
                            s.push(" fill ");
                            s.append(fill);
                        }
                        Ok(vec![Inverse::MoveUp {
                            text: s,
                            from: names,
                        }])
                    }
                    _ => Ok(Vec::new()),
                }
            }
            _ => unreachable!("field_inverse is given a field operator"),
        }
    }

    fn body_inverse(&self, at: &Inverting) -> R<Vec<Inverse>> {
        let Inverting { logged, op, .. } = *at;
        match &op.kind {
            OperatorKind::DefineFn(f) => Ok(vec![decl(
                "fn",
                logged.module.clone(),
                f.name.text.clone(),
                None,
            )]),
            OperatorKind::RedefineFn(f) => Ok(vec![decl(
                "fn",
                logged.module.clone(),
                f.name.text.clone(),
                at.decl_text("fn", &logged.module, &f.name.text),
            )]),
            OperatorKind::DropFn(f) => Ok(vec![decl(
                "fn",
                at.module_of(f),
                f.name.text.clone(),
                at.decl_text("fn", &at.module_of(f), &f.name.text),
            )]),
            OperatorKind::DefineAggregate(a) => Ok(vec![decl(
                "aggregate",
                logged.module.clone(),
                a.name.text.clone(),
                None,
            )]),
            OperatorKind::DropAggregate(a) => Ok(vec![decl(
                "aggregate",
                at.module_of(a),
                a.name.text.clone(),
                at.decl_text("aggregate", &at.module_of(a), &a.name.text),
            )]),
            OperatorKind::AddConstraint(c) => Ok(vec![decl(
                "constraint",
                logged.module.clone(),
                c.name.text.clone(),
                None,
            )]),
            OperatorKind::DropConstraint(c) => Ok(vec![decl(
                "constraint",
                at.module_of(c),
                c.name.text.clone(),
                at.decl_text("constraint", &at.module_of(c), &c.name.text),
            )]),
            OperatorKind::AddCheck(c) => Ok(vec![decl(
                "check",
                logged.module.clone(),
                c.name.text.clone(),
                None,
            )]),
            OperatorKind::DropCheck(c) => Ok(vec![decl(
                "check",
                at.module_of(c),
                c.name.text.clone(),
                at.decl_text("check", &at.module_of(c), &c.name.text),
            )]),
            OperatorKind::Store(s) => Ok(vec![decl(
                "store",
                logged.module.clone(),
                s.view.name.text.clone(),
                None,
            )]),
            OperatorKind::Unstore(v) => Ok(vec![decl(
                "store",
                at.module_of(v),
                v.name.text.clone(),
                at.decl_text("store", &at.module_of(v), &v.name.text),
            )]),
            _ => unreachable!(
                "body_inverse is given a function, aggregate, constraint, check or store operator"
            ),
        }
    }

    fn rekey_inverse(
        &self,
        at: &Inverting,
        relation: &QName,
        carry: &[ast::FieldPath],
    ) -> R<Vec<Inverse>> {
        let Inverting { before, op, .. } = *at;
        let m = at.module_of(relation);
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let key = schema
            .module_named(&m)
            .and_then(
                |mid| match schema.module(mid).values.get(&relation.name.text) {
                    Some(ValueDef::Relation(r)) => Some(schema.relation(*r).key.clone()),
                    _ => None,
                },
            )
            .unwrap_or_default();
        let mut s = format!(
            "rekey relation {m}::{} key ({})",
            relation.name.text,
            key.join(", ")
        );
        if !carry.is_empty() {
            let paths: Vec<String> = carry
                .iter()
                .map(|p| qualified_path(p, &at.module_of(&p.ty)))
                .collect();
            s.push_str(&format!(" carry {}", paths.join(", ")));
        }
        let clause = before
            .find("relation", &m, &relation.name.text)
            .and_then(|i| {
                let Some(Decl::Relation(rd)) = before.parsed(i) else {
                    return None;
                };
                let ranges = clause_ranges(&before.decls[i].text, header(&m).len(), &rd)?;
                let k = rd
                    .clauses
                    .iter()
                    .position(|c| matches!(c, RelationClause::Key(..)))?;
                Some(before.decls[i].text[ranges[k].clone()].to_string())
            });
        let mut out = vec![Inverse::Ops(s.into())];
        if let Some(text) = clause {
            out.push(Inverse::KeyClause {
                relation: QName {
                    module: Some(ast::Ident {
                        text: m,
                        span: op.span,
                    }),
                    name: relation.name.clone(),
                },
                text,
            });
        }
        Ok(out)
    }

    fn default_inverse(
        &self,
        at: &Inverting,
        relation: &QName,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
    ) -> R<Vec<Inverse>> {
        let Inverting { before, op, .. } = *at;
        let path = match ctor {
            Some(c) => format!(
                "{}::{}.{}.{}",
                at.module_of(relation),
                relation.name.text,
                c.text,
                field.text
            ),
            None => format!(
                "{}::{}.{}",
                at.module_of(relation),
                relation.name.text,
                field.text
            ),
        };
        if matches!(op.kind, OperatorKind::AddDefault { .. }) {
            return ops(format!("drop default {path}"));
        }
        let m = at.module_of(relation);
        let i = before
            .find("relation", &m, &relation.name.text)
            .ok_or_else(|| vec![Diag::new(self.span, "the log does not hold the default")])?;
        let Some(Decl::Relation(rd)) = before.parsed(i) else {
            return self.err("the log does not hold the default");
        };
        let h = header(&m).len();
        let value = rd.clauses.iter().find_map(|c| match c {
            RelationClause::Default {
                ctor: dc,
                field: f,
                value,
            } if f.text == field.text
                && dc.as_ref().map(|x| &x.text) == ctor.as_ref().map(|x| &x.text) =>
            {
                Some(match value {
                    ast::DefaultValue::Generate(g) => format!("generate {}", g.text),
                    ast::DefaultValue::Expr(e) => before.decls[i].text
                        [e.span.start as usize - h..e.span.end as usize - h]
                        .to_string(),
                })
            }
            _ => None,
        });
        let Some(value) = value else {
            return self.err("the log does not hold the default");
        };
        ops(format!("add default {path} = {value}"))
    }

    fn computed_inverse(
        &self,
        at: &Inverting,
        relation: &QName,
        field: &ast::Ident,
    ) -> R<Vec<Inverse>> {
        let Inverting { before, op, .. } = *at;
        let m = at.module_of(relation);
        let i = before
            .find("relation", &m, &relation.name.text)
            .ok_or_else(|| {
                vec![Diag::new(
                    self.span,
                    "the log does not hold the computed field",
                )]
            })?;
        let Some(Decl::Relation(rd)) = before.parsed(i) else {
            return self.err("the log does not hold the computed field");
        };
        let h = header(&m).len();
        let found = rd.clauses.iter().find_map(|c| match c {
            RelationClause::Computed(n, t, e) if n.text == field.text => Some((
                before.decls[i].text[t.span.start as usize - h..t.span.end as usize - h]
                    .to_string(),
                before.decls[i].text[e.span.start as usize - h..e.span.end as usize - h]
                    .to_string(),
            )),
            _ => None,
        });
        let Some((t, e)) = found else {
            return self.err("the log does not hold the computed field");
        };
        let verb = if matches!(op.kind, OperatorKind::DropComputed(..)) {
            "add"
        } else {
            "redefine"
        };
        ops(format!(
            "{verb} computed {m}::{}.{}: {t} = {e}",
            relation.name.text, field.text
        ))
    }

    fn removed_ctor_inverse(&self, at: &Inverting, path: &ast::CtorPath) -> R<Vec<Inverse>> {
        let Inverting { before, .. } = *at;
        let m = at.module_of(&path.ty);
        let Some(i) = before.find("type", &m, &path.ty.name.text) else {
            return self.err("the log does not hold the removed constructor");
        };
        let Some(Decl::Type(t)) = before.parsed(i) else {
            return self.err("the log does not hold the removed constructor");
        };
        let h = header(&m).len();
        let ctors = ctor_extents(&before.decls[i].text, h, &t);
        let Some(k) = ctors.iter().position(|c| c.0 == path.ctor.text) else {
            return self.err("the log does not hold the removed constructor");
        };
        let ctext = before.decls[i].text[ctors[k].1.start..ctors[k].2].to_string();
        let after = (k > 0).then(|| ctors[k - 1].0.clone());
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let refs = check::schema_refs(&schema);
        let mut arms = Vec::new();
        for j in 0..before.decls.len() {
            let Some(d @ Decl::Constraint(_)) = before.parsed(j) else {
                continue;
            };
            let jh = header(&before.decls[j].module).len();
            let mut texts = Vec::new();
            for (f, span, r) in &refs {
                if f.0 as usize != j || !matches!(r, Ref::Match(_)) {
                    continue;
                }
                for a in match_arms(&d, *span).unwrap_or_default() {
                    if let Some(ast::Pattern {
                        kind: ast::PatternKind::Ctor { path: p, ctor, .. },
                        ..
                    }) = a.patterns.first()
                        && ctor.as_ref().unwrap_or(&p.name).text == path.ctor.text
                    {
                        let start = a.patterns[0].span.start as usize - jh;
                        let end = a.body.span.end as usize - jh;
                        texts.push(before.decls[j].text[start..end].to_string());
                    }
                }
            }
            if !texts.is_empty() {
                let (_, name) = key_of(&d);
                arms.push((name, texts));
            }
        }
        let mut out = vec![Inverse::Readd {
            ty: path.ty.clone(),
            text: ctext,
            after,
            arms,
        }];
        let mut states = Vec::new();
        for (k2, v) in before
            .states
            .iter()
            .filter(|(k2, _)| k2.2 == path.ctor.text)
        {
            states.push((true, k2.clone(), Some(*v)));
        }
        for (k2, v) in before
            .value_states
            .iter()
            .filter(|(k2, _)| k2.1 == path.ty.name.text && k2.2 == path.ctor.text)
        {
            states.push((false, k2.clone(), Some(*v)));
        }
        out.push(Inverse::States(states));
        Ok(out)
    }

    fn unnest_inverse(
        &self,
        at: &Inverting,
        path: &ast::CtorPath,
        clauses: &[ast::ArmClause],
    ) -> R<Vec<Inverse>> {
        let Inverting { before, .. } = *at;
        let m = at.module_of(&path.ty);
        let Some(i) = before.find("type", &m, &path.ty.name.text) else {
            return self.err("the log does not hold the unnested constructor");
        };
        let Some(Decl::Type(t)) = before.parsed(i) else {
            return self.err("the log does not hold the unnested constructor");
        };
        let h = header(&m).len();
        let ctors = ctor_extents(&before.decls[i].text, h, &t);
        let Some(k) = ctors.iter().position(|c| c.0 == path.ctor.text) else {
            return self.err("the log does not hold the unnested constructor");
        };
        let ctext = before.decls[i].text[ctors[k].1.start..ctors[k].2].to_string();
        let schema = before
            .schema()
            .map_err(|_| vec![Diag::new(self.span, "the log does not replay")])?;
        let def = schema
            .module_named(&m)
            .and_then(|mid| schema.module(mid).types.get(&path.ty.name.text).copied());
        let products: Vec<String> = def
            .and_then(|d| {
                let td = schema.ty(d);
                let c = td.ctor_index(&path.ctor.text)?;
                match &td.ctors()[c as usize].fields.first()?.ty {
                    Ty::Data { def: sd, .. } => Some(
                        schema
                            .ty(*sd)
                            .ctors()
                            .iter()
                            .map(|x| x.name.clone())
                            .collect(),
                    ),
                    _ => None,
                }
            })
            .unwrap_or_default();
        let mut s = format!(
            "nest constructor {m}::{}.({}) into {ctext}",
            path.ty.name.text,
            products.join(" | ")
        );
        s.push_str(&self.restored_arms(before, clauses, std::slice::from_ref(&path.ctor.text)));
        ops(s)
    }
}
