use super::*;
use crate::check::ty::{join, meet, subtype};
use crate::types::{Base, Ctors, Fields, KeyCtors, RelRef};

fn type_id(s: &Schema, name: &str) -> TypeId {
    TypeId(
        s.types
            .iter()
            .position(|t| t.name == name)
            .expect("a type of that name") as u32,
    )
}

/// Returns the drawn types: scalars, keys and declared sums by constructor set, then sets,
/// functions, records and anonymous sums over them, each record and sum written in both orders.
pub(super) fn universe(s: &Schema) -> Vec<Ty> {
    let identities = RelRef::Declared(rel_id(s, "identities"));
    let key = |ctors| Ty::Key {
        rel: identities,
        ctors,
    };
    let status = type_id(s, "Status");
    let data = |ctors| Ty::Data {
        def: status,
        args: Vec::new(),
        ctors,
    };
    let leaves = vec![
        Ty::Base(Base::I64),
        Ty::Base(Base::Text),
        key(KeyCtors::All),
        key(KeyCtors::Only(vec![0])),
        key(KeyCtors::Only(vec![1])),
        key(KeyCtors::Only(vec![0, 1])),
        data(None),
        data(Some(vec![0])),
        data(Some(vec![1, 2])),
    ];
    let field = |n: &str, t: &Ty| (n.to_string(), t.clone());
    let mut out = leaves.clone();
    for t in &leaves {
        out.push(Ty::Set(Box::new(t.clone())));
        out.push(Ty::Fn(Box::new(t.clone()), Box::new(Ty::Base(Base::I64))));
        out.push(Ty::Fn(Box::new(Ty::Base(Base::I64)), Box::new(t.clone())));
        out.push(Ty::Record(Fields::new([field("x", t)])));
    }
    for t in &leaves[2..6] {
        for u in &leaves[2..6] {
            for (a, b) in [("x", "y"), ("y", "x")] {
                out.push(Ty::Record(Fields::new([field(a, t), field(b, u)])));
            }
            for sel in [None, Some(vec![0]), Some(vec![1])] {
                for (a, b) in [("Bar", "Foo"), ("Foo", "Bar")] {
                    let fa = Fields::new([field("v", t)]);
                    let fb = Fields::new([field("v", u)]);
                    let (fa, fb) = if a == "Bar" { (fa, fb) } else { (fb, fa) };
                    out.push(Ty::Sum {
                        ctors: Ctors::new([(a.to_string(), fa), (b.to_string(), fb)]),
                        sel: sel.clone(),
                    });
                }
            }
        }
        out.push(Ty::Sum {
            ctors: Ctors::new([("Foo".to_string(), Fields::new([field("v", t)]))]),
            sel: None,
        });
    }
    out
}

/// Returns the constructors `ctors` selects, every one for `None`, `all` being how many there are.
fn selected(sel: &Option<CtorSet>, all: usize) -> Vec<u32> {
    sel.clone().unwrap_or_else(|| (0..all as u32).collect())
}

/// Returns whether the subtyping table admits `a <: b` for the drawn types, which hold no type
/// parameter of variance other than covariant.
pub(super) fn table(s: &Schema, a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (
            Ty::Data {
                def: d1,
                args: a1,
                ctors: c1,
            },
            Ty::Data {
                def: d2,
                args: a2,
                ctors: c2,
            },
        ) => {
            let all = s.ty(*d1).ctors().len();
            d1 == d2
                && a1 == a2
                && selected(c1, all)
                    .iter()
                    .all(|c| selected(c2, all).contains(c))
        }
        (Ty::Key { rel: r1, ctors: k1 }, Ty::Key { rel: r2, ctors: k2 }) => {
            let only = |k: &KeyCtors| match k {
                KeyCtors::Only(cs) => Some(cs.clone()),
                _ => None,
            };
            let RelRef::Declared(r) = r1 else {
                return a == b;
            };
            let all = s.ty(s.relation(*r).elem_def).ctors().len();
            r1 == r2
                && selected(&only(k1), all)
                    .iter()
                    .all(|c| selected(&only(k2), all).contains(c))
        }
        (Ty::Set(x), Ty::Set(y)) => table(s, x, y),
        (Ty::Fn(p1, r1), Ty::Fn(p2, r2)) => table(s, p2, p1) && table(s, r1, r2),
        (Ty::Sum { ctors: x, sel: s1 }, Ty::Sum { ctors: y, sel: s2 }) => {
            let names = |c: &Ctors, sel: &Option<CtorSet>| -> Vec<String> {
                selected(sel, c.len())
                    .iter()
                    .map(|&i| c[i as usize].0.clone())
                    .collect()
            };
            x == y && names(x, s1).iter().all(|n| names(y, s2).contains(n))
        }
        _ => a == b,
    }
}

#[test]
fn subtype_is_the_subtyping_table_and_join_and_meet_are_its_bounds() {
    let s = schema("").expect("the running schema builds");
    let tys = universe(&s);
    let n = tys.len();
    let sub: Vec<Vec<bool>> = tys
        .iter()
        .map(|a| tys.iter().map(|b| subtype(&s, a, b)).collect())
        .collect();
    let mut related = 0;
    for i in 0..n {
        for j in 0..n {
            assert_eq!(
                table(&s, &tys[i], &tys[j]),
                sub[i][j],
                "{:?} <: {:?}",
                tys[i],
                tys[j]
            );
            if i != j && sub[i][j] {
                related += 1;
            }
        }
    }
    assert!(related > n, "only {related} distinct pairs are related");
    let (mut joined, mut met) = (0, 0);
    for i in 0..n {
        for j in 0..n {
            let (a, b) = (&tys[i], &tys[j]);
            let upper: Vec<&Ty> = (0..n)
                .filter(|&k| sub[i][k] && sub[j][k])
                .map(|k| &tys[k])
                .collect();
            match join(&s, a, b) {
                Some(l) => {
                    joined += 1;
                    assert!(
                        table(&s, a, &l) && table(&s, b, &l),
                        "join {l:?} of {a:?} and {b:?} is not an upper bound"
                    );
                    for u in &upper {
                        assert!(
                            table(&s, &l, u),
                            "join {l:?} of {a:?} and {b:?} is not below the upper bound {u:?}"
                        );
                    }
                }
                None => assert!(
                    upper.is_empty(),
                    "{a:?} and {b:?} have no join but the upper bound {:?}",
                    upper[0]
                ),
            }
            let lower: Vec<&Ty> = (0..n)
                .filter(|&k| sub[k][i] && sub[k][j])
                .map(|k| &tys[k])
                .collect();
            match meet(&s, a, b) {
                Some(m) => {
                    met += 1;
                    assert!(
                        table(&s, &m, a) && table(&s, &m, b),
                        "meet {m:?} of {a:?} and {b:?} is not a lower bound"
                    );
                    for l in &lower {
                        assert!(
                            table(&s, l, &m),
                            "meet {m:?} of {a:?} and {b:?} is not above the lower bound {l:?}"
                        );
                    }
                }
                None => assert!(
                    lower.is_empty(),
                    "{a:?} and {b:?} have no meet but the lower bound {:?}",
                    lower[0]
                ),
            }
        }
    }
    assert!(
        joined > n && met > n,
        "only {joined} pairs join and {met} meet"
    );
}
