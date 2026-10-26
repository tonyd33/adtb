use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use adtb_lang::kernel::{Kind, Terms, Tm, Var};

/// Returns `kind` with its children replaced by `cs`, given in the order of `Terms::children`.
pub fn with_children(kind: &Kind, cs: &[Tm]) -> Kind {
    let mut it = cs.iter().copied();
    let mut next = || it.next().expect("one child per child position");
    match kind {
        Kind::Var(_)
        | Kind::Const(_)
        | Kind::Caller
        | Kind::Rel(..)
        | Kind::Transition(..)
        | Kind::EmptySet
        | Kind::Fresh(..)
        | Kind::Unreachable
        | Kind::Skip => kind.clone(),
        Kind::Prim(p, xs) => Kind::Prim(*p, xs.iter().map(|_| next()).collect()),
        Kind::Par(xs) => Kind::Par(xs.iter().map(|_| next()).collect()),
        Kind::Record(fs) => Kind::Record(fs.iter().map(|(n, _)| (n.clone(), next())).collect()),
        Kind::Ctor { ctor, fields } => Kind::Ctor {
            ctor: *ctor,
            fields: fields.iter().map(|(n, _)| (n.clone(), next())).collect(),
        },
        Kind::Field(_, l) => Kind::Field(next(), l.clone()),
        Kind::Key(_) => Kind::Key(next()),
        Kind::KeyOf(r, _) => Kind::KeyOf(*r, next()),
        Kind::Deref { reference, .. } => Kind::Deref {
            key: next(),
            reference: *reference,
        },
        Kind::Tag(_) => Kind::Tag(next()),
        Kind::Try(_) => Kind::Try(next()),
        Kind::Restrict(_, s) => Kind::Restrict(next(), s.clone()),
        Kind::Singleton(_) => Kind::Singleton(next()),
        Kind::IsEmpty(_) => Kind::IsEmpty(next()),
        Kind::Exists(_) => Kind::Exists(next()),
        Kind::Only(_) => Kind::Only(next()),
        Kind::Reject(_) => Kind::Reject(next()),
        Kind::Insert(r, _) => Kind::Insert(*r, next()),
        Kind::Delete(r, _) => Kind::Delete(*r, next()),
        Kind::Fail(_) => Kind::Fail(next()),
        Kind::Transact(_, o) => Kind::Transact(next(), *o),
        Kind::Rejects { into, .. } => Kind::Rejects {
            into: *into,
            insert: next(),
        },
        Kind::Lam(x, _) => Kind::Lam(*x, next()),
        Kind::Upsert {
            rel,
            on,
            old,
            new,
            single,
            ..
        } => Kind::Upsert {
            rel: *rel,
            rows: next(),
            on: *on,
            old: *old,
            new: *new,
            otherwise: next(),
            single: *single,
        },
        Kind::Case { arms, .. } => Kind::Case {
            scrutinee: next(),
            arms: arms.iter().map(|(c, v, _)| (*c, *v, next())).collect(),
        },
        Kind::If(..) => Kind::If(next(), next(), next()),
        Kind::Let(x, ..) => Kind::Let(*x, next(), next()),
        Kind::LetBang(x, ..) => Kind::LetBang(*x, next(), next()),
        Kind::Union(..) => Kind::Union(next(), next()),
        Kind::For(x, ..) => Kind::For(*x, next(), next()),
        Kind::Replace(r, ..) => Kind::Replace(*r, next(), next()),
        Kind::Block(..) => Kind::Block(next(), next()),
        Kind::App(..) => Kind::App(next(), next()),
        Kind::Call {
            func, type_args, ..
        } => Kind::Call {
            func: *func,
            type_args: type_args.clone(),
            args: cs.to_vec(),
        },
        Kind::Agg { op, var, .. } => Kind::Agg {
            op: *op,
            var: *var,
            source: next(),
            body: next(),
        },
        Kind::Group { var, .. } => Kind::Group {
            var: *var,
            source: next(),
            by: next(),
        },
        Kind::Top { keys, var, .. } => {
            let count = next();
            let source = next();
            Kind::Top {
                count,
                keys: keys.iter().map(|(_, d)| (next(), *d)).collect(),
                var: *var,
                source,
            }
        }
        Kind::Fix(binds, _) => Kind::Fix(binds.iter().map(|(x, _)| (*x, next())).collect(), next()),
        Kind::At { instant, .. } => Kind::At {
            instant: *instant,
            point: next(),
            body: next(),
        },
        Kind::Present {
            var,
            keys,
            take,
            after,
            ..
        } => Kind::Present {
            var: *var,
            source: next(),
            keys: keys.iter().map(|(_, d)| (next(), *d)).collect(),
            take: take.map(|_| next()),
            after: after.map(|_| next()),
        },
    }
}

