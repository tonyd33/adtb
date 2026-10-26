use std::cmp::Ordering;
use std::collections::BTreeSet;

use super::frame::Frame;
use super::prim::{Arith, Op, Width, apply};
use super::value::{F32, F64, Val};
use super::wide::{Wide, cmp_decimal};
use super::{Budget, Error, Exceeded, Fault};

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
fn a_concatenation_needing_normalization_is_not_run_yet() {
    let r = apply(&Op::ConcatText, &[&Val::text("e"), &Val::text("\u{301}")]);
    assert!(matches!(r, Err(Error::Unsupported(_))));
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
