use adtb_lang::kernel::{Build, Kind, PathStep, Prim, Tm, Var};
use adtb_lang::schema::CtorRef;
use adtb_lang::types::{CtorSet, Ty};

use super::{Fixed, Fixes, Moved, Planner};
use crate::plan::Broken;

impl Planner<'_> {
    /// Returns `for y in src do body` with the first condition of its total prefix that reads `y`
    /// only under a constructor taken apart, when there is one:
    /// - a case over a value that does not read `y` becomes a case around the loop;
    /// - an equality of such a value with a constructor value that reads `y` becomes a case over
    ///   that value first;
    /// - a case over `y` itself with one arm that is not false narrows `src` to that arm's
    ///   constructor, and the arm's variable binds the rows.
    pub(super) fn split_join(&mut self, y: Var, src: Tm, body: Tm) -> Result<Option<Tm>, Broken> {
        let ty = self.ty(body);
        let (conds, rest) = self.peel(body);
        let prefix = self.total_prefix(&conds);
        for (slot, moved) in prefix.iter() {
            let c = match self.ctor_equality(&moved, y) {
                Some(c) => c,
                None => moved,
            };
            let Kind::Case { scrutinee, arms } = self.moved_kind(&c) else {
                continue;
            };
            if matches!(self.kind(scrutinee), Kind::Var(v) if v == y) {
                let live: Vec<&(u32, Var, Tm)> = arms
                    .iter()
                    .filter(|(_, _, a)| self.literal(*a) != Some(false))
                    .collect();
                let [(k, v, arm)] = live[..] else { continue };
                let (k, v, arm) = (*k, *v, *arm);
                let cs = prefix.with(slot, arm)?;
                let inner = self.guarded(&cs, rest, &ty);
                let schema = self.schema;
                let inner = self.t.substitute(schema, inner, |terms, s, _| {
                    matches!(terms.kind(s), Kind::Var(x) if *x == y)
                        .then(|| Build::new(schema, terms).var(v))
                });
                let narrowed = self.b().restrict(src, CtorSet::from([k]));
                return Ok(Some(self.b().for_(v, narrowed, inner, ty)));
            }
            let Some(scrutinee) = self.part(&c, scrutinee) else {
                continue;
            };
            if self.reads(&scrutinee, &[y]) || !self.reads(&c, &[y]) {
                continue;
            }
            let mut out = Vec::new();
            for (k, v, arm) in arms {
                if self.literal(arm) == Some(false) {
                    out.push((k, v, self.empty(ty.clone())));
                    continue;
                }
                let cs = prefix.with(slot, arm)?;
                let inner = self.guarded(&cs, rest, &ty);
                out.push((k, v, self.b().for_(y, src, inner, ty.clone())));
            }
            return Ok(Some(self.b().case(scrutinee.place(), out, ty)));
        }
        Ok(None)
    }

    /// Returns equality `c` of a value that does not read `y` with a constructor value that does,
    /// as a case over the value whose arm for that constructor compares each field.
    fn ctor_equality(&mut self, c: &Moved, y: Var) -> Option<Moved> {
        let Kind::Prim(Prim::Eq, args) = self.moved_kind(c) else {
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
                let row = self.b().var(v);
                let mut out: Option<Tm> = None;
                for (f, value) in fields.iter().rev() {
                    let got = self.b().field(row, f.clone());
                    let eq = self.b().prim(Prim::Eq, vec![got, *value], bool_ty.clone());
                    out = Some(match out {
                        None => eq,
                        Some(rest) => self.b().if_(eq, rest, no, bool_ty.clone()),
                    });
                }
                out.unwrap_or_else(|| self.truth())
            } else {
                no
            };
            arms.push((k, v, body));
        }
        let case = self.b().case(other, arms, bool_ty);
        self.part(c, case)
    }

    /// Add to `out`, after `from`, the constructor value the path `path` holds when the
    /// equalities from `from` on fix every field of constructor `ctor` at that path, with
    /// `scrutinee` the term reading the path.
    pub(super) fn whole_ctor(
        &mut self,
        of: &Moved,
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
                Some((_, v)) => fields.push((name, v.clone().place())),
                None => return,
            }
        }
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        let ty = self.ty(scrutinee);
        let value = self.b().ctor(ctor, fields, ty);
        let Some(value) = self.part(of, value) else {
            return;
        };
        out.push((
            Fixed {
                path: Some(path.to_vec()),
                term: scrutinee,
            },
            value,
        ));
    }

    /// Returns the constructor of the one arm of `arms` that `dead` does not hold of.
    pub(super) fn arm_ctor(
        &self,
        arms: &[(u32, Var, Tm)],
        dead: impl Fn(&Self, Tm) -> bool,
    ) -> Option<u32> {
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
        self.boolean(self.schema.engine_ctors().bool_false())
    }

    fn truth(&mut self) -> Tm {
        self.boolean(self.schema.engine_ctors().bool_true())
    }

    fn boolean(&mut self, value: CtorRef) -> Tm {
        let ty = self.bool_ty();
        let ctor = value.index();
        self.b().ctor(ctor, Vec::new(), ty)
    }
}
