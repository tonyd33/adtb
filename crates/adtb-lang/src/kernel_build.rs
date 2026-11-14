use super::{Agg, Conflict, Const, Kind, Prim, Terms, Tm, Transition, TxnOptions, Var};
use crate::check::ty::{elem, fields, restrict};
use crate::schema::{Generator, Schema};
use crate::types::{Base, CtorSet, FnId, RelId, RelRef, Ty};

/// Build kernel terms into one arena, typing each node from its children where its kind's rule
/// fixes the type and at a given type where the rule leaves it open. Sort record and constructor
/// fields and `Case` arms.
///
/// Postconditions:
/// - Under debug assertions or the `testing` feature, every node built satisfies its kind's typing
///   rule, or the build panics naming the node and the rule.
#[derive(Debug)]
pub struct Build<'a> {
    schema: &'a Schema,
    terms: &'a mut Terms,
    type_nodes: Option<&'a mut usize>,
}

impl<'a> Build<'a> {
    pub fn new(schema: &'a Schema, terms: &'a mut Terms) -> Self {
        Build {
            schema,
            terms,
            type_nodes: None,
        }
    }

    /// Add the size of each built node's type to `count`.
    pub(crate) fn counting(mut self, count: &'a mut usize) -> Self {
        self.type_nodes = Some(count);
        self
    }

    fn put(&mut self, kind: Kind, ty: Ty) -> Tm {
        if let Some(n) = self.type_nodes.as_deref_mut() {
            *n = n.saturating_add(ty.nodes());
        }
        let t = self.terms.add(kind, ty);
        if cfg!(any(debug_assertions, feature = "testing"))
            && let Err(reason) = super::valid::node(self.schema, self.terms, t)
        {
            panic!(
                "kernel node {t:?} breaks its kind's typing rule: {reason} ({:?} : {})",
                self.terms.kind(t),
                self.schema.show_ty(self.terms.ty(t))
            );
        }
        t
    }

    fn typed(&mut self, kind: Kind) -> Tm {
        let ty = match self.computed(&kind) {
            Ok(Some(ty)) => ty,
            Ok(None) => panic!("{kind:?} takes its type as an argument"),
            Err(reason) => self.untypable(&kind, &reason),
        };
        self.put(kind, ty)
    }

    /// Panic naming `kind` and `reason` under debug assertions or the `testing` feature. Returns
    /// `Unit` otherwise.
    fn untypable(&self, kind: &Kind, reason: &str) -> Ty {
        if cfg!(any(debug_assertions, feature = "testing")) {
            panic!("kernel node {kind:?} breaks its kind's typing rule: {reason}");
        }
        Ty::Unit
    }

    /// Returns `kind` built with the type its children give, or at `given` when its typing rule
    /// leaves the type open.
    ///
    /// Preconditions:
    /// - The children of `kind` are terms of this arena.
    pub(crate) fn rebuilt(&mut self, kind: Kind, given: &Ty) -> Tm {
        let ty = match self.computed(&kind) {
            Ok(ty) => ty.unwrap_or_else(|| given.clone()),
            Err(reason) => self.untypable(&kind, &reason),
        };
        self.put(kind, ty)
    }

    /// Returns `kind` at `checked`, except that a `Var`, `Rel` or `Transition` takes the type its
    /// children give, a `Case` drops the arms of constructors its scrutinee's type excludes, and a
    /// `Deref` admits only constructors its key's type names.
    ///
    /// Preconditions:
    /// - `checked` is the type of the node `kind` rebuilds.
    pub(crate) fn kept(&mut self, kind: Kind, checked: &Ty) -> Tm {
        match kind {
            Kind::Var(_) | Kind::Rel(..) | Kind::Transition(..) => self.typed(kind),
            Kind::Case { scrutinee, arms } => self.kept_case(scrutinee, arms, checked.clone()),
            Kind::Deref {
                key,
                reference,
                admits,
            } => {
                let admits = self.within_key(key, admits);
                self.put(
                    Kind::Deref {
                        key,
                        reference,
                        admits,
                    },
                    checked.clone(),
                )
            }
            _ => self.put(kind, checked.clone()),
        }
    }

    /// Returns a `Case` over `scrutinee` at `ty` holding the arms of the constructors its
    /// scrutinee's type names, the others dropped.
    pub(crate) fn kept_case(&mut self, scrutinee: Tm, mut arms: Vec<(u32, Var, Tm)>, ty: Ty) -> Tm {
        if let Ty::Data { ctors: Some(s), .. } = self.terms.ty(scrutinee) {
            arms.retain(|(c, _, _)| s.contains(c));
        }
        self.case(scrutinee, arms, ty)
    }

