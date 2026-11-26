use super::field::Applied;
use super::lift::Lifting;
use super::op::{Fail, Res, broken, refuse, refuse_at, refuse_diag, refuse_diags};
use super::run::{Late, Run, unbuilt};
use super::*;
use crate::ast;
use crate::canon::{Names, canonical_term};
use crate::check::{self, Ref};
use crate::evolve::grant::{StatedRefs, Statement, log_privileges};
use crate::evolve::{Clause, LogGrant, Privilege, Resolved, SourceDecl};
use crate::parse::{Options, parse_schema};
use crate::schema::Schema;
use crate::span::{FileId, Span};

/// Returns the canonical form of `span` of the migration, `refs` naming what it writes, empty
/// with a note when a name of it is not among them.
pub(super) fn canonical(
    run: &mut Run,
    schema: &Schema,
    refs: &[(Span, Ref)],
    span: Span,
) -> String {
    run.note(schema, refs);
    let mut names = Names::default();
    for (s, r) in refs {
        names.note(schema, *s, r);
    }
    canonical_term(run.text(), span, &names).unwrap_or_else(|_| run.late(Late::Unresolved))
}

/// Returns the name and canonical target of the first of `privileges`, and a `Privilege` clause
/// for each later one.
fn privileges(
    run: &mut Run,
    schema: &Schema,
    refs: &[(Span, Ref)],
    privileges: &[ast::Privilege],
) -> Res<(String, String, Vec<Clause>)> {
    let mut terms = Vec::new();
    for p in privileges {
        let Some((name, span)) = privilege_term(p) else {
            return broken("a log privilege that resolves to no term");
        };
        terms.push((name.name().to_string(), canonical(run, schema, refs, span)));
    }
    let mut terms = terms.into_iter();
    let Some((privilege, target)) = terms.next() else {
        return broken("a grant of no privilege");
    };
    let clauses = terms
        .map(|(privilege, target)| Clause::Privilege { privilege, target })
        .collect();
    Ok((privilege, target, clauses))
}

impl Model {
    /// Resolve `privileges` of a grant or revoke against the declarations now.
    fn stated(
        &self,
        run: &mut Run,
        privileges: &[ast::Privilege],
        statement: Statement,
    ) -> Res<(Schema, StatedRefs)> {
        let schema = self.schema_now(run)?.into_schema();
        let stated = log_privileges(&schema, run.module(), privileges, statement, run.span())
            .or_else(|ds| refuse_diags(&ds))?;
        Ok((schema, stated))
    }

    pub(super) fn grant(&mut self, run: &mut Run, g: &ast::Grant) -> Res<Applied> {
        let (schema, (stated, refs)) = self.stated(run, &g.privileges, Statement::Grant)?;
        for st in stated {
            let (privilege, covered) = st.into_covered();
            self.unstated
                .retain(|u| !u.stated_by(&g.to.text, &privilege, &covered));
            match self
                .grants
                .iter_mut()
                .find(|x| x.role == g.to.text && x.privilege == privilege)
            {
                Some(held) if held.dormant => {
                    held.grant_option = g.grant_option;
                    held.dormant = false;
                }
                Some(held) => held.grant_option |= g.grant_option,
                None => self.grants.push(LogGrant {
                    role: g.to.text.clone(),
                    privilege,
                    grant_option: g.grant_option,
                    row: None,
                    dormant: false,
                }),
            }
        }
        let (privilege, target, clauses) = privileges(run, &schema, &refs, &g.privileges)?;
        Ok((
            Resolved::Grant {
                privilege,
                target,
                grantee: g.to.text.clone(),
                grant_option: g.grant_option,
            },
            clauses,
            Ok(Vec::new()),
        ))
    }

