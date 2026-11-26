use std::collections::BTreeSet;

use crate::ast::{
    Arm, CallArgs, Clause, Comprehension, CtorSel, Expr, ExprKind, Generator, Ident, KeyArgs,
    LambdaParams, Pattern, PatternKind, Payload, QName, RecordExpr, RelationRef, Type, TypeArg,
    TypeKind,
};

/// A piece of an element of a unique or index: a name, as what it resolves to; a name bound
/// inside the element, as the place of its binder among the element's binders in scope; or a
/// token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Atom<N> {
    Name(N),
    Bound(usize),
    Token(String),
}

/// An element of a unique or index as its resolved tree, written out node by node.
pub(crate) type Tree<N> = Vec<Atom<N>>;

/// Where a refusal of two uniques or indexes that are one stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Said {
    /// An operator of a migration made the second while the relation variable held the first.
    Migration,
    /// One declaration of the relation variable writes both.
    Declaration,
}

/// Returns the refusal of `x` and `y`, two uniques or indexes of relation variable `rel` that
/// are one, as `said` words it.
pub(crate) fn one_refusal(said: Said, rel: &str, x: &str, y: &str) -> String {
    match said {
        Said::Migration => {
            format!("`{x}` of `{rel}` is one with `{y}`, which `{rel}` already holds")
        }
        Said::Declaration => format!("`{rel}` declares `{x}` and `{y}`, which are one"),
    }
}

/// Returns element `e` as its tree: each node a token of its kind with its parts after it;
/// parentheses dropped; a name bound inside the element, a field a pattern names alone among
/// them, as the place of its binder; a
/// constructor as that constructor however qualified, `T.A`, `A` and `app::A` alike; and any
/// other name as what `name` resolves it to, its text when that is `None`. Returns `None` for an
/// element holding a write, an elision or a delete action, which no index admits.
pub(crate) fn tree<N>(e: &Expr, name: &mut dyn FnMut(&QName) -> Option<N>) -> Option<Tree<N>> {
    placed_tree(e, &mut |q, _| name(q))
}

/// Where a name of an element stands, which decides what kind of object it can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// A value: a function, a relation variable, or a field of the row.
    Value,
    /// A callee: a value, a type or a constructor.
    Callee,
    /// A field after a `.`, in a record or in a pattern.
    Member,
    /// A constructor.
    Ctor,
    /// A constructor or a named set of a selection.
    Sel,
    /// A type.
    Type,
    /// A relation variable.
    Relation,
}

/// Returns element `e` as [`tree`] does, `name` told where each name stands.
pub(crate) fn placed_tree<N>(
    e: &Expr,
    name: &mut dyn FnMut(&QName, Place) -> Option<N>,
) -> Option<Tree<N>> {
    let mut w = Walk {
        out: Vec::new(),
        bound: Vec::new(),
        name,
        visit: None,
    };
    w.expr(e)?;
    Some(w.out)
}

/// Call `f` on `e` and, for each expression `f` returns true for, on each expression that one
/// holds, outermost first.
pub(crate) fn each_expr(e: &Expr, f: &mut dyn FnMut(&Expr) -> bool) {
    let mut w: Walk<()> = Walk {
        out: Vec::new(),
        bound: Vec::new(),
        name: &mut |_, _| None,
        visit: Some(f),
    };
    w.expr(e);
}

/// Returns whether two uniques of one relation variable are one: their scopes reach the same
/// constructors, every constructor counted alike however written, and their elements' trees
/// are alike, `name` deciding whether two names are one.
pub(crate) fn same_unique<C: Ord, A, B>(
    a: (&BTreeSet<C>, &[Tree<A>]),
    b: (&BTreeSet<C>, &[Tree<B>]),
    name: &dyn Fn(&A, &B) -> bool,
) -> bool {
    a.0 == b.0 && alike(a.1, b.1, name)
}

/// Returns whether two indexes of one relation variable are one: they are of one kind,
/// interval or not, their `in` names the same constructors and named sets in any order or both
/// name none, and their elements' trees are alike, `name` deciding whether two names are one.
///
/// Preconditions:
/// - Neither is a unique.
pub(crate) fn same_index<P: Ord, A, B>(
    a: (bool, Option<&BTreeSet<P>>, &[Tree<A>]),
    b: (bool, Option<&BTreeSet<P>>, &[Tree<B>]),
    name: &dyn Fn(&A, &B) -> bool,
) -> bool {
    a.0 == b.0 && a.1 == b.1 && alike(a.2, b.2, name)
}

fn alike<A, B>(a: &[Tree<A>], b: &[Tree<B>], name: &dyn Fn(&A, &B) -> bool) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.len() == y.len()
                && x.iter().zip(y).all(|p| match p {
                    (Atom::Name(m), Atom::Name(n)) => name(m, n),
                    (Atom::Bound(m), Atom::Bound(n)) => m == n,
                    (Atom::Token(m), Atom::Token(n)) => m == n,
                    _ => false,
                })
        })
}

