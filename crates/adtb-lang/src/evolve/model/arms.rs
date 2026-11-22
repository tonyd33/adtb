use super::op::{Res, broken, refuse_at};
use super::reselect::Edit;
use super::run::{Built, Late, Run};
use super::*;
use crate::ast;
use crate::canon::canonical_term;
use crate::check::{self, Ref};
use crate::evolve::text::{holds, match_arms, match_end};
use crate::evolve::{ArmTarget, Clause};
use crate::lex::{Tok, lex};
use crate::span::{FileId, Span};

/// An arm an operator wrote into the one match of its target over some of the constructors
/// `ctors` of type `ty`: the `at`th of the `of` arms it appended to that match, written `text`.
#[derive(Clone, Debug)]
pub(super) struct ArmM {
    item: Item,
    target: ArmTarget,
    ty: u64,
    ctors: Vec<u64>,
    at: usize,
    of: usize,
    text: String,
}

/// The match of a target that arms go into: its declaration, its span, and the arms by their
/// position in the operator.
struct Placed {
    decl: usize,
    at: Span,
    item: Item,
    target: ArmTarget,
    arms: Vec<usize>,
}

impl Model {
    /// Returns the edits that append each of `arms` to the one match of its target over a value
    /// covering one of `cs` of type `t`, as `built` resolves the declarations, and remove from
    /// that match each arm naming one of `removed`; with the arms as written.
    pub(super) fn arm_writes(
        &self,
        run: &Run,
        built: &Built,
        t: u64,
        (cs, removed): (&[u64], &[u64]),
        arms: &[ast::ArmClause],
    ) -> Res<(Vec<Edit>, Vec<ArmM>)> {
        let schema = built.schema();
        let def = structural::def_in(schema, t);
        let cis: Vec<u32> = cs.iter().map(|&c| structural::ctor_in(schema, c)).collect();
        let refs = check::schema_refs(schema);
        let mut placed: Vec<Placed> = Vec::new();
        for (n, a) in arms.iter().enumerate() {
            let (item, target, k) = self.arm_target(run, a)?;
            let j = self.decl_at(item).expect("a declared target");
            let file = FileId(j as u32);
            let within = computed_span(built, file, k);
            let mut found: Vec<Span> = refs
                .iter()
                .filter(|(f, s, r)| {
                    *f == file
                        && within.is_none_or(|w| w.start <= s.start && s.end <= w.end)
                        && matches!(r, Ref::Match(ty) if cis.iter().any(|&c| holds(schema, ty, def, c)))
                })
                .map(|(_, s, _)| *s)
                .collect();
            found.sort_by_key(|s| (s.start, s.end));
            found.dedup();
            let at = match found.as_slice() {
                [one] => *one,
                [] => {
                    return refuse_at(
                        a.target.span(),
                        format!(
                            "`{}` has no match over the constructors this arm is for; redefine it in the same migration instead",
                            a.target.name.text
                        ),
                    );
                }
                _ => {
                    return refuse_at(
                        a.target.span(),
                        format!(
                            "`{}` has more than one match over the constructors this arm is for; redefine it in the same migration instead",
                            a.target.name.text
                        ),
                    );
                }
            };
            match placed.iter_mut().find(|p| p.decl == j && p.at == at) {
                Some(p) => p.arms.push(n),
                None => placed.push(Placed {
                    decl: j,
                    at,
                    item,
                    target,
                    arms: vec![n],
                }),
            }
        }
        let removed: Vec<u32> = removed
            .iter()
            .map(|&c| structural::ctor_in(schema, c))
            .collect();
        let text = |a: &ast::ArmClause| {
            run.text()[a.pattern.span.start as usize..a.body.span.end as usize].to_string()
        };
        let mut edits = Vec::new();
        let mut written: Vec<Option<ArmM>> = vec![None; arms.len()];
        for p in placed {
            if !removed.is_empty() {
                edits.extend(self.arm_removals(built, &refs, p.decl, p.at, def, &removed));
            }
            let file = FileId(p.decl as u32);
            let h = format!("module {}\n", self.decls[p.decl].module).len();
            let end = built
                .decl(file)
                .and_then(|d| match_end(d, p.at))
                .unwrap_or(p.at.end as usize)
                - h;
            let joined: String = p
                .arms
                .iter()
                .map(|&n| format!("\n  | {}", text(&arms[n])))
                .collect();
            edits.push(Edit::new(p.decl, end..end, Term::text(&joined)));
            for (at, &n) in p.arms.iter().enumerate() {
                written[n] = Some(ArmM {
                    item: p.item,
                    target: p.target.clone(),
                    ty: t,
                    ctors: Vec::new(),
                    at,
                    of: p.arms.len(),
                    text: text(&arms[n]),
                });
            }
        }
        Ok((edits, written.into_iter().flatten().collect()))
    }