    pub(super) fn revoke(&mut self, run: &mut Run, r: &ast::Revoke) -> Res<Applied> {
        let (schema, (gone, refs)) = self.stated(run, &r.privileges, Statement::Revoke)?;
        let role = &r.from.text;
        for st in &gone {
            if !self
                .grants
                .iter()
                .any(|x| x.role == *role && st.revokes(&x.privilege))
            {
                return refuse(format!(
                    "`{}` matches no grant `{role}` holds",
                    &run.text()[r.span.range()]
                ));
            }
        }
        self.grants
            .retain(|x| x.role != *role || !gone.iter().any(|p| p.revokes(&x.privilege)));
        let (privilege, target, clauses) = privileges(run, &schema, &refs, &r.privileges)?;
        Ok((
            Resolved::Revoke {
                privilege,
                target,
                grantee: role.clone(),
            },
            clauses,
            Ok(Vec::new()),
        ))
    }

    pub(super) fn script(&mut self, run: &mut Run, w: &ast::Write) -> Res<Applied> {
        let compiled = self.checked(run)?;
        let schema = compiled.schema();
        let mut refs = Vec::new();
        check::check_script(&compiled, w, Some(&mut refs)).or_else(|ds| refuse_diags(&ds))?;
        let write = canonical(run, schema, &refs, w.span);
        Ok((Resolved::Script { write }, Vec::new(), Ok(Vec::new())))
    }

    pub(super) fn install(
        &mut self,
        run: &mut Run,
        name: &ast::Ident,
        version: &str,
        from: &str,
    ) -> Res<Applied> {
        let m = name.text.clone();
        self.reserved(name)?;
        let Some(package) = self.packages.get(from).cloned() else {
            return refuse(format!("the package `{from}` is not available"));
        };
        let text = format!("module {m}\n{package}");
        let parsed = parse_schema(&text, Options::default()).ok().or_else(|ds| {
            refuse(format!(
                "the package `{from}` does not parse: {}",
                ds[0].message
            ))
        })?;
        self.modules.insert(m.clone());
        self.installed.insert(m.clone());
        let more: Vec<SourceDecl> = parsed
            .decls
            .iter()
            .map(|d| SourceDecl {
                module: m.clone(),
                text: text[d.span().range()].to_string(),
            })
            .collect();
        let first = self.decls.len();
        let own = |f: FileId| f.0 as usize >= first && f != FileId::EMPTY_MODULE;
        let built = match self.build_adopting(&more, &run.ahead(self)) {
            Ok(b) => b,
            Err(ds) => {
                return match ds.iter().filter(|d| own(d.file)).min_by_key(|d| d.file.0) {
                    Some(d) => refuse_diag(&d.diag),
                    None => unbuilt(&ds),
                };
            }
        };
        if let Err(ds) = check::check_schema(built.schema())
            && let Some(d) = ds.iter().filter(|d| own(d.file)).min_by_key(|d| d.file.0)
        {
            return refuse_diag(&d.diag);
        }
        for k in 0..more.len() {
            let file = FileId((first + k) as u32);
            let Some(d) = built.decl(file) else {
                return broken("an installed declaration that does not parse alone");
            };
            let names = built.names(file);
            let ahead = built.ahead_ranges(file);
            Lifting::new(built.schema(), file, built.text(file), &names, &ahead, &m)
                .decl(self, d)
                .map_err(Fail::Broken)?;
        }
        Ok((
            Resolved::Install {
                module: m,
                version: version.to_string(),
                package,
            },
            Vec::new(),
            Ok(Vec::new()),
        ))
    }

    pub(super) fn uninstall(&mut self, ident: &ast::Ident) -> Res<Applied> {
        let (at, m) = (ident.span, ident.text.as_str());
        if !self.installed.contains(m) {
            return refuse_at(at, format!("`{m}` is not an installed extension"));
        }
        for i in 0..self.decls.len() {
            if self.decls[i].module == m {
                continue;
            }
            let names_it = self
                .item_objects(self.decls[i].item)
                .into_iter()
                .any(|o| self.home_of(o).as_deref() == Some(m));
            if names_it {
                return refuse_at(
                    at,
                    format!(
                        "`uninstall {m}` is refused while {} names it",
                        self.shown_decl(i)
                    ),
                );
            }
        }
        let gone: Vec<Item> = self
            .decls
            .iter()
            .filter(|d| d.module == m)
            .map(|d| d.item)
            .collect();
        for item in gone {
            self.remove(item);
        }
        self.modules.remove(m);
        self.installed.remove(m);
        Ok((
            Resolved::Uninstall {
                module: m.to_string(),
            },
            Vec::new(),
            Ok(Vec::new()),
        ))
    }

