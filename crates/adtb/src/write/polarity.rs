use std::collections::{BTreeSet, HashMap};

use adtb_lang::kernel::{Kind, Prim, Terms, Tm, Transition, Var};
use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use super::delta::scope_name;

/// The positions a read can sit in, relative to the query's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Place {
    /// Adding rows to the read can add rows to the result.
    up: bool,
    /// Adding rows to the read can remove rows from the result.
    down: bool,
    aggregated: bool,
}

impl Place {
    fn negated(self) -> Place {
        Place {
            up: self.down,
            down: self.up,
            ..self
        }
    }

    fn both(self) -> Place {
        let any = self.up || self.down;
        Place {
            up: any,
            down: any,
            ..self
        }
    }

    fn in_aggregate(self) -> Place {
        Place {
            aggregated: true,
            ..self
        }
    }
}

/// Returns, for each read of `inserted` or `deleted` in `t` that sits under negation or inside an
/// aggregate, why it is outside the bound.
pub fn non_monotone(schema: &Schema, terms: &Terms, t: Tm) -> Vec<String> {
    let mut walk = Walk {
        schema,
        terms,
        seen: BTreeSet::new(),
        bound: HashMap::new(),
        out: Vec::new(),
    };
    let top = Place {
        up: true,
        down: false,
        aggregated: false,
    };
    walk.visit(t, top);
    walk.out
}

struct Walk<'a> {
    schema: &'a Schema,
    terms: &'a Terms,
    seen: BTreeSet<(Tm, Place)>,
    /// The bound expression of each `let` variable.
    bound: HashMap<Var, Tm>,
    out: Vec<String>,
}

impl Walk<'_> {
    fn visit(&mut self, t: Tm, at: Place) {
        if !self.seen.insert((t, at)) {
            return;
        }
        match self.terms.kind(t).clone() {
            Kind::Transition(x, r, s) => {
                let which = match x {
                    Transition::Inserted => "inserted",
                    Transition::Deleted => "deleted",
                    Transition::Prior => return,
                };
                let ctors = s.unwrap_or_else(|| {
                    let n = self
                        .schema
                        .ty(self.schema.relation(r).elem_def)
                        .ctors()
                        .len() as u32;
                    (0..n).collect()
                });
                let place = if at.aggregated {
                    "inside an aggregate"
                } else if at.down {
                    "under negation"
                } else {
                    return;
                };
                let why = format!(
                    "`{which}({})` is read {place}",
                    scope_name(self.schema, r, &ctors)
                );
                if !self.out.contains(&why) {
                    self.out.push(why);
                }
            }
            Kind::Var(v) => {
                if let Some(e) = self.bound.get(&v).copied() {
                    self.visit(e, at);
                }
            }
            Kind::Let(x, e, b) => {
                self.bound.insert(x, e);
                self.visit(b, at);
            }
            Kind::If(c, a, b) => {
                let cond = match (self.truth(a), self.truth(b)) {
                    (_, Some(false)) | (Some(true), _) => at,
                    (Some(false), _) | (_, Some(true)) => at.negated(),
                    _ => at.both(),
                };
                self.visit(c, cond);
                self.visit(a, at);
                self.visit(b, at);
            }
            Kind::IsEmpty(s) => self.visit(s, at.negated()),
            Kind::Prim(Prim::Eq | Prim::Ne, args) => {
                for a in args {
                    self.visit(a, at.both());
                }
            }
            Kind::Case { scrutinee, arms } => {
                self.visit(scrutinee, at.both());
                for (_, _, a) in arms {
                    self.visit(a, at);
                }
            }
            Kind::Agg { .. } | Kind::Group { .. } | Kind::Top { .. } => {
                for c in self.terms.children(t) {
                    self.visit(c, at.in_aggregate());
                }
            }
            _ => {
                for c in self.terms.children(t) {
                    self.visit(c, at);
                }
            }
        }
    }

    /// Returns the truth `t` always has when it is a constant: `false` for the empty set.
    fn truth(&self, t: Tm) -> Option<bool> {
        let bool_def = self.schema.prelude_type("Bool");
        match self.terms.kind(t) {
            Kind::EmptySet => Some(false),
            Kind::Ctor { ctor, fields }
                if fields.is_empty()
                    && matches!(self.terms.ty(t), Ty::Data { def, .. } if *def == bool_def) =>
            {
                Some(Some(*ctor) == self.schema.ty(bool_def).ctor_index("True"))
            }
            _ => None,
        }
    }
}
