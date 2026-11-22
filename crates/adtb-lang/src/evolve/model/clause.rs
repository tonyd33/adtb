use super::field::Applied;
use super::lift::Lifting;
use super::op::{Fail, Res, Resolving, broken, refuse, refuse_at, unmodelled};
use super::run::{Built, Late, Run};
use super::*;
use crate::ast::{self, Expr, RelationClause};
use crate::canon::canonical_term;
use crate::evolve::{Clause, Resolved};
use crate::ids::CatRelId;
use crate::lex::{Tok, lex};
use crate::span::{FileId, Span};
use crate::types::RelId;

/// Returns the tokens of `text`, joined by spaces.
fn joined(text: &str) -> String {
    lex(text)
        .0
        .iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| &text[t.span.range()])
        .collect::<Vec<_>>()
        .join(" ")
}

/// Returns the text of a constructor selection as a refusal names it: ` in C` or ` in (C | D)`,
/// empty without one.
fn scope_shown(sel: Option<&ast::CtorSel>) -> String {
    let names: Vec<&str> = match sel {
        None => return String::new(),
        Some(ast::CtorSel::One(c)) => vec![c.text.as_str()],
        Some(ast::CtorSel::Many(cs)) => cs.iter().map(|c| c.text.as_str()).collect(),
    };
    match names.as_slice() {
        [one] => format!(" in {one}"),
        many => format!(" in ({})", many.join(" | ")),
    }
}

/// Returns the span from the first token after the first `=` at or after `from` of `text` to
/// `end`.
fn after_eq(text: &str, from: usize, end: usize) -> Span {
    let toks = lex(&text[from..end]).0;
    let at = toks
        .iter()
        .position(|t| t.tok == Tok::Eq)
        .and_then(|i| toks.get(i + 1))
        .map_or(end, |t| from + t.span.start as usize);
    Span::new(at, end)
}

impl Model {
    /// Lift declaration `item` again from `built`, every name in it resolving as `built` resolves
    /// it. Returns the file of the declaration.
    pub(super) fn relift(&mut self, built: &Built, item: Item) -> Res<FileId> {
        let i = self.decl_at(item).expect("a declared item");
        let file = FileId(i as u32);
        let Some(d) = built.decl(file) else {
            return broken("a declaration the model printed does not parse");
        };
        let names = built.names(file);
        let ahead = built.ahead_ranges(file);
        let module = self.decls[i].module.clone();
        Lifting::new(
            built.schema(),
            file,
            built.text(file),
            &names,
            &ahead,
            &module,
        )
        .decl(self, d)
        .map_err(Fail::Broken)?;
        self.decls.pop();
        Ok(file)
    }

    /// Give each object the declarations now hold without an id, such as an index a reference
    /// field implies once no clause covers it, its id, when they build.
    pub(super) fn settle_ids(&mut self, run: &Run) {
        let ahead = run.ahead(self);
        self.build_adopting(&[], &ahead).ok();
    }

    /// Returns the relation variable `r` is in `schema`.
    fn rel_in(schema: &crate::schema::Schema, r: u64) -> Res<RelId> {
        match schema.ids().relation_of(CatRelId::from_raw(r)) {
            Some(rel) => Ok(rel),
            None => broken("a relation variable the built schema does not hold"),
        }
    }

    /// Append `clause` to relation variable `r` and build. Returns the build and the file of the
    /// relation's declaration, its clauses lifted from the build.
    fn appended(&mut self, run: &Run, r: u64, clause: ClauseM) -> Res<(Built, FileId)> {
        self.relations
            .get_mut(&r)
            .expect("a resolved relation variable")
            .clauses
            .push(clause);
        let Ok(built) = self.build_adopting(&[], &run.ahead(self)) else {
            return unmodelled("a clause added to declarations that do not build");
        };
        let file = self.relift(&built, Item::Relation(r))?;
        Ok((built, file))
    }

    /// Returns the clause of the relation declaration of `file` in `built` at `k`.
    fn clause_written(built: &Built, file: FileId, k: usize) -> Res<RelationClause> {
        match built.decl(file) {
            Some(ast::Decl::Relation(rd)) => match rd.clauses.get(k) {
                Some(c) => Ok(c.clone()),
                None => broken("a relation the model printed lacks a clause"),
            },
            _ => broken("a relation the model printed does not parse"),
        }
    }

    /// Returns the canonical form of `span` of `file` in `built`, empty with a note that it
    /// names what only the migration's end resolves.
    pub(super) fn canonical_at(built: &Built, run: &Run, file: FileId, span: Span) -> String {
        canonical_term(built.text(file), span, &built.names(file))
            .unwrap_or_else(|_| run.late(Late::Unresolved))
    }

