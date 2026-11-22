use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use super::{Agg, Const, Kind, Prim, Terms, Tm, Var};
use crate::ast::FnBody;
use crate::check::prim_ty::{Bounds, prim_ty};
use crate::check::rel::key_value_ty;
use crate::check::ty::{ctor_fields, ctor_names, elem, fields, restrict, subtype};
use crate::schema::{Generator, Schema};
use crate::types::{Base, CtorSet, FnId, KeyCtors, RelId, RelRef, Ty, TypeId};

/// A node that breaks the kernel's typing invariants, with the invariant it breaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid {
    at: Tm,
    reason: String,
}

impl Invalid {
    pub(super) fn new(at: Tm, reason: String) -> Invalid {
        Invalid { at, reason }
    }

    pub fn at(&self) -> Tm {
        self.at
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

type Checked = Result<(), String>;

/// An arena whose terms reachable from the roots it was validated at hold the kernel's typing
/// invariants and are closed over their roots' parameters.
#[derive(Clone, Copy, Debug)]
pub struct Valid<'a> {
    terms: &'a Terms,
}

impl<'a> Valid<'a> {
    /// Returns `terms` as valid.
    ///
    /// Preconditions: [`validate`] accepted `terms` at roots that reach every term planned or
    /// compiled from it, and nothing was added to `terms` since.
    pub(crate) fn trusted(terms: &'a Terms) -> Valid<'a> {
        Valid { terms }
    }

    pub fn terms(self) -> &'a Terms {
        self.terms
    }
}

/// An arena that [`validate`] accepted, owned.
#[derive(Clone, Debug)]
pub struct ValidTerms {
    terms: Terms,
}

impl ValidTerms {
    /// Returns `terms`, refused as [`validate`] refuses it at `roots`.
    pub fn new(schema: &Schema, terms: Terms, roots: &[(Tm, Vec<Var>)]) -> Result<Self, Invalid> {
        validate(schema, &terms, roots)?;
        Ok(ValidTerms { terms })
    }

    /// Returns an arena holding only `()`, and that term.
    pub fn unit(schema: &Schema) -> (ValidTerms, Tm) {
        let mut terms = Terms::default();
        let unit = super::Build::new(schema, &mut terms).unit();
        (ValidTerms { terms }, unit)
    }

    pub fn valid(&self) -> Valid<'_> {
        Valid::trusted(&self.terms)
    }
}

impl std::ops::Deref for ValidTerms {
    type Target = Terms;

    fn deref(&self) -> &Terms {
        &self.terms
    }
}

/// Check every term reachable from `roots` against the typing invariants of its kind, and that
/// every variable it reads is bound by an enclosing term or is one of the root's parameters.
/// Fail at the node a builder recorded as broken, when `terms` records one. A type parameter
/// has every bound.
pub fn validate<'a>(
    schema: &Schema,
    terms: &'a Terms,
    roots: &[(Tm, Vec<Var>)],
) -> Result<Valid<'a>, Invalid> {
    validate_bounded(schema, terms, roots, Bounds::Unknown)
}

/// Check `roots` as [`validate`] does, a type parameter having the bounds `bounds` gives it.
pub(crate) fn validate_bounded<'a>(
    schema: &Schema,
    terms: &'a Terms,
    roots: &[(Tm, Vec<Var>)],
    bounds: Bounds<'_>,
) -> Result<Valid<'a>, Invalid> {
    if let Some(e) = terms.broken() {
        return Err(e.clone());
    }
    typed(schema, terms, bounds, roots.iter().map(|(t, _)| *t))?;
    let mut frees = HashMap::new();
    for (root, params) in roots {
        if let Some(x) = free(terms, *root, &mut frees)
            .iter()
            .find(|x| !params.contains(x))
        {
            return Err(Invalid {
                at: *root,
                reason: format!(
                    "`{}` is read where nothing binds it",
                    terms.var_info(*x).name
                ),
            });
        }
    }
    Ok(Valid { terms })
}

/// Check that every term reachable from `roots` is in normal form: no `Lam` but the new-value
/// function of a `Replace`, no `App`, no `Call` but of a Wasm or host function, and no `Only`.
#[deny(
    clippy::wildcard_enum_match_arm,
    clippy::match_wildcard_for_single_variants
)]
pub fn validate_normal(
    schema: &Schema,
    terms: &Terms,
    roots: &[(Tm, Vec<Var>)],
) -> Result<(), Invalid> {
    let mut replacing = HashSet::new();
    let mut seen = HashSet::new();
    let mut todo: Vec<Tm> = roots.iter().map(|(t, _)| *t).collect();
    while let Some(t) = todo.pop() {
        if !seen.insert(t) {
            continue;
        }
        let refused = match terms.kind(t) {
            Kind::Replace(_, _, f) => {
                replacing.insert(*f);
                (!matches!(terms.kind(*f), Kind::Lam(..)))
                    .then_some("the new value of a `Replace` is not a `Lam`")
            }
            Kind::Lam(..) => (!replacing.contains(&t)).then_some("a `Lam` remains"),
            Kind::App(..) => Some("an `App` remains"),
            Kind::Call { func, .. } => matches!(schema.function(*func).body, FnBody::Expr(_))
                .then_some("a call of a schema function remains"),
            Kind::Only(_) => Some("an `Only` remains"),
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
            | Kind::Deref { .. }
            | Kind::Tag(_)
            | Kind::Try(_)
            | Kind::Rel(..)
            | Kind::Transition(..)
            | Kind::Restrict(..)
            | Kind::EmptySet
            | Kind::Singleton(_)
            | Kind::Union(..)
            | Kind::For(..)
            | Kind::IsEmpty(_)
            | Kind::Exists(_)
            | Kind::Agg { .. }
            | Kind::Group { .. }
            | Kind::Top { .. }
            | Kind::Fix(..)
            | Kind::Fresh(..)
            | Kind::Placeholder { .. }
            | Kind::At { .. }
            | Kind::Present { .. }
            | Kind::Reject(_)
            | Kind::Unreachable
            | Kind::Skip
            | Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Par(_)
            | Kind::Fail(_)
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => None,
        };
        if let Some(reason) = refused {
            return Err(Invalid {
                at: t,
                reason: reason.to_string(),
            });
        }
        todo.extend(terms.children(t));
    }
    Ok(())
}

