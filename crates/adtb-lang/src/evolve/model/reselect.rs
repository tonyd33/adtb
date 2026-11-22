use std::ops::Range;

use super::field::Applied;
use super::op::{Fail, Res, Resolving, broken, refuse};
use super::run::{Built, Run};
use super::*;
use crate::ast::{self, Decl as AstDecl, QName};
use crate::check::{self, Ref};
use crate::evolve::ctor::remove::removal_blocked;
use crate::evolve::text::{arm_range, holds, match_arms};
use crate::evolve::{Cx, Resolved, each_type};
use crate::schema::Stage;
use crate::span::{FileId, Span};
use crate::types::TypeId;

/// Which selections a reselection rewrites, and how its refusals name the operator.
#[derive(Clone, Copy)]
pub(super) struct Mode {
    /// Whether a selection holding only some of the constructors replaced is refused.
    whole: bool,
    /// Whether the scopes of the relation variables' uniques and indexes are rewritten.
    clauses: bool,
    /// Whether the selections of checks are rewritten.
    checks: bool,
    what: &'static str,
}

impl Mode {
    /// Returns the reselection of a structural operator named `what`, which leaves the scopes
    /// of uniques and indexes to the operator; `whole` and `checks` as [`Mode`] states.
    pub(super) fn structural(what: &'static str, whole: bool, checks: bool) -> Mode {
        Mode {
            whole,
            clauses: false,
            checks,
            what,
        }
    }
}

/// An edit of declaration `decl`: `range` of its text without its header replaced by `with`.
pub(super) struct Edit {
    decl: usize,
    range: Range<usize>,
    with: Term,
}

impl Edit {
    /// Returns the edit of declaration `decl` replacing `range` of its text without its header
    /// by `with`.
    pub(super) fn new(decl: usize, range: Range<usize>, with: Term) -> Edit {
        Edit { decl, range, with }
    }

    /// Returns whether this edit and `o` replace the same text.
    pub(super) fn same_place(&self, o: &Edit) -> bool {
        self.decl == o.decl && self.range == o.range
    }
}

/// Returns `names` with the constructors `gone` replaced by `by` in place of the first of them;
/// `None` when it names none of `gone`, `Some(None)` when `whole` and it names only some.
pub(super) fn reselected(
    names: &[u64],
    gone: &[u64],
    by: &[u64],
    whole: bool,
) -> Option<Option<Vec<u64>>> {
    let hit = names.iter().filter(|n| gone.contains(n)).count();
    if hit == 0 {
        return None;
    }
    if whole && hit != gone.len() {
        return Some(None);
    }
    let mut out = Vec::new();
    for n in names {
        if gone.contains(n) {
            for b in by {
                if !out.contains(b) {
                    out.push(*b);
                }
            }
        } else {
            out.push(*n);
        }
    }
    Some(Some(out))
}

/// Returns the range of `text` a selection of `idents` spans, its parentheses included.
pub(super) fn sel_range(text: &str, idents: &[ast::Ident]) -> Range<usize> {
    let from = idents[0].span.start as usize;
    let to = idents.last().expect("an ident").span.end as usize;
    if idents.len() > 1 || text[..from].trim_end().ends_with('(') {
        let open = text[..from].trim_end().len() - 1;
        let close = to + text[to..].find(')').map_or(0, |k| k + 1);
        open..close
    } else {
        from..to
    }
}

fn sel_idents(sel: &ast::CtorSel) -> Vec<ast::Ident> {
    match sel {
        ast::CtorSel::One(c) => vec![c.clone()],
        ast::CtorSel::Many(cs) => cs.clone(),
    }
}

impl Model {
    /// Returns a selection of the constructors `ctors`: `C`, or `(C | D)`.
    pub(super) fn selection(&self, ctors: &[u64]) -> Term {
        let names = ctors
            .iter()
            .map(|&c| Term::name((Space::Shared, c), &self.ctors[&c].name));
        if ctors.len() == 1 {
            return Term::concat(names);
        }
        let mut parts = vec![Term::text("(")];
        for (k, n) in names.enumerate() {
            if k > 0 {
                parts.push(Term::text(" | "));
            }
            parts.push(n);
        }
        parts.push(Term::text(")"));
        Term::concat(parts)
    }

