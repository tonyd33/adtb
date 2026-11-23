use super::*;
use crate::check::ty::{sum_sel, sum_selected};
use crate::types::{Ctors, Fields, SumClass};

/// The anonymous sums of the statement being checked: one class per constructor occurrence,
/// united by each join the statement makes.
#[derive(Clone, Default)]
pub(super) struct Sums {
    /// Each class's parent, a class its own root.
    parents: Vec<SumClass>,
    classes: Vec<Class>,
    /// The first term and variable of the statement.
    start: (usize, usize),
}

#[derive(Clone)]
struct Class {
    ctors: Ctors,
    /// Whether the class is a sum written in a type, to which no join adds a constructor.
    fixed: bool,
}

/// Returns the root of class `c` among `parents`.
fn find(parents: &[SumClass], mut c: SumClass) -> SumClass {
    while parents[c.index()] != c {
        c = parents[c.index()];
    }
    c
}

/// Returns `ty` with each sum still being inferred at the root of its class among `parents`.
pub(crate) fn canonical(ty: &Ty, parents: &[SumClass]) -> Ty {
    if !mentions_open(ty) {
        return ty.clone();
    }
    map_classes(ty, &|c| find(parents, c))
}

impl Sums {
    /// Begin a statement whose first term and variable are `tm` and `var`.
    pub(super) fn begin(&mut self, tm: usize, var: usize) {
        self.start = (tm, var);
    }

    pub(super) fn parents(&self) -> &[SumClass] {
        &self.parents
    }

    fn find(&self, c: SumClass) -> SumClass {
        find(&self.parents, c)
    }

    fn class(&self, c: SumClass) -> &Class {
        &self.classes[self.find(c).index()]
    }
}

