use super::*;
use crate::ast::{BinOp, Clause, Comprehension, Expr, ExprKind, Pattern, PatternKind, Payload};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowOrigin {
    Empty,
    Relation(RelId, Provenance),
    Unknown(Provenance),
}

impl RowOrigin {
    pub(super) const BUILT: Self = Self::Unknown(Provenance::Built);

    pub(super) fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Empty, x) | (x, Self::Empty) => x,
            (Self::Relation(a, p), Self::Relation(b, q)) if a == b => Self::Relation(a, p.join(q)),
            (x, y) => Self::Unknown(x.provenance().join(y.provenance())),
        }
    }

    pub(super) fn relation(self) -> Option<RelId> {
        match self {
            Self::Relation(r, _) => Some(r),
            Self::Empty | Self::Unknown(_) => None,
        }
    }

    pub(super) fn provenance(self) -> Provenance {
        match self {
            Self::Relation(_, p) | Self::Unknown(p) => p,
            Self::Empty => Provenance::Built,
        }
    }
}

impl Checker<'_> {
    pub(super) fn row_origin(&self, out: &Out) -> RowOrigin {
        match out.rows {
            Some(r) => RowOrigin::Relation(r, out.provenance),
            None if self.empty_row_term(out.tm) => RowOrigin::Empty,
            None => RowOrigin::Unknown(out.provenance),
        }
    }

    pub(super) fn empty_row_term(&self, tm: Tm) -> bool {
        self.empty_row_terms.contains(&tm) || matches!(self.terms.kind(tm), Kind::EmptySet)
    }

    pub(super) fn with_row_origin(&mut self, mut out: Out, origin: RowOrigin) -> Out {
        out.rows = origin.relation();
        out.provenance = origin.provenance();
        if origin == RowOrigin::Empty {
            self.empty_row_terms.insert(out.tm);
        }
        out
    }

    pub(super) fn bind_origin(&mut self, name: &str, ty: Ty, origin: RowOrigin) -> Var {
        let v = self.bind(name, ty, origin.relation(), origin.provenance());
        if origin == RowOrigin::Empty {
            self.empty_row_vars.insert(v);
        }
        v
    }

    pub(super) fn fix_origins(
        &mut self,
        binds: &[(ast::Ident, Expr)],
        outer: &BTreeMap<String, RowOrigin>,
    ) -> R<Vec<RowOrigin>> {
        let mut origins = vec![RowOrigin::Empty; binds.len()];
        loop {
            let mut env = outer.clone();
            for ((name, _), origin) in binds.iter().zip(&origins) {
                env.insert(name.text.clone(), *origin);
            }
            let next = binds
                .iter()
                .zip(&origins)
                .map(|((_, e), old)| self.result_origin(e, &env).map(|new| old.join(new)))
                .collect::<R<Vec<_>>>()?;
            if next == origins {
                return Ok(origins);
            }
            origins = next;
        }
    }

    fn result_origin(&mut self, e: &Expr, env: &BTreeMap<String, RowOrigin>) -> R<RowOrigin> {
        match &e.kind {
            ExprKind::Name(q) => {
                if q.module.is_none()
                    && let Some(origin) = env.get(&q.name.text)
                {
                    return Ok(*origin);
                }
                self.synth(e).map(|o| self.row_origin(&o))
            }
            ExprKind::Set(es) if es.is_empty() => Ok(RowOrigin::Empty),
            ExprKind::Set(es) => {
                let mut provenance = None;
                for e in es {
                    let p = self.result_origin(e, env)?.provenance();
                    provenance = Some(provenance.map_or(p, |q: Provenance| q.join(p)));
                }
                Ok(RowOrigin::Unknown(provenance.unwrap_or(Provenance::Built)))
            }
            ExprKind::Binary(BinOp::Union, a, b) | ExprKind::If(_, a, b) => {
                let a = self.result_origin(a, env)?;
                let b = self.result_origin(b, env)?;
                Ok(a.join(b))
            }
            ExprKind::Select(source, _) => self.result_origin(source, env),
            ExprKind::Comprehension(c) | ExprKind::Top { body: c, .. } => {
                self.projection_origin(c, env)
            }
            ExprKind::Fix(binds, body) => {
                let origins = self.fix_origins(binds, env)?;
                let mut env = env.clone();
                for ((name, _), origin) in binds.iter().zip(origins) {
                    env.insert(name.text.clone(), origin);
                }
                self.result_origin(body, &env)
            }
            ExprKind::Match(_, _) if !depends(e, env) => self.synth(e).map(|o| self.row_origin(&o)),
            ExprKind::Match(Some(source), arms) => {
                let source = self.result_origin(source, env)?;
                let mut result = RowOrigin::Empty;
                for arm in arms {
                    let mut env = env.clone();
                    for p in &arm.patterns {
                        pattern_origins(p, source, &mut env);
                    }
                    result = result.join(self.result_origin(&arm.body, &env)?);
                }
                Ok(result)
            }
            ExprKind::Call { callee, .. } if depends(e, env) => {
                if callee.module.is_none()
                    && (env.contains_key(&callee.name.text)
                        || self.scope.lookup(&callee.name.text).is_some())
                {
                    return Ok(RowOrigin::BUILT);
                }
                Ok(match self.lookup_value(callee) {
                    Ok(ValueDef::Function(f)) => self.fn_rows[f.0 as usize]
                        .map_or(RowOrigin::BUILT, |r| {
                            RowOrigin::Relation(r, Provenance::Snapshot)
                        }),
                    _ => RowOrigin::BUILT,
                })
            }
            ExprKind::Call { .. }
            | ExprKind::Field(..)
            | ExprKind::Backlink(..)
            | ExprKind::At { .. } => {
                if depends(e, env) {
                    Ok(RowOrigin::BUILT)
                } else {
                    self.synth(e).map(|o| self.row_origin(&o))
                }
            }
            ExprKind::Lit(_)
            | ExprKind::TypeCall { .. }
            | ExprKind::Ctor { .. }
            | ExprKind::Record(_)
            | ExprKind::Unit
            | ExprKind::Unary(..)
            | ExprKind::Binary(..)
            | ExprKind::KeyCall(..)
            | ExprKind::Match(None, _)
            | ExprKind::Lambda(..)
            | ExprKind::Exists(..)
            | ExprKind::Reject(_)
            | ExprKind::Write(_)
            | ExprKind::Elided => Ok(RowOrigin::BUILT),
        }
    }

    fn projection_origin(
        &mut self,
        c: &Comprehension,
        outer: &BTreeMap<String, RowOrigin>,
    ) -> R<RowOrigin> {
        if c.group.is_some() {
            return Ok(RowOrigin::BUILT);
        }
        let mut env = outer.clone();
        let mut binders = BTreeSet::new();
        let mut empty = false;
        for clause in &c.clauses {
            if let Clause::From(gens) = clause {
                for g in gens {
                    let origin = self.result_origin(&g.source, &env)?;
                    empty |= origin == RowOrigin::Empty;
                    env.insert(g.binder.text.clone(), origin);
                    binders.insert(g.binder.text.clone());
                }
            }
        }
        if empty {
            return Ok(RowOrigin::Empty);
        }
        match &c.select.kind {
            ExprKind::Name(q) if q.module.is_none() && binders.contains(&q.name.text) => {
                Ok(env[&q.name.text])
            }
            _ => Ok(RowOrigin::BUILT),
        }
    }
}

fn depends(e: &Expr, env: &BTreeMap<String, RowOrigin>) -> bool {
    let mut found = false;
    crate::visit::each_expr(e, &mut |x| {
        if let ExprKind::Name(q) = &x.kind
            && q.module.is_none()
            && env.contains_key(&q.name.text)
        {
            found = true;
        }
    });
    found
}

fn pattern_origins(p: &Pattern, origin: RowOrigin, env: &mut BTreeMap<String, RowOrigin>) {
    match &p.kind {
        PatternKind::Bind(name) => {
            env.insert(name.text.clone(), origin);
        }
        PatternKind::Ctor { payload, .. } => match payload {
            Some(Payload::Bind(name)) => {
                env.insert(name.text.clone(), origin);
            }
            Some(Payload::Record(fields)) => {
                for field in fields {
                    if let Some(p) = &field.pattern {
                        pattern_origins(p, RowOrigin::BUILT, env);
                    } else {
                        env.insert(field.field.text.clone(), RowOrigin::BUILT);
                    }
                }
            }
            Some(Payload::Wildcard) | None => {}
        },
        PatternKind::Wildcard | PatternKind::Lit { .. } => {}
    }
}