    /// Returns the clause of each arm of `written`, which now goes into a match over some of the
    /// constructors `cs`: its canonical form in `after`, the declarations once they build, or,
    /// while they do not, the form they give it once they first build.
    pub(super) fn arm_clauses(
        &mut self,
        run: &Run,
        after: Option<&Built>,
        cs: &[u64],
        written: Vec<ArmM>,
    ) -> Res<Vec<Clause>> {
        let mut items: Vec<Item> = Vec::new();
        for w in &written {
            if !items.contains(&w.item) {
                items.push(w.item);
            }
        }
        let mut out = Vec::new();
        match after {
            Some(after) => {
                for &item in &items {
                    self.relift(after, item)?;
                }
                for mut w in written {
                    w.ctors = cs.to_vec();
                    let arm = self
                        .arm_written(after, &w)?
                        .unwrap_or_else(|| run.late(Late::Unresolved));
                    out.push(Clause::Arm {
                        target: w.target,
                        arm,
                    });
                }
            }
            None => {
                for item in items {
                    if !self.unlifted.contains(&item) {
                        self.unlifted.push(item);
                    }
                }
                for (nth, mut w) in written.into_iter().enumerate() {
                    w.ctors = cs.to_vec();
                    let key = self
                        .arms
                        .keys()
                        .chain(self.arms_written.keys())
                        .max()
                        .map_or(0, |k| k + 1);
                    let target = w.target.clone();
                    self.arms.insert(key, w);
                    out.push(Clause::Arm {
                        target,
                        arm: run.late(Late::Arm { key, nth }),
                    });
                }
            }
        }
        Ok(out)
    }

    /// Give each arm written while the declarations did not build the canonical form it has in
    /// `built`, the declarations as they first build.
    pub(super) fn arms_built(&mut self, built: &Built) -> Res<()> {
        for (key, w) in std::mem::take(&mut self.arms) {
            let arm = self.arm_written(built, &w)?;
            self.arms_written.insert(key, arm);
        }
        Ok(())
    }

    /// Returns the canonical form of arm `w` in `built`, `None` when its target no longer holds
    /// it as written or a name of it does not resolve there.
    fn arm_written(&self, built: &Built, w: &ArmM) -> Res<Option<String>> {
        let Some(j) = self.decl_at(w.item) else {
            return Ok(None);
        };
        let file = FileId(j as u32);
        let k = match (&w.target, w.item) {
            (ArmTarget::OfComputed { computed }, Item::Relation(r)) => {
                self.relations[&r].clauses.iter().position(
                    |c| matches!(c, ClauseM::Computed { id: Some(id), .. } if id == computed),
                )
            }
            _ => None,
        };
        let schema = built.schema();
        let def = structural::def_in(schema, w.ty);
        let cis: Vec<u32> = w
            .ctors
            .iter()
            .map(|&c| structural::ctor_in(schema, c))
            .collect();
        let within = computed_span(built, file, k);
        let mut found: Vec<Span> = check::schema_refs(schema)
            .into_iter()
            .filter(|(f, s, r)| {
                *f == file
                    && within.is_none_or(|x| x.start <= s.start && s.end <= x.end)
                    && matches!(r, Ref::Match(ty) if cis.iter().any(|&c| holds(schema, ty, def, c)))
            })
            .map(|(_, s, _)| s)
            .collect();
        found.sort_by_key(|s| (s.start, s.end));
        found.dedup();
        let [at] = found.as_slice() else {
            return Ok(None);
        };
        let Some(d) = built.decl(file) else {
            return broken("a declaration the model printed does not parse");
        };
        let Some(held) = match_arms(d, *at) else {
            return Ok(None);
        };
        let Some(arm) = (held.len() + w.at)
            .checked_sub(w.of)
            .and_then(|n| held.get(n))
        else {
            return Ok(None);
        };
        let span = Span::new(
            arm.patterns[0].span.start as usize,
            arm.body.span.end as usize,
        );
        let text = built.text(file);
        if tokens(&text[span.range()]) != tokens(&w.text) {
            return Ok(None);
        }
        Ok(canonical_term(text, span, &built.names(file)).ok())
    }
}

/// Returns the span of the expression of computed clause `k` of the relation variable `file` of
/// `built` declares, `None` without `k`.
fn computed_span(built: &Built, file: FileId, k: Option<usize>) -> Option<Span> {
    match (built.decl(file), k) {
        (Some(ast::Decl::Relation(rd)), Some(k)) => match rd.clauses.get(k) {
            Some(ast::RelationClause::Computed(_, _, e)) => Some(e.span),
            _ => None,
        },
        _ => None,
    }
}

fn tokens(text: &str) -> Vec<&str> {
    lex(text)
        .0
        .into_iter()
        .filter(|t| t.tok != Tok::Eof)
        .map(|t| &text[t.span.range()])
        .collect()
}