    /// Apply `edits`, each replacing a range of a declaration's text by a term.
    ///
    /// Fail when an edit spans text outside one term.
    pub(super) fn edit_decls(&mut self, mut edits: Vec<Edit>) -> Res<()> {
        edits.sort_by_key(|e| std::cmp::Reverse((e.decl, e.range.start, e.range.end)));
        let names = self.clone();
        for e in edits {
            let printed = self.print_mapped(e.decl);
            let Some(k) = printed
                .terms()
                .iter()
                .position(|r| r.start <= e.range.start && e.range.end <= r.end)
            else {
                return broken("an edit of a declaration outside its terms");
            };
            let at = printed.terms()[k].start;
            self.decl_terms_mut(e.decl)[k]
                .replace(&names, e.range.start - at..e.range.end - at, e.with)
                .map_err(Fail::Broken)?;
        }
        Ok(())
    }

    /// Returns the relation variables of `t` in `built`'s order, with their modules and names.
    pub(super) fn rels_of(&self, built: &Built, t: u64) -> Vec<(u64, String, String)> {
        let schema = built.schema();
        let Some(def) = schema.ids().type_of(crate::ids::CatTypeId::from_raw(t)) else {
            return Vec::new();
        };
        Cx::relations_of(schema, def)
            .into_iter()
            .filter_map(|(rel, m, name)| Some((schema.ids().relation_id(rel)?.raw(), m, name)))
            .collect()
    }

    /// Put `by` in place of the constructors `gone` of type `t` in every named set, and every
    /// reference type, scope and check selection `mode` asks for, of a relation variable of `t`,
    /// as `built` resolves the declarations. Returns the edits of declarations' terms.
    ///
    /// Fail with each selection that holds only some of `gone` when `mode` asks for all or none.
    pub(super) fn reselect(
        &mut self,
        built: &Built,
        t: u64,
        gone: &[u64],
        by: &[u64],
        mode: Mode,
    ) -> Res<Vec<Edit>> {
        let mut errs = Vec::new();
        let mut edits = Vec::new();
        for (r, m, rname) in self.rels_of(built, t) {
            for s in self.sets_of(r) {
                let names: Vec<u64> = self.sets[&s]
                    .members
                    .iter()
                    .filter_map(|x| x.objects().next().map(|o| o.1))
                    .collect();
                match reselected(&names, gone, by, mode.whole) {
                    None => {}
                    Some(None) => errs.push(format!(
                        "{} is refused: the named set `{}` of `{rname}` holds some but not all of its constructors",
                        mode.what, self.sets[&s].name
                    )),
                    Some(Some(out)) if out.is_empty() => {}
                    Some(Some(out)) => {
                        let members = out
                            .iter()
                            .map(|&c| Term::name((Space::Shared, c), &self.ctors[&c].name))
                            .collect();
                        self.sets.get_mut(&s).expect("the set").members = members;
                    }
                }
            }
            let sets: BTreeSet<String> = self
                .sets_of(r)
                .iter()
                .map(|s| self.sets[s].name.clone())
                .collect();
            let ctor_ids = |idents: &[ast::Ident]| -> Option<Vec<u64>> {
                idents.iter().map(|x| self.ctor_named(t, &x.text)).collect()
            };
            for j in 0..self.decls.len() {
                let file = FileId(j as u32);
                let Some(d) = built.decl(file) else {
                    continue;
                };
                let home = self.decls[j].module.clone();
                let names_rel = |qn: &QName| {
                    qn.name.text == rname && qn.module.as_ref().map_or(home == m, |x| x.text == m)
                };
                let mut sels: Vec<Vec<ast::Ident>> = Vec::new();
                each_type(d, &mut |ty| {
                    if let ast::TypeKind::Key(_, rr) = &ty.kind
                        && names_rel(&rr.relation)
                        && let Some(sel) = &rr.sel
                    {
                        sels.push(sel_idents(sel));
                    }
                });
                if let AstDecl::Check(c) = d
                    && mode.checks
                    && names_rel(&c.on.relation)
                    && let Some(sel) = &c.on.sel
                {
                    sels.push(sel_idents(sel));
                }
                let h = format!("module {home}\n").len();
                for idents in sels {
                    if idents.iter().any(|x| sets.contains(&x.text)) {
                        continue;
                    }
                    let Some(names) = ctor_ids(&idents) else {
                        continue;
                    };
                    match reselected(&names, gone, by, mode.whole) {
                        None => {}
                        Some(None) => {
                            let shown: Vec<&str> = idents.iter().map(|x| x.text.as_str()).collect();
                            let shown = match shown.as_slice() {
                                [one] => one.to_string(),
                                many => format!("({})", many.join(" | ")),
                            };
                            errs.push(format!(
                                "{} is refused: `{rname}.{shown}` names some but not all of its constructors",
                                mode.what
                            ));
                        }
                        Some(Some(out)) if out.is_empty() => {}
                        Some(Some(out)) => {
                            let range = sel_range(built.text(file), &idents);
                            edits.push(Edit {
                                decl: j,
                                range: range.start - h..range.end - h,
                                with: self.selection(&out),
                            });
                        }
                    }
                }
            }
            if mode.clauses {
                let rel = self.relations.get_mut(&r).expect("a relation variable");
                let mut scoped = Vec::new();
                for (k, c) in rel.clauses.iter().enumerate() {
                    if let ClauseM::Index { scope: Some(s), .. } = c {
                        let ids: Vec<Obj> =
                            s.parts.iter().filter_map(|p| p.objects().next()).collect();
                        if ids.len() != s.parts.len() || ids.iter().any(|o| o.0 == Space::Set) {
                            continue;
                        }
                        let names: Vec<u64> = ids.iter().map(|o| o.1).collect();
                        scoped.push((k, names));
                    }
                }
                let ctors = &self.ctors;
                for (k, names) in scoped {
                    match reselected(&names, gone, by, mode.whole) {
                        None => {}
                        Some(None) => {
                            let shown: Vec<&str> =
                                names.iter().map(|c| ctors[c].name.as_str()).collect();
                            let shown = match shown.as_slice() {
                                [one] => one.to_string(),
                                many => format!("({})", many.join(" | ")),
                            };
                            errs.push(format!(
                                "{} is refused: `{rname}.{shown}` names some but not all of its constructors",
                                mode.what
                            ));
                        }
                        Some(Some(out)) if out.is_empty() => {}
                        Some(Some(out)) => {
                            if let ClauseM::Index {
                                id, scope: Some(s), ..
                            } = &mut rel.clauses[k]
                            {
                                *id = None;
                                s.listed = out.len() > 1;
                                s.parts = out
                                    .iter()
                                    .map(|&c| Term::name((Space::Shared, c), &ctors[&c].name))
                                    .collect();
                            }
                        }
                    }
                }
            }
        }
        match errs.into_iter().next() {
            Some(e) => refuse(e),
            None => Ok(edits),
        }
    }

