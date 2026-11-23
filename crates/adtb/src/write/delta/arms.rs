//! A generator over a sum read through `match` as the union of its arm queries.

use adtb_lang::kernel::{Build, Kind, Tm, Var};
use adtb_lang::types::Ty;

use super::Compiler;

impl Compiler<'_> {
    /// Returns `q` with each generator over several constructors, whose row a `match` reads
    /// outside any aggregate, group, `top` or fixpoint, replaced by the union of one generator
    /// per constructor, each `match` on the row replaced by that constructor's arm.
    pub(super) fn arm_queries(&mut self, q: Tm) -> Tm {
        let schema = self.schema;
        let mut split = Vec::new();
        self.splittable(q, &mut split);
        if split.is_empty() {
            return q;
        }
        self.terms.substitute(schema, q, |terms, t, new| {
            let Kind::For(x, _, _) = terms.kind(t).clone() else {
                return None;
            };
            let (_, ctors, elem) = split.iter().find(|(v, _, _)| *v == x)?.clone();
            let [src, body] = new[..] else { return None };
            let ty = terms.ty(t).clone();
            let mut out: Option<Tm> = None;
            for k in ctors {
                let narrow = match &elem {
                    Ty::Data { def, args, .. } => Ty::Data {
                        def: *def,
                        args: args.clone(),
                        ctors: Some(vec![k]),
                    },
                    _ => return None,
                };
                let xk = terms.var(&format!("%arm{k}"), narrow);
                let arm = arm_of(schema, terms, body, x, k, xk);
                let mut b = Build::new(schema, terms);
                let src = b.restrict(src, vec![k]);
                let one = b.for_(xk, src, arm, ty.clone());
                out = Some(match out {
                    None => one,
                    Some(o) => b.union(o, one, ty.clone()),
                });
            }
            out
        })
    }

    /// Add to `out`, with its constructors and row type, each generator under `t` over several
    /// constructors whose row a `match` reads, each `match` on it having an arm for every one of
    /// them, and whose row nothing reads inside an aggregate, group, `top` or fixpoint.
    fn splittable(&self, t: Tm, out: &mut Vec<(Var, Vec<u32>, Ty)>) {
        if let Kind::For(x, src, body) = self.kind(t)
            && let Some(s) = self.source(src)
            && s.ctors.len() > 1
        {
            let mut cases = 0;
            if read_by_arms(self.terms, body, x, &s.ctors, false, &mut cases) && cases > 0 {
                out.push((x, s.ctors.clone(), s.elem.clone()));
            }
        }
        for c in self.terms.children(t) {
            self.splittable(c, out);
        }
    }
}

/// Returns whether every `match` on `x` under `t` has an arm for each of `ctors` and lies outside
/// any aggregate, group, `top` or fixpoint, counting them in `cases`.
fn read_by_arms(
    terms: &adtb_lang::kernel::Terms,
    t: Tm,
    x: Var,
    ctors: &[u32],
    ranged: bool,
    cases: &mut usize,
) -> bool {
    let kind = terms.kind(t);
    let ranged = ranged
        || matches!(
            kind,
            Kind::Agg { .. } | Kind::Group { .. } | Kind::Top { .. } | Kind::Fix(..)
        );
    if let Kind::Case { scrutinee, arms } = kind
        && matches!(terms.kind(*scrutinee), Kind::Var(v) if *v == x)
    {
        if ranged || !ctors.iter().all(|k| arms.iter().any(|(c, _, _)| c == k)) {
            return false;
        }
        *cases += 1;
    }
    if ranged && matches!(kind, Kind::Var(v) if *v == x) {
        return false;
    }
    terms
        .children(t)
        .into_iter()
        .all(|c| read_by_arms(terms, c, x, ctors, ranged, cases))
}

/// Returns `body` with `x` and the variable of each arm of constructor `k` of a `match` on `x`
/// read as `xk`, and each such `match` replaced by its arm of `k`.
fn arm_of(
    schema: &adtb_lang::schema::Committed,
    terms: &mut adtb_lang::kernel::Terms,
    body: Tm,
    x: Var,
    k: u32,
    xk: Var,
) -> Tm {
    let mut arm_vars = vec![x];
    collect_arm_vars(terms, body, x, k, &mut arm_vars);
    terms.substitute(schema, body, |terms, t, new| match terms.kind(t).clone() {
        Kind::Var(v) if arm_vars.contains(&v) => Some(Build::new(schema, terms).var(xk)),
        Kind::Case { scrutinee, arms }
            if matches!(terms.kind(scrutinee), Kind::Var(v) if *v == x) =>
        {
            let i = arms.iter().position(|(c, _, _)| *c == k)?;
            Some(new[1 + i])
        }
        _ => None,
    })
}

fn collect_arm_vars(terms: &adtb_lang::kernel::Terms, t: Tm, x: Var, k: u32, out: &mut Vec<Var>) {
    if let Kind::Case { scrutinee, arms } = terms.kind(t)
        && matches!(terms.kind(*scrutinee), Kind::Var(v) if *v == x)
    {
        out.extend(arms.iter().filter(|(c, _, _)| *c == k).map(|(_, v, _)| *v));
    }
    for c in terms.children(t) {
        collect_arm_vars(terms, c, x, k, out);
    }
}
