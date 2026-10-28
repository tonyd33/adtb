use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::Arc;

use adtb_lang::check::check_schema;
use adtb_lang::kernel::{Const, Kind, Terms, Tm};
use adtb_lang::parse::{Options, parse_schema};
use adtb_lang::schema::{Schema, build};
use adtb_lang::span::FileId;
use adtb_lang::textual::Form;
use adtb_lang::types::{Base, Ty};

use super::code::{Compiler, Shared};
use super::frame::Frame;
use super::ir::AggOp;
use super::prim::{Arith, Op, Width, apply};
use super::sink::aggregate;
use super::spill::{put, record};
use super::types::At;
use super::value::{F32, F64, Val};
use super::wide::{Wide, cmp_decimal};
use super::{Budget, Error, Exceeded, Fault, arm, to_key, to_stored};
use crate::heap::FieldType;
use crate::key::{KeyType, Scalar};

fn run(op: Op, xs: &[Val]) -> Result<Val, Fault> {
    let refs: Vec<&Val> = xs.iter().collect();
    apply(&op, &refs).expect("a supported operation")
}

#[test]
fn integer_arithmetic_faults_outside_its_type() {
    let add8 = Op::Int(Arith::Add, Width::I8);
    assert_eq!(
        Ok(Val::Int(127)),
        run(add8.clone(), &[Val::Int(100), Val::Int(27)])
    );
    assert_eq!(
        Err(Fault::ArithmeticOverflow),
        run(add8, &[Val::Int(100), Val::Int(28)])
    );
    let div = Op::Int(Arith::Div, Width::I64);
    assert_eq!(
        Ok(Val::Int(-2)),
        run(div.clone(), &[Val::Int(-7), Val::Int(3)])
    );
    assert_eq!(
        Err(Fault::DivisionByZero),
        run(div.clone(), &[Val::Int(1), Val::Int(0)])
    );
    assert_eq!(
        Err(Fault::ArithmeticOverflow),
        run(div, &[Val::Int(i64::MIN), Val::Int(-1)])
    );
    assert_eq!(
        Ok(Val::Int(-1)),
        run(
            Op::Int(Arith::Rem, Width::I64),
            &[Val::Int(-7), Val::Int(3)]
        )
    );
    assert_eq!(
        Err(Fault::ArithmeticOverflow),
        run(Op::IntNeg(Width::I16), &[Val::Int(i64::from(i16::MIN))])
    );
}

#[test]
fn a_checked_operation_is_none_where_its_operator_faults() {
    let op = Op::Checked(Box::new(Op::Int(Arith::Div, Width::I64)));
    assert_eq!(
        Ok(Val::none()),
        run(op.clone(), &[Val::Int(1), Val::Int(0)])
    );
    assert_eq!(
        Ok(Val::some(Val::Int(5))),
        run(op, &[Val::Int(10), Val::Int(2)])
    );
}

#[test]
fn decimals_add_at_the_larger_scale_and_compare_across_scales() {
    let add = Op::DecAdd {
        s: 1,
        t: 3,
        sub: false,
    };
    assert_eq!(
        Ok(Val::Decimal(1_500 + 25)),
        run(add, &[Val::Decimal(15), Val::Decimal(25)])
    );
    let sub = Op::DecAdd {
        s: 0,
        t: 1,
        sub: true,
    };
    assert_eq!(
        Ok(Val::Decimal(85 * 10i128.pow(36))),
        run(
            sub,
            &[
                Val::Decimal(175 * 10i128.pow(35)),
                Val::Decimal(9 * 10i128.pow(37))
            ]
        )
    );
    let big = 10i128.pow(37) * 9;
    assert_eq!(
        Err(Fault::ArithmeticOverflow),
        run(
            Op::DecAdd {
                s: 0,
                t: 0,
                sub: false
            },
            &[Val::Decimal(big), Val::Decimal(big)]
        )
    );
    assert_eq!(Ordering::Equal, cmp_decimal(10, 1, 100, 2));
    assert_eq!(Ordering::Less, cmp_decimal(-big, 0, 1, 38));
}

