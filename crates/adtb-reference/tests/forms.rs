//! Kernel forms, primitives and faults, driven through the front end.

mod common;

use adtb_reference::{Abort, Change, Database, Fault, Limit, Value, days_from_civil};
use common::*;

const SCHEMA: &str = r#"
type User = { id: Key users, name: Text }
relation users : User key id generate sequence
type Order = { id: Key orders, user: Key users, total: I64 }
relation orders : Order key id generate sequence
type Price = { id: Key prices, amount: Decimal 2 }
relation prices : Price key id generate sequence
type Reading = { id: Key readings, v: F64 }
relation readings : Reading key id generate sequence
type Stage ordered = Draft | Sent { at: I64 } | Paid { at: I64, ref: Text }
type Doc = { id: Key docs, stage: Stage }
relation docs : Doc key id generate sequence
type Member = { user: Key users, team: Text }
relation members : Member key (user, team)

fn twice [a: Num] (x: a) -> a = x + x
fn apply2 (f: I64 -> I64, x: I64) -> I64 = f(f(x))
fn ignore (x: I64) -> I64 = 0
fn named () -> Set Text = from u in users select u.name

constraint no_empty_names = from u in users where u.name == "" select u
constraint new_users = from u in inserted(users) select u
check positive on orders (total > 0)
"#;

fn fixture() -> Fixture {
    let mut f = Fixture::new(SCHEMA);
    f.rows(
        "users",
        r#"[User { id: users.key(1), name: "ann" },
            User { id: users.key(2), name: "bo" },
            User { id: users.key(3), name: "" }]"#,
    );
    f.rows(
        "orders",
        r"[Order { id: orders.key(1), user: users.key(1), total: 9223372036854775807 },
            Order { id: orders.key(2), user: users.key(1), total: 1 },
            Order { id: orders.key(3), user: users.key(2), total: -1 },
            Order { id: orders.key(4), user: users.key(2), total: 0 }]",
    );
    f.rows(
        "prices",
        "[Price { id: prices.key(1), amount: 1.25 }, Price { id: prices.key(2), amount: 2.50 }]",
    );
    f.rows_with(
        "readings",
        "[Reading { id: readings.key(1), v: x }, Reading { id: readings.key(2), v: y },
          Reading { id: readings.key(3), v: z }]",
        &[
            ("x", "F64", Value::F64(f64::NAN)),
            ("y", "F64", Value::F64(f64::INFINITY)),
            ("z", "F64", Value::F64(-1.5)),
        ],
    );
    f.rows(
        "docs",
        r#"[Doc { id: docs.key(1), stage: Paid { at: 1, ref: "a" } },
            Doc { id: docs.key(2), stage: Sent { at: 9 } },
            Doc { id: docs.key(3), stage: Draft }]"#,
    );
    f.rows(
        "members",
        r#"[Member { user: users.key(1), team: "x" }, Member { user: users.key(2), team: "x" }]"#,
    );
    f
}

fn fault(f: Fault) -> Result<Value, Abort> {
    Err(Abort::Fault(f))
}

fn dec(d: i128, s: u8) -> Value {
    Value::Decimal(d, s)
}

fn i64p(name: &str, v: i64) -> (&str, &'static str, Value) {
    (name, "I64", int(v))
}

#[test]
fn integer_arithmetic_faults_outside_its_type() {
    let f = fixture();
    let max = [i64p("x", i64::MAX)];
    assert_eq!(fault(Fault::ArithmeticOverflow), f.eval_with("x + 1", &max));
    let min = [i64p("x", i64::MIN)];
    assert_eq!(fault(Fault::ArithmeticOverflow), f.eval_with("-x", &min));
    assert_eq!(
        fault(Fault::ArithmeticOverflow),
        f.eval_with("x / -1", &min)
    );
    assert_eq!(Ok(int(0)), f.eval_with("x % -1", &min));
    assert_eq!(
        fault(Fault::ArithmeticOverflow),
        f.eval_with("abs(x)", &min)
    );
    let small = [("x", "I8", int(127))];
    assert_eq!(
        fault(Fault::ArithmeticOverflow),
        f.eval_with("x + 1", &small)
    );
    assert_eq!(Ok(int(126)), f.eval_with("x - 1", &small));
}

