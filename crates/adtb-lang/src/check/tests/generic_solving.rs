use super::type_laws::{PHANTOM, table, universe};
use super::*;
use crate::types::{Base, Ctors, Fields};

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

impl Reached {
    fn count(&mut self, admitted: bool) {
        if admitted {
            self.admitted += 1;
        } else {
            self.refused += 1;
        }
    }

    fn assert_both(&self, name: &str) {
        assert!(
            self.admitted > 0 && self.refused > 0,
            "`{name}` admitted {} and refused {}",
            self.admitted,
            self.refused
        );
    }
}

/// An argument: a parameter of a drawn type, or a literal and the least drawn type it checks
/// against.
enum Arg<'t> {
    Param(&'t Ty),
    Literal(&'static str, Ty),
}

impl Arg<'_> {
    fn ty(&self) -> &Ty {
        match self {
            Arg::Param(t) => t,
            Arg::Literal(_, t) => t,
        }
    }
}

struct Calls {
    schema: Committed,
    program: Program,
    tys: Vec<Ty>,
    leaves: Vec<Ty>,
}

impl Calls {
    /// Returns the type of `text` with `args` for `$p` and then `$q`, or its first refusal.
    fn outcome(&self, text: &str, args: &[&Arg]) -> Result<Ty, String> {
        let mut params = BTreeMap::new();
        let mut text = text.to_string();
        for (name, a) in ["p", "q"].iter().zip(args) {
            let written = match a {
                Arg::Param(t) => {
                    params.insert(name.to_string(), (*t).clone());
                    name
                }
                Arg::Literal(l, _) => l,
            };
            text = text.replace(&format!("${name}"), written);
        }
        let items = parse_adhoc(&text, Options::default()).ok().expect("parsed");
        check_adhoc(&self.schema, &self.program, &items, &params)
            .map(|a| a.terms.ty(a.items[0]).clone())
            .map_err(|ds| ds[0].message.clone())
    }

    /// Check `text` over `args` and assert it is admitted exactly when `oracle` holds.
    fn agrees(&self, text: &str, args: &[&Arg], oracle: bool, reached: &mut Reached) {
        let got = self.outcome(text, args);
        assert_eq!(
            got.as_ref().map(drop).map_err(drop) == Ok(()),
            oracle,
            "{text} at {:?}: {got:?}, but the table {}",
            args.iter().map(|a| a.ty()).collect::<Vec<_>>(),
            if oracle { "admits it" } else { "refuses it" }
        );
        reached.count(oracle);
    }

    /// Returns the drawn types `holds` admits.
    fn all(&self, holds: impl Fn(&Ty) -> bool) -> Vec<&Ty> {
        self.tys.iter().filter(|t| holds(t)).collect()
    }

    fn some(&self, holds: impl Fn(&Ty) -> bool) -> bool {
        self.tys.iter().any(holds)
    }

    fn sub(&self, a: &Ty, b: &Ty) -> bool {
        table(&self.schema, a, b)
    }

    /// Returns the type of `tys` below every other.
    fn least<'t>(&self, tys: &[&'t Ty]) -> Option<&'t Ty> {
        tys.iter()
            .copied()
            .find(|l| tys.iter().all(|u| self.sub(l, u)))
    }
}

/// Returns the callees, with `want{i}` taking and `takes{i}` a function of the `i`th leaf type.
fn calls() -> Calls {
    let base = schema(PHANTOM).expect("the running schema builds");
    let leaves: Vec<Ty> = universe(&base)
        .into_iter()
        .filter(|t| !matches!(t, Ty::Fn(..) | Ty::Sum { .. } | Ty::Record(_)))
        .collect();
    let mut text = format!("{PHANTOM}{CALLEES}");
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

/// Returns literals, each with the least drawn type it checks against.
fn literals(c: &Calls) -> Vec<Arg<'static>> {
    let status = c
        .tys
        .iter()
        .find(|t| match t {
            Ty::Data {
                def,
                ctors: Some(cs),
                ..
            } => c.schema.ty(*def).name == "Status" && *cs == [0],
            _ => false,
        })
        .expect("`Status.Draft` is drawn")
        .clone();
    let option = |t: Ty| Ty::Data {
        def: c.schema.prelude().option(),
        args: vec![t],
        ctors: Some(vec![1]),
    };
    vec![
        Arg::Literal("1", i64()),
        Arg::Literal("Draft", status.clone()),
        Arg::Literal("Some { value: Draft }", option(status)),
        Arg::Literal("Some { value: 1 }", option(i64())),
        Arg::Literal(
            "{ x: 1 }",
            Ty::Record(Fields::new([("x".to_string(), i64())])),
        ),
    ]
}