    /// Returns the edits that remove, from the match at `at` of declaration `j` over a value of
    /// type `def`, each arm whose pattern names a constructor of `replaced`.
    pub(super) fn arm_removals(
        &self,
        built: &Built,
        refs: &[(FileId, Span, Ref)],
        j: usize,
        at: Span,
        def: TypeId,
        replaced: &[u32],
    ) -> Vec<Edit> {
        let file = FileId(j as u32);
        let Some(d) = built.decl(file) else {
            return Vec::new();
        };
        let Some(arms) = match_arms(d, at) else {
            return Vec::new();
        };
        let text = built.text(file);
        let h = format!("module {}\n", self.decls[j].module).len();
        let mut out = Vec::new();
        for a in &arms {
            let names = a.patterns.iter().any(|p| {
                let ast::PatternKind::Ctor { path, ctor, .. } = &p.kind else {
                    return false;
                };
                let name = ctor.as_ref().unwrap_or(&path.name).span;
                refs.iter().any(|(f, s, r)| {
                    *f == file
                        && *s == name
                        && matches!(r, Ref::Ctor(d2, c) if *d2 == def && replaced.contains(c))
                })
            });
            if names {
                let r = arm_range(&text[h..], h, a);
                out.push(Edit {
                    decl: j,
                    range: r,
                    with: Term::default(),
                });
            }
        }
        out
    }

