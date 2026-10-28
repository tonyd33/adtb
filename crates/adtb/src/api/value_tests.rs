use std::sync::Arc;

use adtb_lang::check::check_schema;
use adtb_lang::parse::{Options, parse_schema};
use adtb_lang::schema::{Schema, build};
use adtb_lang::span::FileId;
use adtb_lang::types::{Base, Ty};

use super::valid;
use crate::exec::{F32, F64, Val};

fn schema(text: &str) -> Schema {
    let file = parse_schema(text, Options::default())
        .ok()
        .expect("the fixture parses");
    let schema = build(&[(FileId(0), file)]).expect("the fixture builds");
    check_schema(&schema).expect("the fixture typechecks");
    schema
}

#[test]
fn scalar_representations_and_boundaries_are_checked_without_narrowing() {
    let schema = crate::catalog::engine_schema();
    for base in [Base::I8, Base::I16, Base::I32, Base::I64] {
        let (lo, hi) = base.int_range().unwrap();
        for n in [lo, hi] {
            assert!(
                valid(&schema, &Ty::Base(base), &Val::Int(n)),
                "{base:?} {n}"
            );
        }
        for n in [lo.checked_sub(1), hi.checked_add(1)].into_iter().flatten() {
            assert!(
                !valid(&schema, &Ty::Base(base), &Val::Int(n)),
                "{base:?} {n}"
            );
        }
    }
    let limit = 10_i128.pow(38);
    let cases = [
        (Base::Decimal(38), Val::Decimal(limit - 1), true),
        (Base::Decimal(0), Val::Decimal(1 - limit), true),
        (Base::Decimal(0), Val::Decimal(limit), false),
        (Base::Decimal(0), Val::Decimal(-limit), false),
        (Base::Decimal(0), Val::Decimal(i128::MIN), false),
        (Base::Date, Val::Date(0), true),
        (Base::Date, Val::Date(i32::MAX), false),
        (Base::Date, Val::Date(i32::MIN), false),
        (Base::Time, Val::Time(86_399_999_999), true),
        (Base::Time, Val::Time(86_400_000_000), false),
        (Base::Time, Val::Time(-1), false),
        (
            Base::ZonedDateTime,
            Val::Zoned {
                local: 0,
                zone: Arc::from("UTC"),
            },
            true,
        ),
        (
            Base::ZonedDateTime,
            Val::Zoned {
                local: 0,
                zone: Arc::from("Invalid/Zone"),
            },
            false,
        ),
        (Base::F32, Val::F32(F32::new(f32::NAN)), true),
        (Base::F64, Val::F64(F64::new(-0.0)), true),
        (Base::F32, Val::F64(F64::new(1.0)), false),
        (Base::Bytes, Val::Bytes(Arc::from([1])), true),
        (Base::Bytes, Val::SortKey(Arc::from([1])), false),
        (Base::Instant, Val::Instant(i64::MIN), true),
        (Base::DateTime, Val::DateTime(i64::MAX), true),
        (Base::Duration, Val::Duration(i64::MIN), true),
        (
            Base::Period,
            Val::Period {
                months: i32::MIN,
                days: i32::MAX,
            },
            true,
        ),
    ];
    for (base, value, expected) in cases {
        assert_eq!(
            expected,
            valid(&schema, &Ty::Base(base), &value),
            "{base:?} {value:?}"
        );
    }
}