#[test]
fn division_truncates_and_faults_on_zero() {
    let f = fixture();
    assert_eq!(Ok(int(-3)), f.eval("-7 / 2"));
    assert_eq!(Ok(int(-1)), f.eval("-7 % 2"));
    assert_eq!(Ok(int(1)), f.eval("7 % -2"));
    let zero = [i64p("z", 0)];
    assert_eq!(fault(Fault::DivisionByZero), f.eval_with("1 / z", &zero));
    assert_eq!(fault(Fault::DivisionByZero), f.eval_with("1 % z", &zero));
}

#[test]
fn checked_forms_return_none_where_the_operator_faults() {
    let f = fixture();
    let max = [i64p("x", i64::MAX)];
    assert_eq!(Ok(Value::none()), f.eval_with("checked_add(x, 1)", &max));
    assert_eq!(
        Ok(Value::some(int(i64::MAX - 1))),
        f.eval_with("checked_sub(x, 1)", &max)
    );
    assert_eq!(Ok(Value::none()), f.eval("checked_div(1, 0)"));
    assert_eq!(Ok(Value::some(int(-4))), f.eval("checked_neg(4)"));
}

#[test]
fn guards_evaluate_only_the_selected_branch() {
    let f = fixture();
    let zero = [i64p("z", 0)];
    assert_eq!(
        Ok(Value::bool(false)),
        f.eval_with("z != 0 and 1 / z == 0", &zero)
    );
    assert_eq!(
        Ok(Value::bool(true)),
        f.eval_with("z == 0 or 1 / z == 0", &zero)
    );
    assert_eq!(Ok(int(0)), f.eval_with("if z > 0 then 1 / z else 0", &zero));
}

#[test]
fn call_arguments_are_evaluated_whether_or_not_the_body_uses_them() {
    let f = fixture();
    let zero = [i64p("z", 0)];
    assert_eq!(
        fault(Fault::DivisionByZero),
        f.eval_with("ignore(1 / z)", &zero)
    );
    assert_eq!(Ok(int(0)), f.eval_with("ignore(z)", &zero));
}

#[test]
fn try_catches_faults_only() {
    let f = fixture();
    let zero = [i64p("z", 0)];
    let err = |x: Fault| Value::Ctor(1, vec![("error".into(), f.fault(x))]);
    let ok = |v: Value| Value::Ctor(0, vec![("value".into(), v)]);
    assert_eq!(
        Ok(err(Fault::DivisionByZero)),
        f.eval_with("try (1 / z)", &zero)
    );
    assert_eq!(Ok(ok(int(1))), f.eval_with("try (z + 1)", &zero));
    assert_eq!(Ok(err(Fault::NoSuchRow)), f.eval("try users.key(9).name"));
}

#[test]
fn following_a_key_that_names_no_row_faults() {
    let f = fixture();
    assert_eq!(Ok(text("ann")), f.eval("users.key(1).name"));
    assert_eq!(fault(Fault::NoSuchRow), f.eval("users.key(9).name"));
    let k = |i| [("k", "Key users", key(i))];
    assert_eq!(
        Ok(text("bo")),
        f.eval_with("match k | User u -> u.name", &k(2))
    );
    assert_eq!(
        fault(Fault::NoSuchRow),
        f.eval_with("match k | User u -> u.name", &k(9))
    );
}

