//! The reads of a constraint's query, each where it occurs.

use std::collections::HashMap;

use adtb_lang::kernel::{Kind, Tm, Var};
use adtb_lang::types::Ty;

use super::{
    At, Binder, BinderTm, Choices, Compiler, ForTm, Occurrence, Origin, Row, Source, Within,
};

impl Within {
    /// Returns what encloses an expression of the query's top level reached through `choices`.
    fn in_top(choices: &Choices) -> Within {
        Within {
            choices: choices.clone(),
            ..Within::default()
        }
    }
}

impl Compiler<'_> {
    /// Collect the reads of the query's top level `t`, reached through `choices`, recording each
    /// variable it binds.
    pub(super) fn top(
        &mut self,
        t: Tm,
        binders: &mut HashMap<Var, Binder>,
        choices: &mut Choices,
        out: &mut Vec<Occurrence>,
    ) {
        let bind = |binders: &mut HashMap<Var, Binder>, x: Var, node: BinderTm| {
            let order = binders.len();
            binders.insert(x, Binder { node, order });
        };
        match self.kind(t) {
            Kind::For(x, src, body) => {
                match self.source(src) {
                    Some(Source {
                        rel,
                        ctors,
                        origin,
                        elem,
                    }) => out.push(Occurrence {
                        rel,
                        ctors,
                        origin,
                        node: src,
                        elem,
                        at: At::Top(
                            ForTm {
                                node: t,
                                var: x,
                                body,
                            },
                            choices.clone(),
                        ),
                    }),
                    None => self.nested(src, &mut Within::in_top(choices), out),
                }
                bind(binders, x, BinderTm::For { node: t, src, body });
                self.top(body, binders, choices, out);
            }
            Kind::If(c, a, b) => {
                self.nested(c, &mut Within::in_top(choices), out);
                self.top_choice(t, 1, a, binders, choices, out);
                self.top_choice(t, 2, b, binders, choices, out);
            }
            Kind::Let(x, e, b) => {
                self.nested(e, &mut Within::in_top(choices), out);
                bind(
                    binders,
                    x,
                    BinderTm::Let {
                        node: t,
                        value: e,
                        body: b,
                    },
                );
                self.top(b, binders, choices, out);
            }
            Kind::Union(a, b) => {
                self.top_choice(t, 0, a, binders, choices, out);
                self.top_choice(t, 1, b, binders, choices, out);
            }
            Kind::Case { scrutinee, arms } => {
                self.nested(scrutinee, &mut Within::in_top(choices), out);
                for (i, (_, _, a)) in arms.iter().enumerate() {
                    self.top_choice(t, i + 1, *a, binders, choices, out);
                }
            }
            _ => self.nested(t, &mut Within::in_top(choices), out),
        }
    }

    /// Collect the reads of `a`, child `at` of the alternative `t` of the query's top level.
    fn top_choice(
        &mut self,
        t: Tm,
        at: usize,
        a: Tm,
        binders: &mut HashMap<Var, Binder>,
        choices: &mut Choices,
        out: &mut Vec<Occurrence>,
    ) {
        choices.push((t, at));
        self.top(a, binders, choices, out);
        choices.pop();
    }

    /// Collect the reads inside expression `t`, under what of the expression encloses it.
    fn nested(&mut self, t: Tm, within: &mut Within, out: &mut Vec<Occurrence>) {
        let here = |row: Row, within: &Within, guards: Vec<Tm>| At::Nested {
            row,
            within: Within {
                guards,
                ..within.clone()
            },
        };
        match self.kind(t) {
            Kind::For(y, src, body) => {
                let closed = match self.source(src) {
                    Some(Source {
                        rel,
                        ctors,
                        origin,
                        elem,
                    }) => {
                        let mut own = within.guards.clone();
                        own.extend(self.peel(body).0);
                        out.push(Occurrence {
                            rel,
                            ctors,
                            origin,
                            node: src,
                            elem,
                            at: here(Row::Gen(y), within, own),
                        });
                        Some(src)
                    }
                    None => {
                        self.nested(src, within, out);
                        None
                    }
                };
                within.path.push((y, closed));
                self.nested(body, within, out);
                within.path.pop();
            }
            Kind::If(c, a, b) => {
                self.nested(c, within, out);
                if matches!(self.terms.kind(b), Kind::EmptySet) || self.is_false(b) {
                    let n = within.guards.len();
                    self.conjuncts(c, &mut within.guards);
                    self.nested(a, within, out);
                    within.guards.truncate(n);
                } else {
                    self.nested(a, within, out);
                    self.nested(b, within, out);
                }
            }
            Kind::Case { scrutinee, arms } => {
                self.nested(scrutinee, within, out);
                for (i, (_, _, a)) in arms.iter().enumerate() {
                    within.arms.push((t, i));
                    self.nested(*a, within, out);
                    within.arms.pop();
                }
            }
            Kind::Deref { key, .. } => {
                if let Ty::Key { rel, ctors } = self.ty(key) {
                    let rel = self.schema.declared(rel);
                    let ctors = ctors.committed().cloned();
                    out.push(Occurrence {
                        rel,
                        ctors: self.ctors_or_all(rel, &ctors),
                        origin: Origin::State,
                        node: t,
                        elem: self.ty(t),
                        at: here(Row::Deref(key), within, within.guards.clone()),
                    });
                }
                self.nested(key, within, out);
            }
            Kind::Rel(..) | Kind::Restrict(..) | Kind::Var(_)
                if let Some(Source {
                    rel,
                    ctors,
                    origin,
                    elem,
                }) = self.source(t) =>
            {
                out.push(Occurrence {
                    rel,
                    ctors,
                    origin,
                    node: t,
                    elem,
                    at: here(Row::Bare, within, within.guards.clone()),
                });
            }
            _ => {
                for c in self.terms.children(t) {
                    self.nested(c, within, out);
                }
            }
        }
    }
}