/// Returns the variables `t` binds around each of its children, in the order of `Terms::children`.
fn bound(kind: &Kind, n: usize) -> Vec<Vec<Var>> {
    let mut out = vec![Vec::new(); n];
    match kind {
        Kind::Let(x, ..) | Kind::LetBang(x, ..) | Kind::For(x, ..) => out[1].push(*x),
        Kind::Lam(x, _) => out[0].push(*x),
        Kind::Upsert { old, new, .. } => out[1].extend([*old, *new]),
        Kind::Case { arms, .. } => {
            for (i, (_, v, _)) in arms.iter().enumerate() {
                out[i + 1].push(*v);
            }
        }
        Kind::Agg { var, .. } | Kind::Group { var, .. } => out[1].push(*var),
        Kind::Top { var, .. } => {
            for o in out.iter_mut().skip(2) {
                o.push(*var);
            }
        }
        Kind::Present { var, keys, .. } => {
            for o in out.iter_mut().skip(1).take(keys.len()) {
                o.push(*var);
            }
        }
        Kind::Fix(binds, _) => {
            let xs: Vec<Var> = binds.iter().map(|(x, _)| *x).collect();
            for o in &mut out {
                o.extend(&xs);
            }
        }
        _ => {}
    }
    out
}

/// Facts about terms of one arena, remembered per term.
#[derive(Debug, Default)]
pub struct Facts {
    free: HashMap<Tm, Rc<BTreeSet<Var>>>,
    scalar: HashMap<Tm, bool>,
    binders: HashMap<Tm, Rc<BTreeSet<Var>>>,
}

impl Facts {
    pub fn free(&mut self, terms: &Terms, t: Tm) -> Rc<BTreeSet<Var>> {
        if let Some(s) = self.free.get(&t) {
            return s.clone();
        }
        let kind = terms.kind(t);
        let mut out = BTreeSet::new();
        if let Kind::Var(v) = kind {
            out.insert(*v);
        }
        let cs = terms.children(t);
        for (c, b) in cs.iter().zip(bound(kind, cs.len())) {
            let f = self.free(terms, *c);
            out.extend(f.iter().filter(|v| !b.contains(v)));
        }
        let out = Rc::new(out);
        self.free.insert(t, out.clone());
        out
    }

    /// Returns every variable a binder inside `t` binds.
    pub fn binders(&mut self, terms: &Terms, t: Tm) -> Rc<BTreeSet<Var>> {
        if let Some(s) = self.binders.get(&t) {
            return s.clone();
        }
        let kind = terms.kind(t);
        let cs = terms.children(t);
        let mut out: BTreeSet<Var> = bound(kind, cs.len()).into_iter().flatten().collect();
        for c in cs {
            out.extend(self.binders(terms, c).iter());
        }
        let out = Rc::new(out);
        self.binders.insert(t, out.clone());
        out
    }

    pub fn mentions(&mut self, terms: &Terms, t: Tm, vs: &[Var]) -> bool {
        let f = self.free(terms, t);
        vs.iter().any(|v| f.contains(v))
    }

    /// Returns whether `t` is a scalar expression: it reads nothing and holds no comprehension.
    pub fn scalar(&mut self, terms: &Terms, t: Tm) -> bool {
        if let Some(&s) = self.scalar.get(&t) {
            return s;
        }
        let here = matches!(
            terms.kind(t),
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
                | Kind::Tag(_)
                | Kind::Try(_)
                | Kind::Singleton(_)
                | Kind::EmptySet
                | Kind::Union(..)
                | Kind::Restrict(..)
                | Kind::Reject(_)
                | Kind::Unreachable
        );
        let s = here && terms.children(t).into_iter().all(|c| self.scalar(terms, c));
        self.scalar.insert(t, s);
        s
    }
}