/// Returns `ty` with each anonymous sum replaced by `sum` of its constructors and selection.
fn map_sums(ty: &Ty, sum: &mut impl FnMut(&Ctors, &Option<CtorSet>) -> Ty) -> Ty {
    let mut go = |t: &Ty| map_sums(t, sum);
    match ty {
        Ty::Sum { ctors, sel } => sum(ctors, sel),
        Ty::Base(_) | Ty::Param(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => ty.clone(),
        Ty::Data { def, args, ctors } => Ty::Data {
            def: *def,
            args: args.iter().map(go).collect(),
            ctors: ctors.clone(),
        },
        Ty::Set(t) => Ty::Set(Box::new(go(t))),
        Ty::NonEmpty(t) => Ty::NonEmpty(Box::new(go(t))),
        Ty::Group(k, t) => Ty::Group(Box::new(go(k)), Box::new(go(t))),
        Ty::Tag(t) => Ty::Tag(Box::new(go(t))),
        Ty::Record(fs) => Ty::Record(fs.map(go)),
        Ty::Fn(a, b) => Ty::Fn(Box::new(go(a)), Box::new(go(b))),
    }
}

/// Returns `ty` with the class `c` of each sum still being inferred replaced by `to` of it.
fn map_classes(ty: &Ty, to: &impl Fn(SumClass) -> SumClass) -> Ty {
    map_sums(ty, &mut |ctors, sel| Ty::Sum {
        ctors: Ctors::open(
            ctors.map(|t| map_classes(t, to)).iter().cloned(),
            ctors.class().map(to),
        ),
        sel: sel.clone(),
    })
}

/// Returns whether `ty` holds a sum still being inferred.
pub(super) fn mentions_open(ty: &Ty) -> bool {
    let mut open = false;
    map_sums(ty, &mut |ctors, _| {
        open |= ctors.class().is_some()
            || ctors
                .iter()
                .any(|(_, fs)| fs.iter().any(|(_, t)| mentions_open(t)));
        Ty::Unit
    });
    open
}

/// Returns the form of `ctors`'s class at `into` that selects the constructors `ctors` and `sel`
/// select.
fn view(ctors: &Ctors, sel: &Option<CtorSet>, into: Ctors) -> Ty {
    let names = sum_selected(ctors, sel);
    Ty::Sum {
        sel: sum_sel(&into, &names),
        ctors: into,
    }
}

impl Checker<'_> {
    /// Returns a constructor `name` with `fields`, the one constructor of a new class.
    pub(super) fn open_sum(&mut self, name: &str, fields: Fields) -> Ty {
        let c = SumClass::new(self.sums.classes.len() as u32);
        let ctors = Ctors::open([(name.to_string(), fields)], Some(c));
        self.sums.parents.push(c);
        self.sums.classes.push(Class {
            ctors: ctors.clone(),
            fixed: false,
        });
        Ty::Sum { ctors, sel: None }
    }

    /// Returns `ty` with each sum still being inferred at the constructors its class holds now.
    pub(super) fn current(&self, ty: &Ty) -> Ty {
        if !mentions_open(ty) {
            return ty.clone();
        }
        map_sums(ty, &mut |ctors, sel| {
            let into = match ctors.class() {
                Some(c) => {
                    let r = self.sums.find(c);
                    Ctors::open(self.sums.class(r).ctors.map(|t| self.current(t)), Some(r))
                }
                None => ctors.map(|t| self.current(t)),
            };
            view(ctors, sel, into)
        })
    }

    /// Returns `ty` with each sum still being inferred at the constructors its class holds, as a
    /// sum no longer inferred.
    pub(super) fn closed(&self, ty: &Ty) -> Ty {
        map_sums(ty, &mut |ctors, sel| {
            let all = match ctors.class() {
                Some(c) => &self.sums.class(c).ctors,
                None => ctors,
            };
            view(ctors, sel, Ctors::new(all.map(|t| self.closed(t))))
        })
    }

    pub(super) fn subtype(&self, a: &Ty, b: &Ty) -> bool {
        subtype(self.schema, &self.current(a), &self.current(b))
    }

    /// Returns the join of `a` and `b`, where one exists, uniting the classes of the sums still
    /// being inferred at each position the two share.
    pub(super) fn join(&mut self, a: &Ty, b: &Ty, span: Span) -> R<Option<Ty>> {
        if !mentions_open(a) && !mentions_open(b) {
            return Ok(join(self.schema, a, b));
        }
        let before = self.sums.clone();
        let joined = self.joined(a, b, span);
        if matches!(joined, Ok(Some(_))) && self.sums.parents != before.parents {
            let (tm, var) = self.sums.start;
            let parents = &self.sums.parents;
            self.terms.map_types(tm, var, |t| canonical(t, parents));
        } else {
            self.sums = before;
        }
        joined
    }

    fn joined(&mut self, a: &Ty, b: &Ty, span: Span) -> R<Option<Ty>> {
        let (a, b) = (self.current(a), self.current(b));
        if !mentions_open(&a) && !mentions_open(&b) {
            return Ok(join(self.schema, &a, &b));
        }
        let joined = match (&a, &b) {
            (Ty::Sum { ctors: x, sel: s1 }, Ty::Sum { ctors: y, sel: s2 }) => {
                let c = match (x.class(), y.class()) {
                    (Some(p), Some(q)) => self.unite(p, q, span)?,
                    (Some(p), None) => self.fix_class(p, y),
                    (None, Some(q)) => self.fix_class(q, x),
                    (None, None) => None,
                };
                c.map(|c| {
                    let mut names = sum_selected(x, s1);
                    names.extend(sum_selected(y, s2));
                    let into = self.current(&Ty::Sum {
                        ctors: Ctors::open([], Some(c)),
                        sel: None,
                    });
                    let Ty::Sum { ctors, .. } = into else {
                        unreachable!("a class resolves to a sum")
                    };
                    Ty::Sum {
                        sel: sum_sel(&ctors, &names),
                        ctors,
                    }
                })
            }
            (Ty::Set(x) | Ty::NonEmpty(x), Ty::Set(y) | Ty::NonEmpty(y)) => {
                self.joined(x, y, span)?.map(|t| match (&a, &b) {
                    (Ty::NonEmpty(_), Ty::NonEmpty(_)) => Ty::NonEmpty(Box::new(t)),
                    _ => Ty::Set(Box::new(t)),
                })
            }
            (Ty::Record(xs), Ty::Record(ys)) if xs.len() == ys.len() => {
                let mut fields = Vec::new();
                for ((n, x), (m, y)) in xs.iter().zip(ys) {
                    if n != m {
                        return Ok(None);
                    }
                    match self.same_or_united(x, y, span)? {
                        Some(t) => fields.push((n.clone(), t)),
                        None => return Ok(None),
                    }
                }
                Some(Ty::Record(Fields::new(fields)))
            }
            _ => join(self.schema, &a, &b),
        };
        Ok(joined.map(|t| self.current(&t)))
    }

    /// Returns the type of a position that holds `a` and `b`, which must be equal but for the
    /// classes of the sums still being inferred, which are united.
    fn same_or_united(&mut self, a: &Ty, b: &Ty, span: Span) -> R<Option<Ty>> {
        if !mentions_open(a) && !mentions_open(b) {
            let equal = subtype(self.schema, a, b) && subtype(self.schema, b, a);
            return Ok(equal.then(|| a.clone()));
        }
        self.joined(a, b, span)
    }

    /// Unite classes `p` and `q`, joining the fields of each constructor both hold. Returns the
    /// united class, `None` when a constructor's fields have no join or a fixed class would grow.
    fn unite(&mut self, p: SumClass, q: SumClass, span: Span) -> R<Option<SumClass>> {
        let (p, q) = (self.sums.find(p), self.sums.find(q));
        if p == q {
            return Ok(Some(p));
        }
        let (mine, theirs) = (
            self.sums.class(p).ctors.clone(),
            self.sums.class(q).ctors.clone(),
        );
        let fixed = self.sums.class(p).fixed || self.sums.class(q).fixed;
        self.sums.parents[q.index()] = p;
        let mut ctors: Vec<(String, Fields)> = Vec::new();
        for (c, fs) in mine.iter().chain(theirs.iter()) {
            match ctors.iter().position(|(d, _)| d == c) {
                None => ctors.push((c.clone(), fs.clone())),
                Some(i) => {
                    let had = ctors[i].1.clone();
                    if had.len() != fs.len() {
                        return Ok(None);
                    }
                    let mut merged = Vec::new();
                    for ((n, x), (m, y)) in had.iter().zip(fs) {
                        if n != m {
                            return Ok(None);
                        }
                        match self.same_or_united(x, y, span)? {
                            Some(t) => merged.push((n.clone(), t)),
                            None => return Ok(None),
                        }
                    }
                    ctors[i].1 = Fields::new(merged);
                }
            }
        }
        if fixed && (ctors.len() != mine.len() || ctors.len() != theirs.len()) {
            return Ok(None);
        }
        let ctors = Ctors::open(ctors, Some(p));
        self.sums.classes[p.index()].ctors = ctors.clone();
        self.sums.classes[p.index()].fixed = fixed;
        for (c, fs) in &ctors {
            for (f, t) in fs {
                if self.reaches(t, p) {
                    let names: Vec<String> = ctors.iter().map(|(c, _)| format!("`{c}`")).collect();
                    return Err(diag(
                        span,
                        format!(
                            "the anonymous sum of {} holds itself in field `{f}` of `{c}`",
                            names.join(" and ")
                        ),
                    ));
                }
            }
        }
        Ok(Some(p))
    }

    /// Fix class `p` at the written sum `to`, which holds each of its constructors at equal
    /// fields. Returns `p`, `None` when `to` does not hold one of them or `p` is fixed elsewhere.
    fn fix_class(&mut self, p: SumClass, to: &Ctors) -> Option<SumClass> {
        let p = self.sums.find(p);
        let class = self.sums.class(p);
        let held = class.ctors.iter().all(|(c, fs)| {
            to.iter().any(|(d, gs)| {
                c == d
                    && fs.len() == gs.len()
                    && fs.iter().zip(gs).all(|((n, x), (m, y))| {
                        let x = self.current(x);
                        n == m && subtype(self.schema, &x, y) && subtype(self.schema, y, &x)
                    })
            })
        });
        if !held || class.fixed && class.ctors.len() != to.len() {
            return None;
        }
        self.sums.classes[p.index()].ctors = Ctors::open(to.iter().cloned(), Some(p));
        self.sums.classes[p.index()].fixed = true;
        Some(p)
    }

    /// Returns whether a value of `ty` can hold a value of class `p`.
    fn reaches(&self, ty: &Ty, p: SumClass) -> bool {
        let mut found = false;
        map_sums(ty, &mut |ctors, _| {
            found |= ctors.class().is_some_and(|c| self.sums.find(c) == p)
                || ctors
                    .iter()
                    .any(|(_, fs)| fs.iter().any(|(_, t)| self.reaches(t, p)))
                || ctors.class().is_some_and(|c| {
                    self.sums
                        .class(c)
                        .ctors
                        .iter()
                        .any(|(_, fs)| fs.iter().any(|(_, t)| self.reaches(t, p)))
                });
            Ty::Unit
        });
        found
    }

    /// Give every term and variable of the statement its type with each sum still being inferred
    /// closed, each constructor and case arm renumbered in it, and forget the classes.
    pub(super) fn close_sums(&mut self) {
        if self.sums.parents.is_empty() {
            return;
        }
        let (tm, var) = self.sums.start;
        let closed = |terms: &Terms, t: Tm| -> (Kind, Ty) {
            let old = terms.ty(t);
            let ty = self.closed(old);
            let index = |of: &Ty, into: &Ty, c: u32| -> u32 {
                let (Ty::Sum { ctors: from, .. }, Ty::Sum { ctors: to, .. }) = (of, into) else {
                    return c;
                };
                to.iter()
                    .position(|(d, _)| *d == from[c as usize].0)
                    .expect("a class holds every constructor of its forms") as u32
            };
            let kind = match terms.kind(t) {
                Kind::Ctor { ctor, fields } => Kind::Ctor {
                    ctor: index(old, &ty, *ctor),
                    fields: fields.clone(),
                },
                Kind::Case { scrutinee, arms } => {
                    let of = terms.ty(*scrutinee);
                    let into = self.closed(of);
                    let mut arms: Vec<(u32, Var, Tm)> = arms
                        .iter()
                        .map(|&(c, x, b)| (index(of, &into, c), x, b))
                        .collect();
                    arms.sort_by_key(|a| a.0);
                    Kind::Case {
                        scrutinee: *scrutinee,
                        arms,
                    }
                }
                Kind::Call {
                    func,
                    type_args,
                    args,
                } => Kind::Call {
                    func: *func,
                    type_args: type_args.iter().map(|t| self.closed(t)).collect(),
                    args: args.clone(),
                },
                Kind::Restrict(e, set) => {
                    let of = ty::elem(terms.ty(*e)).cloned().unwrap_or(Ty::Unit);
                    let into = self.closed(&of);
                    let mut set: CtorSet = set.iter().map(|&c| index(&of, &into, c)).collect();
                    set.sort_unstable();
                    Kind::Restrict(*e, set)
                }
                k => k.clone(),
            };
            (kind, ty)
        };
        let (kinds, vars) = (
            (tm..self.terms.len())
                .map(|t| closed(&self.terms, Tm(t as u32)))
                .collect::<Vec<_>>(),
            self.terms.vars()[var..]
                .iter()
                .map(|v| self.closed(&v.ty))
                .collect::<Vec<_>>(),
        );
        self.terms.retype(tm, var, kinds, vars);
        self.sums.classes.clear();
        self.sums.parents.clear();
    }
}