#[test]
fn composite_keys_are_records_of_their_fields() {
    let f = fixture();
    let got = f.value("from m in members select key m");
    let k = |u| Value::key(Value::record([("team", text("x")), ("user", key(u))]));
    assert_eq!(set([k(1), k(2)]), got);
    assert_eq!(
        Ok(Value::bool(true)),
        f.eval(r#"exists m in members where key m == members.key(user: users.key(2), team: "x")"#)
    );
    assert_eq!(
        Ok(Value::record([("team", text("x")), ("user", key(1))])),
        f.eval(r#"value(members.key(user: users.key(1), team: "x"))"#)
    );
}

#[test]
fn an_integer_sum_is_exact_and_checked_once() {
    let f = fixture();
    assert_eq!(
        fault(Fault::ArithmeticOverflow),
        f.eval("sum(o.total for o in orders where o.user == users.key(1))")
    );
    assert_eq!(
        Ok(int(i64::MAX)),
        f.eval("sum(o.total for o in orders)"),
        "max + 1 - 1 + 0 is in range whatever the order"
    );
}

#[test]
fn aggregates_over_an_empty_range() {
    let f = fixture();
    let none = "from o in orders where o.total > 9223372036854775806 and o.total < 0 select o";
    assert_eq!(
        Ok(int(0)),
        f.eval(&format!("sum(o.total for o in ({none}))"))
    );
    assert_eq!(Ok(int(0)), f.eval(&format!("count(o for o in ({none}))")));
    assert_eq!(
        Ok(Value::none()),
        f.eval(&format!("min(o.total for o in ({none}))"))
    );
    assert_eq!(
        Ok(Value::some(int(-1))),
        f.eval("min(o.total for o in orders)")
    );
    assert_eq!(Ok(int(4)), f.eval("count(o for o in orders)"));
    assert_eq!(
        Ok(int(2)),
        f.eval("count(u for u in (from o in orders select o.user))")
    );
}

#[test]
fn float_aggregates_and_nan() {
    let f = fixture();
    assert_eq!(
        Ok(Value::some(Value::F64(f64::NAN))),
        f.eval("max(r.v for r in readings)"),
        "NaN is greater than every other float"
    );
    assert_eq!(
        Ok(Value::some(Value::F64(-1.5))),
        f.eval("min(r.v for r in readings)")
    );
    assert_eq!(
        Ok(Value::some(Value::F64(f64::NAN))),
        f.eval("avg(r.v for r in readings)")
    );
    let p = [
        ("x", "F64", Value::F64(f64::NAN)),
        ("y", "F64", Value::F64(f64::INFINITY)),
    ];
    assert_eq!(Ok(Value::bool(true)), f.eval_with("x == x", &p));
    assert_eq!(Ok(Value::bool(true)), f.eval_with("x > y", &p));
    let z = [("x", "F64", Value::F64(0.0))];
    assert_eq!(Ok(Value::F64(0.0)), f.eval_with("-x", &z));
    assert_eq!(Ok(Value::F64(f64::NAN)), f.eval_with("x / x", &z));
    assert_eq!(Ok(Value::F64(f64::INFINITY)), f.eval_with("1.0 / x", &z));
}

#[test]
fn decimals_keep_their_scale() {
    let f = fixture();
    assert_eq!(Ok(dec(175, 2)), f.eval("1.25 + 0.5"));
    assert_eq!(Ok(dec(1875, 3)), f.eval("1.5 * 1.25"));
    assert_eq!(Ok(dec(-125, 2)), f.eval("-1.25"));
    let scales = [
        ("x", "Decimal 1", dec(10, 1)),
        ("y", "Decimal 2", dec(100, 2)),
    ];
    assert_eq!(Ok(Value::bool(true)), f.eval_with("x == y", &scales));
    let scales = [
        ("x", "Decimal 1", dec(15, 1)),
        ("y", "Decimal 2", dec(149, 2)),
    ];
    assert_eq!(Ok(Value::bool(true)), f.eval_with("x > y", &scales));
    assert_eq!(Ok(dec(3333, 4)), f.eval("div(1.00, 3.00, 4, HalfEven)"));
    assert_eq!(Ok(dec(6667, 4)), f.eval("div(2.0, 3.0, 4, HalfUp)"));
    assert_eq!(Ok(dec(234, 2)), f.eval("round(2.345, 2, HalfEven)"));
    assert_eq!(Ok(dec(235, 2)), f.eval("round(2.345, 2, HalfUp)"));
    assert_eq!(Ok(dec(-235, 2)), f.eval("round(-2.345, 2, Floor)"));
    assert_eq!(Ok(dec(1500, 3)), f.eval("rescale(1.5, 3)"));
    assert_eq!(fault(Fault::DivisionByZero), f.eval("div(1.0, 0.0, 2, Up)"));
    assert_eq!(Ok(dec(375, 2)), f.eval("sum(p.amount for p in prices)"));
}

#[test]
fn decimal_overflow_is_past_38_digits() {
    let f = fixture();
    let big = [
        ("x", "Decimal 0", dec(10i128.pow(38) - 1, 0)),
        ("y", "Decimal 0", dec(1, 0)),
    ];
    assert_eq!(fault(Fault::ArithmeticOverflow), f.eval_with("x + y", &big));
    assert_eq!(Ok(Value::none()), f.eval_with("checked_add(x, y)", &big));
    assert_eq!(Ok(dec(10i128.pow(38) - 2, 0)), f.eval_with("x - y", &big));
    assert_eq!(
        fault(Fault::ArithmeticOverflow),
        f.eval_with("rescale(x, 2)", &big)
    );
}

#[test]
fn conversions_fault_or_round() {
    let f = fixture();
    assert_eq!(fault(Fault::OutOfRange), f.eval("i8(300)"));
    assert_eq!(Ok(Value::none()), f.eval("checked_i8(300)"));
    assert_eq!(Ok(Value::some(int(-128))), f.eval("checked_i8(-128)"));
    assert_eq!(Ok(int(2)), f.eval("i32(2.5, HalfEven)"));
    assert_eq!(Ok(int(3)), f.eval("i32(2.5, HalfUp)"));
    assert_eq!(Ok(int(-3)), f.eval("i64(-2.5, Floor)"));
    assert_eq!(Ok(dec(300, 2)), f.eval("decimal(3, 2)"));
    assert_eq!(Ok(Value::F64(0.1)), f.eval("f64(0.1)"));
    assert_eq!(Ok(Value::F32(0.1)), f.eval("f32(0.1)"));
    let nan = [("x", "F64", Value::F64(f64::NAN))];
    assert_eq!(
        fault(Fault::NotFinite),
        f.eval_with("i64(x, HalfEven)", &nan)
    );
    assert_eq!(
        fault(Fault::NotFinite),
        f.eval_with("decimal(x, 2, HalfEven)", &nan)
    );
    let huge = [("x", "F64", Value::F64(1e300))];
    assert_eq!(fault(Fault::OutOfRange), f.eval_with("f32(x)", &huge));
    assert_eq!(Ok(Value::none()), f.eval_with("checked_f32(x)", &huge));
    assert_eq!(fault(Fault::OutOfRange), f.eval_with("i64(x, Down)", &huge));
    let tenth = [("x", "F64", Value::F64(0.1))];
    assert_eq!(
        Ok(dec(10, 2)),
        f.eval_with("decimal(x, 2, HalfEven)", &tenth)
    );
    assert_eq!(Ok(dec(11, 2)), f.eval_with("decimal(x, 2, Up)", &tenth));
}

#[test]
fn text_functions() {
    let f = fixture();
    assert_eq!(Ok(text("ann bo")), f.eval(r#""ann" ++ " " ++ "bo""#));
    assert_eq!(Ok(int(3)), f.eval(r#"scalar_len("añb")"#));
    assert_eq!(Ok(int(4)), f.eval(r#"bytes_len("añb")"#));
    assert_eq!(Ok(Value::bool(true)), f.eval(r#"starts_with("abc", "ab")"#));
    assert_eq!(Ok(Value::bool(true)), f.eval(r#"ends_with("abc", "bc")"#));
    assert_eq!(Ok(Value::bool(true)), f.eval(r#"contains("abc", "")"#));
    assert_eq!(Ok(text("straße")), f.eval(r#"lower("STRAßE")"#));
    assert_eq!(Ok(text("STRASSE")), f.eval(r#"upper("straße")"#));
    assert_eq!(Ok(Value::some(text("hé"))), f.eval(r#"utf8(bytes("hé"))"#));
}

#[test]
fn grapheme_len_counts_clusters() {
    let f = fixture();
    assert_eq!(Ok(int(2)), f.eval("grapheme_len(\"e\u{301}x\")"));
}

#[test]
fn fold_is_full_case_folding() {
    let f = fixture();
    assert_eq!(Ok(text("strasse")), f.eval(r#"fold("STRAßE")"#));
}

#[test]
fn matches_finds_a_pattern_anywhere() {
    let f = fixture();
    assert_eq!(
        Ok(Value::bool(true)),
        f.eval(r#"matches("a@b", "^[^@]+@[^@]+$")"#)
    );
    assert_eq!(Ok(Value::bool(true)), f.eval(r#"matches("xaby", "ab")"#));
    assert_eq!(Ok(Value::bool(false)), f.eval(r#"matches("xaby", "^ab")"#));
}

#[test]
fn text_of_a_value() {
    let f = fixture();
    assert_eq!(Ok(text("-12")), f.eval("text(-12)"));
    assert_eq!(Ok(text("1.50")), f.eval("text(1.50)"));
    assert_eq!(Ok(text("-0.05")), f.eval("text(-0.05)"));
    assert_eq!(Ok(text("30s")), f.eval("text(30s)"));
    let i = [("t", "Instant", Value::Instant(1_000_001))];
    assert_eq!(
        Ok(text("1970-01-01T00:00:01.000001Z")),
        f.eval_with("text(t)", &i)
    );
    let canonical = "0190a4b2-7c3e-7d4f-9a21-5b6c7d8e9f01";
    assert_eq!(
        Ok(text(canonical)),
        f.eval(r#"text(uuid("0190A4B2-7C3E-7D4F-9A21-5B6C7D8E9F01"))"#)
    );
    assert_eq!(fault(Fault::InvalidText), f.eval(r#"uuid("0190a4b2")"#));
    let nan = [("x", "F64", Value::F64(f64::NAN))];
    assert_eq!(Ok(text("NaN")), f.eval_with("text(x)", &nan));
}

#[test]
fn print_gives_the_textual_syntax() {
    let f = fixture();
    assert_eq!(Ok(text("users.key(1)")), f.eval("print(users.key(1))"));
    assert_eq!(
        Ok(text(r#"User { id: users.key(1), name: "a\"b" }"#)),
        f.eval(r#"print(User { id: users.key(1), name: "a\"b" })"#)
    );
    assert_eq!(Ok(text(r"[2, 10]")), f.eval("print([10, 2, 10])"));
    assert_eq!(
        Ok(text(r#"members.key(team: "x", user: users.key(1))"#)),
        f.eval(r#"print(members.key(user: users.key(1), team: "x"))"#)
    );
}

#[test]
fn spans() {
    let f = fixture();
    assert_eq!(fault(Fault::InvalidSpan), f.eval("span(2, 1)"));
    assert_eq!(
        Ok(Value::bool(true)),
        f.eval("overlaps(span(1, 3), span(2, 4))")
    );
    assert_eq!(
        Ok(Value::bool(false)),
        f.eval("overlaps(span(1, 2), span(2, 4))")
    );
    assert_eq!(
        Ok(Value::bool(true)),
        f.eval("covers(span(1, 5), span(2, 4))")
    );
    assert_eq!(Ok(Value::bool(true)), f.eval("covers(span(1, 5), 1)"));
    assert_eq!(Ok(Value::bool(false)), f.eval("covers(span(1, 5), 5)"));
    assert_eq!(Ok(int(3)), f.eval("end(span(1, 3))"));
}

#[test]
fn time_arithmetic() {
    let f = fixture();
    assert_eq!(Ok(Value::Duration(330_000_000)), f.eval("30s + 5min"));
    assert_eq!(Ok(Value::Duration(60_000_000)), f.eval("30s * 2"));
    assert_eq!(fault(Fault::DivisionByZero), f.eval("30s / 0"));
    assert_eq!(Ok(Value::Duration(-1_000)), f.eval("-millis(1)"));
    assert_eq!(fault(Fault::OutOfRange), f.eval("days(3000000000)"));
    assert_eq!(
        Ok(Value::Period {
            months: 24,
            days: 0
        }),
        f.eval("years(2)")
    );
    let jan31 = Value::Date(days_from_civil(2021, 1, 31));
    let d = [("d", "Date", jan31)];
    assert_eq!(
        Ok(Value::Date(days_from_civil(2021, 2, 28))),
        f.eval_with("add_period(d, months(1), Clamp)", &d)
    );
    assert_eq!(
        Ok(Value::none()),
        f.eval_with("add_period(d, months(1), Reject)", &d)
    );
    assert_eq!(
        Ok(Value::some(Value::Date(days_from_civil(2021, 3, 31)))),
        f.eval_with("add_period(d, months(2), Reject)", &d)
    );
    let i = [
        ("a", "Instant", Value::Instant(10)),
        ("b", "Instant", Value::Instant(4)),
    ];
    assert_eq!(Ok(Value::Duration(6)), f.eval_with("a - b", &i));
    assert_eq!(
        Ok(Value::Instant(10 + 30_000_000)),
        f.eval_with("a + 30s", &i)
    );
}

#[test]
fn ordered_sums_order_by_constructor_then_fields() {
    let f = fixture();
    let got = f.value("from d in docs select d.stage order by d.stage");
    let Value::List(stages) = got else {
        panic!("an ordered result")
    };
    assert_eq!(
        vec![Some(0), Some(1), Some(2)],
        stages.iter().map(Value::tag).collect::<Vec<_>>()
    );
    let less = |a: &str, b: &str| {
        f.eval_with(
            "x < y",
            &[("x", "Stage", f.value(a)), ("y", "Stage", f.value(b))],
        )
    };
    assert_eq!(
        Ok(Value::bool(true)),
        less("Sent { at: 100 }", r#"Paid { at: 0, ref: "a" }"#)
    );
    assert_eq!(
        Ok(Value::bool(true)),
        less(r#"Paid { at: 1, ref: "b" }"#, r#"Paid { at: 2, ref: "a" }"#)
    );
    assert_eq!(Ok(Value::bool(true)), less("Draft", "Sent { at: 0 }"));
}

#[test]
fn top_faults_on_a_negative_count() {
    let f = fixture();
    let q = "top n by key u (from u in users select u)";
    assert_eq!(
        fault(Fault::NegativeCount),
        f.eval_with(q, &[i64p("n", -1)])
    );
    assert_eq!(
        2,
        f.eval_with(q, &[i64p("n", 2)])
            .unwrap()
            .as_set()
            .unwrap()
            .len()
    );
    assert_eq!(
        3,
        f.eval_with(q, &[i64p("n", 10)])
            .unwrap()
            .as_set()
            .unwrap()
            .len()
    );
}

#[test]
fn order_by_without_take_allows_ties() {
    let f = fixture();
    let got = f.value("from o in orders select o order by o.user");
    let Value::List(rows) = got else {
        panic!("an ordered result")
    };
    let users: Vec<Value> = rows
        .iter()
        .map(|r| r.field("user").unwrap().clone())
        .collect();
    assert_eq!(vec![key(1), key(1), key(2), key(2)], users);
}

#[test]
fn generic_functions_run_at_their_type_arguments() {
    let f = fixture();
    assert_eq!(Ok(int(6)), f.eval("twice(3)"));
    assert_eq!(Ok(dec(300, 2)), f.eval("twice(1.50)"));
    let small = [("x", "I8", int(100))];
    assert_eq!(
        fault(Fault::ArithmeticOverflow),
        f.eval_with("twice(x)", &small)
    );
}

#[test]
fn higher_order_functions_apply_their_arguments() {
    let f = fixture();
    assert_eq!(Ok(int(18)), f.eval("apply2(fn y -> y * 3, 2)"));
    assert_eq!(Ok(int(4)), f.eval("apply2(fn y -> twice(y) / 2 + 1, 2)"));
    assert_eq!(Ok(texts(["ann", "bo", ""])), f.eval("named()"));
}

#[test]
fn a_bounded_fix_terminates() {
    let f = fixture();
    let q = "fix n = [1] union (from r in n where r < 5 select r + 1) in count(x for x in n)";
    assert_eq!(Ok(int(5)), f.eval(q));
}

#[test]
fn an_unbounded_fix_reaches_the_statement_limit() {
    let f = fixture();
    let q = "fix n = [1] union (from r in n select r + 1) in count(x for x in n)";
    let text = q;
    let items = adtb_lang::parse::parse_adhoc(text, adtb_lang::parse::Options::default())
        .ok()
        .unwrap();
    let adhoc =
        adtb_lang::check::check_adhoc(&f.schema, &f.program, &items, &Default::default()).unwrap();
    let got = f
        .interp()
        .with_fix_limit(50)
        .eval(&adhoc.terms, adhoc.items[0], &[]);
    assert_eq!(Err(Abort::Limit(Limit::Rounds { rounds: 50 })), got);
}

#[test]
fn a_constraint_returns_its_violations() {
    let f = fixture();
    let c = f.program.constraints[0];
    let got = f.interp().eval(&f.program.terms, c, &[]).unwrap();
    assert_eq!(
        set([f.one(r#"from u in users where u.name == "" select u"#)]),
        got
    );
    let check = f.program.checks[0];
    let got = f.interp().eval(&f.program.terms, check, &[]).unwrap();
    assert_eq!(
        2,
        got.as_set().unwrap().len(),
        "orders with totals -1 and 0"
    );
}

#[test]
fn a_transition_constraint_reads_the_change() {
    let f = fixture();
    let c = f.program.constraints[1];
    assert_eq!(
        Ok(set([])),
        f.interp().eval(&f.program.terms, c, &[]),
        "nothing is inserted on the empty change"
    );
    let users = f.rel("users");
    let mut inserted = Database::new();
    let ann = f.one(r#"from u in users where u.name == "ann" select u"#);
    inserted.entry((users, 0)).or_default().insert(ann.clone());
    let change = Change {
        prior: Database::new(),
        inserted,
        deleted: Database::new(),
    };
    assert_eq!(
        Ok(set([ann])),
        f.interp()
            .with_change(&change)
            .eval(&f.program.terms, c, &[])
    );
}

#[test]
fn caller_is_the_given_principal() {
    let f = fixture();
    let items = adtb_lang::parse::parse_adhoc("caller", adtb_lang::parse::Options::default())
        .ok()
        .unwrap();
    let adhoc =
        adtb_lang::check::check_adhoc(&f.schema, &f.program, &items, &Default::default()).unwrap();
    let anon = Value::Ctor(2, Vec::new());
    assert_eq!(
        Ok(anon.clone()),
        f.interp()
            .with_caller(anon)
            .eval(&adhoc.terms, adhoc.items[0], &[])
    );
}

#[test]
fn at_needs_history() {
    let f = fixture();
    let got = f.eval_with(
        "at instant t { from u in users select u }",
        &[("t", "Instant", Value::Instant(0))],
    );
    assert!(matches!(got, Err(Abort::NotYet(_))), "{got:?}");
}

#[test]
fn parse_reads_the_textual_syntax() {
    let f = fixture();
    assert_eq!(Ok(Value::some(int(12))), f.eval(r#"I64.parse("12")"#));
    assert_eq!(Ok(Value::none()), f.eval(r#"I64.parse("twelve")"#));
}

#[test]
fn zones_name_instants() {
    let f = fixture();
    let i = [("t", "Instant", Value::Instant(1_000_000))];
    assert_eq!(
        Ok(Value::Instant(1_000_000)),
        f.eval_with(r#"to_instant(in_zone(t, "UTC"))"#, &i)
    );
    assert_eq!(
        fault(Fault::UnknownZone),
        f.eval_with(r#"in_zone(t, "Nowhere/Special")"#, &i)
    );
}