#[test]
fn a_wide_sum_is_exact_whatever_its_order() {
    let big = Wide::from_i128(i128::MAX);
    let s = big.add(big).and_then(|s| s.add(Wide::from_i128(i128::MIN)));
    assert_eq!(Some(i128::MAX - 1), s.and_then(Wide::to_i128));
    assert_eq!(None, Wide::from_i128(10i128.pow(38)).digits38());
    assert_eq!(
        Some(-(10i128.pow(38) - 1)),
        Wide::from_i128(-(10i128.pow(38) - 1)).digits38()
    );
}

#[test]
fn rounding_follows_each_mode() {
    let round = |v: i128, mode: u32| {
        run(
            Op::Round { from: 1, to: 0 },
            &[Val::Decimal(v), Val::Int(0), Val::tag(mode)],
        )
        .unwrap()
    };
    let cases = [
        (25, [2, 3, 2, 3, 2, 3, 2]),
        (-25, [-2, -3, -2, -3, -2, -2, -3]),
        (35, [4, 4, 3, 4, 3, 4, 3]),
        (21, [2, 2, 2, 3, 2, 3, 2]),
    ];
    for (v, want) in cases {
        for (mode, w) in want.iter().enumerate() {
            assert_eq!(
                Val::Decimal(*w),
                round(v, mode as u32),
                "{v} by mode {mode}"
            );
        }
    }
}

#[test]
fn floats_are_canonical_and_nan_is_greatest() {
    assert_eq!(F64::new(0.0), F64::new(-0.0));
    assert_eq!(F64::new(f64::NAN), F64::new(-f64::NAN));
    assert!(Val::F64(F64::new(f64::NAN)) > Val::F64(F64::new(f64::INFINITY)));
    assert!(Val::F32(F32::new(-1.0)) < Val::F32(F32::new(0.5)));
    let lt = Op::Cmp(Ordering::Less, false);
    assert_eq!(
        Ok(Val::bool(true)),
        run(
            lt,
            &[Val::F64(F64::new(1e300)), Val::F64(F64::new(f64::NAN))]
        )
    );
}

#[test]
fn text_compares_by_scalar_values() {
    let set: BTreeSet<Val> = ["é", "z", "a", "ä"].iter().map(|t| Val::text(t)).collect();
    let order: Vec<&Val> = set.iter().collect();
    assert_eq!(
        vec![
            &Val::text("a"),
            &Val::text("z"),
            &Val::text("ä"),
            &Val::text("é")
        ],
        order
    );
}

#[test]
fn a_concatenation_is_normalized_to_nfc() {
    assert_eq!(
        Ok(Ok(Val::text("\u{e9}"))),
        apply(&Op::ConcatText, &[&Val::text("e"), &Val::text("\u{301}")])
    );
    assert_eq!(
        Ok(Ok(Val::text("ab"))),
        apply(&Op::ConcatText, &[&Val::text("a"), &Val::text("b")])
    );
}

#[test]
fn grants_take_from_the_budget_and_return_to_it() {
    let budget = Budget::new(1000);
    let a = budget.admit(600).expect("fits");
    assert!(budget.admit(600).is_none());
    let b = budget.admit(400).expect("fits");
    assert_eq!(1000, budget.granted());
    drop(a);
    assert_eq!(400, budget.granted());
    let mut arena = b.take(100).expect("within the grant");
    assert_eq!(
        Err(Error::StatementLimit(Exceeded::Memory { limit: 400 })),
        arena.grow(301)
    );
    arena.grow(300).expect("within the grant");
    assert_eq!(400, b.used());
    drop(arena);
    assert_eq!(0, b.used());
    assert_eq!(400, b.peak());
    drop(b);
    assert_eq!(0, budget.granted());
}