    /// Returns node `t` of this arena rebuilt over `children` in place of its own, as `kept`
    /// rebuilds it at the type of `t`.
    ///
    /// Preconditions:
    /// - `children` are terms of this arena, as many as `t` has.
    pub fn rebuild_kept(&mut self, t: Tm, children: &[Tm]) -> Tm {
        assert_eq!(
            self.terms.children(t).len(),
            children.len(),
            "a rebuilt node keeps the number of children of {t:?}"
        );
        let kind = self.terms.kind(t).with_children(children);
        let ty = self.terms.ty(t).clone();
        self.kept(kind, &ty)
    }

    /// Returns node `t` of arena `from` copied into this arena over `children`, each variable it
    /// names replaced by its image under `rename`, as `kept` rebuilds it at the type of `t`.
    ///
    /// Preconditions:
    /// - `children` are terms of this arena, as many as `t` has.
    /// - `rename` maps every variable `t` names to a variable of this arena of the same type.
    pub fn import_kept(
        &mut self,
        from: &Terms,
        t: Tm,
        children: &[Tm],
        mut rename: impl FnMut(Var) -> Var,
    ) -> Tm {
        assert_eq!(
            from.children(t).len(),
            children.len(),
            "an imported node keeps the number of children of {t:?}"
        );
        let kind = from.kind(t).with_children(children).with_vars(&mut rename);
        self.kept(kind, from.ty(t))
    }

    /// Returns the type `kind`'s children give it, or `None` when its typing rule leaves the
    /// type open.
    fn computed(&self, kind: &Kind) -> Result<Option<Ty>, String> {
        let ty = |t: &Tm| self.terms.ty(*t);
        let show = |t: &Ty| self.schema.show_ty(t);
        let change = Some(Ty::Change);
        Ok(match kind {
            Kind::Var(x) => Some(self.terms.var_info(*x).ty.clone()),
            Kind::Field(e, l) => Some(self.field_ty(ty(e), l)?),
            Kind::App(f, _) => match ty(f) {
                Ty::Fn(_, result) => Some(result.as_ref().clone()),
                other => return Err(format!("an application of `{}`", show(other))),
            },
            Kind::Call {
                func, type_args, ..
            } => Some(self.schema.function(*func).result.subst(type_args)),
            Kind::Caller => Some(self.data(self.schema.principal, Vec::new())),
            Kind::KeyOf(r, _)
            | Kind::Fresh(Generator::Sequence | Generator::Uuidv7, r)
            | Kind::Placeholder {
                generator: Generator::Sequence | Generator::Uuidv7,
                rel: r,
                ..
            } => Some(Ty::Key {
                rel: RelRef::Declared(*r),
                ctors: None,
            }),
            Kind::Fresh(Generator::Clock, _)
            | Kind::Placeholder {
                generator: Generator::Clock,
                ..
            } => Some(Ty::Base(Base::Instant)),
            Kind::Deref { key, admits, .. } => match ty(key) {
                Ty::Key {
                    rel: RelRef::Declared(r),
                    ..
                } => Some(self.row(*r, admits.clone())),
                other => return Err(format!("a dereference of `{}`", show(other))),
            },
            Kind::Tag(e) => match ty(e) {
                Ty::Data { def, args, .. } => Some(Ty::Tag(Box::new(Ty::Data {
                    def: *def,
                    args: args.clone(),
                    ctors: None,
                }))),
                other => return Err(format!("`tag` of `{}`", show(other))),
            },
            Kind::Try(e) => Some(self.prelude(
                "Result",
                vec![ty(e).clone(), self.prelude("Fault", Vec::new())],
            )),
            Kind::Rel(r, s) | Kind::Transition(_, r, s) => {
                Some(Ty::Set(Box::new(self.row(*r, s.clone()))))
            }
            Kind::Restrict(e, s) => {
                let Some(el) = elem(ty(e)) else {
                    return Err(format!("a restriction of `{}`", show(ty(e))));
                };
                Some(Ty::Set(Box::new(restrict(self.schema, el, s.clone()))))
            }
            Kind::Agg {
                op, source, body, ..
            } => Some(self.aggregate(*op, ty(source), ty(body))),
            Kind::IsEmpty(_) | Kind::Exists(_) => Some(self.prelude("Bool", Vec::new())),
            Kind::Group { var, by, .. } => Some(Ty::Set(Box::new(Ty::Record(vec![
                ("key".into(), ty(by).clone()),
                (
                    "rows".into(),
                    Ty::NonEmpty(Box::new(self.terms.var_info(*var).ty.clone())),
                ),
            ])))),
            Kind::Insert(..)
            | Kind::Delete(..)
            | Kind::Replace(..)
            | Kind::Upsert { .. }
            | Kind::Rejects { .. }
            | Kind::Par(_)
            | Kind::Fail(_)
            | Kind::Skip => change,
            Kind::Const(_)
            | Kind::Prim(..)
            | Kind::Record(_)
            | Kind::Ctor { .. }
            | Kind::Case { .. }
            | Kind::If(..)
            | Kind::Let(..)
            | Kind::Lam(..)
            | Kind::Key(_)
            | Kind::EmptySet
            | Kind::Singleton(_)
            | Kind::Union(..)
            | Kind::For(..)
            | Kind::Top { .. }
            | Kind::Fix(..)
            | Kind::Only(_)
            | Kind::At { .. }
            | Kind::Present { .. }
            | Kind::Reject(_)
            | Kind::Unreachable
            | Kind::LetBang(..)
            | Kind::Block(..)
            | Kind::Transact(..) => None,
        })
    }

