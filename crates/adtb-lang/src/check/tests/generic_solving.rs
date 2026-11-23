use super::type_laws::{table, universe};
use super::*;
use crate::types::Base;

const CALLEES: &str = "
fn same [a] (x: a, y: a) -> I64 = 1
fn first [a] (x: a, y: a) -> a = x
fn both [a] (p: { x: a, y: a }) -> I64 = 1
fn pick [a] (f: a -> I64, g: a -> I64) -> I64 = 1
fn idf [a] (f: a -> I64) -> a -> I64 = f
";

fn i64() -> Ty {
    Ty::Base(Base::I64)
}

fn to_i64(t: &Ty) -> Ty {
    Ty::Fn(Box::new(t.clone()), Box::new(i64()))
}

#[derive(Default)]
struct Reached {
    admitted: usize,
    refused: usize,
}

struct Calls {
    schema: Committed,
    program: Program,
    tys: Vec<Ty>,
    leaves: Vec<Ty>,
}

impl Calls {
    /// Check `text` with each parameter at its type and assert it is admitted exactly when
    /// `oracle` holds.
    fn agrees(&self, params: &[(&str, &Ty)], text: &str, oracle: bool, reached: &mut Reached) {
        let tys = params
            .iter()
            .map(|(n, t)| (n.to_string(), (*t).clone()))
            .collect();
        let items = parse_adhoc(text, Options::default()).ok().expect("parsed");
        let got = check_adhoc(&self.schema, &self.program, &items, &tys)
            .map(drop)
            .map_err(|ds| ds[0].message.clone());
        assert_eq!(
            got == Ok(()),
            oracle,
            "{text} at {params:?}: {got:?}, but the table {}",
            if oracle { "admits it" } else { "refuses it" }
        );
        if oracle {
            reached.admitted += 1;
        } else {
            reached.refused += 1;
        }
    }

    /// Returns whether some drawn type satisfies `holds`.
    fn some(&self, holds: impl Fn(&Ty) -> bool) -> bool {
        self.tys.iter().any(holds)
    }

    fn sub(&self, a: &Ty, b: &Ty) -> bool {
        table(&self.schema, a, b)
    }
}

/// Returns the callees, with `want{i}` taking and `takes{i}` a function of the `i`th leaf type.
fn calls() -> Calls {
    let base = schema("").expect("the running schema builds");
    let leaves: Vec<Ty> = universe(&base)
        .into_iter()
        .filter(|t| !matches!(t, Ty::Fn(..) | Ty::Sum { .. } | Ty::Record(_)))
        .collect();
    let mut text = CALLEES.to_string();
    for (i, t) in leaves.iter().enumerate() {
        let t = base.show_ty(t);
        text.push_str(&format!(
            "fn want{i} (e: {t}) -> I64 = 1\nfn takes{i} (f: {t} -> I64) -> I64 = 1\n"
        ));
    }
    let schema = schema(&text).expect("the schema builds");
    let program = check_schema(&schema).expect("the schema checks");
    let tys = universe(&schema);
    Calls {
        schema,
        program,
        tys,
        leaves,
    }
}

#[test]
fn a_covariant_type_argument_is_the_least_type_above_its_arguments_and_below_the_expected_type() {
    let c = calls();
    let values: Vec<&Ty> = c
        .tys
        .iter()
        .filter(|t| !matches!(t, Ty::Fn(..) | Ty::Sum { .. }))
        .collect();
    for a in &values {
        for b in &values {
            let bounded: Vec<&Ty> = values
                .iter()
                .copied()
                .filter(|t| c.sub(a, t) && c.sub(b, t))
                .collect();
            assert!(
                bounded.is_empty() || bounded.iter().any(|l| bounded.iter().all(|u| c.sub(l, u))),
                "the drawn types hold no least type above {a:?} and {b:?}"
            );
        }
    }
    let (mut same, mut both, mut first) = Default::default();
    for p in &values {
        for q in &values {
            let above = c.some(|t| c.sub(p, t) && c.sub(q, t));
            c.agrees(&[("p", p), ("q", q)], "same(p, q)", above, &mut same);
            c.agrees(
                &[("p", p), ("q", q)],
                "both({ x: p, y: q })",
                above,
                &mut both,
            );
        }
    }
    for p in &c.leaves {
        for q in &c.leaves {
            for (i, e) in c.leaves.iter().enumerate() {
                let between = c.some(|t| c.sub(p, t) && c.sub(q, t) && c.sub(t, e));
                c.agrees(
                    &[("p", p), ("q", q)],
                    &format!("want{i}(first(p, q))"),
                    between,
                    &mut first,
                );
            }
        }
    }
    for (name, r) in [("same", same), ("both", both), ("first", first)] {
        assert!(
            r.admitted > 0 && r.refused > 0,
            "`{name}` admitted {} and refused {}",
            r.admitted,
            r.refused
        );
    }
}

#[test]
fn a_contravariant_type_argument_is_the_greatest_type_below_its_arguments_and_above_the_expected_type()
 {
    let c = calls();
    let fns: Vec<&Ty> = c.tys.iter().filter(|t| matches!(t, Ty::Fn(..))).collect();
    let (mut pick, mut idf) = (Reached::default(), Reached::default());
    for f in &fns {
        for g in &fns {
            let below = c.some(|t| c.sub(f, &to_i64(t)) && c.sub(g, &to_i64(t)));
            c.agrees(&[("f", f), ("g", g)], "pick(f, g)", below, &mut pick);
        }
        for (i, e) in c.leaves.iter().enumerate() {
            let between = c.some(|t| c.sub(f, &to_i64(t)) && c.sub(e, t));
            c.agrees(&[("f", f)], &format!("takes{i}(idf(f))"), between, &mut idf);
        }
    }
    for (name, r) in [("pick", pick), ("idf", idf)] {
        assert!(
            r.admitted > 0 && r.refused > 0,
            "`{name}` admitted {} and refused {}",
            r.admitted,
            r.refused
        );
    }
}