struct Walk<'a, N> {
    out: Vec<Atom<N>>,
    bound: Vec<String>,
    name: &'a mut dyn FnMut(&QName, Place) -> Option<N>,
    /// Called on each expression before its parts, which are walked only when it returns true.
    visit: Option<&'a mut dyn FnMut(&Expr) -> bool>,
}

impl<N> Walk<'_, N> {
    fn tag(&mut self, t: &str) {
        self.out.push(Atom::Token(t.to_string()));
    }

    fn count(&mut self, n: usize) {
        self.out.push(Atom::Token(n.to_string()));
    }

    fn text(&mut self, i: &Ident) {
        self.out.push(Atom::Token(i.text.clone()));
    }

    fn bind(&mut self, i: &Ident) {
        self.bound.push(i.text.clone());
    }

    fn scoped(&mut self, f: impl FnOnce(&mut Self) -> Option<()>) -> Option<()> {
        let depth = self.bound.len();
        f(self)?;
        self.bound.truncate(depth);
        Some(())
    }

    fn value(&mut self, q: &QName) {
        self.bound_or(q, Place::Value);
    }

    fn callee(&mut self, q: &QName) {
        self.bound_or(q, Place::Callee);
    }

    fn bound_or(&mut self, q: &QName, place: Place) {
        let bound = self.bound.iter().rposition(|b| *b == q.name.text);
        match bound {
            Some(at) if q.module.is_none() => self.out.push(Atom::Bound(at)),
            _ => self.named(q, place),
        }
    }

    fn named(&mut self, q: &QName, place: Place) {
        match (self.name)(q, place) {
            Some(n) => self.out.push(Atom::Name(n)),
            None => {
                let text = match &q.module {
                    Some(m) => format!("{}::{}", m.text, q.name.text),
                    None => q.name.text.clone(),
                };
                self.out.push(Atom::Token(text));
            }
        }
    }

    fn member(&mut self, i: &Ident) {
        self.placed(i, Place::Member);
    }

    fn placed(&mut self, i: &Ident, place: Place) {
        let q = QName {
            module: None,
            name: i.clone(),
        };
        self.named(&q, place);
    }

    fn ctor(&mut self, path: &QName, ctor: Option<&Ident>) {
        let Some(c) = ctor else {
            return self.named(path, Place::Ctor);
        };
        let q = QName {
            module: None,
            name: c.clone(),
        };
        match (self.name)(&q, Place::Ctor) {
            Some(n) => self.out.push(Atom::Name(n)),
            None => {
                self.named(path, Place::Type);
                self.text(c);
            }
        }
    }

    fn sel(&mut self, s: &CtorSel) {
        let cs = match s {
            CtorSel::One(c) => std::slice::from_ref(c),
            CtorSel::Many(cs) => cs.as_slice(),
        };
        self.count(cs.len());
        cs.iter().for_each(|c| self.placed(c, Place::Sel));
    }

    fn exprs(&mut self, es: &[Expr]) -> Option<()> {
        self.count(es.len());
        es.iter().try_for_each(|e| self.expr(e))
    }

    fn opt(&mut self, e: Option<&Expr>) -> Option<()> {
        match e {
            Some(e) => {
                self.tag("some");
                self.expr(e)
            }
            None => {
                self.tag("none");
                Some(())
            }
        }
    }

    fn pins(&mut self, pins: Option<&crate::ast::Version>) {
        self.out.push(Atom::Token(format!(
            "{:?}",
            pins.map(|v| (v.number, &v.hash))
        )));
    }

    fn expr(&mut self, e: &Expr) -> Option<()> {
        if let Some(v) = self.visit.as_mut()
            && !v(e)
        {
            return Some(());
        }
        match &e.kind {
            ExprKind::Lit(l) => {
                self.tag("lit");
                self.out.push(Atom::Token(format!("{l:?}")));
            }
            ExprKind::Name(q) => {
                self.tag("name");
                self.value(q);
            }
            ExprKind::Call { callee, args, pins } => {
                self.tag("call");
                self.callee(callee);
                self.pins(pins.as_ref());
                self.args(args)?;
            }
            ExprKind::TypeCall { ty, func, args } => {
                self.tag("typecall");
                self.ty(ty)?;
                self.text(func);
                self.args(args)?;
            }
            ExprKind::Ctor { path, ctor, record } => {
                self.tag("ctor");
                self.ctor(path, ctor.as_ref());
                match record {
                    Some(r) => self.record(r)?,
                    None => self.tag("none"),
                }
            }
            ExprKind::Record(r) => self.record(r)?,
            ExprKind::Set(es) => {
                self.tag("set");
                self.exprs(es)?;
            }
            ExprKind::Unit => self.tag("unit"),
            ExprKind::Unary(op, x) => {
                self.out.push(Atom::Token(format!("{op:?}")));
                self.expr(x)?;
            }
            ExprKind::Binary(op, a, b) => {
                self.out.push(Atom::Token(format!("{op:?}")));
                self.expr(a)?;
                self.expr(b)?;
            }
            ExprKind::Field(x, f) => {
                self.tag("field");
                self.expr(x)?;
                self.member(f);
            }
            ExprKind::Select(x, s) => {
                self.tag("select");
                self.expr(x)?;
                self.sel(s);
            }
            ExprKind::Backlink(x, r, f) => {
                self.tag("backlink");
                self.expr(x)?;
                self.relation(r);
                self.member(f);
            }
            ExprKind::KeyCall(x, args) => {
                self.tag("keycall");
                self.expr(x)?;
                self.key_args(args)?;
            }
            ExprKind::Comprehension(c) => {
                self.tag("comprehension");
                self.scoped(|w| w.comprehension(c))?;
            }
            ExprKind::Match(s, arms) => {
                self.tag("match");
                self.opt(s.as_deref())?;
                self.count(arms.len());
                arms.iter().try_for_each(|a| self.arm(a))?;
            }
            ExprKind::Fix(binds, body) => {
                self.tag("fix");
                self.scoped(|w| {
                    w.count(binds.len());
                    binds.iter().for_each(|(b, _)| w.bind(b));
                    binds.iter().try_for_each(|(_, x)| w.expr(x))?;
                    w.expr(body)
                })?;
            }
            ExprKind::Lambda(params, body) => {
                self.tag("lambda");
                self.scoped(|w| {
                    w.params(params)?;
                    w.expr(body)
                })?;
            }
            ExprKind::Exists(gens, filter) => {
                self.tag("exists");
                self.scoped(|w| {
                    w.generators(gens)?;
                    w.opt(filter.as_deref())
                })?;
            }
            ExprKind::Top { count, keys, body } => {
                self.tag("top");
                self.expr(count)?;
                self.scoped(|w| {
                    w.comprehension(body)?;
                    w.count(keys.len());
                    keys.iter().try_for_each(|k| {
                        w.tag(if k.desc { "desc" } else { "asc" });
                        w.expr(&k.expr)
                    })
                })?;
            }
            ExprKind::If(c, a, b) => {
                self.tag("if");
                self.expr(c)?;
                self.expr(a)?;
                self.expr(b)?;
            }
            ExprKind::At {
                instant,
                point,
                pins,
                body,
            } => {
                self.tag(if *instant { "at instant" } else { "at" });
                self.expr(point)?;
                self.pins(pins.as_ref());
                self.expr(body)?;
            }
            ExprKind::Reject(x) => {
                self.tag("reject");
                self.expr(x)?;
            }
            ExprKind::Write(_) | ExprKind::Elided => return None,
        }
        Some(())
    }

    fn key_args(&mut self, args: &KeyArgs) -> Option<()> {
        match args {
            KeyArgs::Value(v) => self.expr(v),
            KeyArgs::Named(ns) => {
                self.count(ns.len());
                ns.iter().try_for_each(|(n, v)| {
                    self.member(n);
                    self.expr(v)
                })
            }
        }
    }

    fn params(&mut self, params: &LambdaParams) -> Option<()> {
        match params {
            LambdaParams::One(p) => self.bind(p),
            LambdaParams::List(ps) => {
                self.count(ps.len());
                for p in ps {
                    self.ty(&p.ty)?;
                    self.bind(&p.name);
                }
            }
        }
        Some(())
    }

    fn args(&mut self, a: &CallArgs) -> Option<()> {
        match a {
            CallArgs::List(es) => {
                self.tag("list");
                self.exprs(es)
            }
            CallArgs::Aggregate {
                expr,
                generators,
                filter,
            } => {
                self.tag("aggregate");
                self.scoped(|w| {
                    w.generators(generators)?;
                    w.opt(filter.as_deref())?;
                    w.expr(expr)
                })
            }
        }
    }

    fn record(&mut self, r: &RecordExpr) -> Option<()> {
        let fields = match r {
            RecordExpr::Fields(fs, _) => {
                self.tag("record");
                fs
            }
            RecordExpr::With(x, fs, _) => {
                self.tag("with");
                self.expr(x)?;
                fs
            }
            RecordExpr::Elided(_) => return None,
        };
        self.count(fields.len());
        fields.iter().try_for_each(|(f, v)| {
            self.member(f);
            self.expr(v)
        })
    }

    fn relation(&mut self, r: &RelationRef) {
        self.named(&r.relation, Place::Relation);
        match &r.sel {
            Some(s) => self.sel(s),
            None => self.tag("none"),
        }
    }

    fn generators(&mut self, gens: &[Generator]) -> Option<()> {
        self.count(gens.len());
        gens.iter().try_for_each(|g| {
            self.expr(&g.source)?;
            self.bind(&g.binder);
            Some(())
        })
    }

    fn clauses(&mut self, cs: &[Clause]) -> Option<()> {
        self.count(cs.len());
        cs.iter().try_for_each(|c| match c {
            Clause::From(gens) => {
                self.tag("from");
                self.generators(gens)
            }
            Clause::Where(x) => {
                self.tag("where");
                self.expr(x)
            }
        })
    }

    fn comprehension(&mut self, c: &Comprehension) -> Option<()> {
        self.clauses(&c.clauses)?;
        match &c.group {
            Some((g, after)) => {
                self.tag("group");
                self.expr(&g.value)?;
                self.expr(&g.by)?;
                self.bind(&g.into);
                self.clauses(after)?;
            }
            None => self.tag("none"),
        }
        self.expr(&c.select)?;
        match &c.order {
            Some(o) => {
                self.tag("order");
                self.count(o.keys.len());
                for k in &o.keys {
                    self.tag(if k.desc { "desc" } else { "asc" });
                    self.expr(&k.expr)?;
                }
                self.opt(o.take.as_ref())?;
                self.opt(o.after.as_ref())
            }
            None => {
                self.tag("none");
                Some(())
            }
        }
    }

    fn arm(&mut self, a: &Arm) -> Option<()> {
        self.scoped(|w| {
            w.count(a.patterns.len());
            a.patterns.iter().for_each(|p| w.pattern(p));
            w.opt(a.guard.as_ref())?;
            w.expr(&a.body)
        })
    }

    fn pattern(&mut self, p: &Pattern) {
        match &p.kind {
            PatternKind::Wildcard => self.tag("_"),
            PatternKind::Bind(b) => {
                self.tag("bind");
                self.bind(b);
            }
            PatternKind::Ctor {
                path,
                ctor,
                payload,
            } => {
                self.tag("ctor");
                self.ctor(path, ctor.as_ref());
                match payload {
                    None => self.tag("none"),
                    Some(Payload::Wildcard) => self.tag("_"),
                    Some(Payload::Bind(b)) => {
                        self.tag("bind");
                        self.bind(b);
                    }
                    Some(Payload::Record(fs)) => {
                        self.tag("record");
                        self.count(fs.len());
                        for f in fs {
                            self.member(&f.field);
                            match &f.pattern {
                                Some(p) => self.pattern(p),
                                None => {
                                    self.tag("bind");
                                    self.bind(&f.field);
                                }
                            }
                        }
                    }
                }
            }
            PatternKind::Lit { neg, lit } => {
                self.tag(if *neg { "-lit" } else { "lit" });
                self.out.push(Atom::Token(format!("{lit:?}")));
            }
        }
    }

    fn ty(&mut self, t: &Type) -> Option<()> {
        match &t.kind {
            TypeKind::Named { name, sel, args } => {
                self.tag("type");
                self.named(name, Place::Type);
                match sel {
                    Some(s) => self.sel(s),
                    None => self.tag("none"),
                }
                self.count(args.len());
                args.iter().try_for_each(|a| match a {
                    TypeArg::Type(t) => self.ty(t),
                    TypeArg::Int(n, _) => {
                        self.out.push(Atom::Token(format!("int {n}")));
                        Some(())
                    }
                })
            }
            TypeKind::Var(v) => {
                self.tag("var");
                self.text(v);
                Some(())
            }
            TypeKind::Key(_, r) => {
                self.tag("Key");
                self.relation(r);
                Some(())
            }
            TypeKind::Unit => {
                self.tag("unit");
                Some(())
            }
            TypeKind::Record(fs) => self.field_types(fs),
            TypeKind::Sum(cs) => {
                self.tag("sum");
                self.count(cs.len());
                cs.iter().try_for_each(|c| {
                    self.text(&c.name);
                    match &c.record {
                        Some(fs) => self.field_types(fs),
                        None => {
                            self.tag("none");
                            Some(())
                        }
                    }
                })
            }
            TypeKind::Fn(a, b) => {
                self.tag("fn");
                self.ty(a)?;
                self.ty(b)
            }
        }
    }

    fn field_types(&mut self, fs: &[crate::ast::FieldType]) -> Option<()> {
        self.tag("record");
        self.count(fs.len());
        fs.iter().try_for_each(|f| {
            if f.on_delete.is_some() {
                return None;
            }
            self.member(&f.name);
            self.ty(&f.ty)
        })
    }
}