#[test]
fn an_arena_returns_what_it_releases_and_takes_over_what_it_absorbs() {
    let budget = Budget::new(1000);
    let g = budget.admit(500).expect("fits");
    let mut a = g.take(300).expect("within the grant");
    a.release(100);
    assert_eq!(200, g.used());
    a.release(1000);
    assert_eq!(0, a.bytes());
    assert_eq!(0, g.used());
    let b = g.take(150).expect("within the grant");
    a.absorb(b);
    assert_eq!(150, a.bytes());
    assert_eq!(150, g.used());
    drop(a);
    assert_eq!(0, g.used());
}

#[test]
fn a_spilled_value_reads_back_equal() {
    let vals = vec![
        Val::Int(-5),
        Val::F32(F32::new(f32::NAN)),
        Val::F64(F64::new(-2.5)),
        Val::Decimal(-123_456_789_012_345_678_901_234_567),
        Val::text("naïve"),
        Val::Bytes(Arc::from(&[0u8, 255, 7][..])),
        Val::SortKey(Arc::from(&[9u8][..])),
        Val::Instant(i64::MIN),
        Val::Date(-3),
        Val::Time(86_399_999_999),
        Val::DateTime(42),
        Val::Zoned {
            local: 7,
            zone: Arc::from("Europe/Paris"),
        },
        Val::Duration(-1),
        Val::Period {
            months: -2,
            days: 30,
        },
        Val::tag(3),
        Val::ctor(1, vec![Val::Int(1), Val::empty_set()]),
        Val::record(vec![Val::bool(true), Val::none()]),
        Val::set([Val::Int(2), Val::Int(1)].into_iter().collect()),
        Val::List(Arc::from(vec![Val::Int(2), Val::Int(1)])),
    ];
    let mut bytes = Vec::new();
    for v in &vals {
        put(v, &mut bytes);
    }
    assert_eq!(Ok(vals), record(&bytes));
    assert!(record(&bytes[..bytes.len() - 1]).is_err());
}

#[test]
fn a_frame_rebuilt_from_its_rows_binds_what_it_bound() {
    let mut f = Frame::unit(4).widen(2);
    f.bind_one(0, Val::Int(7));
    f.bind_all(2, vec![Val::Int(10), Val::Int(11)]);
    let fixed = f.fixed();
    let rows: Vec<Vec<(u32, Val)>> = (0..2).map(|r| f.varying(r)).collect();
    assert_eq!(vec![(0, Val::Int(7))], fixed);
    assert_eq!(vec![(2, Val::Int(11))], rows[1]);
    let g = Frame::rebuild(4, &fixed, rows);
    assert_eq!(2, g.len());
    assert_eq!(&Val::Int(7), g.get(0, 1));
    assert_eq!(&Val::Int(10), g.get(2, 0));
    assert_eq!(&Val::Int(11), g.get(2, 1));
}

#[test]
fn a_frame_gathers_rows_and_keeps_constant_columns() {
    let mut f = Frame::unit(3).widen(3);
    f.bind_one(0, Val::Int(7));
    f.bind_all(1, vec![Val::Int(10), Val::Int(11), Val::Int(12)]);
    f.bind(2, [(1, Val::Int(21))]);
    let g = f.gather(&[2, 1, 1]);
    assert_eq!(3, g.len());
    assert_eq!(&Val::Int(7), g.get(0, 2));
    assert_eq!(&Val::Int(12), g.get(1, 0));
    assert_eq!(&Val::Int(11), g.get(1, 2));
    assert_eq!(&Val::Int(21), g.get(2, 1));
    let r = f.row(2);
    assert_eq!(1, r.len());
    assert_eq!(&Val::Int(12), r.get(1, 0));
}