    /// Add a unique or index over `fields` to the relation variable `on` names.
    pub(super) fn add_index(
        &mut self,
        run: &Run,
        kind: IndexKind,
        fields: &[Expr],
        on: &ast::RelationRef,
    ) -> Res<Applied> {
        let resolving = Resolving::new(self, run.module());
        let r = if let Some(sel) = &on.sel {
            let built = self.schema_now(run)?;
            let r = resolving.relation(&on.relation)?;
            let rel = Self::rel_in(built.schema(), r)?;
            if let Err(e) = built.schema().relation_sel(rel, sel) {
                return refuse_at(on.span, e);
            }
            if kind == IndexKind::Unique {
                self.members_declare(built.schema(), r, sel, fields)?;
            }
            r
        } else {
            resolving.relation(&on.relation)?
        };
        let (Some(first), Some(last)) = (fields.first(), fields.last()) else {
            return broken("an index of no field");
        };
        let names: Vec<&ast::Ident> = match &on.sel {
            None => Vec::new(),
            Some(ast::CtorSel::One(c)) => vec![c],
            Some(ast::CtorSel::Many(cs)) => cs.iter().collect(),
        };
        let scope = on.sel.as_ref().map(|_| Scope {
            parts: names.iter().map(|c| Term::text(&c.text)).collect(),
            listed: names.len() > 1,
        });
        let clause = ClauseM::Index {
            id: None,
            kind,
            elems: vec![Term::text(&run.text()[first.span.to(last.span).range()])],
            scope,
        };
        let (built, file) = self.appended(run, r, clause)?;
        let k = self.relations[&r].clauses.len() - 1;
        let (fs, _) = match Self::clause_written(&built, file, k)? {
            RelationClause::Unique(fs, s, _) => (fs, s),
            RelationClause::Index { fields, scope, .. } => (fields, scope),
            _ => return broken("an added index the model printed as another clause"),
        };
        let span = fs[0].span.to(fs[fs.len() - 1].span);
        let text = Self::canonical_at(&built, run, file, span);
        let ClauseM::Index {
            id: Some(index),
            scope,
            ..
        } = &self.relations[&r].clauses[k]
        else {
            return broken("an added index the build gave no id");
        };
        let clauses = scope
            .iter()
            .flat_map(|s| s.parts.iter())
            .filter_map(|p| match p.objects().next()? {
                (Space::Set, s) => Some(Clause::Scope { set: s }),
                (_, c) => Some(Clause::Member { ctor: c }),
            })
            .collect();
        let resolved = match kind {
            IndexKind::Unique => Resolved::AddUnique {
                index: *index,
                relation: r,
                fields: text,
            },
            _ => Resolved::AddIndex {
                index: *index,
                relation: r,
                interval: kind == IndexKind::Interval,
                fields: text,
            },
        };
        Ok((resolved, clauses, Ok(Vec::new())))
    }

    /// Refuse a unique over a field named alone that a member of its scope `sel` of relation
    /// variable `r` does not declare, or declares at another type than the first member does in
    /// `schema`, at the span of the name of the member in `sel`, or of the named set holding it.
    fn members_declare(
        &self,
        schema: &crate::schema::Schema,
        r: u64,
        sel: &ast::CtorSel,
        fields: &[Expr],
    ) -> Res<()> {
        let Some(t) = self.relation_type(r) else {
            return Ok(());
        };
        let Some(def) = self.def_in(schema, t) else {
            return broken("a relation variable's type the built schema does not hold");
        };
        let td = schema.ty(def);
        let tname = &self.types[&t].name;
        let named = match sel {
            ast::CtorSel::One(c) => std::slice::from_ref(c),
            ast::CtorSel::Many(cs) => cs.as_slice(),
        };
        let mut members: Vec<(u64, Span)> = Vec::new();
        for n in named {
            let ctors = match self.ctor_named(t, &n.text) {
                Some(c) => vec![c],
                None => self
                    .sets_of(r)
                    .into_iter()
                    .find(|s| self.sets[s].name == n.text)
                    .map(|s| self.scope_ctors_of_set(s))
                    .unwrap_or_default(),
            };
            for c in ctors {
                if !members.iter().any(|(x, _)| *x == c) {
                    members.push((c, n.span));
                }
            }
        }
        for f in fields {
            let ast::ExprKind::Name(q) = &f.kind else {
                continue;
            };
            if q.module.is_some() {
                continue;
            }
            let u = &q.name.text;
            let mut first: Option<(u64, &crate::types::Ty)> = None;
            for &(c, at) in &members {
                let cname = &self.ctors[&c].name;
                let held = self
                    .ctor_index(schema, c)
                    .and_then(|k| td.field_named(k, u));
                let (Some(x), Some(held)) = (
                    self.field_of(t, Some(c), u)
                        .or_else(|| self.field_of(t, None, u)),
                    held,
                ) else {
                    return refuse_at(
                        at,
                        format!("member `{cname}` of `{tname}` has no field `{u}`"),
                    );
                };
                match first {
                    None => first = Some((x, &held.ty)),
                    Some((y, b)) if !b.alike(&held.ty) => {
                        return refuse_at(
                            at,
                            format!(
                                "member `{cname}` declares `{u}` as `{}`, not `{}`",
                                self.fields[&x].ty.print(self),
                                self.fields[&y].ty.print(self)
                            ),
                        );
                    }
                    Some(_) => {}
                }
            }
        }
        Ok(())
    }

