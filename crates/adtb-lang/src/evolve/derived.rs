use super::*;

impl Resolver<'_> {
    /// Returns the resolved form of an operator on defaults, computed fields, stored views,
    /// grants, extensions, zone rules or data; `None` when an object it names is in neither
    /// schema or it is no such operator.
    pub(super) fn derived(
        &self,
        kind: &OperatorKind,
        clauses: &mut Vec<Clause>,
    ) -> Option<Resolved> {
        let text = &self.logged.text;
        let term = |span: Span| canonical_term(text, span, self.names);
        let (before, after) = (self.before, self.schema);
        let package = |from: &str| self.logged.before.packages.get(from).cloned();
        Some(match kind {
            OperatorKind::AddDefault {
                relation,
                ctor,
                field,
                value,
            } => {
                let (rel, default, field_id) =
                    self.default_in(after, relation, ctor.as_ref(), &field.text)?;
                clauses.extend(self.default_members(
                    after,
                    relation,
                    ctor.as_ref(),
                    &field.text,
                )?);
                let end = match value {
                    ast::DefaultValue::Generate(g) => g.span.end,
                    ast::DefaultValue::Expr(e) => e.span.end,
                };
                Resolved::AddDefault {
                    default,
                    relation: after.ids.relations[&rel],
                    field: field_id,
                    value: term(after_eq(text, field.span.end, end)),
                }
            }
            OperatorKind::DropDefault {
                relation,
                ctor,
                field,
            } => {
                clauses.extend(self.default_members(
                    before,
                    relation,
                    ctor.as_ref(),
                    &field.text,
                )?);
                Resolved::DropDefault {
                    default: self
                        .default_in(before, relation, ctor.as_ref(), &field.text)?
                        .1,
                }
            }
            OperatorKind::Store(s) => {
                let f = self.view_fn(after, &s.view)?;
                Resolved::Store {
                    view: *after.ids.views.get(&f)?,
                    function: *after.ids.functions.get(&f)?,
                    decl: term(s.span),
                }
            }
            OperatorKind::Unstore(q) => Resolved::Unstore {
                view: *before.ids.views.get(&self.view_fn(before, q)?)?,
            },
            OperatorKind::AddComputed {
                relation,
                field,
                expr,
                ..
            } => {
                let (rel, computed) = self.computed_in(after, relation, &field.text)?;
                Resolved::AddComputed {
                    computed,
                    relation: after.ids.relations[&rel],
                    name: field.text.clone(),
                    decl: term(field.span.to(expr.span)),
                }
            }
            OperatorKind::RedefineComputed {
                relation,
                field,
                expr,
                ..
            } => Resolved::RedefineComputed {
                computed: self.computed_in(after, relation, &field.text)?.1,
                decl: term(field.span.to(expr.span)),
            },
            OperatorKind::DropComputed(relation, field) => Resolved::DropComputed {
                computed: self.computed_in(before, relation, &field.text)?.1,
            },
            OperatorKind::Grant(g) => {
                let (privilege, target) = self.privileges(&g.privileges, clauses)?;
                Resolved::Grant {
                    privilege,
                    target,
                    grantee: g.to.text.clone(),
                    grant_option: g.grant_option,
                }
            }
            OperatorKind::Revoke(r) => {
                let (privilege, target) = self.privileges(&r.privileges, clauses)?;
                Resolved::Revoke {
                    privilege,
                    target,
                    grantee: r.from.text.clone(),
                }
            }
            OperatorKind::Install {
                name,
                version,
                from,
            } => Resolved::Install {
                module: name.text.clone(),
                version: version.clone(),
                package: package(from)?,
            },
            OperatorKind::Uninstall(m) => Resolved::Uninstall {
                module: m.text.clone(),
            },
            OperatorKind::AdoptTzdb { release, from } => Resolved::AdoptTzdb {
                release: release.clone(),
                data: package(from)?,
            },
            OperatorKind::Script(w) => Resolved::Script {
                write: term(w.span),
            },
            _ => return None,
        })
    }

    /// Returns the name and target of the first of `privileges`; each later one becomes a clause.
    fn privileges(
        &self,
        privileges: &[ast::Privilege],
        clauses: &mut Vec<Clause>,
    ) -> Option<(String, String)> {
        let mut terms = privileges.iter().map(|p| {
            let (name, span) = privilege_term(p)?;
            let target = canonical_term(&self.logged.text, span, self.names);
            Some((name.to_owned(), target))
        });
        let first = terms.next()??;
        for t in terms {
            let (privilege, target) = t?;
            clauses.push(Clause::Privilege { privilege, target });
        }
        Some(first)
    }

    /// Returns a `Member` clause for each constructor whose field the default on `field` fills,
    /// in declared order, none for a header field.
    fn default_members(
        &self,
        schema: &Schema,
        relation: &QName,
        ctor: Option<&ast::Ident>,
        field: &str,
    ) -> Option<Vec<Clause>> {
        let rel = self.relation_in(schema, relation)?;
        let def = schema.relation(rel).elem_def;
        let c = match ctor {
            Some(c) => Some(schema.ty(def).ctor_index(&c.text)?),
            None => None,
        };
        let at = schema
            .relation(rel)
            .defaults
            .iter()
            .position(|d| d.ctor == c && d.field == field)?;
        schema
            .ids
            .defaults
            .range((rel, at, None)..=(rel, at, Some(u32::MAX)))
            .filter_map(|(&(_, _, filled), _)| filled)
            .map(|c| {
                Some(Clause::Member {
                    ctor: *schema.ids.ctors.get(&(def, c))?,
                })
            })
            .collect()
    }

    /// Returns the relation of the default `relation` declares for field `field` of `ctor`, the
    /// default's first id and the id of the field that default fills.
    fn default_in(
        &self,
        schema: &Schema,
        relation: &QName,
        ctor: Option<&ast::Ident>,
        field: &str,
    ) -> Option<(RelId, u64, u64)> {
        let rel = self.relation_in(schema, relation)?;
        let def = schema.relation(rel).elem_def;
        let c = match ctor {
            Some(c) => Some(schema.ty(def).ctor_index(&c.text)?),
            None => None,
        };
        let at = schema
            .relation(rel)
            .defaults
            .iter()
            .position(|d| d.ctor == c && d.field == field)?;
        let (&(_, _, filled), &id) = schema
            .ids
            .defaults
            .range((rel, at, None)..=(rel, at, Some(u32::MAX)))
            .next()?;
        let field = *schema.ids.fields.get(&(def, filled, field.to_string()))?;
        Some((rel, id, field))
    }

    fn computed_in(&self, schema: &Schema, relation: &QName, name: &str) -> Option<(RelId, u64)> {
        let rel = self.relation_in(schema, relation)?;
        let at = schema
            .relation(rel)
            .computed
            .iter()
            .position(|c| c.name == name)?;
        Some((rel, *schema.ids.computed.get(&(rel, at))?))
    }

    fn view_fn(&self, schema: &Schema, q: &QName) -> Option<FnId> {
        let m = schema.module_named(&self.module_of(q))?;
        match schema.module(m).values.get(&q.name.text)? {
            ValueDef::Function(f) => Some(*f),
            _ => None,
        }
    }
}

/// Returns the name of a privilege of a log grant or revoke and the span of its target; `None`
/// for a privilege a log operator cannot hold.
fn privilege_term(p: &ast::Privilege) -> Option<(&'static str, Span)> {
    let listed = |r: &ast::RelationRef, fs: &Option<ast::GrantFieldList>| {
        fs.as_ref().map_or(r.span, |fs| r.span.to(fs.span))
    };
    Some(match p {
        ast::Privilege::Read(r, fs) => ("read", listed(r, fs)),
        ast::Privilege::Update(r, fs) => ("update", listed(r, fs)),
        ast::Privilege::Insert(r) => ("insert", r.span),
        ast::Privilege::Delete(r) => ("delete", r.span),
        ast::Privilege::Retag(a, b, except) => (
            "retag",
            except
                .as_ref()
                .map_or(a.span.to(b.span), |e| a.span.to(e.span)),
        ),
        ast::Privilege::CallFn(f) => ("call", f.span()),
        ast::Privilege::Call(..) | ast::Privilege::CallAt(_) | ast::Privilege::Capability(_) => {
            return None;
        }
    })
}
