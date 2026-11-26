use super::field::Applied;
use super::indexes::{DropName, DropPart, Held, NewIndex, Part};
use super::lift::Lifting;
use super::op::{Fail, Res, broken, refuse, refuse_at, refuse_diag};
use super::run::{Built, Decls, Late, Run, unbuilt};
use super::*;
use crate::ast::{self, Expr, RelationClause};
use crate::canon::{Names, canonical_term};
use crate::check;
use crate::evolve::{Clause, Resolved};
use crate::ids::{KeyKind, ObjectKey, Owner};
use crate::lex::{Tok, lex};
use crate::schema::same::{self, Place};
use crate::span::{FileId, Span};
use crate::types::RelId;

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
    pub(super) fn rel_in(schema: &crate::schema::Schema, r: u64) -> Res<RelId> {
        match schema.ids().relation_of(cat_rel(r)) {
            Some(rel) => Ok(rel),
            None => broken("a relation variable the built schema does not hold"),
        }
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
        let parts = match &on.sel {
            None => None,
            Some(sel) => {
                let built = self.schema_now(run)?;
                let rel = Self::rel_in(built.schema(), r)?;
                if let Err(e) = built.schema().relation_sel(rel, sel) {
                    return refuse_at(on.span, e);
                }
                let names = crate::evolve::sel_idents(sel);
                let found = scope_objects(
                    built.schema(),
                    rel,
                    &names.iter().map(|&c| c.clone()).collect::<Vec<_>>(),
                );
                let Some(parts) = names
                    .iter()
                    .zip(found)
                    .map(|(c, o)| Some(Part::held(o?, &c.text)))
                    .collect::<Option<Vec<_>>>()
                else {
                    return broken(
                        "a scope the checker resolves names no constructor or named set",
                    );
                };
                Some(parts)
            }
        };
        let index = self.fresh(Space::Index);
        let new = NewIndex::new(kind, vec![elems], parts, Vec::new()).identified(index);
        self.relations
            .get_mut(&r)
            .expect("a resolved relation variable")
            .push_index(new);
        let built = self.rebuilt_relation(run, r)?;
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
        self.relations[&r]
            .index_at(k)
            .map(Held::scope_clauses)
            .unwrap_or_default()
    }

    /// Drop the one unique or index over `fields` of the relation variable `on` names whose
    /// scope is the one `on` writes. The drop's names resolve as the checker resolves them
    /// against the declarations without the uniques and indexes naming an object the migration
    /// dropped; a name that resolves to none is one only with such an object of the clause it is
    /// compared with.
    pub(super) fn drop_index(
        &mut self,
        run: &mut Run,
        kind: IndexKind,
        r: u64,
        fields: &[Expr],
        on: &ast::RelationRef,
    ) -> Res<Applied> {
        let word = match kind {
            IndexKind::Unique => "unique",
            IndexKind::Plain => "index",
            IndexKind::Interval => "interval index",
        };
        let rname = self.relations[&r].name.clone();
        let want: Vec<&str> = fields.iter().map(|e| &run.text()[e.span.range()]).collect();
        let missing = format!(
            "`{rname}` has no {word} ({}){}",
            want.join(", "),
            scope_shown(on.sel.as_ref())
        );
        let module = run.module().to_string();
        let held = self.without_gone();
        let schema = match held.schema(&run.ahead(&held)) {
            Ok(s) => s,
            Err(ds) => return unbuilt(&ds),
        };
        let rel = Self::rel_in(&schema, r)?;
        let scope = on.sel.as_ref().map(|sel| {
            let idents: Vec<ast::Ident> = crate::evolve::sel_idents(sel)
                .into_iter()
                .cloned()
                .collect();
            let found = scope_objects(&schema, rel, &idents);
            idents
                .iter()
                .zip(found)
                .map(|(c, o)| match o {
                    Some(o) => DropPart::Held(o),
                    None => DropPart::Unheld(ObjectKey::named(
                        KeyKind::Set,
                        Owner::Object(Space::Shared, r),
                        &c.text,
                    )),
                })
                .collect::<Vec<_>>()
        });
        let refs = |reach: Option<&BTreeSet<u64>>| {
            let ctors: Option<Vec<u32>> = reach.map(|cs| {
                let mut out: Vec<u32> = cs
                    .iter()
                    .filter_map(|&c| self.ctor_index(&schema, c))
                    .collect();
                out.sort_unstable();
                out.dedup();
                out
            });
            check::element_refs(&schema, rel, ctors.as_ref(), fields)
        };
        let found = self.indexes_dropped(r, (kind, scope.as_deref()), &mut |reach| {
            let (refs, checked) = refs(reach);
            let names = noted(&schema, &refs);
            fields
                .iter()
                .map(|e| {
                    same::placed_tree(e, &mut |q, p| drop_name(&names, &checked, &module, (q, p)))
                })
                .collect()
        });
        let k = match found.as_slice() {
            [k] => *k,
            [] => {
                if let Some(sel) = &on.sel
                    && let Err(e) = schema.written_scope(rel, Some(sel))
                {
                    return refuse_at(on.span, e);
                }
                let own = scope.as_deref().and_then(|ps| self.drop_reach(ps));
                if let Some(at) = self.unmatched_name(&schema, (r, own, &module), fields, &refs) {
                    return refuse_at(at, format!("no `{}` in scope", &run.text()[at.range()]));
                }
                return refuse(missing);
            }
            _ => return broken("a relation variable holds a unique or an index twice"),
        };
        if run.is_copying() {
            let reach = self.relations[&r]
                .index_at(k)
                .and_then(|h| h.reach(self))
                .map(|cs| cs.into_iter().collect::<BTreeSet<u64>>());
            run.note(&schema, &refs(reach.as_ref()).0);
        }
        let Some(index) = self
            .relations
            .get_mut(&r)
            .expect("the relation variable")
            .remove_clause(k)
            .and_then(|h| h.id())
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

    /// Returns the constructors a drop's scope `parts` reaches, `None` when a part of it names
    /// nothing the model holds.
    fn drop_reach(&self, parts: &[DropPart]) -> Option<BTreeSet<u64>> {
        let mut out = BTreeSet::new();
        for p in parts {
            match p {
                DropPart::Held((Space::Set, s)) => out.extend(self.set_reach(*s)),
                DropPart::Held((_, c)) => {
                    out.insert(*c);
                }
                DropPart::Unheld(_) => return None,
            }
        }
        Some(out)
    }

    /// Returns the first name of `fields`, a drop in module `module` over relation variable `r`
    /// written with a scope reaching `own`, that resolves through `refs` under neither that
    /// scope, a scope a unique or index of `r` reaches, nor every constructor, and spells no
    /// object the migration dropped that one of them names.
    fn unmatched_name(
        &self,
        schema: &crate::schema::Schema,
        (r, own, module): (u64, Option<BTreeSet<u64>>, &str),
        fields: &[Expr],
        refs: &dyn Fn(Option<&BTreeSet<u64>>) -> ElementRefs,
    ) -> Option<Span> {
        let rel = &self.relations[&r];
        let mut reaches: Vec<Option<BTreeSet<u64>>> = vec![None];
        if own.is_some() {
            reaches.push(own);
        }
        for (_, h) in rel.indexes() {
            let reach = h.reach(self).map(|cs| cs.into_iter().collect());
            if !reaches.contains(&reach) {
                reaches.push(reach);
            }
        }
        let mut unmatched: Option<Vec<Span>> = None;
        for reach in &reaches {
            let (refs, checked) = refs(reach.as_ref());
            let names = noted(schema, &refs);
            let mut here = Vec::new();
            for e in fields {
                same::placed_tree(e, &mut |q, p| {
                    let n = drop_name(&names, &checked, module, (q, p));
                    if let Some(w @ DropName::Unheld { .. }) = &n
                        && !rel.names_dropped(w)
                    {
                        here.push(q.span());
                    }
                    n
                });
            }
            unmatched = Some(match unmatched {
                None => here,
                Some(was) => was.into_iter().filter(|s| here.contains(s)).collect(),
            });
        }
        unmatched?.first().copied()
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
        self.relations
            .get_mut(&r)
            .expect("a resolved relation variable")
            .push_computed(computed, field.text.clone(), ty, expr);
        let Some((built, file)) = self.rebuilt_relation(run, r)? else {
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
        if !self
            .relations
            .get_mut(&r)
            .expect("the relation variable")
            .redefine_computed(k, ty, expr)
        {
            return broken("a computed field's position holds another clause");
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
        let Some(computed) = self
            .relations
            .get_mut(&r)
            .expect("the relation variable")
            .remove_computed(k)
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

/// The names of elements the checker resolved, and the spans of the expressions that check.
type ElementRefs = (Vec<(Span, check::Ref)>, Vec<Span>);

/// Returns the constructor or named set each of `idents`, a scope of relation `rel`, names in
/// `schema` as the checker resolves it, `None` for one it resolves to neither.
pub(super) fn scope_objects(
    schema: &crate::schema::Schema,
    rel: RelId,
    idents: &[ast::Ident],
) -> Vec<Option<Obj>> {
    let sel = ast::CtorSel::Many(idents.to_vec());
    let names = noted(schema, &check::scope_refs(schema, rel, &sel));
    idents
        .iter()
        .map(|c| ids_at(&names, c.span).into_iter().next())
        .collect()
}

/// Returns the names `refs` resolve, `schema` holding what they name.
fn noted(schema: &crate::schema::Schema, refs: &[(Span, check::Ref)]) -> Names {
    let mut names = Names::default();
    for (span, r) in refs {
        names.note(schema, *span, r);
    }
    names
}

/// Returns the objects `names` resolves the name at `span` to.
fn ids_at(names: &Names, span: Span) -> BTreeSet<Obj> {
    names
        .ids_within(span.range())
        .filter(|(r, ..)| *r == span.range())
        .flat_map(|(_, ids, _)| ids.iter().copied())
        .collect()
}

/// Returns what `names` resolves name `q` of a dropped element to: its objects, `None` for an
/// object with no catalog id, which its text names, and its text when it resolves to nothing.
/// A name inside an expression of `checked` resolves; an unqualified one is of `module`.
fn drop_name(
    names: &Names,
    checked: &[Span],
    module: &str,
    (q, place): (&ast::QName, Place),
) -> Option<DropName> {
    let ids = ids_at(names, q.name.span);
    if !ids.is_empty() {
        return Some(DropName::Objects(ids));
    }
    let at = q.name.span;
    if names.get(at).is_some()
        || checked
            .iter()
            .any(|c| c.start <= at.start && at.end <= c.end)
    {
        return None;
    }
    Some(DropName::Unheld {
        module: q.module.as_ref().map_or(module, |m| &m.text).to_string(),
        name: q.name.text.clone(),
        place,
    })
}