    /// Returns the constructors named set `s` holds.
    fn scope_ctors_of_set(&self, s: u64) -> Vec<u64> {
        self.sets[&s]
            .members
            .iter()
            .filter_map(|m| m.objects().next().map(|o| o.1))
            .collect()
    }

    /// Drop the one unique or index over `fields` of the relation variable `on` names whose
    /// scope is the one `on` writes.
    pub(super) fn drop_index(
        &mut self,
        run: &Run,
        kind: IndexKind,
        fields: &[Expr],
        on: &ast::RelationRef,
    ) -> Res<Applied> {
        let built = self.schema_now(run)?;
        let r = Resolving::new(self, run.module()).relation(&on.relation)?;
        let schema = built.schema();
        let rel = Self::rel_in(schema, r)?;
        let want_scope = schema
            .written_scope(rel, on.sel.as_ref())
            .or_else(|e| refuse_at(on.span, e))?;
        let want: Vec<String> = fields
            .iter()
            .map(|e| run.text()[e.span.range()].to_string())
            .collect();
        let tokens: Vec<String> = want.iter().map(|w| joined(w)).collect();
        let defs = &schema.relation(rel).indexes;
        let mut found = Vec::new();
        let mut ix = 0;
        for (k, c) in self.relations[&r].clauses.iter().enumerate() {
            let ClauseM::Index { kind: held, .. } = c else {
                continue;
            };
            let d = &defs[ix];
            ix += 1;
            let texts: Vec<&str> = (0..d.fields.len()).map(|j| d.text(j)).collect();
            if *held == kind
                && d.written() == want_scope.as_deref()
                && texts.len() == tokens.len()
                && texts.iter().zip(&tokens).all(|(a, b)| a == b)
            {
                found.push(k);
            }
        }
        let word = match kind {
            IndexKind::Unique => "unique",
            IndexKind::Plain => "index",
            IndexKind::Interval => "interval index",
        };
        let rname = self.relations[&r].name.clone();
        let shown = scope_shown(on.sel.as_ref());
        let k = match found.as_slice() {
            [] => {
                return refuse(format!(
                    "`{rname}` has no {word} ({}){shown}",
                    want.join(", ")
                ));
            }
            [k] => *k,
            many => {
                return refuse(format!(
                    "`{rname}` has {} clauses {word} ({}){shown}, so the drop names none of them alone",
                    many.len(),
                    want.join(", ")
                ));
            }
        };
        let ClauseM::Index {
            id: Some(index), ..
        } = self
            .relations
            .get_mut(&r)
            .expect("the relation variable")
            .clauses
            .remove(k)
        else {
            return broken("a dropped index held no id");
        };
        self.settle_ids(run);
        let resolved = match kind {
            IndexKind::Unique => Resolved::DropUnique { index },
            _ => Resolved::DropIndex { index },
        };
        Ok((resolved, Vec::new(), Ok(Vec::new())))
    }

    /// Returns a `Member` clause for each constructor whose field default clause `ids` fills.
    fn default_members(&self, ids: &[(u64, u64)]) -> Vec<Clause> {
        ids.iter()
            .filter_map(|(f, _)| {
                let owner = self.fields.get(f)?.owner;
                self.ctors
                    .contains_key(&owner)
                    .then_some(Clause::Member { ctor: owner })
            })
            .collect()
    }

