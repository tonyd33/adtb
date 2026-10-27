use adtb_lang::types::{Base, Ty};
use adtb_reference::Value;
use adtb_testkit::fixture::{Fixture, int, key, set, text};
use adtb_testkit::inhabit::{Judge, Mismatch, inhabits};

const SCHEMA: &str = "
type Pos = new I64 where self > 0
type Status = Draft | Sent { at: I64 } | Paid { at: I64, by: Text }
type Item = { id: Key items, name: Text, n: Pos, status: Status }
type Tagged { id: Key tags, item: Key items } = Small { size: I64 } | Large { label: Text }
type Note = { tag: Key tags.Small, body: Text }
type Pair = { a: I64, b: Text, w: I64 }
relation items : Item key id generate sequence
relation tags : Tagged key id generate sequence
relation notes : Note key tag
relation pairs : Pair key (a, b)
";

fn fixture() -> Fixture {
    let mut f = Fixture::new(SCHEMA);
    f.ok(r#"insert items { name: "a", n: Pos(1), status: Draft }"#);
    f.ok("insert tags.Small { item: items.key(1), size: 3 }");
    f.ok(r#"insert tags.Large { item: items.key(1), label: "l" }"#);
    f
}

fn holds(f: &Fixture, v: &Value, ty: &str) {
    let t = f.ty(ty);
    if let Err(m) = inhabits(v, &t, &f.schema) {
        panic!("{v:?} inhabits `{ty}`: {m}");
    }
}

fn fails(f: &Fixture, v: &Value, ty: &str) -> Mismatch {
    let t = f.ty(ty);
    inhabits(v, &t, &f.schema).expect_err(&format!("{v:?} does not inhabit `{ty}`"))
}

fn ctor(c: u32, fs: Vec<(&str, Value)>) -> Value {
    Value::Ctor(c, fs.into_iter().map(|(n, v)| (n.to_string(), v)).collect())
}

#[test]
fn integers_inhabit_their_width() {
    let f = fixture();
    holds(&f, &int(127), "I8");
    holds(&f, &int(-128), "I8");
    fails(&f, &int(128), "I8");
    fails(&f, &int(-32769), "I16");
    holds(&f, &int(i64::from(i32::MAX)), "I32");
    fails(&f, &int(i64::from(i32::MAX) + 1), "I32");
    holds(&f, &int(i64::MIN), "I64");
    fails(&f, &text("1"), "I64");
}

#[test]
fn a_decimal_has_its_scale_and_at_most_38_digits() {
    let f = fixture();
    holds(&f, &Value::Decimal(105, 2), "Decimal 2");
    fails(&f, &Value::Decimal(105, 1), "Decimal 2");
    let max = 10i128.pow(38) - 1;
    holds(&f, &Value::Decimal(-max, 0), "Decimal 0");
    fails(&f, &Value::Decimal(max + 1, 0), "Decimal 0");
}

#[test]
fn a_float_is_canonical() {
    let f = fixture();
    holds(&f, &Value::F64(1.5), "F64");
    holds(&f, &Value::F64(f64::NAN), "F64");
    holds(&f, &Value::F32(f32::INFINITY), "F32");
    fails(&f, &Value::F64(-0.0), "F64");
    fails(&f, &Value::F32(-0.0), "F32");
    fails(
        &f,
        &Value::F64(f64::from_bits(f64::NAN.to_bits() | 1)),
        "F64",
    );
    fails(&f, &Value::F32(1.0), "F64");
}

#[test]
fn text_is_in_nfc() {
    let f = fixture();
    holds(&f, &text("caf\u{e9}"), "Text");
    let m = fails(&f, &text("cafe\u{301}"), "Text");
    assert!(m.why.contains("NFC"), "{m}");
}

#[test]
fn bytes_and_sort_keys_are_distinct() {
    let f = fixture();
    holds(&f, &Value::Bytes(vec![1, 2]), "Bytes");
    holds(&f, &Value::SortKey(vec![1, 2]), "SortKey");
    fails(&f, &Value::SortKey(vec![1]), "Bytes");
    fails(&f, &Value::Bytes(vec![1]), "SortKey");
}

#[test]
fn times_are_in_range() {
    let f = fixture();
    holds(&f, &Value::Instant(i64::MIN), "Instant");
    holds(&f, &Value::Date(-719_528), "Date");
    fails(&f, &Value::Date(i64::MAX / 1000), "Date");
    holds(&f, &Value::Time(86_399_999_999), "Time");
    fails(&f, &Value::Time(86_400_000_000), "Time");
    fails(&f, &Value::Time(-1), "Time");
    holds(&f, &Value::DateTime(0), "DateTime");
    holds(&f, &Value::Zoned(0, "Europe/Paris".into()), "ZonedDateTime");
    fails(&f, &Value::Zoned(0, "Mars/Olympus".into()), "ZonedDateTime");
    holds(&f, &Value::Duration(-5), "Duration");
    holds(
        &f,
        &Value::Period {
            months: 1,
            days: -2,
        },
        "Period",
    );
    fails(&f, &Value::Duration(5), "Instant");
}

#[test]
fn a_sum_value_is_one_of_its_constructors_with_its_fields() {
    let f = fixture();
    holds(&f, &ctor(0, vec![]), "Status");
    holds(&f, &ctor(1, vec![("at", int(3))]), "Status");
    holds(
        &f,
        &ctor(2, vec![("at", int(3)), ("by", text("x"))]),
        "Status",
    );
    fails(&f, &ctor(3, vec![]), "Status");
    fails(&f, &ctor(1, vec![]), "Status");
    fails(
        &f,
        &ctor(2, vec![("by", text("x")), ("at", int(3))]),
        "Status",
    );
    let m = fails(&f, &ctor(1, vec![("at", text("3"))]), "Status");
    assert_eq!(m.path, "#Sent.at", "{m}");
    fails(&f, &Value::Record(vec![]), "Status");
}

#[test]
fn a_row_type_holds_only_its_constructors() {
    let f = fixture();
    let (t, small) = f.ctor("Tagged", "Small");
    let row = ctor(
        1,
        vec![("id", key(1)), ("item", key(1)), ("label", text("l"))],
    );
    let all = Ty::Data {
        def: t,
        args: vec![],
        ctors: None,
    };
    let only_small = Ty::Data {
        def: t,
        args: vec![],
        ctors: Some(vec![small]),
    };
    assert_eq!(inhabits(&row, &all, &f.schema), Ok(()));
    let m = inhabits(&row, &only_small, &f.schema).expect_err("a `Large` is no `Small`");
    assert!(m.why.contains("constructor set"), "{m}");
}

#[test]
fn header_fields_come_first() {
    let f = fixture();
    let row = ctor(0, vec![("id", key(1)), ("item", key(2)), ("size", int(1))]);
    holds(&f, &row, "Tagged");
    let swapped = ctor(0, vec![("size", int(1)), ("id", key(1)), ("item", key(2))]);
    fails(&f, &swapped, "Tagged");
}

#[test]
fn a_record_has_exactly_its_fields() {
    let f = fixture();
    let r = Value::record([("a", int(1)), ("b", text("x"))]);
    holds(&f, &r, "{ a: I64, b: Text }");
    fails(&f, &r, "{ a: I64 }");
    fails(&f, &r, "{ a: I64, b: I64 }");
    fails(&f, &r, "{ a: I64, b: Text, c: I64 }");
    holds(&f, &Value::unit(), "()");
    fails(&f, &r, "()");
}

#[test]
fn a_declared_record_is_its_one_constructor() {
    let f = fixture();
    let p = ctor(0, vec![("a", int(1)), ("b", text("x")), ("w", int(2))]);
    holds(&f, &p, "Pair");
    fails(
        &f,
        &Value::record([("a", int(1)), ("b", text("x")), ("w", int(2))]),
        "Pair",
    );
}

#[test]
fn an_anonymous_sum_holds_its_selected_constructors() {
    let f = fixture();
    let ctors = vec![
        (
            "A".to_string(),
            vec![("x".to_string(), Ty::Base(Base::I64))],
        ),
        ("B".to_string(), vec![]),
    ];
    let ty = Ty::Sum {
        ctors: ctors.clone(),
        sel: None,
    };
    let a = ctor(0, vec![("x", int(1))]);
    assert_eq!(inhabits(&a, &ty, &f.schema), Ok(()));
    assert!(inhabits(&ctor(2, vec![]), &ty, &f.schema).is_err());
    assert!(inhabits(&ctor(0, vec![("x", text("1"))]), &ty, &f.schema).is_err());
    let only_b = Ty::Sum {
        ctors,
        sel: Some(vec![1]),
    };
    assert!(inhabits(&a, &only_b, &f.schema).is_err());
    assert_eq!(inhabits(&ctor(1, vec![]), &only_b, &f.schema), Ok(()));
}

#[test]
fn a_tag_is_a_constructor_without_fields() {
    let f = fixture();
    holds(&f, &ctor(2, vec![]), "Tag Status");
    fails(&f, &ctor(3, vec![]), "Tag Status");
    fails(&f, &ctor(1, vec![("at", int(3))]), "Tag Status");
}

#[test]
fn library_sums_take_their_arguments() {
    let f = fixture();
    holds(&f, &Value::some(int(1)), "Option I64");
    holds(&f, &Value::none(), "Option I64");
    fails(&f, &Value::some(text("1")), "Option I64");
    holds(&f, &Value::bool(true), "Bool");
    fails(&f, &int(1), "Bool");
}

#[test]
fn a_key_holds_the_value_of_its_key_fields() {
    let f = fixture();
    holds(&f, &key(7), "Key items");
    fails(&f, &int(7), "Key items");
    fails(&f, &Value::key(text("7")), "Key items");
    holds(&f, &Value::key(key(1)), "Key notes");
    let composite = Value::key(Value::record([("a", int(1)), ("b", text("x"))]));
    holds(&f, &composite, "Key pairs");
    fails(&f, &Value::key(int(1)), "Key pairs");
}

#[test]
fn a_newtype_value_is_its_base_value() {
    let f = fixture();
    holds(&f, &int(5), "Pos");
    holds(&f, &int(-5), "Pos");
    fails(&f, &text("5"), "Pos");
}

#[test]
fn a_set_holds_values_of_its_element_type() {
    let f = fixture();
    holds(&f, &set([int(1), int(2)]), "Set I8");
    holds(&f, &set([]), "Set I8");
    let m = fails(&f, &set([int(1), int(200)]), "Set I8");
    assert_eq!(m.path, "[1]", "{m}");
    fails(&f, &Value::List(vec![int(1)]), "Set I8");
    let nonempty = Ty::NonEmpty(Box::new(Ty::Base(Base::I64)));
    assert!(inhabits(&set([]), &nonempty, &f.schema).is_err());
    assert_eq!(inhabits(&set([int(1)]), &nonempty, &f.schema), Ok(()));
}

#[test]
fn a_group_is_its_key_and_its_nonempty_rows() {
    let f = fixture();
    let ty = Ty::Group(
        Box::new(Ty::Base(Base::Text)),
        Box::new(Ty::Base(Base::I64)),
    );
    let group = |rows: Vec<Value>| {
        Value::Record(vec![("key".into(), text("k")), ("rows".into(), set(rows))])
    };
    assert_eq!(inhabits(&group(vec![int(1)]), &ty, &f.schema), Ok(()));
    assert!(inhabits(&group(vec![]), &ty, &f.schema).is_err());
    assert!(inhabits(&group(vec![text("1")]), &ty, &f.schema).is_err());
}

#[test]
fn parameters_and_change_sets_have_no_values() {
    let f = fixture();
    assert!(inhabits(&int(1), &Ty::Param(0), &f.schema).is_err());
    assert!(inhabits(&Value::unit(), &Ty::Change, &f.schema).is_err());
}

#[test]
fn a_result_ordered_by_order_by_is_a_list_of_distinct_elements() {
    let f = fixture();
    let judge = Judge::new(&f.schema);
    let ty = f.ty("Set I64");
    assert_eq!(
        judge.result(&Value::List(vec![int(2), int(1)]), &ty),
        Ok(())
    );
    assert!(
        judge
            .result(&Value::List(vec![int(1), int(1)]), &ty)
            .is_err()
    );
    assert!(judge.result(&Value::List(vec![text("1")]), &ty).is_err());
    assert!(judge.value(&Value::List(vec![int(1)]), &ty).is_err());
}

#[test]
fn a_key_of_a_constructor_set_names_a_row_of_that_set() {
    let f = fixture();
    let judge = Judge::over(&f.schema, &f.program, &f.db);
    let small = f.ty("Key tags.Small");
    let large_key = f.one("from t in tags.Large select key t");
    let small_key = f.one("from t in tags.Small select key t");
    assert_eq!(judge.value(&small_key, &small), Ok(()));
    let m = judge
        .value(&large_key, &small)
        .expect_err("a key of a `Large` is no `Key tags.Small`");
    assert!(m.why.contains("names no row"), "{m}");
    assert!(judge.value(&key(99), &small).is_err());
    assert_eq!(judge.value(&key(99), &f.ty("Key tags")), Ok(()));
    assert_eq!(inhabits(&large_key, &small, &f.schema), Ok(()));
}

#[test]
fn a_stored_row_references_rows_that_exist() {
    let f = fixture();
    let judge = Judge::over(&f.schema, &f.program, &f.db);
    let tags = f.rel("tags");
    let (_, small) = f.ctor("Tagged", "Small");
    let good = ctor(0, vec![("id", key(1)), ("item", key(1)), ("size", int(3))]);
    assert_eq!(judge.row(tags, small, &good), Ok(()));
    let dangling = ctor(0, vec![("id", key(1)), ("item", key(9)), ("size", int(3))]);
    let m = judge
        .row(tags, small, &dangling)
        .expect_err("a reference names a row");
    assert_eq!(m.path, "#Small.item", "{m}");
}

#[test]
fn a_stored_row_holds_newtype_values_within_their_domain() {
    let f = fixture();
    let judge = Judge::over(&f.schema, &f.program, &f.db);
    let items = f.rel("items");
    let row = |n: i64| {
        ctor(
            0,
            vec![
                ("id", key(1)),
                ("name", text("a")),
                ("n", int(n)),
                ("status", ctor(0, vec![])),
            ],
        )
    };
    assert_eq!(judge.row(items, 0, &row(1)), Ok(()));
    let m = judge
        .row(items, 0, &row(0))
        .expect_err("0 is outside `Pos`");
    assert_eq!(m.path, ".n", "{m}");
    assert!(m.why.contains("domain"), "{m}");
    assert_eq!(judge.value(&row(0), &f.ty("Item")), Ok(()));
}

#[test]
fn a_mismatch_names_the_value_and_the_type() {
    let f = fixture();
    let m = fails(&f, &Value::record([("a", text("x"))]), "{ a: I64 }");
    let shown = m.to_string();
    assert!(
        shown.contains(".a") && shown.contains("I64") && shown.contains("Text(\"x\")"),
        "{shown}"
    );
}