/// The body the roots of [`validate_placed`] are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    /// A constraint's, where a `Transition` may be read.
    Constraint,
    /// A body whose types may name its own type parameters, those below the count.
    Generic(u32),
    Plain,
}

/// Check that every term reachable from `roots` sits where its kind may: a `Transition` only in
/// a constraint's body, a `Present` only as the final result of a root, and no type naming a
/// type parameter the body does not have.
pub fn validate_placed(terms: &Terms, roots: &[Tm], at: Body) -> Result<(), Invalid> {
    let mut seen = HashSet::new();
    let mut todo: Vec<(Tm, bool)> = roots.iter().map(|t| (*t, true)).collect();
    while let Some((t, last)) = todo.pop() {
        if !seen.insert((t, last)) {
            continue;
        }
        let refused = match terms.kind(t) {
            Kind::Transition(..) if at != Body::Constraint => {
                Some("a `Transition` outside the body of a constraint")
            }
            Kind::Present { .. } if !last => Some("a `Present` nested in another term"),
            _ if terms.ty(t).params_named() > at.params() => {
                Some("a type names a type parameter its body does not have")
            }
            _ => None,
        };
        if let Some(reason) = refused {
            return Err(Invalid {
                at: t,
                reason: reason.to_string(),
            });
        }
        let tails = tails(terms.kind(t));
        todo.extend(
            terms
                .children(t)
                .into_iter()
                .map(|c| (c, last && tails.contains(&c))),
        );
    }
    Ok(())
}

impl Body {
    fn params(self) -> u32 {
        match self {
            Body::Generic(n) => n,
            Body::Constraint | Body::Plain => 0,
        }
    }
}

/// Returns the children of a node of `kind` whose value is its own.
fn tails(kind: &Kind) -> Vec<Tm> {
    match kind {
        Kind::Let(_, _, b) | Kind::Transact(b, _) | Kind::At { body: b, .. } => vec![*b],
        Kind::Block(_, k) | Kind::LetBang(_, _, k) => vec![*k],
        Kind::If(_, a, b) => vec![*a, *b],
        Kind::Case { arms, .. } => arms.iter().map(|(_, _, b)| *b).collect(),
        _ => Vec::new(),
    }
}

/// Panic naming the first term reachable from `roots` that fails [`validate`], when built with
/// debug assertions or the `testing` feature.
pub fn assert_valid(schema: &Schema, terms: &Terms, roots: &[(Tm, Vec<Var>)]) {
    if cfg!(any(debug_assertions, feature = "testing")) {
        refused(schema, terms, validate(schema, terms, roots));
    }
}

/// Panic naming the first term reachable from `roots` that fails [`validate_bounded`] under
/// `bounds`, when built with debug assertions or the `testing` feature.
pub(crate) fn assert_valid_bounded(
    schema: &Schema,
    terms: &Terms,
    roots: &[(Tm, Vec<Var>)],
    bounds: Bounds<'_>,
) {
    if cfg!(any(debug_assertions, feature = "testing")) {
        refused(
            schema,
            terms,
            validate_bounded(schema, terms, roots, bounds),
        );
    }
}

/// Panic naming the first term reachable from `roots` that fails [`validate`] or
/// [`validate_normal`], when built with debug assertions or the `testing` feature.
pub fn assert_normal(schema: &Schema, terms: &Terms, roots: &[(Tm, Vec<Var>)]) {
    if cfg!(any(debug_assertions, feature = "testing")) {
        refused(schema, terms, validate(schema, terms, roots));
        refused(schema, terms, validate_normal(schema, terms, roots));
    }
}

/// Check every term reachable from `roots` against the typing invariants of its kind, its
/// variables bound by whatever runs it.
pub fn validate_typed(schema: &Schema, terms: &Terms, roots: &[Tm]) -> Result<(), Invalid> {
    typed(schema, terms, Bounds::Unknown, roots.iter().copied())
}

fn refused<T>(schema: &Schema, terms: &Terms, r: Result<T, Invalid>) {
    if let Err(e) = r {
        panic!(
            "kernel term {:?} breaks the invariant that the checker guarantees: {} ({:?} : {})",
            e.at,
            e.reason,
            terms.kind(e.at),
            schema.show_ty(terms.ty(e.at))
        );
    }
}