    pub(super) fn add_default(
        &mut self,
        run: &Run,
        relation: &ast::QName,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
        value: &ast::DefaultValue,
    ) -> Res<Applied> {
        let r = Resolving::new(self, run.module()).relation(relation)?;
        let value = match value {
            ast::DefaultValue::Generate(g) => Term::text(&format!("generate {}", g.text)),
            ast::DefaultValue::Expr(e) => Term::text(&run.text()[e.span.range()]),
        };
        let clause = ClauseM::Default {
            ctor: ctor.map(|c| Term::text(&c.text)),
            field: Term::text(&field.text),
            value,
            ids: Vec::new(),
        };
        let (built, file) = self.appended(run, r, clause)?;
        let k = self.relations[&r].clauses.len() - 1;
        let RelationClause::Default {
            field: written,
            value,
            ..
        } = Self::clause_written(&built, file, k)?
        else {
            return broken("an added default the model printed as another clause");
        };
        let end = match &value {
            ast::DefaultValue::Generate(g) => g.span.end,
            ast::DefaultValue::Expr(e) => e.span.end,
        };
        let span = after_eq(built.text(file), written.span.end as usize, end as usize);
        let ClauseM::Default { ids, .. } = &self.relations[&r].clauses[k] else {
            return broken("an added default lifted as another clause");
        };
        let value =
            canonical_term(built.text(file), span, &built.names(file)).unwrap_or_else(|_| {
                run.late(match ids.first() {
                    Some(&(_, default)) => Late::Default {
                        relation: r,
                        default,
                    },
                    None => Late::Unresolved,
                })
            });
        let Some(&(field, default)) = ids.first() else {
            run.late(Late::Unresolved);
            return Ok((
                Resolved::AddDefault {
                    default: 0,
                    relation: r,
                    field: 0,
                    value,
                },
                Vec::new(),
                Ok(Vec::new()),
            ));
        };
        let clauses = self.default_members(ids);
        Ok((
            Resolved::AddDefault {
                default,
                relation: r,
                field,
                value,
            },
            clauses,
            Ok(Vec::new()),
        ))
    }

    pub(super) fn drop_default(
        &mut self,
        run: &Run,
        relation: &ast::QName,
        ctor: Option<&ast::Ident>,
        field: &ast::Ident,
    ) -> Res<Applied> {
        let r = Resolving::new(self, run.module()).relation(relation)?;
        let found = self.relations[&r].clauses.iter().position(|c| {
            matches!(c, ClauseM::Default { ctor: c, field: f, .. }
                if f.print(self) == field.text
                    && c.as_ref().map(|c| c.print(self)).as_deref() == ctor.map(|c| c.text.as_str()))
        });
        let Some(k) = found else {
            return refuse_at(
                field.span,
                format!(
                    "`{}` has no default for `{}`",
                    relation.name.text, field.text
                ),
            );
        };
        let ClauseM::Default { ids, .. } = &self.relations[&r].clauses[k] else {
            return broken("a found default is another clause");
        };
        let Some(&(_, default)) = ids.first() else {
            return unmodelled("a dropped default that fills no field");
        };
        let clauses = self.default_members(ids);
        self.relations
            .get_mut(&r)
            .expect("the relation variable")
            .clauses
            .remove(k);
        self.settle_ids(run);
        Ok((Resolved::DropDefault { default }, clauses, Ok(Vec::new())))
    }

    /// Returns the position of computed field `field` among relation variable `r`'s clauses.
    fn computed_at(&self, r: u64, relation: &ast::QName, field: &ast::Ident) -> Res<usize> {
        let found = self.relations[&r]
            .clauses
            .iter()
            .position(|c| matches!(c, ClauseM::Computed { name, .. } if *name == field.text));
        match found {
            Some(k) => Ok(k),
            None => refuse_at(
                field.span,
                format!(
                    "`{}` has no computed field `{}`",
                    relation.name.text, field.text
                ),
            ),
        }
    }

    /// Returns the id of computed clause `k` of relation `r` and the canonical form of it from its
    /// name through its expression, as `file` of `built` writes it.
    fn computed_written(
        &self,
        built: &Built,
        run: &Run,
        file: FileId,
        r: u64,
        k: usize,
    ) -> Res<(u64, String)> {
        let RelationClause::Computed(n, _, e) = Self::clause_written(built, file, k)? else {
            return broken("a computed field the model printed as another clause");
        };
        let ClauseM::Computed { id: Some(id), .. } = &self.relations[&r].clauses[k] else {
            return broken("a computed field the build gave no id");
        };
        let canonical = canonical_term(built.text(file), n.span.to(e.span), &built.names(file))
            .unwrap_or_else(|_| {
                run.late(Late::Computed {
                    relation: r,
                    computed: *id,
                })
            });
        Ok((*id, canonical))
    }

