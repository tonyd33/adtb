use super::field::Applied;
use super::lift::Lifting;
use super::op::{Fail, Res, broken, refuse, refuse_at, refuse_diag};
use super::run::{Built, Decls, Late, Run};
use super::*;
use crate::ast::{self, Expr, RelationClause};
use crate::canon::canonical_term;
use crate::evolve::{Clause, Resolved};
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
pub(super) fn after_eq(text: &str, from: usize, end: usize) -> Span {
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
    pub(super) fn settle_ids(&mut self, run: &mut Run) {
        let ahead = run.ahead(self);
        self.build_adopting(&[], &ahead).ok();
    }

    /// Returns the relation variable `r` is in `schema`.
    fn rel_in(schema: &crate::schema::Schema, r: u64) -> Res<RelId> {
        match schema.ids().relation_of(cat_rel(r)) {
            Some(rel) => Ok(rel),
            None => broken("a relation variable the built schema does not hold"),
        }
    }

    /// Append `clause` to relation variable `r` and build as [`Model::rebuilt_relation`] does.
    fn appended(&mut self, run: &mut Run, r: u64, clause: ClauseM) -> Res<Option<(Built, FileId)>> {
        self.relations
            .get_mut(&r)
            .expect("a resolved relation variable")
            .clauses
            .push(clause);
        self.rebuilt_relation(run, r)
    }

    /// Build after an operator changed the clauses of relation variable `r`. Returns the build
    /// and the file of the relation's declaration, its clauses lifted from the build; `None` when
    /// the declarations do not build, the relation then lifted again at the next build.
    pub(super) fn rebuilt_relation(
        &mut self,
        run: &mut Run,
        r: u64,
    ) -> Res<Option<(Built, FileId)>> {
        let Ok(built) = self.build_adopting(&[], &run.ahead(self)) else {
            if !self.unlifted.contains(&Item::Relation(r)) {
                self.unlifted.push(Item::Relation(r));
            }
            return Ok(None);
        };
        let file = self.relift(&built, Item::Relation(r))?;
        Ok(Some((built, file)))
    }

    /// Returns the clause of the relation declaration of `file` in `built` at `k`.
    pub(super) fn clause_written(
        built: &impl Decls,
        file: FileId,
        k: usize,
    ) -> Res<RelationClause> {
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
    pub(super) fn canonical_at(built: &Built, run: &mut Run, file: FileId, span: Span) -> String {
        canonical_term(built.text(file), span, &built.names(file))
            .unwrap_or_else(|_| run.late(Late::Unresolved))
    }

    /// Add a unique or index over elements `elems` to the relation variable `on` names.
    pub(super) fn add_index(
        &mut self,
        run: &mut Run,
        kind: IndexKind,
        r: u64,
        elems: Term,
        on: &ast::RelationRef,
    ) -> Res<Applied> {
        if let Some(sel) = &on.sel {
            let built = self.schema_now(run)?;
            let rel = Self::rel_in(built.schema(), r)?;
            if let Err(e) = built.schema().relation_sel(rel, sel) {
                return refuse_at(on.span, e);
            }
        }
        let names: Vec<&ast::Ident> = match &on.sel {
            None => Vec::new(),
            Some(ast::CtorSel::One(c)) => vec![c],
            Some(ast::CtorSel::Many(cs)) => cs.iter().collect(),
        };
        let scope = on.sel.as_ref().map(|_| Scope {
            parts: names.iter().map(|c| Term::text(&c.text)).collect(),
            listed: names.len() > 1,
        });
        let index = self.fresh(Space::Index);
        let clause = ClauseM::Index {
            id: Some(index),
            was: Vec::new(),
            kind,
            elems: vec![elems],
            scope,
        };
        let built = self.appended(run, r, clause)?;
        let k = self.relations[&r].clauses.len() - 1;
        let (fields, clauses) = match built {
            Some((built, _)) => {
                let fields = self
                    .index_canonical(&built, r, k)
                    .unwrap_or_else(|| run.late(Late::Unresolved));
                (fields, self.index_scope(r, k))
            }
            None => {
                self.unbuilt_clauses.insert((Space::Index, index));
                let fields = run.late(Late::Index { relation: r, index });
                (fields, Vec::new())
            }
        };
        let resolved = match kind {
            IndexKind::Unique => Resolved::AddUnique {
                index,
                relation: r,
                fields,
            },
            _ => Resolved::AddIndex {
                index,
                relation: r,
                interval: kind == IndexKind::Interval,
                fields,
            },
        };
        Ok((resolved, clauses, Ok(Vec::new())))
    }

    /// Returns the canonical form of the fields of clause `k` of relation variable `r`, a unique
    /// or index, as `built` writes it, `None` when it does not resolve there.
    pub(super) fn index_canonical(&self, built: &Built, r: u64, k: usize) -> Option<String> {
        let file = FileId(self.decl_at(Item::Relation(r))? as u32);
        let (RelationClause::Unique(fs, _, _) | RelationClause::Index { fields: fs, .. }) =
            Self::clause_written(built, file, k).ok()?
        else {
            return None;
        };
        let span = fs.first()?.span.to(fs.last()?.span);
        canonical_term(built.text(file), span, &built.names(file)).ok()
    }

    /// Returns the constructors and named sets the scope of clause `k` of relation variable `r`
    /// names.
    pub(super) fn index_scope(&self, r: u64, k: usize) -> Vec<Clause> {
        let ClauseM::Index { scope, .. } = &self.relations[&r].clauses[k] else {
            return Vec::new();
        };
        scope
            .iter()
            .flat_map(|s| s.parts.iter())
            .filter_map(|p| match p.objects().next()? {
                (Space::Set, s) => Some(Clause::Scope { set: s }),
                (_, c) => Some(Clause::Member { ctor: c }),
            })
            .collect()
    }

    /// Drop the one unique or index over `fields` of the relation variable `on` names whose
    /// scope is the one `on` writes.
    pub(super) fn drop_index(
        &mut self,
        run: &mut Run,
        kind: IndexKind,
        r: u64,
        fields: &[Expr],
        on: &ast::RelationRef,
    ) -> Res<Applied> {
        let built = self.schema_now(run)?;
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
                found.push((k, ix - 1));
            }
        }
        let word = match kind {
            IndexKind::Unique => "unique",
            IndexKind::Plain => "index",
            IndexKind::Interval => "interval index",
        };
        let rname = self.relations[&r].name.clone();
        let shown = scope_shown(on.sel.as_ref());
        let (k, at) = match found.as_slice() {
            [] => {
                return refuse(format!(
                    "`{rname}` has no {word} ({}){shown}",
                    want.join(", ")
                ));
            }
            [k] => *k,
            _ => return broken("a relation variable holds a unique or an index twice"),
        };
        if run.is_copying() {
            let Some(i) = self.decl_at(Item::Relation(r)) else {
                return broken("a dropped clause's relation variable has no declaration");
            };
            let file = FileId(i as u32);
            run.read(&built, file, &defs[at].fields, fields);
        }
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

    /// Returns the position of computed field `field` among relation variable `r`'s clauses.
    fn computed_at(&self, r: u64, field: &ast::Ident) -> Res<usize> {
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
                    self.relations[&r].name, field.text
                ),
            ),
        }
    }

    /// Returns the id of computed clause `k` of relation `r` and the canonical form of it from its
    /// name through its expression, as `file` of `built` writes it.
    fn computed_written(
        &self,
        built: &Built,
        run: &mut Run,
        file: FileId,
        r: u64,
        k: usize,
    ) -> Res<(u64, String)> {
        let RelationClause::Computed(n, _, e) = Self::clause_written(built, file, k)? else {
            return broken("a computed field the model printed as another clause");
        };
        self.clause_typed(run, r, k, Some(built))?;
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
        run: &mut Run,
        r: u64,
        field: &ast::Ident,
        ty: Term,
        expr: Term,
    ) -> Res<Applied> {
        let computed = self.fresh(Space::Computed);
        let clause = ClauseM::Computed {
            id: Some(computed),
            name: field.text.clone(),
            ty,
            expr,
        };
        let Some((built, file)) = self.appended(run, r, clause)? else {
            let k = self.relations[&r].clauses.len() - 1;
            self.clause_typed(run, r, k, None)?;
            self.computed_unbuilt.insert((r, computed));
            let decl = run.late(Late::Computed {
                relation: r,
                computed,
            });
            return Ok((
                Resolved::AddComputed {
                    computed,
                    relation: r,
                    name: field.text.clone(),
                    decl,
                },
                Vec::new(),
                Ok(Vec::new()),
            ));
        };
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
        run: &mut Run,
        r: u64,
        field: &ast::Ident,
        ty: Term,
        expr: Term,
    ) -> Res<Applied> {
        let k = self.computed_at(r, field)?;
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
            *held_ty = ty;
            *held_expr = expr;
        }
        let Some((built, file)) = self.rebuilt_relation(run, r)? else {
            self.clause_typed(run, r, k, None)?;
            let ClauseM::Computed {
                id: Some(computed), ..
            } = self.relations[&r].clauses[k]
            else {
                return broken("a redefined computed field held no id");
            };
            self.computed_unbuilt.insert((r, computed));
            let decl = run.late(Late::Computed {
                relation: r,
                computed,
            });
            return Ok((
                Resolved::RedefineComputed { computed, decl },
                Vec::new(),
                Ok(Vec::new()),
            ));
        };
        let (computed, decl) = self.computed_written(&built, run, file, r, k)?;
        Ok((
            Resolved::RedefineComputed { computed, decl },
            Vec::new(),
            Ok(Vec::new()),
        ))
    }

    pub(super) fn drop_computed(
        &mut self,
        run: &mut Run,
        r: u64,
        field: &ast::Ident,
    ) -> Res<Applied> {
        let k = self.computed_at(r, field)?;
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

impl Model {
    /// Refuse the operator with the first failure of the declarations within the text it wrote
    /// in clause `k` of relation variable `r`, a default or a computed field: of checking `built`,
    /// the declarations as they build, else of building them.
    pub(super) fn clause_typed(
        &self,
        run: &mut Run,
        r: u64,
        k: usize,
        built: Option<&Built>,
    ) -> Res<()> {
        let Some(i) = self.decl_at(Item::Relation(r)) else {
            return broken("a relation variable with a clause and no declaration");
        };
        let file = FileId(i as u32);
        let (span, ds) = match built {
            Some(b) => {
                let ds = b.unbound(|f| f == file)?.into_iter();
                (written_span(b, file, k)?, ds.map(|d| d.diag).collect())
            }
            None => {
                let Err(ds) = self.built(&[], &run.ahead(self)) else {
                    return Ok(());
                };
                let ds = ds
                    .into_iter()
                    .filter(|d| d.file == file)
                    .map(|d| d.diag)
                    .collect::<Vec<_>>();
                (written_span(&self.parsed(), file, k)?, ds)
            }
        };
        match ds
            .iter()
            .find(|d| span.start <= d.span.start && d.span.end <= span.end)
        {
            Some(d) => refuse_diag(d),
            None => Ok(()),
        }
    }
}

/// Returns the span of the text an operator writes in clause `k` of the relation declaration of
/// `file`: a default's value, or a computed field from its name through its expression.
fn written_span(decls: &impl Decls, file: FileId, k: usize) -> Res<Span> {
    match Model::clause_written(decls, file, k)? {
        RelationClause::Default { field, value, .. } => {
            let end = match &value {
                ast::DefaultValue::Generate(g) => g.span.end,
                ast::DefaultValue::Expr(e) => e.span.end,
            };
            Ok(after_eq(
                decls.text(file),
                field.span.end as usize,
                end as usize,
            ))
        }
        RelationClause::Computed(n, _, e) => Ok(n.span.to(e.span)),
        _ => broken("a default or computed field the model printed as another clause"),
    }
}
