use std::collections::HashMap;

use crate::kernel::{Agg, Kind, Terms, Tm, Var};

/// Returns an error when the `sum` or `avg` at `t` ranges over a comprehension whose `select` may merge equal values of distinct bindings.
pub fn ranges_over_bindings(terms: &Terms, t: Tm) -> Result<(), String> {
    let Kind::Agg { op, source, .. } = terms.kind(t) else {
        return Ok(());
    };
    let name = match op {
        Agg::Sum => "sum",
        Agg::Avg => "avg",
        _ => return Ok(()),
    };
    let mut k = Keyed {
        terms,
        chain: Vec::new(),
        alias: HashMap::new(),
    };
    if k.merges(*source) {
        return Err(format!(
            "`{name}` ranges over rows, not over values a `select` may have merged; range over the rows and aggregate the field"
        ));
    }
    Ok(())
}

struct Keyed<'t> {
    terms: &'t Terms,
    /// The generators enclosing the current position, each with whether its source has distinct elements.
    chain: Vec<(Var, bool)>,
    /// Arm variables standing for the generator variable their `case` scrutinises.
    alias: HashMap<Var, Var>,
}

impl Keyed<'_> {
    /// Returns whether `t` is a comprehension whose `select` is neither key-preserving nor the record of its bindings.
    fn merges(&mut self, t: Tm) -> bool {
        match self.terms.kind(t) {
            Kind::For(..) => !self.selects(t, true),
            Kind::Union(a, b) | Kind::If(_, a, b) => self.merges(*a) || self.merges(*b),
            Kind::Case { arms, .. } => arms.iter().any(|(_, _, e)| self.merges(*e)),
            Kind::Let(_, _, b) => self.merges(*b),
            Kind::Restrict(x, _) | Kind::Top { source: x, .. } | Kind::At { body: x, .. } => {
                self.merges(*x)
            }
            _ => false,
        }
    }

    /// Returns whether no two distinct elements behind `t` were merged into one by a `select`.
    fn distinct(&mut self, t: Tm) -> bool {
        match self.terms.kind(t) {
            Kind::Restrict(x, _) | Kind::Top { source: x, .. } | Kind::At { body: x, .. } => {
                self.distinct(*x)
            }
            Kind::Union(a, b) | Kind::If(_, a, b) => self.distinct(*a) && self.distinct(*b),
            Kind::Case { arms, .. } => arms.iter().all(|(_, _, e)| self.distinct(*e)),
            Kind::Let(_, _, b) => self.distinct(*b),
            Kind::For(..) => {
                let saved = std::mem::take(&mut self.chain);
                let r = self.selects(t, false);
                self.chain = saved;
                r
            }
            _ => true,
        }
    }

    /// Returns whether every element `t` yields is the variable of a generator over distinct elements, or, when `bindings`, the record of every generator's variable.
    fn selects(&mut self, t: Tm, bindings: bool) -> bool {
        match self.terms.kind(t) {
            Kind::For(y, s, b) => {
                let keyed = self.distinct(*s);
                self.chain.push((*y, keyed));
                let r = self.selects(*b, bindings);
                self.chain.pop();
                r
            }
            Kind::If(_, a, b) | Kind::Union(a, b) => {
                self.selects(*a, bindings) && self.selects(*b, bindings)
            }
            Kind::Case { scrutinee, arms } => {
                let s = match self.terms.kind(*scrutinee) {
                    Kind::Var(v) => Some(self.resolve(*v)),
                    _ => None,
                };
                arms.iter().all(|(_, v, e)| {
                    if let Some(s) = s {
                        self.alias.insert(*v, s);
                    }
                    self.selects(*e, bindings)
                })
            }
            Kind::Let(_, _, b) => self.selects(*b, bindings),
            Kind::EmptySet => true,
            Kind::Singleton(e) => match self.terms.kind(*e) {
                Kind::Var(v) => {
                    let v = self.resolve(*v);
                    self.chain.iter().any(|(x, keyed)| *x == v && *keyed)
                }
                Kind::Record(fs) if bindings => {
                    let vars: Vec<Var> = fs
                        .iter()
                        .filter_map(|(_, f)| match self.terms.kind(*f) {
                            Kind::Var(v) => Some(self.resolve(*v)),
                            _ => None,
                        })
                        .collect();
                    !self.chain.is_empty() && self.chain.iter().all(|(x, _)| vars.contains(x))
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn resolve(&self, mut v: Var) -> Var {
        while let Some(s) = self.alias.get(&v) {
            v = *s;
        }
        v
    }
}