/// Check `t` against the typing invariant of its kind, its children taken as they are.
pub(super) fn node(schema: &Schema, terms: &Terms, t: Tm) -> Result<(), String> {
    Validator {
        schema,
        terms,
        bounds: Bounds::Unknown,
    }
    .node(t)
}

/// Check `t` against the typing invariant of its kind, its children taken as they are.
pub fn validate_node(schema: &Schema, terms: &Terms, t: Tm) -> Result<(), Invalid> {
    node(schema, terms, t).map_err(|reason| Invalid { at: t, reason })
}

/// Check every term reachable from `roots` against the typing invariants of its kind.
fn typed(
    schema: &Schema,
    terms: &Terms,
    bounds: Bounds<'_>,
    roots: impl Iterator<Item = Tm>,
) -> Result<(), Invalid> {
    let v = Validator {
        schema,
        terms,
        bounds,
    };
    let mut seen = HashSet::new();
    let mut todo: Vec<Tm> = roots.collect();
    while let Some(t) = todo.pop() {
        if !seen.insert(t) {
            continue;
        }
        v.node(t).map_err(|reason| Invalid { at: t, reason })?;
        todo.extend(terms.children(t));
    }
    Ok(())
}

/// Returns the variables free in `t`, keeping each term's in `memo`, on a stack of its own.
fn free(terms: &Terms, t: Tm, memo: &mut HashMap<Tm, Rc<BTreeSet<Var>>>) -> Rc<BTreeSet<Var>> {
    let mut stack = vec![(t, false)];
    while let Some((s, ready)) = stack.pop() {
        if memo.contains_key(&s) {
            continue;
        }
        let kind = terms.kind(s);
        if !ready {
            stack.push((s, true));
            stack.extend(kind.children().into_iter().map(|c| (c, false)));
            continue;
        }
        let mut out = BTreeSet::new();
        if let Kind::Var(v) = kind {
            out.insert(*v);
        }
        for (c, b) in kind.children().into_iter().zip(kind.bound()) {
            out.extend(memo[&c].iter().filter(|v| !b.contains(v)));
        }
        memo.insert(s, Rc::new(out));
    }
    Rc::clone(&memo[&t])
}

struct Validator<'a> {
    schema: &'a Schema,
    terms: &'a Terms,
    bounds: Bounds<'a>,
}

fn expect(holds: bool, what: impl FnOnce() -> String) -> Checked {
    if holds { Ok(()) } else { Err(what()) }
}

fn sorted_distinct<T: Ord>(xs: impl IntoIterator<Item = T>) -> bool {
    let xs: Vec<T> = xs.into_iter().collect();
    xs.windows(2).all(|w| w[0] < w[1])
}