    /// Returns the canonical form of the value of default `default` of relation variable `r` as
    /// `built` writes it, `None` when it does not resolve there.
    pub(super) fn default_canonical(&self, built: &Built, r: u64, default: u64) -> Option<String> {
        let i = self.decl_at(Item::Relation(r))?;
        let file = FileId(i as u32);
        let k = self.relations[&r].clauses.iter().position(
            |c| matches!(c, ClauseM::Default { ids, .. } if ids.iter().any(|&(_, d)| d == default)),
        )?;
        let Ok(RelationClause::Default {
            field: written,
            value,
            ..
        }) = Self::clause_written(built, file, k)
        else {
            return None;
        };
        let end = match &value {
            ast::DefaultValue::Generate(g) => g.span.end,
            ast::DefaultValue::Expr(e) => e.span.end,
        };
        let span = after_eq(built.text(file), written.span.end as usize, end as usize);
        canonical_term(built.text(file), span, &built.names(file)).ok()
    }

    /// Returns the canonical form of computed field `computed` of relation variable `r` as
    /// `built` writes it, `None` when it does not resolve there.
    pub(super) fn computed_canonical(
        &self,
        built: &Built,
        r: u64,
        computed: u64,
    ) -> Option<String> {
        let i = self.decl_at(Item::Relation(r))?;
        let file = FileId(i as u32);
        let k = self.relations[&r]
            .clauses
            .iter()
            .position(|c| matches!(c, ClauseM::Computed { id: Some(x), .. } if *x == computed))?;
        let Ok(RelationClause::Computed(n, _, e)) = Self::clause_written(built, file, k) else {
            return None;
        };
        canonical_term(built.text(file), n.span.to(e.span), &built.names(file)).ok()
    }

    pub(super) fn add_computed(
        &mut self,
        run: &Run,
        relation: &ast::QName,
        field: &ast::Ident,
        ty: &ast::Type,
        expr: &Expr,
    ) -> Res<Applied> {
        let r = Resolving::new(self, run.module()).relation(relation)?;
        let clause = ClauseM::Computed {
            id: None,
            name: field.text.clone(),
            ty: Term::text(&run.text()[ty.span.range()]),
            expr: Term::text(&run.text()[expr.span.range()]),
        };
        let (built, file) = self.appended(run, r, clause)?;
        let k = self.relations[&r].clauses.len() - 1;
        let (computed, decl) = self.computed_written(&built, run, file, r, k)?;
        Ok((
            Resolved::AddComputed {
                computed,
                relation: r,
                name: field.text.clone(),
                decl,
            },
            Vec::new(),
            Ok(Vec::new()),
        ))
    }

    pub(super) fn redefine_computed(
        &mut self,
        run: &Run,
        relation: &ast::QName,
        field: &ast::Ident,
        ty: &ast::Type,
        expr: &Expr,
    ) -> Res<Applied> {
        let r = Resolving::new(self, run.module()).relation(relation)?;
        let k = self.computed_at(r, relation, field)?;
        if let ClauseM::Computed {
            ty: held_ty,
            expr: held_expr,
            ..
        } = &mut self
            .relations
            .get_mut(&r)
            .expect("the relation variable")
            .clauses[k]
        {
            *held_ty = Term::text(&run.text()[ty.span.range()]);
            *held_expr = Term::text(&run.text()[expr.span.range()]);
        }
        let Ok(built) = self.build_adopting(&[], &run.ahead(self)) else {
            return unmodelled("a clause added to declarations that do not build");
        };
        let file = self.relift(&built, Item::Relation(r))?;
        let (computed, decl) = self.computed_written(&built, run, file, r, k)?;
        Ok((
            Resolved::RedefineComputed { computed, decl },
            Vec::new(),
            Ok(Vec::new()),
        ))
    }

    pub(super) fn drop_computed(
        &mut self,
        run: &Run,
        relation: &ast::QName,
        field: &ast::Ident,
    ) -> Res<Applied> {
        let r = Resolving::new(self, run.module()).relation(relation)?;
        let k = self.computed_at(r, relation, field)?;
        let ClauseM::Computed {
            id: Some(computed), ..
        } = self
            .relations
            .get_mut(&r)
            .expect("the relation variable")
            .clauses
            .remove(k)
        else {
            return broken("a dropped computed field held no id");
        };
        self.settle_ids(run);
        Ok((
            Resolved::DropComputed { computed },
            Vec::new(),
            Ok(Vec::new()),
        ))
    }
}