#[test]
fn decimal_division_rounds_the_exact_quotient() {
    let third = |mode: u32| {
        run(
            Op::DecDiv { s: 0, t: 0, u: 38 },
            &[
                Val::Decimal(2),
                Val::Decimal(3),
                Val::Int(38),
                Val::tag(mode),
            ],
        )
    };
    let sixes = (10i128.pow(38) - 1) / 9 * 6;
    assert_eq!(Ok(Val::Decimal(sixes + 1)), third(1));
    assert_eq!(Ok(Val::Decimal(sixes)), third(4));
    assert_eq!(
        Err(Fault::ArithmeticOverflow),
        run(
            Op::DecDiv { s: 0, t: 0, u: 38 },
            &[Val::Decimal(1), Val::Decimal(1), Val::Int(38), Val::tag(0)],
        )
    );
    assert_eq!(
        Err(Fault::DivisionByZero),
        run(
            Op::DecDiv { s: 2, t: 1, u: 2 },
            &[Val::Decimal(1), Val::Decimal(0), Val::Int(2), Val::tag(0)],
        )
    );
    assert_eq!(
        Ok(Val::Decimal(-3)),
        run(
            Op::DecDiv { s: 6, t: 0, u: 0 },
            &[
                Val::Decimal(-2_500_000),
                Val::Decimal(1),
                Val::Int(0),
                Val::tag(6)
            ],
        )
    );
}

#[test]
fn floats_round_to_integers_and_decimals_exactly() {
    let to_int = |x: f64, mode: u32| {
        run(
            Op::FloatToInt(Width::I64),
            &[Val::F64(F64::new(x)), Val::tag(mode)],
        )
    };
    assert_eq!(Ok(Val::Int(2)), to_int(2.5, 0));
    assert_eq!(Ok(Val::Int(-2)), to_int(-2.5, 0));
    assert_eq!(Ok(Val::Int(-3)), to_int(-2.5, 1));
    assert_eq!(Ok(Val::Int(2)), to_int(2.5, 2));
    assert_eq!(Ok(Val::Int(-3)), to_int(-2.1, 3));
    assert_eq!(Err(Fault::NotFinite), to_int(f64::NAN, 0));
    assert_eq!(Err(Fault::OutOfRange), to_int(9.3e18, 0));
    let to_dec = |x: f64, s: u8, mode: u32| {
        run(
            Op::FloatToDec(s),
            &[
                Val::F64(F64::new(x)),
                Val::Int(i64::from(s)),
                Val::tag(mode),
            ],
        )
    };
    assert_eq!(
        Ok(Val::Decimal(10_000_000_000_000_000_555)),
        to_dec(0.1, 20, 0)
    );
    assert_eq!(
        Ok(Val::Decimal(10_000_000_000_000_000_556)),
        to_dec(0.1, 20, 3)
    );
    assert_eq!(Ok(Val::Decimal(-1)), to_dec(-1e-300, 2, 6));
    assert_eq!(Ok(Val::Decimal(0)), to_dec(-1e-300, 2, 5));
    assert_eq!(Err(Fault::OutOfRange), to_dec(1e39, 0, 0));
}

#[test]
fn a_period_adds_months_then_days_and_clamps_or_rejects() {
    let jan31 = 19_753;
    let add = |months: i32, days: i32, reject: bool| {
        run(
            Op::AddPeriod { reject },
            &[
                Val::Date(jan31),
                Val::Period { months, days },
                Val::tag(u32::from(reject)),
            ],
        )
    };
    assert_eq!(Ok(Val::Date(jan31 + 29)), add(1, 0, false));
    assert_eq!(Ok(Val::none()), add(1, 0, true));
    assert_eq!(Ok(Val::some(Val::Date(jan31 + 60))), add(2, 0, true));
    assert_eq!(Ok(Val::Date(jan31 + 30)), add(1, 1, false));
    assert_eq!(Ok(Val::Date(jan31 - 365 - 31)), add(-13, 0, false));
    assert_eq!(Err(Fault::ArithmeticOverflow), add(i32::MAX, 0, false));
}