    /// Returns the constructors scope `s` names, a named set's members in its place.
    pub(super) fn scope_ctors(&self, s: &Scope) -> Vec<u64> {
        let mut out = Vec::new();
        for o in s.parts.iter().filter_map(|p| p.objects().next()) {
            let more = match o {
                (Space::Set, set) => self.sets[&set]
                    .members
                    .iter()
                    .filter_map(|m| m.objects().next().map(|o| o.1))
                    .collect(),
                (_, c) => vec![c],
            };
            for c in more {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        out
    }

    /// Remove constructor `c` of type `t` and its fields.
    pub(super) fn drop_ctor(&mut self, t: u64, c: u64) {
        for f in self.ctors[&c].fields.clone().into_iter().flatten() {
            self.fields.remove(&f);
        }
        self.ctors.remove(&c);
        if let TypeBody::Ctors(cs) = &mut self.types.get_mut(&t).expect("the type").body {
            cs.retain(|x| *x != c);
        }
    }

    pub(super) fn remove_constructor(&mut self, run: &Run, path: &ast::CtorPath) -> Res<Applied> {
        self.schema_now(run)?;
        let resolving = Resolving::new(self, run.module());
        let t = resolving.ty(&path.ty)?;
        let tname = self.types[&t].name.clone();
        let name = path.ctor.text.clone();
        let Some(c) = self.ctor_named(t, &name) else {
            return refuse(format!("`{name}` is not a constructor of `{tname}`"));
        };
        self.remove_ctor(run, t, c, false)?;
        Ok((
            Resolved::RemoveConstructor { ctor: c },
            Vec::new(),
            Ok(Vec::new()),
        ))
    }

    /// Remove constructor `c` of type `t` with its arms, as the inverse of adding it when
    /// `inverse`: then it need not be deprecated, and its arms leave every declaration.
    pub(super) fn remove_ctor(&mut self, run: &Run, t: u64, c: u64, inverse: bool) -> Res<()> {
        let built = self.schema_now(run)?;
        let tname = self.types[&t].name.clone();
        let name = self.ctors[&c].name.clone();
        let schema = built.schema();
        let def = schema
            .ids()
            .type_of(crate::ids::CatTypeId::from_raw(t))
            .expect("the type");
        let ci = schema
            .ids()
            .ctor_of(crate::ids::CatCtorId::from_raw(c))
            .expect("the constructor")
            .index();
        let rels = Cx::relations_of(schema, def);
        let earlier = |s: Option<&State>| {
            s.is_some_and(|s| s.stage == Stage::Deprecated && s.since < self.version)
        };
        let why = if inverse {
            None
        } else if rels.is_empty() {
            (!earlier(self.value_states.get(&c)))
                .then(|| format!("`{name}` is not deprecated in its type by an earlier migration"))
        } else {
            rels.iter()
                .find(|(rel, ..)| {
                    let r = schema
                        .ids()
                        .relation_id(*rel)
                        .expect("a held relation")
                        .raw();
                    !earlier(self.states.get(&(r, c)))
                })
                .map(|(_, _, r)| {
                    format!("`{name}` is not deprecated in `{r}` by an earlier migration")
                })
        };
        let Some(AstDecl::Type(td)) =
            built.decl(FileId(self.decl_at(Item::Type(t)).expect("the type") as u32))
        else {
            return broken("a type the model printed does not parse");
        };
        if let Some(why) = why.or_else(|| removal_blocked(schema, td, &name, def, ci, &rels)) {
            return refuse(format!(
                "`remove constructor {tname}.{name}` is refused: {why}"
            ));
        }
        let mode = Mode {
            whole: false,
            clauses: true,
            checks: true,
            what: "the remove",
        };
        let mut edits = self.reselect(&built, t, &[c], &[], mode)?;
        let refs = check::schema_refs(schema);
        for j in 0..self.decls.len() {
            if !inverse && !matches!(self.decls[j].item, Item::Constraint(_)) {
                continue;
            }
            let file = FileId(j as u32);
            for (_, at, _) in refs.iter().filter(|(f, _, r)| {
                *f == file && matches!(r, Ref::Match(ty) if holds(schema, ty, def, ci))
            }) {
                edits.extend(self.arm_removals(&built, &refs, j, *at, def, &[ci]));
            }
        }
        self.edit_decls(edits)?;
        for (rel, ..) in &rels {
            let r = schema
                .ids()
                .relation_id(*rel)
                .expect("a held relation")
                .raw();
            let only: Vec<usize> = self.relations[&r]
                .clauses
                .iter()
                .enumerate()
                .filter(|(_, cl)| match cl {
                    ClauseM::Index { scope: Some(s), .. } => self.scope_ctors(s) == [c],
                    ClauseM::Default { ctor: Some(x), .. } => {
                        x.objects().next() == Some((Space::Shared, c))
                    }
                    _ => false,
                })
                .map(|(k, _)| k)
                .collect();
            let held = self.relations.get_mut(&r).expect("a relation variable");
            for k in only.into_iter().rev() {
                held.clauses.remove(k);
            }
        }
        let before = self.clone();
        self.drop_ctor(t, c);
        self.rebind(&before);
        self.settle_ids(run);
        Ok(())
    }
}