#[test]
fn a_covariant_type_argument_is_the_least_type_above_its_arguments_and_below_the_expected_type() {
    let c = calls();
    let mut args: Vec<Arg> = c
        .tys
        .iter()
        .filter(|t| !matches!(t, Ty::Fn(..) | Ty::Sum { .. }))
        .map(Arg::Param)
        .collect();
    args.extend(literals(&c));
    let (mut same, mut both, mut first) =
        (Reached::default(), Reached::default(), Reached::default());
    let sum = Ty::Sum {
        ctors: Ctors::new([
            ("Xa".to_string(), Fields::new([("x".to_string(), i64())])),
            ("Yb".to_string(), Fields::new([("y".to_string(), i64())])),
        ]),
        sel: None,
    };
    let option = |t: Ty| Ty::Data {
        def: c.schema.prelude().option(),
        args: vec![t],
        ctors: Some(vec![1]),
    };
    let wrappers: [(&str, Ty); 4] = [
        ("{}", sum.clone()),
        ("Some { value: {} }", option(sum.clone())),
        (
            "{ x: {} }",
            Ty::Record(Fields::new([("x".to_string(), sum.clone())])),
        ),
        ("[{}]", Ty::Set(Box::new(sum))),
    ];
    for (w, want) in &wrappers {
        let text = format!(
            "first({}, {})",
            w.replace("{}", "Xa { x: 1 }"),
            w.replace("{}", "Yb { y: 2 }")
        );
        match c.outcome(&text, &[]) {
            Ok(t) if c.sub(&t, want) && c.sub(want, &t) => first.count(true),
            got => panic!("{text}: {got:?}, not {want:?}"),
        }
    }
    for p in &args {
        for q in &args {
            if matches!((p, q), (Arg::Literal(..), Arg::Literal(..))) {
                continue;
            }
            let above = c.all(|t| c.sub(p.ty(), t) && c.sub(q.ty(), t));
            let least = c.least(&above);
            assert!(
                above.is_empty() || least.is_some(),
                "the drawn types hold no least type above {:?} and {:?}",
                p.ty(),
                q.ty()
            );
            c.agrees("same($p, $q)", &[p, q], least.is_some(), &mut same);
            c.agrees(
                "both({ x: $p, y: $q })",
                &[p, q],
                least.is_some(),
                &mut both,
            );
            let got = c.outcome("first($p, $q)", &[p, q]);
            match (least, &got) {
                (Some(l), Ok(t)) => assert!(
                    c.sub(l, t) && c.sub(t, l),
                    "first(p, q) at {:?} and {:?} is {t:?}, not the least {l:?}",
                    p.ty(),
                    q.ty()
                ),
                (None, Err(_)) => {}
                _ => panic!(
                    "first(p, q) at {:?} and {:?}: {got:?}, the least type above both {least:?}",
                    p.ty(),
                    q.ty()
                ),
            }
            first.count(least.is_some());
        }
    }
    let mut want = Reached::default();
    for p in &c.leaves {
        for q in &c.leaves {
            for (i, e) in c.leaves.iter().enumerate() {
                let between = c.some(|t| c.sub(p, t) && c.sub(q, t) && c.sub(t, e));
                c.agrees(
                    &format!("want{i}(first($p, $q))"),
                    &[&Arg::Param(p), &Arg::Param(q)],
                    between,
                    &mut want,
                );
            }
        }
    }
    same.assert_both("same");
    both.assert_both("both");
    first.assert_both("first");
    want.assert_both("want");
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
            c.agrees(
                "pick($p, $q)",
                &[&Arg::Param(f), &Arg::Param(g)],
                below,
                &mut pick,
            );
        }
        for (i, e) in c.leaves.iter().enumerate() {
            let between = c.some(|t| c.sub(f, &to_i64(t)) && c.sub(e, t));
            c.agrees(
                &format!("takes{i}(idf($p))"),
                &[&Arg::Param(f)],
                between,
                &mut idf,
            );
        }
    }
    pick.assert_both("pick");
    idf.assert_both("idf");
}
