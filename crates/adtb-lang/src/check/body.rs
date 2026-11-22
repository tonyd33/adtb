use super::*;
use crate::norm::Root;

impl Program {
    /// Returns every body of the program, by the object it belongs to.
    pub fn roots(&self) -> Vec<ProgramBody<'_>> {
        let Program {
            terms: _,
            functions,
            constraints,
            checks,
            domains,
            retags,
            defaults,
            computed,
            index_fields,
            aggregates,
            fn_rows: _,
        } = self;
        let mut out = Vec::new();
        for (i, f) in functions.iter().enumerate() {
            out.extend(f.map(|t| ProgramBody::Function(FnId(i as u32), t)));
        }
        out.extend(
            constraints
                .iter()
                .map(|(&c, &t)| ProgramBody::Constraint(c, t)),
        );
        out.extend(checks.iter().map(|(&c, &t)| ProgramBody::Check(c, t)));
        out.extend(
            domains
                .iter()
                .map(|(&d, &(v, t))| ProgramBody::Domain(d, v, t)),
        );
        out.extend(retags.iter().map(|(&r, &t)| ProgramBody::Retag(r, t)));
        out.extend(
            defaults
                .iter()
                .map(|(&(r, i), &t)| ProgramBody::Default(r, i, t)),
        );
        out.extend(
            computed
                .iter()
                .map(|(&(r, i), &(v, t))| ProgramBody::Computed(r, i, v, t)),
        );
        out.extend(
            index_fields
                .iter()
                .map(|(&(r, i, j), f)| ProgramBody::IndexField(r, i, j, f)),
        );
        for (i, a) in aggregates.iter().enumerate() {
            out.extend(a.as_ref().map(|a| ProgramBody::Aggregate(i, a)));
        }
        out
    }

    /// Returns every term of every body of the program, by the object it belongs to, with the
    /// variables free in it; the parts of an aggregate under one root.
    pub fn bodies(&self) -> Vec<(Root, Tm, Vec<Var>)> {
        self.roots()
            .into_iter()
            .flat_map(|b| {
                let root = b.root();
                b.terms().into_iter().map(move |(t, vs)| (root, t, vs))
            })
            .collect()
    }

    /// Returns every term of every body of the program with the variables free in it.
    pub fn body_terms(&self) -> Vec<(Tm, Vec<Var>)> {
        self.bodies()
            .into_iter()
            .map(|(_, t, vs)| (t, vs))
            .collect()
    }
}

/// A body of a checked program, by the object it belongs to.
#[derive(Clone, Copy, Debug)]
pub enum ProgramBody<'a> {
    /// A function's term, a `Lam` over its parameters.
    Function(FnId, Tm),
    Constraint(ConstraintIdx, Tm),
    Check(CheckIdx, Tm),
    /// A newtype's domain predicate over its `self` variable.
    Domain(TypeId, Var, Tm),
    /// A relation's `on delete retag` new value, a `Lam` over the referrer's row.
    Retag(RelId, Tm),
    /// A default expression, by relation and position among its defaults.
    Default(RelId, usize, Tm),
    /// A computed field's expression over its `self` variable, by relation and position.
    Computed(RelId, usize, Var, Tm),
    /// An index field, by relation, index position and field position.
    IndexField(RelId, usize, usize, &'a IndexField),
    /// A user-defined aggregate's parts, by position among `Schema::aggregates`.
    Aggregate(usize, &'a AggregateTerms),
}

impl ProgramBody<'_> {
    pub fn root(&self) -> Root {
        match *self {
            ProgramBody::Function(f, _) => Root::Function(f),
            ProgramBody::Constraint(c, _) => Root::Constraint(c),
            ProgramBody::Check(c, _) => Root::Check(c),
            ProgramBody::Domain(d, ..) => Root::Domain(d),
            ProgramBody::Retag(r, _) => Root::Retag(r),
            ProgramBody::Default(r, i, _) => Root::Default(r, i),
            ProgramBody::Computed(r, i, ..) => Root::Computed(r, i),
            ProgramBody::IndexField(r, i, j, _) => Root::IndexField(r, i, j),
            ProgramBody::Aggregate(i, _) => Root::Aggregate(i),
        }
    }

    /// Returns each term of the body with the variables free in it.
    pub fn terms(&self) -> Vec<(Tm, Vec<Var>)> {
        match *self {
            ProgramBody::Function(_, t)
            | ProgramBody::Constraint(_, t)
            | ProgramBody::Check(_, t)
            | ProgramBody::Retag(_, t)
            | ProgramBody::Default(_, _, t) => vec![(t, vec![])],
            ProgramBody::Domain(_, v, t) | ProgramBody::Computed(_, _, v, t) => vec![(t, vec![v])],
            ProgramBody::IndexField(.., f) => vec![(f.value, vec![])],
            ProgramBody::Aggregate(_, a) => [a.init, a.step, a.merge, a.finish]
                .map(|t| (t, vec![]))
                .to_vec(),
        }
    }
}
