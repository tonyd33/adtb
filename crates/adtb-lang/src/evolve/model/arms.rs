use std::ops::Range;

use super::op::{Res, broken, refuse, refuse_at};
use super::reselect::Edit;
use super::run::{Built, Late, Run};
use super::*;
use crate::ast;
use crate::canon::canonical_term;
use crate::check::{self, Ref};
use crate::evolve::text::{holds, match_end};
use crate::evolve::{ArmTarget, Clause};
use crate::span::{FileDiag, FileId, Span};

/// An arm an operator wrote into a match of its target `item`, held by `key` in its pieces.
#[derive(Clone, Debug)]
pub(super) struct ArmM {
    item: Item,
    target: ArmTarget,
    key: u64,
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
        &mut self,
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
            let mut joined = Vec::new();
            for &n in &p.arms {
                let a = &arms[n];
                let start = a.pattern.span.start as usize;
                let key = self.next_arm;
                self.next_arm += 1;
                joined.push(Term::text("\n  | "));
                joined.push(Term::arm(key, &run.text()[start..a.body.span.end as usize]));
                if run.is_copying() {
                    self.naming.arm(key, start..a.body.span.end as usize);
                }
                self.arms.insert(key, p.item);
                written[n] = Some(ArmM {
                    item: p.item,
                    target: p.target.clone(),
                    key,
                });
            }
            edits.push(Edit::new(p.decl, end..end, Term::concat(joined)));
        }
        Ok((edits, written.into_iter().flatten().collect()))
    }

    /// Write `arms` for constructor `c` an operator added to type `t`, as `built` resolves the
    /// declarations before them. Returns their clauses.
    pub(super) fn arms_added(
        &mut self,
        run: &Run,
        built: &Built,
        (t, c): (u64, u64),
        arms: &[ast::ArmClause],
    ) -> Res<Vec<Clause>> {
        let (edits, written) = self.arm_writes(run, built, t, (&[c], &[]), arms)?;
        self.edit_decls(edits)?;
        let after = self.build_adopting(&[], &run.ahead(self));
        self.arm_clauses(run, after.as_ref().map_err(Vec::as_slice), written)
    }

    /// Returns the clause of each arm of `written`: the canonical form the declarations gave it
    /// when they built as `after`, or, while they do not build, the form they give it once they
    /// first build.
    ///
    /// Fail when `after` is the failures of declarations that do not build and one is inside
    /// an arm of `written`.
    pub(super) fn arm_clauses(
        &mut self,
        run: &Run,
        after: Result<&Built, &[FileDiag]>,
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
            Ok(after) => {
                for w in written {
                    let arm = match self.arms_written.remove(&w.key) {
                        Some(Ok(Some(arm))) => arm,
                        Some(Err(e)) => return broken(e),
                        _ => run.late(Late::Unresolved),
                    };
                    out.push(Clause::Arm {
                        target: w.target,
                        arm,
                    });
                }
                for item in items {
                    self.relift(after, item)?;
                }
            }
            Err(ds) => {
                self.arms_refused(ds, &written)?;
                for item in items {
                    if !self.unlifted.contains(&item) {
                        self.unlifted.push(item);
                    }
                }
                for (nth, w) in written.into_iter().enumerate() {
                    out.push(Clause::Arm {
                        target: w.target,
                        arm: run.late(Late::Arm { key: w.key, nth }),
                    });
                }
            }
        }
        Ok(out)
    }

    /// Refuse the operator with the first of `ds` inside an arm of `written`.
    fn arms_refused(&self, ds: &[FileDiag], written: &[ArmM]) -> Res<()> {
        for d in ds {
            let i = d.file.0 as usize;
            if !written.iter().any(|w| self.decl_at(w.item) == Some(i)) {
                continue;
            }
            let h = format!("module {}\n", self.decls[i].module).len();
            let (Some(start), Some(end)) = (
                (d.diag.span.start as usize).checked_sub(h),
                (d.diag.span.end as usize).checked_sub(h),
            ) else {
                continue;
            };
            let printed = self.print_mapped(i);
            if printed.arms().iter().any(|(k, r)| {
                written.iter().any(|w| w.key == *k) && r.start <= start && end <= r.end
            }) {
                return refuse(d.diag.message.clone());
            }
        }
        Ok(())
    }

    /// Give each arm written while the declarations did not build the canonical form it has in
    /// `built`, the declarations as they first build.
    pub(super) fn arms_built(&mut self, built: &Built) {
        for (key, item) in std::mem::take(&mut self.arms) {
            let placed = self.arm_place(built, item, key);
            let arm = match &placed {
                Ok(Some((file, r))) => {
                    self.name_arm(built, key, *file, r.clone());
                    Ok(canonical_term(
                        built.text(*file),
                        Span::new(r.start, r.end),
                        &built.names(*file),
                    )
                    .ok())
                }
                Ok(None) => Ok(None),
                Err(e) => Err(format!("{e:?}")),
            };
            self.arms_written.insert(key, arm);
        }
    }

    /// Returns the file of arm `key` of `item` in `built`, which the declarations as the model
    /// holds them built, and the range it takes there; `None` when `item` no longer holds it.
    fn arm_place(
        &self,
        built: &Built,
        item: Item,
        key: u64,
    ) -> Res<Option<(FileId, Range<usize>)>> {
        let Some(j) = self.decl_at(item) else {
            return Ok(None);
        };
        let Some((_, r)) = self
            .print_mapped(j)
            .arms()
            .iter()
            .find(|(k, _)| *k == key)
            .cloned()
        else {
            return Ok(None);
        };
        let file = FileId(j as u32);
        let h = format!("module {}\n", self.decls[j].module).len();
        if built.text(file).get(h + r.start..h + r.end).is_none() {
            return broken("an arm the model printed is outside its declaration's build");
        }
        Ok(Some((file, h + r.start..h + r.end)))
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