#[test]
fn exact_fields_constructor_restrictions_and_nonempty_sets_are_enforced() {
    let schema = crate::catalog::engine_schema();
    let boolean = Ty::Data {
        def: schema.prelude_type("Bool"),
        args: vec![],
        ctors: Some(vec![0]),
    };
    assert!(valid(&schema, &boolean, &Val::bool(false)));
    assert!(!valid(&schema, &boolean, &Val::bool(true)));
    let tag = Ty::Tag(Box::new(boolean.clone()));
    assert!(valid(&schema, &tag, &Val::tag(0)));
    assert!(!valid(&schema, &tag, &Val::tag(1)));
    assert!(!valid(&schema, &tag, &Val::ctor(0, vec![Val::Int(1)])));
    let option = Ty::Data {
        def: schema.prelude_type("Option"),
        args: vec![Ty::Base(Base::I8)],
        ctors: None,
    };
    assert!(valid(&schema, &option, &Val::some(Val::Int(127))));
    assert!(!valid(&schema, &option, &Val::some(Val::Int(128))));
    assert!(!valid(&schema, &option, &Val::ctor(1, vec![])));
    let record = Ty::Record(vec![("a".into(), option), ("b".into(), boolean)]);
    assert!(valid(
        &schema,
        &record,
        &Val::record(vec![Val::none(), Val::bool(false)])
    ));
    assert!(!valid(&schema, &record, &Val::record(vec![Val::none()])));
    assert!(!valid(
        &schema,
        &record,
        &Val::record(vec![Val::none(), Val::bool(false), Val::Int(0)])
    ));
    let nonempty = Ty::NonEmpty(Box::new(Ty::Base(Base::I8)));
    assert!(!valid(&schema, &nonempty, &Val::empty_set()));
    assert!(valid(&schema, &nonempty, &Val::set([Val::Int(127)].into())));
    assert!(!valid(
        &schema,
        &nonempty,
        &Val::set([Val::Int(128)].into())
    ));
    assert!(!valid(
        &schema,
        &nonempty,
        &Val::List(Arc::from([Val::Int(1)]))
    ));
    assert!(valid(
        &schema,
        &Ty::Set(Box::new(Ty::Base(Base::I8))),
        &Val::empty_set()
    ));
    assert!(valid(&schema, &Ty::Unit, &Val::record(vec![])));
    assert!(!valid(&schema, &Ty::Unit, &Val::tag(0)));
    for ty in [
        Ty::Param(0),
        Ty::Change,
        Ty::Fn(Box::new(Ty::Unit), Box::new(Ty::Unit)),
        Ty::Group(Box::new(Ty::Unit), Box::new(Ty::Unit)),
    ] {
        assert!(!valid(&schema, &ty, &Val::record(vec![])), "{ty:?}");
    }
}

#[test]
fn shared_value_dags_are_validated_without_expanding_every_path() {
    let mut text = "module app\ntype T0 ordered = { leaf: I8 }\n".to_owned();
    let mut value = Val::ctor(0, vec![Val::Int(127)]);
    let mut other = Val::ctor(0, vec![Val::Int(127)]);
    for level in 1..=30 {
        let previous = level - 1;
        text.push_str(&format!(
            "type T{level} ordered = {{ left: T{previous}, right: T{previous} }}\n"
        ));
        value = Val::ctor(0, vec![value.clone(), value]);
        other = Val::ctor(0, vec![other.clone(), other]);
    }
    let schema = schema(&text);
    let module = schema.module_named("app").unwrap();
    let ty = Ty::Data {
        def: schema.module(module).types["T30"],
        args: vec![],
        ctors: None,
    };
    assert!(valid(&schema, &ty, &value));
    let span = Ty::Data {
        def: schema.prelude_type("Span"),
        args: vec![ty],
        ctors: None,
    };
    assert!(valid(&schema, &span, &Val::ctor(0, vec![value, other])));
}

#[test]
fn anonymous_sum_restrictions_and_all_fields_are_checked() {
    let schema = crate::catalog::engine_schema();
    let ty = Ty::Sum {
        ctors: vec![
            ("A".into(), vec![]),
            ("B".into(), vec![("number".into(), Ty::Base(Base::I8))]),
        ],
        sel: Some(vec![1]),
    };
    assert!(valid(&schema, &ty, &Val::ctor(1, vec![Val::Int(127)])));
    assert!(!valid(&schema, &ty, &Val::tag(0)));
    assert!(!valid(&schema, &ty, &Val::ctor(1, vec![Val::Int(128)])));
    assert!(!valid(&schema, &ty, &Val::ctor(1, vec![])));
}
