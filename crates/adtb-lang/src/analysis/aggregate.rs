use std::collections::HashMap;

use crate::kernel::{Agg, Kind, Search, Terms, Tm, Var};

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
        selected: HashMap::new(),
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
    /// The generators enclosing the current position.
    chain: Chain,
    /// Arm variables standing for the generator variable their `case` scrutinises.
    alias: HashMap<Var, Var>,
    /// The result of [`Keyed::selects`] at each term, flag and chain it was asked at.
    selected: HashMap<(Tm, bool, Chain), bool>,
}

/// Generators, each with whether its source has distinct elements.
type Chain = Vec<(Var, bool)>;

impl Keyed<'_> {
    /// Returns whether `t` is a comprehension whose `select` is neither key-preserving nor the record of its bindings.
    fn merges(&mut self, t: Tm) -> bool {
        let terms = self.terms;
        terms.search(t, |t| match terms.kind(t) {
            Kind::For(..) if !self.selects(t, true) => Search::Found,
            kind => Search::Into(sources(kind)),
        })
    }

    /// Returns whether no two distinct elements behind `t` were merged into one by a `select`.
    fn distinct(&mut self, t: Tm) -> bool {
        let terms = self.terms;
        !terms.search(t, |t| match terms.kind(t) {
            Kind::For(..) => {
                let saved = std::mem::take(&mut self.chain);
                let r = self.selects(t, false);
                self.chain = saved;
                if r {
                    Search::Into(Vec::new())
                } else {
                    Search::Found
                }
            }
            kind => Search::Into(sources(kind)),
        })
    }

    /// Returns whether every element `t` yields is the variable of a generator over distinct elements, or, when `bindings`, the record of every generator's variable.
    fn selects(&mut self, t: Tm, bindings: bool) -> bool {
        let key = (t, bindings, self.chain.clone());
        if let Some(&r) = self.selected.get(&key) {
            return r;
        }
        let r = self.select(t, bindings);
        self.selected.insert(key, r);
        r
    }

    fn select(&mut self, t: Tm, bindings: bool) -> bool {
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

/// Returns the sets whose elements a set of kind `kind` holds, outside a comprehension.
fn sources(kind: &Kind) -> Vec<Tm> {
    match kind {
        Kind::Union(a, b) | Kind::If(_, a, b) => vec![*a, *b],
        Kind::Case { arms, .. } => arms.iter().map(|(_, _, e)| *e).collect(),
        Kind::Let(_, _, b)
        | Kind::Restrict(b, _)
        | Kind::Top { source: b, .. }
        | Kind::At { body: b, .. } => vec![*b],
        _ => Vec::new(),
    }
}
