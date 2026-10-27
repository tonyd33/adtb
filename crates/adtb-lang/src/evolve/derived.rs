use super::*;

impl Resolver<'_> {
    /// Returns the resolved form of an operator on defaults, computed fields, stored views,
    /// grants, extensions, zone rules or data; `None` when an object it names is in neither
    /// schema or it is no such operator.
    pub(super) fn derived(&self, kind: &OperatorKind) -> Option<Resolved> {
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
            } => Resolved::DropDefault {
                default: self
                    .default_in(before, relation, ctor.as_ref(), &field.text)?
                    .1,
            },
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
            OperatorKind::Grant(g) => Resolved::Grant {
                privileges: term(privileges(text, g.span, g.to.span)?),
                grantee: g.to.text.clone(),
                grant_option: g.grant_option,
            },
            OperatorKind::Revoke(r) => Resolved::Revoke {
                privileges: term(privileges(text, r.span, r.from.span)?),
                grantee: r.from.text.clone(),
            },
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

/// Returns the span of the privilege list of the grant or revoke at `span`, whose role is at
/// `role`: from the token after its verb to the token before the `to` or `from` before the role.
fn privileges(text: &str, span: Span, role: Span) -> Option<Span> {
    let base = span.start as usize;
    let toks = tokens(&text[span.range()]);
    let through = toks
        .iter()
        .take_while(|(_, r)| base + r.end <= role.start as usize)
        .count();
    let first = &toks.get(1)?.1;
    let last = &toks.get(through.checked_sub(2)?)?.1;
    Some(Span::new(base + first.start, base + last.end))
}