    /// Adopt release `release` of the tzdb from file `from`. Its resolved form holds no data when
    /// the source lacks the file; [`Model::unavailable`] names it.
    pub(super) fn adopt_tzdb(&mut self, release: &str, from: &str) -> Res<Applied> {
        let data = self.packages.get(from).cloned().unwrap_or_default();
        Ok((
            Resolved::AdoptTzdb {
                release: release.to_string(),
                data,
            },
            Vec::new(),
            Ok(Vec::new()),
        ))
    }

    /// Returns the module of the declaration that declares the object of `obj`, a field's that
    /// of its type.
    fn home_of(&self, obj: Obj) -> Option<String> {
        let obj = match obj {
            (Space::Field, f) => (Space::Shared, self.field_type(f)?),
            o => o,
        };
        self.module_of(obj).map(str::to_string)
    }

    /// Returns every object a name in declaration `item` resolves to.
    pub(super) fn item_objects(&self, item: Item) -> Vec<Obj> {
        let mut terms: Vec<&Term> = Vec::new();
        match item {
            Item::Type(t) => {
                if let TypeBody::New { base, domain, .. } = &self.types[&t].body {
                    terms.push(base);
                    terms.extend(domain);
                }
                for f in self.fields_of(t) {
                    terms.push(&self.fields[&f].ty);
                    terms.extend(&self.fields[&f].action);
                }
            }
            Item::Relation(r) => {
                let rel = &self.relations[&r];
                terms.push(&rel.ty);
                for c in &rel.clauses {
                    match c {
                        ClauseM::Key { fields, .. } => terms.extend(fields),
                        ClauseM::Index { elems, scope, .. } => {
                            terms.extend(elems);
                            terms.extend(scope.iter().flat_map(|s| &s.parts));
                        }
                        ClauseM::Computed { ty, expr, .. } => {
                            terms.push(ty);
                            terms.push(expr);
                        }
                        ClauseM::Volatile => {}
                    }
                }
                terms.extend(rel.defaults.values());
            }
            Item::Set(s) => terms.extend(&self.sets[&s].members),
            Item::Fn(f) | Item::Aggregate(f) => terms.push(&self.functions[&f].rest),
            Item::Constraint(c) | Item::Check(c) => terms.push(&self.denials[&c].rest),
            Item::Store(v) => terms.push(&self.stores[&v].rest),
            Item::Held(..) => {}
        }
        terms.into_iter().flat_map(Term::objects).collect()
    }
}
/// Returns the name of a privilege of a log grant or revoke and the span of its target; `None`
/// for a privilege a log operator cannot hold.
fn privilege_term(p: &ast::Privilege) -> Option<(Privilege, Span)> {
    let listed = |r: &ast::RelationRef, fs: &Option<ast::GrantFieldList>| {
        fs.as_ref().map_or(r.span, |fs| r.span.to(fs.span))
    };
    Some(match p {
        ast::Privilege::Read(r, fs) => (Privilege::Read, listed(r, fs)),
        ast::Privilege::Update(r, fs) => (Privilege::Update, listed(r, fs)),
        ast::Privilege::Insert(r) => (Privilege::Insert, r.span),
        ast::Privilege::Delete(r) => (Privilege::Delete, r.span),
        ast::Privilege::Retag(a, b, except) => (
            Privilege::Retag,
            except
                .as_ref()
                .map_or(a.span.to(b.span), |e| a.span.to(e.span)),
        ),
        ast::Privilege::CallFn(f) => (Privilege::Call, f.span()),
        ast::Privilege::Call(..) | ast::Privilege::CallAt(_) | ast::Privilege::Capability(..) => {
            return None;
        }
    })
}
