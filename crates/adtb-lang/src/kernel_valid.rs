use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use super::{Agg, Const, Kind, Prim, Terms, Tm, Var};
use crate::ast::FnBody;
use crate::check::rel::key_value_ty;
use crate::check::ty::{ctor_fields, ctor_names, elem, fields, restrict, subtype};
use crate::schema::{Generator, Schema};
use crate::types::{Base, CtorSet, RelId, RelRef, Ty};

/// A node that breaks the kernel's typing invariants, with the invariant it breaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid {
    at: Tm,
    reason: String,
}

impl Invalid {
    pub fn at(&self) -> Tm {
        self.at
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

type Checked = Result<(), String>;

/// Check every term reachable from `roots` against the typing invariants of its kind, and that
/// every variable it reads is bound by an enclosing term or is one of the root's parameters.
pub fn validate(schema: &Schema, terms: &Terms, roots: &[(Tm, Vec<Var>)]) -> Result<(), Invalid> {
    typed(schema, terms, roots.iter().map(|(t, _)| *t))?;
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
    Ok(())
}

/// Check that every term reachable from `roots` is in normal form: no `Lam` but the new-value
/// function of a `Replace`, no `App`, no `Call` but of a Wasm or host function, and no `Only`.
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
            _ => None,
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

/// Panic naming the first term reachable from `roots` that fails [`validate`], when built with
/// debug assertions or the `testing` feature.
pub fn assert_valid(schema: &Schema, terms: &Terms, roots: &[(Tm, Vec<Var>)]) {
    if cfg!(any(debug_assertions, feature = "testing")) {
        refused(schema, terms, validate(schema, terms, roots));
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

fn refused(schema: &Schema, terms: &Terms, r: Result<(), Invalid>) {
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

/// Check every term reachable from `roots` against the typing invariants of its kind.
fn typed(schema: &Schema, terms: &Terms, roots: impl Iterator<Item = Tm>) -> Result<(), Invalid> {
    let v = Validator { schema, terms };
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

    fn prelude(&self, name: &str, args: Vec<Ty>) -> Ty {
        Ty::Data {
            def: self.schema.prelude_type(name),
            args,
            ctors: None,
        }
    }

    fn is_bool(&self, ty: &Ty) -> bool {
        self.is_prelude("Bool", ty)
    }

    fn is_prelude(&self, name: &str, ty: &Ty) -> bool {
        matches!(ty, Ty::Data { def, .. } if *def == self.schema.prelude_type(name))
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
            Ty::Key { rel, ctors } => match rel {
                RelRef::Declared(r) => self.selection(ctors, self.rel_ctors(*r)),
                RelRef::Ahead(_) => Ok(()),
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
            Kind::If(c, a, b) => {
                expect(self.is_bool(self.ty(*c)), || {
                    "the condition is not `Bool`".into()
                })?;
                self.within("the then branch", self.ty(*a), ty)?;
                self.within("the else branch", self.ty(*b), ty)
            }
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
            Kind::App(f, a) => {
                let Ty::Fn(param, result) = self.ty(*f) else {
                    return Err("the applied term is not a function".into());
                };
                self.within("the argument", self.ty(*a), param)?;
                self.within("the application", result, ty)
            }
            other => self.rest(other, ty),
        }
    }

    fn rest(&self, kind: &Kind, ty: &Ty) -> Checked {
        match kind {
            Kind::Call {
                func,
                type_args,
                args,
            } => {
                let f = self.schema.function(*func);
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
            Kind::Caller => {
                let principal = Ty::Data {
                    def: self.schema.principal,
                    args: Vec::new(),
                    ctors: None,
                };
                self.same("`caller`", ty, &principal)
            }
            Kind::KeyOf(r, e) => {
                let value = key_value_ty(self.schema, *r)?;
                self.within("the key value", self.ty(*e), &value)?;
                let key = Ty::Key {
                    rel: RelRef::Declared(*r),
                    ctors: None,
                };
                self.within("the key", &key, ty)
            }
            Kind::Fresh(g, r) => self.fresh(*g, *r, ty),
            Kind::Only(_) => Err("an `Only` is never produced".into()),
            Kind::Unreachable => Ok(()),
            Kind::Key(e) => self.key(*e, ty),
            Kind::Deref { key, .. } => match self.ty(*key) {
                Ty::Key {
                    rel: RelRef::Declared(r),
                    ctors,
                } => self.within("the dereferenced row", &self.row(*r, ctors), ty),
                Ty::Key { .. } => Ok(()),
                _ => Err("a dereference of a term that is not a key".into()),
            },
            Kind::Tag(e) => self.tag(*e, ty),
            Kind::Try(e) => match ty {
                Ty::Data { def, args, .. } if *def == self.schema.prelude_type("Result") => {
                    self.within("the tried value", self.ty(*e), &args[0])?;
                    self.same(
                        "the failure of `try`",
                        &args[1],
                        &self.prelude("Fault", Vec::new()),
                    )
                }
                _ => Err("`try` is not a `Result`".into()),
            },
            Kind::Rel(r, s) | Kind::Transition(_, r, s) => {
                self.selection(s, self.rel_ctors(*r))?;
                self.same("the relation", ty, &Ty::Set(Box::new(self.row(*r, s))))
            }
            Kind::Restrict(e, s) => self.restricted(*e, s, ty),
            kind => self.sets(kind, ty),
        }
    }

    fn sets(&self, kind: &Kind, ty: &Ty) -> Checked {
        let element = || elem(ty).ok_or_else(|| "a set term is not a set".to_string());
        match kind {
            Kind::EmptySet => {
                element()?;
                expect(!matches!(ty, Ty::NonEmpty(_)), || {
                    "an empty set is typed `Set⁺`".into()
                })
            }
            Kind::Singleton(e) => self.within("the element", self.ty(*e), element()?),
            Kind::Union(a, b) => {
                let el = element()?;
                for x in [a, b] {
                    let Some(xe) = elem(self.ty(*x)) else {
                        return Err("a union of a term that is not a set".into());
                    };
                    self.within("a union's element", xe, el)?;
                }
                Ok(())
            }
            Kind::For(x, src, b) => {
                let Some(se) = elem(self.ty(*src)) else {
                    return Err("a generator over a term that is not a set".into());
                };
                self.within("the generator's element", se, &self.terms.var_info(*x).ty)?;
                element()?;
                self.within("the comprehension's body", self.ty(*b), ty)
            }
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
            Kind::Group { var, source, by } => {
                let Some(se) = elem(self.ty(*source)) else {
                    return Err("a grouping of a term that is not a set".into());
                };
                self.within("the grouping's element", se, &self.terms.var_info(*var).ty)?;
                let want = Ty::Set(Box::new(Ty::Record(vec![
                    ("key".into(), self.ty(*by).clone()),
                    (
                        "rows".into(),
                        Ty::NonEmpty(Box::new(self.terms.var_info(*var).ty.clone())),
                    ),
                ])));
                self.within("the grouping", &want, ty)
            }
            Kind::Top { count, source, .. } => {
                self.same("the count", self.ty(*count), &Ty::Base(Base::I64))?;
                self.within("the top's source", self.ty(*source), ty)
            }
            Kind::Present { source, .. } => self.within("the result", self.ty(*source), ty),
            Kind::Fix(binds, b) => {
                for (x, body) in binds {
                    self.within(
                        "a fixpoint body",
                        self.ty(*body),
                        &self.terms.var_info(*x).ty,
                    )?;
                }
                self.within("the fixpoint's body", self.ty(*b), ty)
            }
            Kind::At {
                instant,
                point,
                body,
                ..
            } => {
                let pt = self.ty(*point);
                let position = matches!(pt, Ty::Data { def, .. } if *def == self.schema.prelude_type("Position"));
                expect(
                    *pt == Ty::Base(Base::Instant) || (!instant && position),
                    || format!("the point is `{}`", self.schema.show_ty(pt)),
                )?;
                self.within("the read at a point", self.ty(*body), ty)
            }
            kind => self.writes(kind, ty),
        }
    }

    fn writes(&self, kind: &Kind, ty: &Ty) -> Checked {
        match kind {
            Kind::Insert(r, e) | Kind::Delete(r, e) => {
                self.rows(*r, *e)?;
                self.same("the write", ty, &Ty::Change)
            }
            Kind::Replace(r, e, f) => {
                let el = self.rows(*r, *e)?;
                let Ty::Fn(param, result) = self.ty(*f) else {
                    return Err("the new value of a `Replace` is not a function".into());
                };
                self.within("a replaced row", el, param)?;
                self.within("a new value", result, &self.row(*r, &None))?;
                self.same("the write", ty, &Ty::Change)
            }
            Kind::Upsert {
                rel,
                rows,
                old,
                new,
                otherwise,
                ..
            } => {
                let el = self.rows(*rel, *rows)?;
                self.within("a proposed row", el, &self.terms.var_info(*new).ty)?;
                self.within(
                    "the conflicting row",
                    &self.terms.var_info(*old).ty,
                    &self.row(*rel, &None),
                )?;
                self.same("the conflict branch", self.ty(*otherwise), &Ty::Change)?;
                self.same("the write", ty, &Ty::Change)
            }
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
            Kind::LetBang(_, w, k) => {
                expect(
                    matches!(
                        self.terms.kind(*w),
                        Kind::Replace(..) | Kind::Upsert { .. } | Kind::Rejects { .. }
                    ),
                    || {
                        "a `LetBang` binds a write other than `Replace`, `Upsert` or `Rejects`"
                            .into()
                    },
                )?;
                self.within("the block's result", self.ty(*k), ty)
            }
            Kind::Transact(b, _) => self.within("the transaction's block", self.ty(*b), ty),
            _ => Ok(()),
        }
    }

    fn constant(&self, c: &Const, ty: &Ty) -> Checked {
        let fits = match c {
            Const::Int(_) => matches!(
                ty,
                Ty::Base(Base::I8 | Base::I16 | Base::I32 | Base::I64) | Ty::Param(_)
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
        let arity = |n: usize| {
            expect(args.len() == n, || {
                format!("`{p:?}` takes {n} arguments, given {}", args.len())
            })
        };
        let text = Ty::Base(Base::Text);
        match p {
            Prim::Eq | Prim::Ne | Prim::Lt | Prim::Le | Prim::Gt | Prim::Ge => {
                arity(2)?;
                expect(self.is_bool(ty), || "a comparison is not `Bool`".into())
            }
            Prim::StartsWith
            | Prim::EndsWith
            | Prim::Contains
            | Prim::Matches
            | Prim::Overlaps
            | Prim::Covers => {
                arity(2)?;
                expect(self.is_bool(ty), || format!("`{p:?}` is not `Bool`"))
            }
            Prim::Add | Prim::Sub | Prim::Mul | Prim::Div | Prim::Rem => arity(2),
            Prim::Neg | Prim::Abs => {
                arity(1)?;
                self.within("the operand", self.ty(args[0]), ty)
            }
            Prim::Concat => {
                arity(2)?;
                expect(matches!(ty, Ty::Base(Base::Text | Base::Bytes)), || {
                    "`++` is not of text or bytes".into()
                })?;
                args.iter()
                    .try_for_each(|a| self.within("an operand of `++`", self.ty(*a), ty))
            }
            Prim::CheckedAdd
            | Prim::CheckedSub
            | Prim::CheckedMul
            | Prim::CheckedDiv
            | Prim::CheckedRem
            | Prim::Parse
            | Prim::Convert { checked: true } => expect(self.is_prelude("Option", ty), || {
                format!("`{p:?}` is not an `Option`")
            }),
            Prim::CheckedNeg => {
                arity(1)?;
                expect(self.is_prelude("Option", ty), || {
                    "`CheckedNeg` is not an `Option`".into()
                })
            }
            Prim::DecimalDiv | Prim::Round | Prim::Rescale => {
                arity(match p {
                    Prim::DecimalDiv => 4,
                    Prim::Round => 3,
                    _ => 2,
                })?;
                expect(matches!(ty, Ty::Base(Base::Decimal(_))), || {
                    format!("`{p:?}` is not a decimal")
                })
            }
            Prim::ToText | Prim::Print | Prim::Lower | Prim::Upper | Prim::Fold => {
                self.same(&format!("`{p:?}`"), ty, &text)
            }
            Prim::ParseUuid => {
                arity(1)?;
                expect(self.is_prelude("Uuid", ty), || {
                    "`ParseUuid` is not a `Uuid`".into()
                })
            }
            Prim::Wrap | Prim::Unwrap | Prim::KeyValue => arity(1),
            Prim::Micros
            | Prim::Millis
            | Prim::Seconds
            | Prim::Minutes
            | Prim::Hours
            | Prim::Days
            | Prim::Weeks
            | Prim::Months
            | Prim::Years => {
                arity(1)?;
                expect(
                    matches!(ty, Ty::Base(Base::Duration | Base::Period)),
                    || format!("`{p:?}` is not a duration or a period"),
                )
            }
            Prim::ReadFile { .. } => {
                arity(1)?;
                self.same("the path", self.ty(args[0]), &text)?;
                expect(matches!(ty, Ty::Set(_)), || {
                    "a file scan is not a set".into()
                })
            }
            Prim::Convert { checked: false }
            | Prim::BytesLen
            | Prim::ScalarLen
            | Prim::GraphemeLen
            | Prim::Collate
            | Prim::Utf8
            | Prim::Bytes
            | Prim::AddPeriod
            | Prim::Zoned
            | Prim::InZone
            | Prim::ToInstant
            | Prim::Span
            | Prim::Start
            | Prim::End => Ok(()),
        }
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
                    ctors: None,
                },
                Ty::Data { def, ctors, .. },
            ) if self.schema.relation(*r).elem_def == *def
                && self.schema.relation(*r).key.iter().any(|k| k == l) =>
            {
                Ty::Key {
                    rel: RelRef::Declared(*r),
                    ctors: ctors.clone(),
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
                    ctors: ctors.clone(),
                };
                self.within("the key", &want, ty)
            }
            Ty::Key { .. } => Ok(()),
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
            Ty::Sum { ctors, .. } => ctors
                .get(ctor as usize)
                .ok_or_else(|| format!("the sum has no constructor {ctor}"))?
                .1
                .clone(),
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
        if let Ty::Data { def, .. } = el {
            self.ctor_set(s, self.schema.ty(*def).ctors().len())?;
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
        let nonempty = matches!(st, Ty::NonEmpty(_));
        let partial = |r: &Ty| -> Ty {
            if nonempty {
                r.clone()
            } else {
                self.prelude("Option", vec![r.clone()])
            }
        };
        match op {
            Agg::Count => self.same("`count`", ty, &Ty::Base(Base::I64)),
            Agg::Sum => self.within("`sum`", self.ty(body), ty),
            Agg::Min | Agg::Max | Agg::Avg => {
                self.within("the aggregate", &partial(self.ty(body)), ty)
            }
            Agg::User(a) => {
                let def = &self.schema.aggregates[a as usize];
                self.within("the aggregate", &partial(&def.result), ty)
            }
        }
    }
}