impl Validator<'_> {
    fn ty(&self, t: Tm) -> &Ty {
        self.terms.ty(t)
    }

    fn sub(&self, a: &Ty, b: &Ty) -> bool {
        subtype(self.schema, a, b)
    }

    fn same(&self, what: &str, got: &Ty, want: &Ty) -> Checked {
        expect(got == want, || {
            format!(
                "{what} is `{}`, not `{}`",
                self.schema.show_ty(got),
                self.schema.show_ty(want)
            )
        })
    }

    fn within(&self, what: &str, got: &Ty, bound: &Ty) -> Checked {
        expect(self.sub(got, bound), || {
            format!(
                "{what} is `{}`, not a subtype of `{}`",
                self.schema.show_ty(got),
                self.schema.show_ty(bound)
            )
        })
    }

    fn data(def: TypeId, args: Vec<Ty>) -> Ty {
        Ty::Data {
            def,
            args,
            ctors: None,
        }
    }

    fn is_bool(&self, ty: &Ty) -> bool {
        Self::is_data(self.schema.prelude().bool(), ty)
    }

    fn is_data(of: TypeId, ty: &Ty) -> bool {
        matches!(ty, Ty::Data { def, .. } if *def == of)
    }

    fn row(&self, rel: RelId, ctors: &Option<CtorSet>) -> Ty {
        let r = self.schema.relation(rel);
        match &r.elem {
            Ty::Data { def, args, .. } => Ty::Data {
                def: *def,
                args: args.clone(),
                ctors: ctors.clone(),
            },
            t => t.clone(),
        }
    }

    fn ctor_set(&self, set: &CtorSet, of: usize) -> Checked {
        expect(sorted_distinct(set.iter()), || {
            format!("constructor set {set:?} is not sorted and distinct")
        })?;
        expect(set.iter().all(|&c| (c as usize) < of), || {
            format!("constructor set {set:?} names a constructor past {of}")
        })
    }

    fn selection(&self, set: &Option<CtorSet>, of: usize) -> Checked {
        let Some(set) = set else { return Ok(()) };
        self.ctor_set(set, of)?;
        expect(set.len() != of, || {
            format!("constructor set {set:?} names every constructor, so is `None`")
        })
    }

    fn rel_ctors(&self, rel: RelId) -> usize {
        self.schema
            .ty(self.schema.relation(rel).elem_def)
            .ctors()
            .len()
    }

    fn well_formed(&self, ty: &Ty) -> Checked {
        match ty {
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => Ok(()),
            Ty::Data { def, args, ctors } => {
                let d = self.schema.ty(*def);
                if !d.ctors().is_empty() {
                    self.selection(ctors, d.ctors().len())?;
                }
                args.iter().try_for_each(|a| self.well_formed(a))
            }
            Ty::Key { rel, ctors } => match (rel, ctors) {
                (RelRef::Declared(r), KeyCtors::Only(cs)) => {
                    self.selection(&Some(cs.clone()), self.rel_ctors(*r))
                }
                (RelRef::Declared(_), KeyCtors::All | KeyCtors::Late(_))
                | (RelRef::Ahead(_), KeyCtors::All | KeyCtors::Only(_) | KeyCtors::Late(_)) => {
                    Ok(())
                }
            },
            Ty::Set(x) | Ty::NonEmpty(x) | Ty::Tag(x) => self.well_formed(x),
            Ty::Group(_, _) => Err("a node has a `Group` type".to_string()),
            Ty::Fn(a, b) => {
                self.well_formed(a)?;
                self.well_formed(b)
            }
            Ty::Record(fs) => {
                expect(sorted_distinct(fs.iter().map(|(n, _)| n)), || {
                    "record type fields are not sorted by name".to_string()
                })?;
                fs.iter().try_for_each(|(_, t)| self.well_formed(t))
            }
            Ty::Sum { ctors, sel } => {
                expect(sorted_distinct(ctors.iter().map(|(n, _)| n)), || {
                    "anonymous sum constructors are not sorted by name".to_string()
                })?;
                for (_, fs) in ctors {
                    expect(sorted_distinct(fs.iter().map(|(n, _)| n)), || {
                        "anonymous sum fields are not sorted by name".to_string()
                    })?;
                    fs.iter().try_for_each(|(_, t)| self.well_formed(t))?;
                }
                self.selection(sel, ctors.len())
            }
        }
    }

    #[deny(
        clippy::wildcard_enum_match_arm,
        clippy::match_wildcard_for_single_variants
    )]
    fn node(&self, t: Tm) -> Checked {
        let ty = self.ty(t);
        self.well_formed(ty)?;
        match self.terms.kind(t) {
            Kind::Var(x) => self.same("the variable's type", &self.terms.var_info(*x).ty, ty),
            Kind::Const(c) => self.constant(c, ty),
            Kind::Prim(p, args) => self.prim(*p, args, ty),
            Kind::Record(fs) => self.record(fs, ty),
            Kind::Field(e, l) => self.field(*e, l, ty),
            Kind::Ctor { ctor, fields } => self.ctor(*ctor, fields, ty),
            Kind::Case { scrutinee, arms } => self.case(*scrutinee, arms, ty),
            Kind::If(c, a, b) => self.conditional(*c, *a, *b, ty),
            Kind::Let(x, e, b) => {
                self.within("the bound value", self.ty(*e), &self.terms.var_info(*x).ty)?;
                self.within("the let's body", self.ty(*b), ty)
            }
            Kind::Lam(x, b) => {
                let want = Ty::Fn(
                    Box::new(self.terms.var_info(*x).ty.clone()),
                    Box::new(self.ty(*b).clone()),
                );
                self.within("the lambda", &want, ty)
            }
            Kind::App(f, a) => self.application(*f, *a, ty),
            Kind::Call {
                func,
                type_args,
                args,
            } => self.call(*func, type_args, args, ty),
            Kind::Caller => {
                let principal = Ty::Data {
                    def: self.schema.principal,
                    args: Vec::new(),
                    ctors: None,
                };
                self.same("`caller`", ty, &principal)
            }
            Kind::KeyOf(r, e) => self.key_of(*r, *e, ty),
            Kind::Fresh(g, r) => self.fresh(*g, *r, ty),
            Kind::Placeholder {
                generator,
                rel,
                scope,
                ..
            } => self.placeholder(*generator, *rel, scope, ty),
            Kind::Only(_) => Err("an `Only` is never produced".into()),
            Kind::Unreachable => Ok(()),
            Kind::Key(e) => self.key(*e, ty),
            Kind::Deref { key, admits, .. } => self.deref(*key, admits, ty),
            Kind::Tag(e) => self.tag(*e, ty),
            Kind::Try(e) => self.tried(*e, ty),
            Kind::Rel(r, s) | Kind::Transition(_, r, s) => {
                self.selection(s, self.rel_ctors(*r))?;
                self.same("the relation", ty, &Ty::Set(Box::new(self.row(*r, s))))
            }
            Kind::Restrict(e, s) => self.restricted(*e, s, ty),
            Kind::EmptySet => {
                element(ty)?;
                expect(!matches!(ty, Ty::NonEmpty(_)), || {
                    "an empty set is typed `Set⁺`".into()
                })
            }
            Kind::Singleton(e) => self.within("the element", self.ty(*e), element(ty)?),
            Kind::Union(a, b) => self.union(*a, *b, ty),
            Kind::For(x, src, b) => self.generator(*x, *src, *b, ty),
            Kind::IsEmpty(e) | Kind::Exists(e) => {
                expect(elem(self.ty(*e)).is_some(), || {
                    "a test of a term that is not a set".into()
                })?;
                expect(self.is_bool(ty), || "a set test is not `Bool`".into())
            }
            Kind::Agg {
                op,
                var,
                source,
                body,
            } => self.aggregate(*op, *var, *source, *body, ty),
            Kind::Group { var, source, by } => self.group(*var, *source, *by, ty),
            Kind::Top {
                count,
                keys,
                source,
                ..
            } => {
                self.same("the count", self.ty(*count), &Ty::Base(Base::I64))?;
                self.ordered(keys)?;
                self.within("the top's source", self.ty(*source), ty)
            }
            Kind::Present {
                source,
                keys,
                take,
                after,
                ..
            } => self.present(*source, keys, *take, *after, ty),
            Kind::Fix(binds, b) => self.fixpoint(binds, *b, ty),
            Kind::At {
                instant,
                point,
                body,
                ..
            } => self.at(*instant, *point, *body, ty),
            Kind::Insert(r, e) | Kind::Delete(r, e) => {
                self.rows(*r, *e)?;
                self.same("the write", ty, &Ty::Change)
            }
            Kind::Replace(r, e, f) => self.replace(*r, *e, *f, ty),
            Kind::Upsert {
                rel,
                rows,
                old,
                new,
                otherwise,
                ..
            } => self.upsert(*rel, *rows, *old, *new, *otherwise, ty),
            Kind::Rejects { .. } | Kind::Skip => self.same("the write", ty, &Ty::Change),
            Kind::Par(ws) => {
                for w in ws {
                    self.same("a joined write", self.ty(*w), &Ty::Change)?;
                }
                self.same("the joined writes", ty, &Ty::Change)
            }
            Kind::Fail(m) => {
                self.same("the failure's text", self.ty(*m), &Ty::Base(Base::Text))?;
                self.same("the failure", ty, &Ty::Change)
            }
            Kind::Reject(m) => {
                self.same("the rejection's text", self.ty(*m), &Ty::Base(Base::Text))
            }
            Kind::Block(w, k) => {
                self.same("a block's write", self.ty(*w), &Ty::Change)?;
                self.within("the block's result", self.ty(*k), ty)
            }
            Kind::LetBang(_, w, k) => self.let_bang(*w, *k, ty),
            Kind::Transact(b, _) => self.within("the transaction's block", self.ty(*b), ty),
        }
    }

    fn ordered(&self, keys: &[(Tm, bool)]) -> Checked {
        for (k, _) in keys {
            let kt = self.ty(*k);
            expect(self.schema.is_ordered(kt) == Ok(true), || {
                format!("a key is `{}`, not ordered", self.schema.show_ty(kt))
            })?;
        }
        Ok(())
    }

    fn present(
        &self,
        source: Tm,
        keys: &[(Tm, bool)],
        take: Option<Tm>,
        after: Option<Tm>,
        ty: &Ty,
    ) -> Checked {
        self.ordered(keys)?;
        if let Some(n) = take {
            self.same("the take", self.ty(n), &Ty::Base(Base::I64))?;
        }
        if let Some(a) = after {
            let at = self.ty(a);
            let Ty::Data { def, args, .. } = at else {
                return Err("the position after is not an `Option`".into());
            };
            expect(*def == self.schema.prelude().option(), || {
                "the position after is not an `Option`".into()
            })?;
            match (keys, &args[0]) {
                ([(k, _)], kt) => self.within("the key after", self.ty(*k), kt)?,
                (_, Ty::Record(fs)) if fs.len() == keys.len() => {
                    for ((k, _), (n, ft)) in keys.iter().zip(fs) {
                        self.within(&format!("key `{n}` after"), self.ty(*k), ft)?;
                    }
                }
                _ => return Err("the position after is not a record of the keys".into()),
            }
        }
        self.within("the result", self.ty(source), ty)
    }

    fn conditional(&self, c: Tm, a: Tm, b: Tm, ty: &Ty) -> Checked {
        expect(self.is_bool(self.ty(c)), || {
            "the condition is not `Bool`".into()
        })?;
        self.within("the then branch", self.ty(a), ty)?;
        self.within("the else branch", self.ty(b), ty)
    }

    fn application(&self, f: Tm, a: Tm, ty: &Ty) -> Checked {
        let Ty::Fn(param, result) = self.ty(f) else {
            return Err("the applied term is not a function".into());
        };
        self.within("the argument", self.ty(a), param)?;
        self.within("the application", result, ty)
    }

    fn call(&self, func: FnId, type_args: &[Ty], args: &[Tm], ty: &Ty) -> Checked {
        let f = self.schema.function(func);
        expect(f.params.len() == args.len(), || {
            format!(
                "`{}` takes {} arguments, given {}",
                f.name,
                f.params.len(),
                args.len()
            )
        })?;
        for ((n, pt), a) in f.params.iter().zip(args) {
            self.within(
                &format!("argument `{n}`"),
                self.ty(*a),
                &pt.subst(type_args),
            )?;
        }
        self.within("the call", &f.result.subst(type_args), ty)
    }

    fn key_of(&self, r: RelId, e: Tm, ty: &Ty) -> Checked {
        let value = key_value_ty(self.schema, r)?;
        self.within("the key value", self.ty(e), &value)?;
        let key = Ty::Key {
            rel: RelRef::Declared(r),
            ctors: KeyCtors::All,
        };
        self.within("the key", &key, ty)
    }

    fn placeholder(&self, generator: Generator, rel: RelId, scope: &[Tm], ty: &Ty) -> Checked {
        expect(
            scope
                .iter()
                .all(|t| matches!(self.terms.kind(*t), Kind::Var(_))),
            || "a placeholder's scope holds a term that is not a variable".into(),
        )?;
        self.fresh(generator, rel, ty)
    }

    fn deref(&self, key: Tm, admits: &Option<CtorSet>, ty: &Ty) -> Checked {
        match self.ty(key) {
            Ty::Key {
                rel: RelRef::Declared(r),
                ctors,
            } => {
                let row = |s: &Option<CtorSet>| self.row(*r, s);
                let named = match ctors {
                    KeyCtors::All | KeyCtors::Late(_) => None,
                    KeyCtors::Only(cs) => Some(cs.clone()),
                };
                expect(self.sub(&row(admits), &row(&named)), || {
                    format!(
                        "the admitted constructors are not within those of `{}`",
                        self.schema.show_ty(self.ty(key))
                    )
                })?;
                self.within("the dereferenced row", &row(admits), ty)
            }
            Ty::Key { .. } => Ok(()),
            _ => Err("a dereference of a term that is not a key".into()),
        }
    }

    fn tried(&self, e: Tm, ty: &Ty) -> Checked {
        match ty {
            Ty::Data { def, args, .. } if *def == self.schema.prelude().result() => {
                self.within("the tried value", self.ty(e), &args[0])?;
                self.same(
                    "the failure of `try`",
                    &args[1],
                    &Self::data(self.schema.prelude().fault(), Vec::new()),
                )
            }
            _ => Err("`try` is not a `Result`".into()),
        }
    }

    fn union(&self, a: Tm, b: Tm, ty: &Ty) -> Checked {
        let el = element(ty)?;
        for x in [a, b] {
            let Some(xe) = elem(self.ty(x)) else {
                return Err("a union of a term that is not a set".into());
            };
            self.within("a union's element", xe, el)?;
        }
        Ok(())
    }

    fn generator(&self, x: Var, src: Tm, b: Tm, ty: &Ty) -> Checked {
        let Some(se) = elem(self.ty(src)) else {
            return Err("a generator over a term that is not a set".into());
        };
        self.within("the generator's element", se, &self.terms.var_info(x).ty)?;
        element(ty)?;
        self.within("the comprehension's body", self.ty(b), ty)
    }

    fn group(&self, var: Var, source: Tm, by: Tm, ty: &Ty) -> Checked {
        let Some(se) = elem(self.ty(source)) else {
            return Err("a grouping of a term that is not a set".into());
        };
        let vt = &self.terms.var_info(var).ty;
        self.within("the grouping's element", se, vt)?;
        if let Ty::Set(el) = ty
            && let Ty::Record(fs) = el.as_ref()
            && let [(k, key), (r, Ty::NonEmpty(rows))] = fs.as_slice()
            && k == "key"
            && r == "rows"
        {
            self.within("the grouping's key", self.ty(by), key)?;
            return self.same("the grouping's rows", rows, vt);
        }
        Err("the grouping is not a `Set` of `key` and `rows` records".into())
    }

    fn fixpoint(&self, binds: &[(Var, Tm)], b: Tm, ty: &Ty) -> Checked {
        for (x, body) in binds {
            self.within(
                "a fixpoint body",
                self.ty(*body),
                &self.terms.var_info(*x).ty,
            )?;
        }
        self.within("the fixpoint's body", self.ty(b), ty)
    }

    fn at(&self, instant: bool, point: Tm, body: Tm, ty: &Ty) -> Checked {
        let pt = self.ty(point);
        let position =
            matches!(pt, Ty::Data { def, .. } if *def == self.schema.prelude().position());
        expect(
            *pt == Ty::Base(Base::Instant) || (!instant && position),
            || format!("the point is `{}`", self.schema.show_ty(pt)),
        )?;
        self.within("the read at a point", self.ty(body), ty)
    }

    fn replace(&self, r: RelId, e: Tm, f: Tm, ty: &Ty) -> Checked {
        let el = self.rows(r, e)?;
        let Ty::Fn(param, result) = self.ty(f) else {
            return Err("the new value of a `Replace` is not a function".into());
        };
        self.within("a replaced row", el, param)?;
        self.within("a new value", result, &self.row(r, &None))?;
        self.same("the write", ty, &Ty::Change)
    }

    fn upsert(&self, rel: RelId, rows: Tm, old: Var, new: Var, otherwise: Tm, ty: &Ty) -> Checked {
        let el = self.rows(rel, rows)?;
        self.within("a proposed row", el, &self.terms.var_info(new).ty)?;
        self.within(
            "the conflicting row",
            &self.terms.var_info(old).ty,
            &self.row(rel, &None),
        )?;
        self.same("the conflict branch", self.ty(otherwise), &Ty::Change)?;
        self.same("the write", ty, &Ty::Change)
    }

    fn let_bang(&self, w: Tm, k: Tm, ty: &Ty) -> Checked {
        expect(
            matches!(
                self.terms.kind(w),
                Kind::Replace(..) | Kind::Upsert { .. } | Kind::Rejects { .. }
            ),
            || "a `LetBang` binds a write other than `Replace`, `Upsert` or `Rejects`".into(),
        )?;
        self.within("the block's result", self.ty(k), ty)
    }

    fn constant(&self, c: &Const, ty: &Ty) -> Checked {
        let fits = match c {
            Const::Int(_) => matches!(
                ty,
                Ty::Base(
                    Base::I8
                        | Base::I16
                        | Base::I32
                        | Base::I64
                        | Base::Decimal(_)
                        | Base::F32
                        | Base::F64
                ) | Ty::Param(_)
            ),
            Const::Decimal(_) => matches!(ty, Ty::Base(Base::Decimal(_))),
            Const::Float(_) => matches!(ty, Ty::Base(Base::F32 | Base::F64)),
            Const::Text(_) => *ty == Ty::Base(Base::Text),
            Const::Duration(_) => *ty == Ty::Base(Base::Duration),
        };
        expect(fits, || {
            format!("the constant is typed `{}`", self.schema.show_ty(ty))
        })
    }

    fn prim(&self, p: Prim, args: &[Tm], ty: &Ty) -> Checked {
        let want = prim_ty(self.schema, self.terms, self.bounds, p, args, Some(ty))?;
        self.within(&format!("`{p:?}`"), &want, ty)
    }

    fn record(&self, fs: &[(String, Tm)], ty: &Ty) -> Checked {
        expect(sorted_distinct(fs.iter().map(|(n, _)| n)), || {
            "the record's fields are not sorted by name".into()
        })?;
        let want = match ty {
            Ty::Record(want) => want.as_slice(),
            Ty::Unit => &[],
            _ => return Err("a record is not typed by a record".into()),
        };
        expect(
            want.len() == fs.len() && want.iter().zip(fs).all(|((n, _), (m, _))| n == m),
            || "the record's labels are not its type's".into(),
        )?;
        for ((n, wt), (_, f)) in want.iter().zip(fs) {
            self.within(&format!("field `{n}`"), self.ty(*f), wt)?;
        }
        Ok(())
    }

    fn field(&self, e: Tm, l: &str, ty: &Ty) -> Checked {
        let row = self.ty(e);
        let fs = fields(self.schema, row);
        let Some((_, ft)) = fs.iter().find(|(n, _)| n == l) else {
            return Err(format!(
                "`{}` has no field `{l}` every constructor holds",
                self.schema.show_ty(row)
            ));
        };
        let ft = match (ft, row) {
            (
                Ty::Key {
                    rel: RelRef::Declared(r),
                    ctors: KeyCtors::All,
                },
                Ty::Data { def, ctors, .. },
            ) if self.schema.relation(*r).elem_def == *def && self.schema.self_key(*r, l) => {
                Ty::Key {
                    rel: RelRef::Declared(*r),
                    ctors: KeyCtors::of_row(ctors.clone()),
                }
            }
            _ => ft.clone(),
        };
        self.within(&format!("field `{l}`"), &ft, ty)
    }

    fn key(&self, e: Tm, ty: &Ty) -> Checked {
        let Ty::Data { def, ctors, .. } = self.ty(e) else {
            return Err("`key` of a term that is not a row".into());
        };
        match ty {
            Ty::Key {
                rel: RelRef::Declared(r),
                ..
            } => {
                expect(self.schema.relation(*r).elem_def == *def, || {
                    format!(
                        "the key's relation does not hold rows of `{}`",
                        self.schema.ty(*def).name
                    )
                })?;
                let want = Ty::Key {
                    rel: RelRef::Declared(*r),
                    ctors: KeyCtors::of_row(ctors.clone()),
                };
                self.within("the key", &want, ty)
            }
            Ty::Key { .. } => Err("`key` names a relation the schema does not declare".into()),
            _ => Err("`key` is not a key".into()),
        }
    }

    fn tag(&self, e: Tm, ty: &Ty) -> Checked {
        let Ty::Data { def, .. } = self.ty(e) else {
            return Err("`tag` of a term that is not of a declared sum".into());
        };
        match ty {
            Ty::Tag(inner) if matches!(inner.as_ref(), Ty::Data { def: d, .. } if d == def) => {
                Ok(())
            }
            _ => Err(format!(
                "`tag` of `{}` is `{}`",
                self.schema.ty(*def).name,
                self.schema.show_ty(ty)
            )),
        }
    }

    fn ctor(&self, ctor: u32, given: &[(String, Tm)], ty: &Ty) -> Checked {
        expect(sorted_distinct(given.iter().map(|(n, _)| n)), || {
            "the constructor's fields are not sorted by name".into()
        })?;
        let mut want: Vec<(String, Ty)> = match ty {
            Ty::Data { def, args, ctors } => {
                let d = self.schema.ty(*def);
                let holds = match ctors {
                    Some(s) => s.contains(&ctor),
                    None => (ctor as usize) < d.ctors().len(),
                };
                expect(holds, || {
                    format!("constructor {ctor} is not one of the node's type")
                })?;
                ctor_fields(self.schema, *def, args, ctor)
                    .into_iter()
                    .map(|(n, t, _)| (n, t))
                    .collect()
            }
            Ty::Sum { ctors, sel } => {
                expect(sel.as_ref().is_none_or(|s| s.contains(&ctor)), || {
                    format!("constructor {ctor} is not one of the node's type")
                })?;
                ctors
                    .get(ctor as usize)
                    .ok_or_else(|| format!("the sum has no constructor {ctor}"))?
                    .1
                    .clone()
            }
            Ty::Tag(inner) => {
                let Ty::Data { def, .. } = inner.as_ref() else {
                    return Err("a tag constructor is not of a declared sum".into());
                };
                let of = self.schema.ty(*def).ctors().len();
                expect((ctor as usize) < of, || {
                    format!("constructor {ctor} is past the {of} of the tag's sum")
                })?;
                Vec::new()
            }
            _ => return Err("a constructor is not typed by a sum".into()),
        };
        want.sort_by(|a, b| a.0.cmp(&b.0));
        expect(
            want.len() == given.len() && want.iter().zip(given).all(|((n, _), (m, _))| n == m),
            || "the constructor's fields are not its declaration's".into(),
        )?;
        for ((n, wt), (_, f)) in want.iter().zip(given) {
            self.within(&format!("field `{n}`"), self.ty(*f), wt)?;
        }
        Ok(())
    }

    fn case(&self, scrutinee: Tm, arms: &[(u32, Var, Tm)], ty: &Ty) -> Checked {
        let st = self.ty(scrutinee);
        let Some(names) = ctor_names(self.schema, st) else {
            return Err("the scrutinee is not a sum".into());
        };
        let want: Vec<u32> = names.iter().map(|(c, _)| *c).collect();
        let got: Vec<u32> = arms.iter().map(|(c, _, _)| *c).collect();
        expect(want == got, || {
            format!("the arms are {got:?}, the scrutinee's constructors {want:?}")
        })?;
        for (c, x, body) in arms {
            if !matches!(st, Ty::Tag(_)) {
                let row = restrict(self.schema, st, vec![*c]);
                self.same("an arm's variable", &self.terms.var_info(*x).ty, &row)?;
            }
            self.within("an arm", self.ty(*body), ty)?;
        }
        Ok(())
    }

    /// Returns the element type of the set `e`, after checking it is a row type of `r`.
    fn rows(&self, r: RelId, e: Tm) -> Result<&Ty, String> {
        let Some(el) = elem(self.ty(e)) else {
            return Err("the rows of a write are not a set".into());
        };
        self.within("a row of the write", el, &self.row(r, &None))?;
        Ok(el)
    }

    fn fresh(&self, g: Generator, r: RelId, ty: &Ty) -> Checked {
        match g {
            Generator::Clock => self.same("a clock value", ty, &Ty::Base(Base::Instant)),
            Generator::Sequence | Generator::Uuidv7 => expect(
                matches!(ty, Ty::Key { rel: RelRef::Declared(own), .. } if *own == r),
                || format!("a generated key is `{}`", self.schema.show_ty(ty)),
            ),
        }
    }

    fn restricted(&self, e: Tm, s: &CtorSet, ty: &Ty) -> Checked {
        let Some(el) = elem(self.ty(e)) else {
            return Err("a restriction of a term that is not a set".into());
        };
        match el {
            Ty::Data { def, .. } if !self.schema.ty(*def).ctors().is_empty() => {
                self.ctor_set(s, self.schema.ty(*def).ctors().len())?;
            }
            Ty::Sum { ctors, .. } => self.ctor_set(s, ctors.len())?,
            _ => return Err("a restriction of a set of other than a sum".into()),
        }
        let want = restrict(self.schema, el, s.clone());
        expect(self.sub(&Ty::Set(Box::new(want.clone())), ty), || {
            format!(
                "the restriction is `{}`, not a supertype of a set of `{}`",
                self.schema.show_ty(ty),
                self.schema.show_ty(&want)
            )
        })
    }

    fn aggregate(&self, op: Agg, var: Var, source: Tm, body: Tm, ty: &Ty) -> Checked {
        let st = self.ty(source);
        let Some(se) = elem(st) else {
            return Err("an aggregate over a term that is not a set".into());
        };
        self.within("the aggregate's element", se, &self.terms.var_info(var).ty)?;
        let partial = |r: &Ty| -> Checked {
            let optional = Self::data(self.schema.prelude().option(), vec![r.clone()]);
            if matches!(st, Ty::NonEmpty(_)) && self.sub(r, ty) {
                return Ok(());
            }
            self.within("the aggregate", &optional, ty)
        };
        match op {
            Agg::Count => self.same("`count`", ty, &Ty::Base(Base::I64)),
            Agg::Sum => self.within("`sum`", self.ty(body), ty),
            Agg::Min | Agg::Max | Agg::Avg => partial(self.ty(body)),
            Agg::User(a) => {
                let def = &self.schema.aggregates[a as usize];
                let [(_, param)] = def.params.as_slice() else {
                    return Err(format!("`{}` takes other than one parameter", def.name));
                };
                self.within("the aggregated value", self.ty(body), param)?;
                partial(&def.result)
            }
        }
    }
}

fn element(ty: &Ty) -> Result<&Ty, String> {
    elem(ty).ok_or_else(|| "a set term is not a set".to_string())
}
