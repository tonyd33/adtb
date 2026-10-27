use super::expr::{Binding, Comp, E, Step};
use super::world::{Field, T, World, value};
use crate::Rng;

/// Draws expressions over the relations of one world, naming variables freshly.
pub(super) struct Gen<'w> {
    pub(super) w: &'w World,
    pub(super) scope: Vec<(String, T)>,
    pub(super) next: usize,
}

/// A field a variable in scope reads: the path, its type and the relation it belongs to.
struct Read {
    path: E,
    ty: T,
    field: Option<(usize, String)>,
}

impl Gen<'_> {
    fn fresh(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}{}", self.next)
    }

    pub(super) fn within<R>(&mut self, gens: &[Binding], f: impl FnOnce(&mut Self) -> R) -> R {
        let depth = self.scope.len();
        for b in gens {
            self.scope
                .push((b.var.clone(), T::Row(b.rel, b.ctors.clone())));
        }
        let r = f(self);
        self.scope.truncate(depth);
        r
    }

    pub(super) fn select_ty(&mut self, rng: &mut Rng) -> T {
        let key = T::Key(rng.below(self.w.rels.len() as u64) as usize);
        match rng.below(11) {
            0 => T::Int,
            1 => T::Text,
            2 => T::Bool,
            3 => key,
            4 => T::Rec(vec![("a".into(), T::Int), ("b".into(), key)]),
            5 => T::Rec(vec![("a".into(), T::Text), ("b".into(), T::Shade)]),
            6 => T::Rec(vec![("a".into(), T::Float), ("b".into(), T::FloatSum)]),
            7 => T::Rec(vec![("a".into(), key), ("b".into(), T::FloatSum)]),
            8 => T::Set(Box::new(T::Int)),
            _ => T::Rec(vec![
                ("a".into(), key),
                ("b".into(), T::Set(Box::new(T::Int))),
            ]),
        }
    }

    fn binding(&mut self, rng: &mut Rng) -> Binding {
        let rel = rng.below(self.w.rels.len() as u64) as usize;
        let n = self.w.rels[rel].ctors.len();
        let all: Vec<usize> = (0..n).collect();
        let ctors = if n > 1 && rng.below(3) == 0 {
            let picked: Vec<usize> = all.iter().copied().filter(|_| rng.below(2) == 0).collect();
            if picked.is_empty() {
                vec![rng.below(n as u64) as usize]
            } else {
                picked
            }
        } else {
            all
        };
        Binding {
            var: self.fresh("v"),
            rel,
            ctors,
        }
    }

    /// Returns a comprehension of `sel` over at most `gens` generators, its filter a conjunction
    /// of joins between its generators and the variables in scope, equalities of a field and a
    /// value the load writes, and a drawn predicate.
    pub(super) fn comp(&mut self, rng: &mut Rng, sel: &T, d: u32, gens: u64) -> Comp {
        let gens: Vec<Binding> = (0..=rng.below(gens)).map(|_| self.binding(rng)).collect();
        self.within(&gens.clone(), |g| {
            let mut conds = Vec::new();
            for (i, b) in gens.iter().enumerate() {
                let chance = if i == 0 { 2 } else { 4 };
                if rng.below(chance) != 0
                    && let Some(j) = g.join(rng, &b.var)
                {
                    conds.push(j);
                }
            }
            if rng.below(2) == 0
                && let Some(p) = g.point(rng, &gens)
            {
                conds.push(p);
            }
            if rng.below(2) == 0 {
                conds.push(g.boolean(rng, d));
            }
            let filter = conds
                .into_iter()
                .reduce(|a, b| E::Bin("and", Box::new(a), Box::new(b)));
            let select = g.expr(rng, sel, d);
            Comp {
                gens,
                filter,
                select,
            }
        })
    }

    pub(super) fn expr(&mut self, rng: &mut Rng, t: &T, d: u32) -> E {
        if d == 0 || rng.below(3) == 0 {
            return self.leaf(rng, t);
        }
        let d = d - 1;
        if t.scalar() {
            match rng.below(8) {
                0 | 1 => return self.matched(rng, t, d),
                2 => {
                    return E::If(
                        Box::new(self.boolean(rng, d)),
                        Box::new(self.expr(rng, t, d)),
                        Box::new(self.expr(rng, t, d)),
                    );
                }
                _ => {}
            }
        }
        match t {
            T::Int => self.int(rng, d),
            T::Text => self.text(rng, d),
            T::Bool => self.boolean(rng, d),
            T::Float => self.float(rng, d),
            T::Shade | T::OptInt | T::Key(_) | T::Named(_) | T::Row(..) | T::FloatSum => {
                self.leaf(rng, t)
            }
            T::Rec(fs) => E::Rec(
                fs.iter()
                    .map(|(n, ft)| (n.clone(), self.expr(rng, ft, d)))
                    .collect(),
            ),
            T::Set(elem) => self.set(rng, elem, d),
        }
    }

    /// Returns the fields of type `t`, or of any scalar type when `t` is `None`, that variable
    /// `var` reads, or every variable in scope when `var` is `None`.
    fn fields(&self, var: Option<&str>, t: Option<&T>) -> Vec<Read> {
        let mut out = Vec::new();
        for (v, vt) in &self.scope {
            let T::Row(r, cs) = vt else { continue };
            if var.is_some_and(|w| w != v) {
                continue;
            }
            let rel = &self.w.rels[*r];
            let key = T::Key(*r);
            if t.is_none_or(|t| *t == key) {
                out.push(Read {
                    path: E::KeyOf(v.clone()),
                    ty: key,
                    field: None,
                });
            }
            let own: &[Field] = match cs.as_slice() {
                [c] => &rel.ctors[*c].fields,
                _ => &[],
            };
            for f in rel.header.iter().chain(own) {
                if t.map_or(!matches!(f.ty, T::Named(_)), |t| f.ty == *t) {
                    out.push(Read {
                        path: E::Path(v.clone(), f.name.clone()),
                        ty: f.ty.clone(),
                        field: Some((*r, f.name.clone())),
                    });
                }
            }
        }
        out
    }

    /// Returns the expressions of type `t` that read a variable in scope.
    fn reads(&self, t: &T) -> Vec<E> {
        self.fields(None, Some(t))
            .into_iter()
            .map(|r| r.path)
            .collect()
    }

    /// Returns a literal of scalar type `t` as an expression of that type.
    fn literal(t: &T, text: String) -> E {
        match t {
            T::Shade => E::Lit(format!("shade({text})")),
            T::OptInt => E::Lit(format!("opt({text})")),
            _ => E::Lit(text),
        }
    }

    fn leaf(&mut self, rng: &mut Rng, t: &T) -> E {
        let reads = self.reads(t);
        if !reads.is_empty() && rng.below(4) != 0 {
            return rng.pick(&reads).clone();
        }
        match t {
            T::Int | T::Text | T::Bool | T::Float | T::Shade | T::OptInt => {
                Self::literal(t, value(rng, t, self.w, &[]))
            }
            T::Key(r) => E::Lit(self.w.key(*r, 1 + rng.below(3))),
            T::FloatSum => E::Agg("sum", Box::new(self.comp(rng, &T::Float, 0, 1))),
            T::Row(..) | T::Named(_) => {
                unreachable!("no expression has a row type or a declared sum")
            }
            T::Rec(fs) => E::Rec(
                fs.iter()
                    .map(|(n, ft)| (n.clone(), self.leaf(rng, ft)))
                    .collect(),
            ),
            T::Set(elem) => match elem.as_ref() {
                T::Int if rng.below(2) == 0 => self
                    .fix(rng, 0)
                    .unwrap_or_else(|| E::List(vec![self.leaf(rng, elem)])),
                T::Int | T::Text | T::Bool => E::List(vec![self.leaf(rng, elem)]),
                _ => E::Set(Box::new(self.comp(rng, elem, 0, 1))),
            },
        }
    }

    /// Returns an equality between a field `var` reads and a field or key of another variable
    /// in scope, of the same type.
    fn join(&self, rng: &mut Rng, var: &str) -> Option<E> {
        let mut pairs = Vec::new();
        for own in self.fields(Some(var), None) {
            if !matches!(own.ty, T::Int | T::Text | T::Key(_)) {
                continue;
            }
            for other in self.fields(None, Some(&own.ty)) {
                let read = other.path.reads();
                if !read.contains(var) {
                    pairs.push((own.path.clone(), other.path));
                }
            }
        }
        let (a, b) = rng.pick_some(&pairs)?.clone();
        let (a, b) = if rng.below(2) == 0 { (a, b) } else { (b, a) };
        Some(E::Bin("==", Box::new(a), Box::new(b)))
    }

    /// Returns a comparison of a field one of `gens` reads with a value the load writes into
    /// it, or a drawn one when the load writes none.
    fn point(&self, rng: &mut Rng, gens: &[Binding]) -> Option<E> {
        let reads: Vec<Read> = gens
            .iter()
            .flat_map(|b| self.fields(Some(&b.var), None))
            .collect();
        let read = rng.pick_some(&reads)?;
        let seen = match &read.field {
            Some((r, f)) => self.w.seen(*r, f),
            None => Vec::new(),
        };
        let text = match (rng.pick_some(&seen), &read.ty) {
            (Some(v), _) => v.clone(),
            (None, T::Key(r)) => self.w.key(*r, 1 + rng.below(3)),
            (None, t) => value(rng, t, self.w, &[]),
        };
        let op = match read.ty {
            T::Int | T::Float => *rng.pick(&["==", "==", "<", ">="]),
            _ => "==",
        };
        Some(E::Bin(
            op,
            Box::new(read.path.clone()),
            Box::new(Self::literal(&read.ty, text)),
        ))
    }

    /// Returns a match of type `t` on a row variable of a sum, a `Shade` or an `Option I64`.
    fn matched(&mut self, rng: &mut Rng, t: &T, d: u32) -> E {
        let rows: Vec<(String, usize, Vec<usize>)> = self
            .scope
            .iter()
            .filter_map(|(v, vt)| match vt {
                T::Row(r, cs) if self.w.rels[*r].sum => Some((v.clone(), *r, cs.clone())),
                _ => None,
            })
            .collect();
        match rng.below(3) {
            0 if !rows.is_empty() => {
                let (v, r, cs) = rng.pick(&rows).clone();
                let mut arms = Vec::new();
                for c in cs {
                    let x = self.fresh("a");
                    let body = self.with(&x, T::Row(r, vec![c]), |g| g.expr(rng, t, d));
                    arms.push((c, x, body));
                }
                E::Row(v, r, arms)
            }
            1 => {
                let s = self.expr(rng, &T::Shade, d);
                let (red, green) = (self.expr(rng, t, d), self.expr(rng, t, d));
                let x = self.fresh("s");
                let n = T::Rec(vec![("n".into(), T::Int)]);
                let blue = self.with(&x, n, |g| g.expr(rng, t, d));
                E::Shade(
                    Box::new(s),
                    Box::new(red),
                    Box::new(green),
                    x,
                    Box::new(blue),
                )
            }
            _ => {
                let o = self.expr(rng, &T::OptInt, d);
                let none = self.expr(rng, t, d);
                let x = self.fresh("o");
                let value = T::Rec(vec![("value".into(), T::Int)]);
                let some = self.with(&x, value, |g| g.expr(rng, t, d));
                E::Opt(Box::new(o), Box::new(none), x, Box::new(some))
            }
        }
    }

    fn with<R>(&mut self, v: &str, t: T, f: impl FnOnce(&mut Self) -> R) -> R {
        self.scope.push((v.to_owned(), t));
        let r = f(self);
        self.scope.pop();
        r
    }

    fn bin(&mut self, rng: &mut Rng, op: &'static str, t: &T, d: u32) -> E {
        E::Bin(
            op,
            Box::new(self.expr(rng, t, d)),
            Box::new(self.expr(rng, t, d)),
        )
    }

    fn int(&mut self, rng: &mut Rng, d: u32) -> E {
        match rng.below(16) {
            0..=2 => self.bin(rng, "+", &T::Int, d),
            3 | 4 => self.bin(rng, "-", &T::Int, d),
            5 => self.bin(rng, "*", &T::Int, d),
            6 => {
                let op = *rng.pick(&["/", "%"]);
                self.bin(rng, op, &T::Int, d)
            }
            7 => E::Call("scalar_len", Box::new(self.expr(rng, &T::Text, d))),
            8..=10 => {
                let sel = self.leaf_ty(rng);
                E::Agg("count", Box::new(self.comp(rng, &sel, d, 1)))
            }
            11 | 12 => E::Agg("sum", Box::new(self.comp(rng, &T::Int, d, 1))),
            _ => self.leaf(rng, &T::Int),
        }
    }

    fn float(&mut self, rng: &mut Rng, d: u32) -> E {
        match rng.below(5) {
            0 => self.bin(rng, "+", &T::Float, d),
            1 => self.bin(rng, "-", &T::Float, d),
            2 => self.bin(rng, "*", &T::Float, d),
            _ => self.leaf(rng, &T::Float),
        }
    }

    fn text(&mut self, rng: &mut Rng, d: u32) -> E {
        match rng.below(3) {
            0 => self.bin(rng, "++", &T::Text, d),
            _ => self.leaf(rng, &T::Text),
        }
    }

    pub(super) fn boolean(&mut self, rng: &mut Rng, d: u32) -> E {
        match rng.below(15) {
            0..=2 => {
                let op = *rng.pick(&["<", "<=", ">", ">=", "==", "!="]);
                self.bin(rng, op, &T::Int, d)
            }
            3 => {
                let op = *rng.pick(&["==", "<"]);
                self.bin(rng, op, &T::Text, d)
            }
            4..=6 => {
                let t = self.joinable(rng);
                self.bin(rng, "==", &t, d)
            }
            7 => {
                let t = rng.pick(&[T::Shade, T::OptInt, T::Bool]).clone();
                self.bin(rng, "==", &t, d)
            }
            8 | 9 => {
                let op = *rng.pick(&["and", "or"]);
                self.bin(rng, op, &T::Bool, d)
            }
            10 => E::Not(Box::new(self.expr(rng, &T::Bool, d))),
            11 | 12 => E::Exists(Box::new(self.comp(rng, &T::Bool, d, 1))),
            13 => {
                let op = *rng.pick(&["<", "==", ">="]);
                self.bin(rng, op, &T::Float, d)
            }
            _ => self.leaf(rng, &T::Bool),
        }
    }

    /// Returns a key type some variable in scope reads, to compare keys across generators.
    fn joinable(&self, rng: &mut Rng) -> T {
        let tys: Vec<T> = (0..self.w.rels.len())
            .map(T::Key)
            .filter(|t| !self.reads(t).is_empty())
            .collect();
        if tys.is_empty() {
            T::Key(0)
        } else {
            rng.pick(&tys).clone()
        }
    }

    pub(super) fn leaf_ty(&self, rng: &mut Rng) -> T {
        let key = T::Key(rng.below(self.w.rels.len() as u64) as usize);
        rng.pick(&[T::Int, T::Text, T::Bool, key]).clone()
    }

    /// Returns the closure of a set of integers under a step from one integer field of the rows
    /// of a relation, or of one of its constructors, to another, when some relation has one.
    fn fix(&mut self, rng: &mut Rng, d: u32) -> Option<E> {
        let mut steps = Vec::new();
        for (r, rel) in self.w.rels.iter().enumerate() {
            let ints = |fs: &[Field]| -> Vec<String> {
                fs.iter()
                    .filter(|f| f.ty == T::Int)
                    .map(|f| f.name.clone())
                    .collect()
            };
            let header = ints(&rel.header);
            let ranges = std::iter::once((None, header.clone())).chain(
                rel.ctors
                    .iter()
                    .enumerate()
                    .filter(|_| rel.sum)
                    .map(|(c, ctor)| (Some(c), [header.clone(), ints(&ctor.fields)].concat())),
            );
            for (ctor, fields) in ranges {
                for into in &fields {
                    for out in &fields {
                        steps.push(Step {
                            rel: r,
                            ctor,
                            into: into.clone(),
                            out: out.clone(),
                        });
                    }
                }
            }
        }
        let step = rng.pick_some(&steps)?.clone();
        let base = self.comp(rng, &T::Int, d, 1);
        let names = [self.fresh("x"), self.fresh("y"), self.fresh("v")];
        Some(E::Fix(names, step, Box::new(base)))
    }

    fn set(&mut self, rng: &mut Rng, elem: &T, d: u32) -> E {
        let set = T::Set(Box::new(elem.clone()));
        match rng.below(5) {
            0 => E::Union(
                Box::new(self.expr(rng, &set, d)),
                Box::new(self.expr(rng, &set, d)),
            ),
            1 if matches!(elem, T::Int | T::Text | T::Bool) => E::List(
                (0..=rng.below(2))
                    .map(|_| self.expr(rng, elem, d))
                    .collect(),
            ),
            2..=4 if *elem == T::Int => self.fix(rng, d).unwrap_or_else(|| self.leaf(rng, &set)),
            _ => E::Set(Box::new(self.comp(rng, elem, d, 2))),
        }
    }
}
