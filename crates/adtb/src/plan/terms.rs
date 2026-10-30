use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use adtb_lang::kernel::{Frees, Kind, Terms, Tm, Var};

/// Facts about terms of one arena, remembered per term.
#[derive(Debug, Default)]
pub struct Facts {
    free: Frees,
    scalar: HashMap<Tm, bool>,
    binders: HashMap<Tm, Rc<BTreeSet<Var>>>,
    sunk: HashMap<(Var, Tm, Tm), Tm>,
}

impl Facts {
    pub fn free(&mut self, terms: &Terms, t: Tm) -> Rc<BTreeSet<Var>> {
        self.free.of(terms, t)
    }

    /// Returns every variable a binder inside `t` binds.
    pub fn binders(&mut self, terms: &Terms, t: Tm) -> Rc<BTreeSet<Var>> {
        if let Some(s) = self.binders.get(&t) {
            return s.clone();
        }
        let kind = terms.kind(t);
        let cs = terms.children(t);
        let mut out: BTreeSet<Var> = kind.bound().into_iter().flatten().collect();
        for c in cs {
            out.extend(self.binders(terms, c).iter());
        }
        let out = Rc::new(out);
        self.binders.insert(t, out.clone());
        out
    }

    pub fn mentions(&mut self, terms: &Terms, t: Tm, vs: &[Var]) -> bool {
        let f = self.free(terms, t);
        vs.iter().any(|v| f.contains(v))
    }

    /// Returns `let x = e in c`, the `let` moved onto the operands of every operator at the root
    /// of `c` and dropped where `x` is not free.
    ///
    /// Preconditions:
    /// - `e` is copyable.
    pub fn sink_let(&mut self, terms: &mut Terms, x: Var, e: Tm, c: Tm) -> Tm {
        if !self.mentions(terms, c, &[x]) {
            return c;
        }
        if let Some(&s) = self.sunk.get(&(x, e, c)) {
            return s;
        }
        let ty = terms.ty(c).clone();
        let s = match terms.kind(c).clone() {
            Kind::Prim(p, args) => {
                let args = args
                    .into_iter()
                    .map(|a| self.sink_let(terms, x, e, a))
                    .collect();
                terms.add(Kind::Prim(p, args), ty)
            }
            _ => terms.add(Kind::Let(x, e, c), ty),
        };
        self.sunk.insert((x, e, c), s);
        s
    }

    /// Returns whether `t` is a scalar expression: it reads nothing and holds no comprehension.
    pub fn scalar(&mut self, terms: &Terms, t: Tm) -> bool {
        if let Some(&s) = self.scalar.get(&t) {
            return s;
        }
        let here = matches!(
            terms.kind(t),
            Kind::Var(_)
                | Kind::Const(_)
                | Kind::Prim(..)
                | Kind::Record(_)
                | Kind::Field(..)
                | Kind::Ctor { .. }
                | Kind::Case { .. }
                | Kind::If(..)
                | Kind::Let(..)
                | Kind::Caller
                | Kind::Key(_)
                | Kind::KeyOf(..)
                | Kind::Tag(_)
                | Kind::Try(_)
                | Kind::Singleton(_)
                | Kind::EmptySet
                | Kind::Union(..)
                | Kind::Restrict(..)
                | Kind::Reject(_)
                | Kind::Unreachable
                | Kind::Call { .. }
        );
        let s = here && terms.children(t).into_iter().all(|c| self.scalar(terms, c));
        self.scalar.insert(t, s);
        s
    }
}