    /// Returns the type of aggregate `op` over a source of type `source`, its body of type `body`.
    fn aggregate(&self, op: Agg, source: &Ty, body: &Ty) -> Ty {
        let partial = |r: &Ty| match source {
            Ty::NonEmpty(_) => r.clone(),
            _ => self.prelude("Option", vec![r.clone()]),
        };
        match op {
            Agg::Count => Ty::Base(Base::I64),
            Agg::Sum => body.clone(),
            Agg::Min | Agg::Max | Agg::Avg => partial(body),
            Agg::User(a) => partial(&self.schema.aggregates[a as usize].result),
        }
    }

    fn data(&self, def: crate::types::TypeId, args: Vec<Ty>) -> Ty {
        Ty::Data {
            def,
            args,
            ctors: None,
        }
    }

    fn prelude(&self, name: &str, args: Vec<Ty>) -> Ty {
        self.data(self.schema.prelude_type(name), args)
    }

    fn row(&self, rel: RelId, ctors: Option<CtorSet>) -> Ty {
        match &self.schema.relation(rel).elem {
            Ty::Data { def, args, .. } => Ty::Data {
                def: *def,
                args: args.clone(),
                ctors: ctors.and_then(|s| self.schema.normalize_ctors(*def, s)),
            },
            t => t.clone(),
        }
    }

    fn field_ty(&self, row: &Ty, l: &str) -> Result<Ty, String> {
        let fs = fields(self.schema, row);
        let Some((_, ft)) = fs.iter().find(|(n, _)| n == l) else {
            return Err(format!(
                "`{}` has no field `{l}` every constructor holds",
                self.schema.show_ty(row)
            ));
        };
        Ok(match (ft, row) {
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
        })
    }

    pub fn var(&mut self, x: Var) -> Tm {
        self.typed(Kind::Var(x))
    }

    pub fn constant(&mut self, c: Const, ty: Ty) -> Tm {
        self.put(Kind::Const(c), ty)
    }

    pub fn prim(&mut self, p: Prim, args: Vec<Tm>, ty: Ty) -> Tm {
        self.put(Kind::Prim(p, args), ty)
    }

    pub fn record(&mut self, mut fields: Vec<(String, Tm)>, ty: Ty) -> Tm {
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        self.put(Kind::Record(fields), ty)
    }

    /// Returns the empty record, typed `Unit`.
    pub fn unit(&mut self) -> Tm {
        self.put(Kind::Record(Vec::new()), Ty::Unit)
    }

    pub fn field(&mut self, e: Tm, l: impl Into<String>) -> Tm {
        self.typed(Kind::Field(e, l.into()))
    }

    /// Returns the field `l` of `row`, read as a field its row's `with` copies.
    pub fn transport_field(&mut self, row: Tm, l: impl Into<String>) -> Tm {
        let t = self.field(row, l);
        self.terms.transport_fields.insert(t);
        t
    }

    pub fn ctor(&mut self, ctor: u32, mut fields: Vec<(String, Tm)>, ty: Ty) -> Tm {
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        self.put(Kind::Ctor { ctor, fields }, ty)
    }

    pub fn case(&mut self, scrutinee: Tm, mut arms: Vec<(u32, Var, Tm)>, ty: Ty) -> Tm {
        arms.sort_by_key(|(c, _, _)| *c);
        self.put(Kind::Case { scrutinee, arms }, ty)
    }

