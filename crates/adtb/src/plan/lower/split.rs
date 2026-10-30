use std::collections::HashMap;

use adtb_lang::kernel::{Kind, PathStep, Prim, Tm, Var};
use adtb_lang::types::{CtorSet, Ty};

use super::{Fixed, Fixes, Planner};

impl Planner<'_> {
    /// Returns `for y in src do body` with the first condition of its total prefix that reads `y`
    /// only under a constructor taken apart, when there is one:
    /// - a case over a value that does not read `y` becomes a case around the loop;
    /// - an equality of such a value with a constructor value that reads `y` becomes a case over
    ///   that value first;
    /// - a case over `y` itself with one arm that is not false narrows `src` to that arm's
    ///   constructor, and the arm's variable binds the rows.
    pub(super) fn split_join(&mut self, y: Var, src: Tm, body: Tm) -> Option<Tm> {
        let ty = self.ty(body);
        let (conds, rest) = self.peel(body);
        let n = self.total_prefix(&conds);
        for i in 0..n {
            let c = self.ctor_equality(conds[i], y).unwrap_or(conds[i]);
            let Kind::Case { scrutinee, arms } = self.kind(c) else {
                continue;
            };
            if matches!(self.kind(scrutinee), Kind::Var(v) if v == y) {
                let live: Vec<&(u32, Var, Tm)> = arms
                    .iter()
                    .filter(|(_, _, a)| self.literal(*a) != Some(false))
                    .collect();
                let [(k, v, arm)] = live[..] else { continue };
                let (k, v, arm) = (*k, *v, *arm);
                let mut cs = conds.clone();
                cs[i] = arm;
                let inner = self.guarded(&cs, rest, &ty);
                let inner = self.rename(inner, y, v, &mut HashMap::new());
                let narrowed = self.add(Kind::Restrict(src, CtorSet::from([k])), self.ty(src));
                return Some(self.add(Kind::For(v, narrowed, inner), ty));
            }
            if self.mentions(scrutinee, &[y])
                || !self.mentions(c, &[y])
                || !self.scalar(scrutinee)
                || !self.total(scrutinee)
            {
                continue;
            }
            let mut out = Vec::new();
            for (k, v, arm) in arms {
                if self.literal(arm) == Some(false) {
                    out.push((k, v, self.empty(ty.clone())));
                    continue;
                }
                let mut cs = conds.clone();
                cs[i] = arm;
                let inner = self.guarded(&cs, rest, &ty);
                out.push((k, v, self.add(Kind::For(y, src, inner), ty.clone())));
            }
            return Some(self.add(
                Kind::Case {
                    scrutinee,
                    arms: out,
                },
                ty,
            ));
        }
        None
    }

    /// Returns equality `c` of a value that does not read `y` with a constructor value that does,
    /// as a case over the value whose arm for that constructor compares each field.
    fn ctor_equality(&mut self, c: Tm, y: Var) -> Option<Tm> {
        let Kind::Prim(Prim::Eq, args) = self.kind(c) else {
            return None;
        };
        let [a, b] = args[..] else { return None };
        let (built, other) = [(a, b), (b, a)].into_iter().find(|(this, other)| {
            matches!(self.t.kind(*this), Kind::Ctor { .. })
                && !self.facts.mentions(&self.t, *other, &[y])
        })?;
        if !self.mentions(built, &[y]) {
            return None;
        }
        let Kind::Ctor { ctor, fields } = self.kind(built) else {
            return None;
        };
        let Ty::Data { def, args, ctors } = self.ty(other) else {
            return None;
        };
        let all: CtorSet = (0..self.schema.ty(def).ctors().len() as u32).collect();
        let bool_ty = self.bool_ty();
        let no = self.falsity();
        let mut arms = Vec::new();
        for k in ctors.unwrap_or(all) {
            let narrowed = Ty::Data {
                def,
                args: args.clone(),
                ctors: Some(CtorSet::from([k])),
            };
            let v = self.fresh("%arm", narrowed.clone());
            let body = if k == ctor {
                let row = self.add(Kind::Var(v), narrowed);
                let mut out: Option<Tm> = None;
                for (f, value) in fields.iter().rev() {
                    let field_ty = self.ty(*value);
                    let got = self.add(Kind::Field(row, f.clone()), field_ty);
                    let eq = self.add(Kind::Prim(Prim::Eq, vec![got, *value]), bool_ty.clone());
                    out = Some(match out {
                        None => eq,
                        Some(rest) => self.add(Kind::If(eq, rest, no), bool_ty.clone()),
                    });
                }
                out.unwrap_or_else(|| self.truth())
            } else {
                no
            };
            arms.push((k, v, body));
        }
        Some(self.add(
            Kind::Case {
                scrutinee: other,
                arms,
            },
            bool_ty,
        ))
    }

    /// Add to `out`, after `from`, the constructor value the path `path` holds when the
    /// equalities from `from` on fix every field of constructor `ctor` at that path, with
    /// `scrutinee` the term reading the path.
    pub(super) fn whole_ctor(
        &mut self,
        scrutinee: Tm,
        path: &[PathStep],
        ctor: u32,
        from: usize,
        out: &mut Fixes,
    ) {
        let Ty::Data { def, .. } = self.ty(scrutinee) else {
            return;
        };
        let d = self.schema.ty(def);
        let names: Vec<String> = d
            .header()
            .iter()
            .chain(&d.ctors()[ctor as usize].fields)
            .map(|f| f.name.clone())
            .collect();
        let mut fields = Vec::new();
        for name in names {
            let found = out[from..].iter().find(|(f, _)| {
                f.path.as_deref().is_some_and(|p| {
                    p.len() == path.len() + 2
                        && p[..path.len()] == *path
                        && p[path.len()] == PathStep::Ctor(ctor)
                        && p[path.len() + 1] == PathStep::Field(name.clone())
                })
            });
            match found {
                Some((_, v)) => fields.push((name, *v)),
                None => return,
            }
        }
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        let value = self.add(Kind::Ctor { ctor, fields }, self.ty(scrutinee));
        out.push((
            Fixed {
                path: Some(path.to_vec()),
                term: scrutinee,
            },
            value,
        ));
    }

    /// Returns the constructor of the one arm of case `t` that `dead` does not hold of.
    pub(super) fn arm_ctor(&self, t: Tm, dead: impl Fn(&Self, Tm) -> bool) -> Option<u32> {
        let Kind::Case { arms, .. } = self.t.kind(t) else {
            return None;
        };
        let live: Vec<u32> = arms
            .iter()
            .filter(|(_, _, a)| !dead(self, *a))
            .map(|(c, _, _)| *c)
            .collect();
        match live[..] {
            [c] => Some(c),
            _ => None,
        }
    }

    fn falsity(&mut self) -> Tm {
        self.boolean("False")
    }

    fn truth(&mut self) -> Tm {
        self.boolean("True")
    }

    fn boolean(&mut self, name: &str) -> Tm {
        let ty = self.bool_ty();
        let ctor = self
            .schema
            .ty(self.schema.prelude_type("Bool"))
            .ctor_index(name)
            .expect("the prelude's Bool has True and False");
        self.add(
            Kind::Ctor {
                ctor,
                fields: Vec::new(),
            },
            ty,
        )
    }

    /// Returns `t` with each read of variable `from` reading `to`, at the type it had.
    fn rename(&mut self, t: Tm, from: Var, to: Var, memo: &mut HashMap<Tm, Tm>) -> Tm {
        if let Some(n) = memo.get(&t) {
            return *n;
        }
        let out = match self.kind(t) {
            Kind::Var(v) if v == from => self.add(Kind::Var(to), self.ty(t)),
            kind => {
                let cs = self.t.children(t);
                let new: Vec<Tm> = cs.iter().map(|c| self.rename(*c, from, to, memo)).collect();
                if new == cs {
                    t
                } else {
                    self.add(kind.with_children(&new), self.ty(t))
                }
            }
        };
        memo.insert(t, out);
        out
    }
}