#[test]
fn zone_rules_repeat_past_the_range_they_are_tabled_for() {
    let cycle = 146_097 * 86_400_000_000i64;
    let summer = 1_782_864_000_000_000;
    let lisbon = |i: i64| {
        let v = run(Op::InZone, &[Val::Instant(i), Val::text("Europe/Lisbon")]).unwrap();
        let Val::Zoned { local, .. } = v else {
            panic!("{v:?} is not zoned")
        };
        local - i
    };
    assert_eq!(3_600_000_000, lisbon(summer));
    assert_eq!(3_600_000_000, lisbon(summer + 25 * cycle));
    assert_eq!(0, lisbon(summer + 25 * cycle + 180 * 86_400_000_000));
    assert_eq!(
        lisbon(-400_000_000_000_000_000),
        lisbon(-400_000_000_000_000_000 + cycle)
    );
    assert_eq!(
        Err(Fault::UnknownZone),
        run(Op::InZone, &[Val::Instant(0), Val::text("europe/lisbon")])
    );
}

fn broken<T: std::fmt::Debug>(r: Result<T, Error>) -> String {
    match r {
        Err(Error::Internal(what)) => what,
        other => panic!("{other:?} is not an internal error"),
    }
}

fn broken_op(op: Op, xs: &[Val]) -> String {
    let refs: Vec<&Val> = xs.iter().collect();
    broken(apply(&op, &refs))
}

#[test]
fn a_primitive_of_mistyped_operands_is_an_internal_error() {
    let day = Val::Duration(86_400_000_000);
    let cases = [
        (Op::ConcatText, vec![Val::Int(1), Val::Int(2)], "text"),
        (
            Op::ConcatBytes,
            vec![Val::text("a"), Val::text("b")],
            "bytes",
        ),
        (
            Op::Int(Arith::Add, Width::I64),
            vec![Val::text("a"), Val::Int(1)],
            "an integer",
        ),
        (Op::DecMul, vec![Val::Int(1), Val::Int(2)], "a decimal"),
        (Op::DurAdd, vec![Val::Int(1), Val::Int(2)], "a duration"),
        (Op::PeriodNeg, vec![Val::Int(1)], "a period"),
        (Op::FloatNeg, vec![Val::Int(1)], "a float"),
        (
            Op::PointShift { sub: false },
            vec![Val::text("a"), day],
            "an instant or a date-time",
        ),
        (
            Op::PointDiff,
            vec![Val::Instant(0), Val::DateTime(0)],
            "two instants or two date-times",
        ),
        (Op::DateDiff, vec![Val::Int(1), Val::Int(2)], "two dates"),
        (Op::ToInstant, vec![Val::Instant(0)], "a zoned date-time"),
        (
            Op::Zoned { reject: false },
            vec![Val::Date(0), Val::text("UTC"), Val::tag(0)],
            "a date-time",
        ),
        (
            Op::InZone,
            vec![Val::Date(0), Val::text("UTC")],
            "an instant",
        ),
        (Op::Start, vec![Val::Int(1)], "values with fields"),
        (
            Op::AddPeriod { reject: false },
            vec![Val::Int(1), Val::Period { months: 1, days: 0 }, Val::tag(0)],
            "a date, a date-time or a zoned date-time",
        ),
        (
            Op::Round { from: 2, to: 0 },
            vec![Val::Decimal(150), Val::Int(0), Val::Int(0)],
            "a Rounding",
        ),
    ];
    for (op, xs, want) in cases {
        let what = broken_op(op.clone(), &xs);
        assert!(
            what.starts_with(&format!("a primitive's operands are {want}, found ")),
            "{op:?} of {xs:?}: {what}"
        );
    }
}

#[test]
fn a_checked_primitive_of_mistyped_operands_is_an_internal_error_not_none() {
    let op = Op::Checked(Box::new(Op::Int(Arith::Div, Width::I64)));
    let what = broken_op(op, &[Val::text("a"), Val::Int(0)]);
    assert!(what.contains("an integer"), "{what}");
}

