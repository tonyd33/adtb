use adtb_lang::kernel::{Const, Kind, Prim, Terms, Tm};
use adtb_lang::types::{Base, Ty};
use adtb_reference::Abort;
use adtb_testkit::fixture::Fixture;

fn int_ty() -> Ty {
    Ty::Base(Base::I64)
}

fn int(terms: &mut Terms, i: i128) -> Tm {
    terms.add(Kind::Const(Const::Int(i)), int_ty())
}

fn text(terms: &mut Terms, t: &str) -> Tm {
    terms.add(Kind::Const(Const::Text(t.into())), Ty::Base(Base::Text))
}

fn broken(f: &Fixture, terms: &Terms, t: Tm) -> String {
    match f.interp().eval(terms, t, &[]) {
        Err(Abort::Internal(what)) => what,
        other => panic!("{other:?} is not an internal error"),
    }
}

#[test]
fn a_primitive_of_mistyped_operands_is_an_internal_error() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let (a, one) = (text(&mut terms, "a"), int(&mut terms, 1));
    let add = terms.add(Kind::Prim(Prim::Add, vec![a, one]), int_ty());
    let what = broken(&f, &terms, add);
    assert!(what.contains("Add"), "{what}");
    let lower = terms.add(Kind::Prim(Prim::Lower, vec![one]), Ty::Base(Base::Text));
    assert_eq!("expected text, found Int(1)", broken(&f, &terms, lower));
    let start = terms.add(Kind::Prim(Prim::Start, vec![one]), int_ty());
    let what = broken(&f, &terms, start);
    assert!(what.starts_with("a span has a start and an end"), "{what}");
}

#[test]
fn a_checked_primitive_of_mistyped_operands_is_an_internal_error_not_none() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let (a, one) = (text(&mut terms, "a"), int(&mut terms, 1));
    let t = terms.add(
        Kind::Prim(Prim::CheckedAdd, vec![a, one]),
        f.ty("Option I64"),
    );
    let what = broken(&f, &terms, t);
    assert!(what.contains("CheckedAdd"), "{what}");
}

#[test]
fn a_condition_that_is_not_a_bool_is_an_internal_error() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let one = int(&mut terms, 1);
    let t = terms.add(Kind::If(one, one, one), int_ty());
    assert_eq!("expected a Bool, found Int(1)", broken(&f, &terms, t));
}

#[test]
fn a_case_over_a_value_that_is_not_a_constructor_is_an_internal_error() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let one = int(&mut terms, 1);
    let t = terms.add(
        Kind::Case {
            scrutinee: one,
            arms: Vec::new(),
        },
        int_ty(),
    );
    let what = broken(&f, &terms, t);
    assert!(
        what.starts_with("a case scrutinee is a constructor value"),
        "{what}"
    );
}

#[test]
fn a_case_without_an_arm_for_the_scrutinee_is_an_internal_error() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let none = terms.add(
        Kind::Ctor {
            ctor: 0,
            fields: Vec::new(),
        },
        f.ty("Option I64"),
    );
    let t = terms.add(
        Kind::Case {
            scrutinee: none,
            arms: Vec::new(),
        },
        int_ty(),
    );
    assert_eq!(
        "a case names constructor 0 of its scrutinee",
        broken(&f, &terms, t)
    );
}

#[test]
fn evaluating_unreachable_is_an_internal_error() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let t = terms.add(Kind::Unreachable, int_ty());
    assert_eq!(
        "evaluation reached a position exhaustiveness excludes",
        broken(&f, &terms, t)
    );
}

#[test]
fn a_reject_of_a_value_that_is_not_text_is_an_internal_error() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let one = int(&mut terms, 1);
    let t = terms.add(Kind::Reject(one), int_ty());
    assert_eq!("`reject` takes a Text", broken(&f, &terms, t));
}

#[test]
fn an_application_of_a_value_that_is_not_a_function_is_an_internal_error() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let one = int(&mut terms, 1);
    let t = terms.add(Kind::App(one, one), int_ty());
    assert_eq!(
        "applied a value that is not a function: Int(1)",
        broken(&f, &terms, t)
    );
}

#[test]
fn a_field_a_value_lacks_is_an_internal_error() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let one = int(&mut terms, 1);
    let t = terms.add(Kind::Field(one, "name".into()), int_ty());
    assert_eq!("Int(1) has no field `name`", broken(&f, &terms, t));
}

#[test]
fn a_decimal_constant_at_another_type_is_an_internal_error() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let t = terms.add(Kind::Const(Const::Decimal(5)), int_ty());
    assert_eq!("decimal constant 5 at Base(I64)", broken(&f, &terms, t));
}

#[test]
fn an_internal_error_is_not_caught_by_try() {
    let f = Fixture::new("");
    let mut terms = Terms::default();
    let t = terms.add(Kind::Unreachable, int_ty());
    let tried = terms.add(Kind::Try(t), f.ty("Result I64 Fault"));
    let got = f.interp().eval(&terms, tried, &[]);
    assert!(matches!(got, Err(Abort::Internal(_))), "{got:?}");
}