    pub fn if_(&mut self, c: Tm, a: Tm, b: Tm, ty: Ty) -> Tm {
        self.put(Kind::If(c, a, b), ty)
    }

    pub fn let_(&mut self, x: Var, e: Tm, b: Tm, ty: Ty) -> Tm {
        self.put(Kind::Let(x, e, b), ty)
    }

    pub fn lam(&mut self, x: Var, b: Tm, ty: Ty) -> Tm {
        self.put(Kind::Lam(x, b), ty)
    }

    pub fn app(&mut self, f: Tm, a: Tm) -> Tm {
        self.typed(Kind::App(f, a))
    }

    pub fn call(&mut self, func: FnId, type_args: Vec<Ty>, args: Vec<Tm>) -> Tm {
        self.typed(Kind::Call {
            func,
            type_args,
            args,
        })
    }

    pub fn caller(&mut self) -> Tm {
        self.typed(Kind::Caller)
    }

    pub fn key(&mut self, e: Tm, r: RelId) -> Tm {
        let ty = match self.terms.ty(e) {
            Ty::Data { ctors, .. } => Ty::Key {
                rel: RelRef::Declared(r),
                ctors: ctors.clone(),
            },
            other => {
                let reason = format!("`key` of `{}`", self.schema.show_ty(other));
                self.untypable(&Kind::Key(e), &reason)
            }
        };
        self.put(Kind::Key(e), ty)
    }

    pub fn key_of(&mut self, r: RelId, e: Tm) -> Tm {
        self.typed(Kind::KeyOf(r, e))
    }

    /// Returns the row `key` names, admitting every constructor its type names.
    pub fn deref(&mut self, key: Tm, reference: bool) -> Tm {
        let admits = match self.terms.ty(key) {
            Ty::Key { ctors, .. } => ctors.clone(),
            _ => None,
        };
        self.deref_within(key, reference, admits)
    }

    /// Returns the row `key` names, admitting the constructors of `admits`, typed at the row type
    /// of the key's relation at `admits`.
    ///
    /// Preconditions:
    /// - `admits` is within the constructor set of the type of `key`.
    pub fn deref_within(&mut self, key: Tm, reference: bool, admits: Option<CtorSet>) -> Tm {
        let admits = match self.terms.ty(key) {
            Ty::Key {
                rel: RelRef::Declared(r),
                ..
            } => self.selection(*r, admits),
            _ => admits,
        };
        self.typed(Kind::Deref {
            key,
            reference,
            admits,
        })
    }

    /// Returns `admits` less the constructors the type of `key` excludes.
    fn within_key(&self, key: Tm, admits: Option<CtorSet>) -> Option<CtorSet> {
        let Ty::Key {
            rel: RelRef::Declared(r),
            ctors: Some(named),
        } = self.terms.ty(key)
        else {
            return admits;
        };
        let kept = match admits {
            Some(mut s) => {
                s.retain(|c| named.contains(c));
                s
            }
            None => named.clone(),
        };
        let def = self.schema.relation(*r).elem_def;
        self.schema.normalize_ctors(def, kept)
    }

    pub fn tag(&mut self, e: Tm) -> Tm {
        self.typed(Kind::Tag(e))
    }

    pub fn try_(&mut self, e: Tm) -> Tm {
        self.typed(Kind::Try(e))
    }

    /// Returns the rows of `r` at `ctors`, a set naming every constructor read as `None`.
    pub fn rel(&mut self, r: RelId, ctors: Option<CtorSet>) -> Tm {
        let ctors = self.selection(r, ctors);
        self.typed(Kind::Rel(r, ctors))
    }

    pub fn transition(&mut self, t: Transition, r: RelId, ctors: Option<CtorSet>) -> Tm {
        let ctors = self.selection(r, ctors);
        self.typed(Kind::Transition(t, r, ctors))
    }

    fn selection(&self, r: RelId, ctors: Option<CtorSet>) -> Option<CtorSet> {
        let def = self.schema.relation(r).elem_def;
        ctors.and_then(|s| self.schema.normalize_ctors(def, s))
    }

    pub fn restrict(&mut self, e: Tm, s: CtorSet) -> Tm {
        self.typed(Kind::Restrict(e, s))
    }

    pub fn empty_set(&mut self, ty: Ty) -> Tm {
        self.put(Kind::EmptySet, ty)
    }

    pub fn singleton(&mut self, e: Tm, ty: Ty) -> Tm {
        self.put(Kind::Singleton(e), ty)
    }