#[test]
fn an_aggregate_over_mistyped_members_is_an_internal_error() {
    let cases = [
        (AggOp::SumInt(Base::I64), vec![Val::text("a")]),
        (AggOp::SumDecimal, vec![Val::Int(1)]),
        (AggOp::SumF32, vec![Val::Int(1)]),
        (AggOp::SumF64, vec![Val::Int(1)]),
        (AggOp::AvgF64 { wrap: true }, vec![Val::Int(1)]),
    ];
    for (op, ms) in cases {
        let what = broken(aggregate(op, ms.clone()));
        assert!(
            what.starts_with("an aggregate's range binds "),
            "{op:?} over {ms:?}: {what}"
        );
    }
}

#[test]
fn an_aggregate_that_is_not_an_option_over_no_members_is_an_internal_error() {
    for op in [
        AggOp::Min { wrap: false },
        AggOp::Max { wrap: false },
        AggOp::AvgF64 { wrap: false },
    ] {
        assert_eq!(
            "an aggregate that is not an Option ranges over a nonempty range",
            broken(aggregate(op, Vec::new())),
            "{op:?}"
        );
    }
    assert_eq!(
        Ok(Val::none()),
        aggregate(AggOp::Min { wrap: true }, Vec::new())
    );
}

#[test]
fn a_field_of_a_constructor_without_it_is_an_internal_error() {
    let at = At::ByCtor(vec![Some(0), None]);
    assert_eq!(Ok(0), at.of(0));
    assert_eq!("constructor 1 of the type has the field", broken(at.of(1)));
    assert_eq!("constructor 7 of the type has the field", broken(at.of(7)));
}

#[test]
fn a_case_without_an_arm_for_the_scrutinee_is_an_internal_error() {
    let arms = [Some('a'), None];
    assert_eq!(Ok(&'a'), arm(&arms, 0));
    assert_eq!("a case has an arm for constructor 1", broken(arm(&arms, 1)));
    assert_eq!("a case has an arm for constructor 9", broken(arm(&arms, 9)));
}

#[test]
fn a_stored_or_key_value_of_another_type_is_an_internal_error() {
    let what = broken(to_key(&KeyType::Scalar(Scalar::I64), &Val::text("a")));
    assert!(what.contains("is not a value of"), "{what}");
    let what = broken(to_stored(&FieldType::Scalar(Scalar::Text), &Val::Int(1)));
    assert!(what.contains("is not a value of"), "{what}");
    let what = broken(to_stored(&FieldType::Sum(Vec::new()), &Val::tag(3)));
    assert!(what.contains("is not a value of"), "{what}");
}

#[test]
fn printing_a_value_not_of_its_form_is_an_internal_error() {
    let what = broken_op(Op::Print(Arc::new(Form::Base(Base::Text))), &[Val::Int(1)]);
    assert!(what.contains("is text"), "{what}");
    let what = broken_op(Op::ToText(Arc::new(Form::Uuid)), &[Val::Int(1)]);
    assert!(what.contains("is bytes"), "{what}");
    let set = Form::Set {
        elem: Box::new(Form::Base(Base::I64)),
        nonempty: false,
    };
    let what = broken_op(Op::Print(Arc::new(set)), &[Val::Int(1)]);
    assert!(what.contains("is a set"), "{what}");
}

#[test]
fn an_ordered_result_prints_its_elements_in_its_order() {
    let set = Form::Set {
        elem: Box::new(Form::Base(Base::I64)),
        nonempty: false,
    };
    let listed = Val::List(Arc::from([Val::Int(3), Val::Int(1), Val::Int(2)]));
    assert_eq!(
        Ok(Val::text("[3, 1, 2]")),
        run(Op::Print(Arc::new(set)), &[listed])
    );
}

