use super::*;
use crate::check::ty::{join, subtype};
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
fn universe(s: &Schema) -> Vec<Ty> {
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

#[test]
fn subtype_is_a_preorder_and_join_is_the_least_upper_bound() {
    let s = schema("").expect("the running schema builds");
    let tys = universe(&s);
    let n = tys.len();
    let sub: Vec<Vec<bool>> = tys
        .iter()
        .map(|a| tys.iter().map(|b| subtype(&s, a, b)).collect())
        .collect();
    let mut related = 0;
    for i in 0..n {
        assert!(sub[i][i], "{:?} is not a subtype of itself", tys[i]);
        for j in 0..n {
            if i != j && sub[i][j] {
                related += 1;
            }
            for k in 0..n {
                assert!(
                    !(sub[i][j] && sub[j][k]) || sub[i][k],
                    "{:?} <: {:?} <: {:?} but not transitively",
                    tys[i],
                    tys[j],
                    tys[k]
                );
            }
        }
    }
    assert!(related > n, "only {related} distinct pairs are related");
    let mut joined = 0;
    for i in 0..n {
        for j in 0..n {
            let upper: Vec<usize> = (0..n).filter(|&k| sub[i][k] && sub[j][k]).collect();
            match join(&s, &tys[i], &tys[j]) {
                Some(l) => {
                    joined += 1;
                    assert!(
                        subtype(&s, &tys[i], &l) && subtype(&s, &tys[j], &l),
                        "join {l:?} of {:?} and {:?} is not an upper bound",
                        tys[i],
                        tys[j]
                    );
                    for &k in &upper {
                        assert!(
                            subtype(&s, &l, &tys[k]),
                            "join {l:?} of {:?} and {:?} is not below the upper bound {:?}",
                            tys[i],
                            tys[j],
                            tys[k]
                        );
                    }
                }
                None if matches!(tys[i], Ty::Fn(..)) => {}
                None => assert!(
                    upper.is_empty(),
                    "{:?} and {:?} have no join but the upper bound {:?}",
                    tys[i],
                    tys[j],
                    tys[upper[0]]
                ),
            }
        }
    }
    assert!(joined > n, "only {joined} pairs join");
}