    pub fn union(&mut self, a: Tm, b: Tm, ty: Ty) -> Tm {
        self.put(Kind::Union(a, b), ty)
    }

    pub fn for_(&mut self, x: Var, src: Tm, b: Tm, ty: Ty) -> Tm {
        self.put(Kind::For(x, src, b), ty)
    }

    pub fn is_empty(&mut self, e: Tm) -> Tm {
        self.typed(Kind::IsEmpty(e))
    }

    pub fn exists(&mut self, e: Tm) -> Tm {
        self.typed(Kind::Exists(e))
    }

    pub fn agg(&mut self, op: Agg, var: Var, source: Tm, body: Tm) -> Tm {
        self.typed(Kind::Agg {
            op,
            var,
            source,
            body,
        })
    }

    pub fn group(&mut self, var: Var, source: Tm, by: Tm) -> Tm {
        self.typed(Kind::Group { var, source, by })
    }

    pub fn top(&mut self, count: Tm, keys: Vec<(Tm, bool)>, var: Var, source: Tm, ty: Ty) -> Tm {
        self.put(
            Kind::Top {
                count,
                keys,
                var,
                source,
            },
            ty,
        )
    }

    pub fn fix(&mut self, binds: Vec<(Var, Tm)>, b: Tm, ty: Ty) -> Tm {
        self.put(Kind::Fix(binds, b), ty)
    }

    pub fn fresh(&mut self, g: Generator, r: RelId) -> Tm {
        self.typed(Kind::Fresh(g, r))
    }

    pub fn placeholder(
        &mut self,
        occurrence: u32,
        generator: Generator,
        rel: RelId,
        scope: Vec<Tm>,
    ) -> Tm {
        self.typed(Kind::Placeholder {
            occurrence,
            generator,
            rel,
            scope,
        })
    }

    pub fn at(&mut self, instant: bool, point: Tm, pins: Option<u64>, body: Tm, ty: Ty) -> Tm {
        self.put(
            Kind::At {
                instant,
                point,
                pins,
                body,
            },
            ty,
        )
    }

    pub fn present(
        &mut self,
        var: Var,
        source: Tm,
        keys: Vec<(Tm, bool)>,
        take: Option<Tm>,
        after: Option<Tm>,
        ty: Ty,
    ) -> Tm {
        self.put(
            Kind::Present {
                var,
                source,
                keys,
                take,
                after,
            },
            ty,
        )
    }

    pub fn reject(&mut self, m: Tm, ty: Ty) -> Tm {
        self.put(Kind::Reject(m), ty)
    }

    pub fn unreachable(&mut self, ty: Ty) -> Tm {
        self.put(Kind::Unreachable, ty)
    }

    pub fn skip(&mut self) -> Tm {
        self.typed(Kind::Skip)
    }

    pub fn insert(&mut self, r: RelId, rows: Tm) -> Tm {
        self.typed(Kind::Insert(r, rows))
    }

    pub fn delete(&mut self, r: RelId, rows: Tm) -> Tm {
        self.typed(Kind::Delete(r, rows))
    }

    pub fn replace(&mut self, r: RelId, rows: Tm, f: Tm) -> Tm {
        self.typed(Kind::Replace(r, rows, f))
    }

    pub fn upsert(
        &mut self,
        rel: RelId,
        rows: Tm,
        on: Conflict,
        (old, new): (Var, Var),
        otherwise: Tm,
        single: bool,
    ) -> Tm {
        self.typed(Kind::Upsert {
            rel,
            rows,
            on,
            old,
            new,
            otherwise,
            single,
        })
    }

    pub fn rejects(&mut self, into: RelId, insert: Tm) -> Tm {
        self.typed(Kind::Rejects { into, insert })
    }

    pub fn par(&mut self, ws: Vec<Tm>) -> Tm {
        self.typed(Kind::Par(ws))
    }

    pub fn fail(&mut self, m: Tm) -> Tm {
        self.typed(Kind::Fail(m))
    }

    pub fn let_bang(&mut self, x: Var, w: Tm, k: Tm, ty: Ty) -> Tm {
        self.put(Kind::LetBang(x, w, k), ty)
    }

    pub fn block(&mut self, w: Tm, k: Tm, ty: Ty) -> Tm {
        self.put(Kind::Block(w, k), ty)
    }

    pub fn transact(&mut self, b: Tm, options: TxnOptions, ty: Ty) -> Tm {
        self.put(Kind::Transact(b, options), ty)
    }
}