#[test]
fn an_internal_error_fails_the_statement_and_keeps_the_transaction() {
    let e = crate::write::Error::from(Error::Internal("an invariant".into()));
    assert_eq!(crate::write::Error::Internal("an invariant".into()), e);
    assert!(!e.ends_transaction());
}

fn empty_schema() -> Schema {
    let file = parse_schema("module app\n", Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("the schema does not parse: {ds:?}"));
    let schema = build(&[(FileId(0), file)])
        .unwrap_or_else(|ds| panic!("the schema does not build: {ds:?}"));
    check_schema(&schema).unwrap_or_else(|ds| panic!("the schema does not check: {ds:?}"));
    schema
}

fn eval_term(schema: &Schema, terms: &Terms, t: Tm) -> Result<Val, Error> {
    let code = Compiler::new(schema, terms).code(t)?;
    let shared = Shared {
        caller: None,
        fault_ctors: Vec::new(),
        hosts: Arc::default(),
    };
    code.eval_one(&mut Frame::unit(terms.vars.len()), &shared)
}

#[test]
fn evaluating_unreachable_is_an_internal_error() {
    let schema = empty_schema();
    let mut terms = Terms::default();
    let t = terms.add(Kind::Unreachable, Ty::Base(Base::I64));
    assert_eq!(
        "an evaluation reaches no `Unreachable`",
        broken(eval_term(&schema, &terms, t))
    );
}

#[test]
fn a_reject_of_a_value_that_is_not_text_is_an_internal_error() {
    let schema = empty_schema();
    let mut terms = Terms::default();
    let m = terms.add(Kind::Const(Const::Int(1)), Ty::Base(Base::I64));
    let t = terms.add(Kind::Reject(m), Ty::Base(Base::I64));
    assert_eq!(
        "`reject` takes a Text",
        broken(eval_term(&schema, &terms, t))
    );
}

#[test]
fn a_literal_outside_its_type_is_an_internal_error() {
    let schema = empty_schema();
    let mut terms = Terms::default();
    let t = terms.add(Kind::Const(Const::Int(i128::MAX)), Ty::Base(Base::I64));
    assert_eq!(
        "an integer literal fits its type",
        broken(eval_term(&schema, &terms, t))
    );
}

#[test]
fn a_key_of_a_term_that_is_not_a_key_type_is_an_internal_error() {
    let schema = empty_schema();
    let mut terms = Terms::default();
    let x = terms.add(Kind::Const(Const::Int(1)), Ty::Base(Base::I64));
    let t = terms.add(Kind::Key(x), Ty::Base(Base::I64));
    let what = broken(eval_term(&schema, &terms, t));
    assert!(what.starts_with("`key` has a key type"), "{what}");
}

#[test]
fn an_accessor_of_a_mistyped_value_is_an_internal_error() {
    let one = Val::Int(1);
    assert!(broken(one.truth()).contains("a Bool"));
    assert!(broken(Val::tag(2).truth()).contains("a Bool"));
    assert!(broken(one.ctor_of()).contains("a constructor value"));
    assert!(broken(one.fields()).contains("a value with fields"));
    assert!(broken(one.as_set()).contains("a set"));
    assert!(broken(Val::text("1").int()).contains("an integer"));
}

#[test]
fn a_condition_or_scrutinee_that_is_not_a_constructor_is_an_internal_error() {
    let schema = empty_schema();
    let int = Ty::Base(Base::I64);
    let mut terms = Terms::default();
    let c = terms.add(Kind::Const(Const::Int(1)), int.clone());
    let a = terms.add(Kind::Const(Const::Int(2)), int.clone());
    let t = terms.add(Kind::If(c, a, a), int.clone());
    assert!(broken(eval_term(&schema, &terms, t)).contains("a Bool"));
    let tag = terms.add(Kind::Tag(c), int);
    assert!(broken(eval_term(&schema, &terms, tag)).contains("a constructor value"));
}
