//! The textual syntax of values: printing, reading at a known type, and versioned documents.

use std::collections::BTreeSet;

use adtb_lang::schema::{Generator, Schema, TypeDefKind};
use adtb_lang::types::{Base, Ty};
use adtb_reference::textual::{self, Document, Header};
use adtb_reference::{Value, days_from_civil};
use adtb_testkit::Rng;
use adtb_testkit::fixture::*;
use unicode_normalization::UnicodeNormalization;

const SCHEMA: &str = r"
type User = { id: Key users, name: Text }
relation users : User key id generate sequence
type Member = { user: Key users, team: Text }
relation members : Member key (user, team)
type Login { id: Key logins, user: Key users }
  = Password { hash: Bytes }
  | Social   { sub: Text, at: Instant }
relation logins : Login key id generate uuidv7
type Email = new Text
type Cents = new I64
type Profile = { user: Key users, email: Email }
relation profiles : Profile key user
type Stage ordered = Draft | Sent { at: I64 } | Paid { at: I64, ref: Text }
type Pair a = { left: a, right: a }
";

const DAY: i64 = 86_400_000_000;

fn fixture() -> Fixture {
    Fixture::new(SCHEMA)
}

fn print(f: &Fixture, v: &Value, ty: &str) -> String {
    textual::print(&f.schema, v, &f.ty(ty))
}

fn parse(f: &Fixture, t: &str, ty: &str) -> Result<Value, textual::ParseError> {
    textual::parse(&f.schema, t, &f.ty(ty))
}

fn reads(f: &Fixture, t: &str, ty: &str) -> Value {
    parse(f, t, ty).unwrap_or_else(|e| panic!("`{t}` does not read at {ty}: {e}"))
}

fn refused(f: &Fixture, t: &str, ty: &str) {
    if let Ok(v) = parse(f, t, ty) {
        panic!("`{t}` reads at {ty} as {v:?}")
    }
}

#[test]
fn literals_print_in_their_query_syntax() {
    let f = fixture();
    assert_eq!("-12", print(&f, &int(-12), "I64"));
    assert_eq!("1.50", print(&f, &Value::Decimal(150, 2), "Decimal 2"));
    assert_eq!("-0.05", print(&f, &Value::Decimal(-5, 2), "Decimal 2"));
    assert_eq!("0.1", print(&f, &Value::F64(0.1), "F64"));
    assert_eq!("1e300", print(&f, &Value::F64(1e300), "F64"));
    assert_eq!("0.1", print(&f, &Value::F32(0.1), "F32"));
    assert_eq!(
        r#""a\"b\\c\nd\u{7}""#,
        print(&f, &text("a\"b\\c\nd\u{7}"), "Text")
    );
    assert_eq!("30s", print(&f, &Value::Duration(30_000_000), "Duration"));
    assert_eq!(
        "-90min",
        print(&f, &Value::Duration(-5_400_000_000), "Duration")
    );
    assert_eq!("0s", print(&f, &Value::Duration(0), "Duration"));
    assert_eq!("True", print(&f, &Value::bool(true), "Bool"));
    assert_eq!("None", print(&f, &Value::none(), "Option I64"));
    assert_eq!(
        "Some { value: 3 }",
        print(&f, &Value::some(int(3)), "Option I64")
    );
}

#[test]
fn values_with_no_literal_print_in_their_forms() {
    let f = fixture();
    let at = (days_from_civil(2026, 10, 25) * DAY) + 9 * 3_600_000_000 + 30 * 60_000_000;
    assert_eq!("NaN", print(&f, &Value::F64(f64::NAN), "F64"));
    assert_eq!("inf", print(&f, &Value::F64(f64::INFINITY), "F64"));
    assert_eq!("-inf", print(&f, &Value::F32(f32::NEG_INFINITY), "F32"));
    assert_eq!(
        "2026-10-25T09:30:00.000000Z",
        print(&f, &Value::Instant(at), "Instant")
    );
    assert_eq!("2026-10-25", print(&f, &Value::Date(at / DAY), "Date"));
    assert_eq!("09:30:00.000000", print(&f, &Value::Time(at % DAY), "Time"));
    assert_eq!(
        "2026-10-25T09:30:00.000000",
        print(&f, &Value::DateTime(at), "DateTime")
    );
    assert_eq!(
        "2026-10-25T09:30:00.000000[Europe/Paris]",
        print(
            &f,
            &Value::Zoned(at, "Europe/Paris".into()),
            "ZonedDateTime"
        )
    );
    let period = |months, days| Value::Period { months, days };
    assert_eq!("P1M0D", print(&f, &period(1, 0), "Period"));
    assert_eq!("P0M-15D", print(&f, &period(0, -15), "Period"));
    assert_eq!("0x", print(&f, &Value::Bytes(vec![]), "Bytes"));
    assert_eq!("0x00ff", print(&f, &Value::Bytes(vec![0, 255]), "Bytes"));
}

#[test]
fn years_outside_four_digits_print_expanded() {
    let f = fixture();
    let date = |y, m, d| Value::Date(days_from_civil(y, m, d));
    assert_eq!("0000-01-01", print(&f, &date(0, 1, 1), "Date"));
    assert_eq!("9999-12-31", print(&f, &date(9999, 12, 31), "Date"));
    assert_eq!("+10000-01-01", print(&f, &date(10_000, 1, 1), "Date"));
    assert_eq!("-00001-12-31", print(&f, &date(-1, 12, 31), "Date"));
    assert_eq!(date(-1, 12, 31), reads(&f, "-00001-12-31", "Date"));
    assert_eq!(date(2026, 1, 2), reads(&f, "+02026-01-02", "Date"));
    refused(&f, "-0001-12-31", "Date");
    refused(&f, "10000-01-01", "Date");
}

#[test]
fn dates_and_times_stop_at_the_range_of_instant() {
    let f = fixture();
    let date = |y, m, d| Value::Date(days_from_civil(y, m, d));
    assert_eq!(date(294_247, 1, 10), reads(&f, "+294247-01-10", "Date"));
    assert_eq!(date(-290_308, 12, 21), reads(&f, "-290308-12-21", "Date"));
    refused(&f, "+294247-01-11", "Date");
    refused(&f, "-290308-12-20", "Date");
    assert_eq!(
        Value::Instant(i64::MAX),
        reads(&f, "+294247-01-10T04:00:54.775807Z", "Instant")
    );
    assert_eq!(
        Value::Instant(i64::MIN),
        reads(&f, "-290308-12-21T19:59:05.224192Z", "Instant")
    );
    refused(&f, "+294247-01-10T04:00:54.775808Z", "Instant");
    refused(&f, "-290308-12-21T19:59:05.224191Z", "Instant");
    assert_eq!(
        "-290308-12-21T19:59:05.224192",
        print(&f, &Value::DateTime(i64::MIN), "DateTime")
    );
    assert_eq!(
        "+294247-01-10T04:00:54.775807",
        print(&f, &Value::DateTime(i64::MAX), "DateTime")
    );
}

#[test]
fn constructors_records_keys_and_sets_print_as_the_spec_states() {
    let f = fixture();
    let ann = Value::Ctor(0, vec![("id".into(), key(1)), ("name".into(), text("ann"))]);
    assert_eq!(
        r#"User { id: users.key(1), name: "ann" }"#,
        print(&f, &ann, "User")
    );
    assert_eq!("users.key(3)", print(&f, &key(3), "Key users"));
    let member = Value::key(Value::record([("team", text("x")), ("user", key(1))]));
    assert_eq!(
        r#"members.key(team: "x", user: users.key(1))"#,
        print(&f, &member, "Key members")
    );
    assert_eq!(
        "profiles.key(users.key(2))",
        print(&f, &Value::key(key(2)), "Key profiles")
    );
    assert_eq!("[1, 2, 10]", print(&f, &ints([10, 2, 1]), "Set I64"));
    assert_eq!("[]", print(&f, &ints([]), "Set I64"));
    assert_eq!(
        "{ a: 1, b: \"x\" }",
        print(
            &f,
            &Value::record([("b", text("x")), ("a", int(1))]),
            "{ a: I64, b: Text }"
        )
    );
    assert_eq!("{}", print(&f, &Value::record([]), "{}"));
    assert_eq!("Email(\"a@b\")", print(&f, &text("a@b"), "Email"));
    assert_eq!("Sent", print(&f, &Value::Ctor(1, vec![]), "Tag Stage"));
    let span = Value::Ctor(0, vec![("start".into(), int(3)), ("end".into(), int(7))]);
    assert_eq!("span(3, 7)", print(&f, &span, "Span I64"));
    assert_eq!(span, reads(&f, " span( 3 ,7 ) ", "Span I64"));
    let social = Value::Ctor(
        1,
        vec![
            ("id".into(), Value::key(Value::Bytes(vec![0x01; 16]))),
            ("user".into(), key(4)),
            ("sub".into(), text("s")),
            ("at".into(), Value::Instant(0)),
        ],
    );
    assert_eq!(
        "Social { id: logins.key(uuid(\"01010101-0101-0101-0101-010101010101\")), user: users.key(4), sub: \"s\", at: 1970-01-01T00:00:00.000000Z }",
        print(&f, &social, "Login")
    );
}

#[test]
fn a_reader_accepts_what_the_printer_does_not_emit() {
    let f = fixture();
    let at = days_from_civil(2026, 10, 25) * DAY + 9 * 3_600_000_000 + 30 * 60_000_000;
    assert_eq!(
        Value::Instant(at),
        reads(&f, "2026-10-25T09:30:00Z", "Instant")
    );
    assert_eq!(
        Value::Instant(at + 500_000),
        reads(&f, "2026-10-25T09:30:00.5Z", "Instant")
    );
    assert_eq!(Value::Bytes(vec![0, 255]), reads(&f, "0x00FF", "Bytes"));
    assert_eq!(int(7), reads(&f, "007", "I64"));
    assert_eq!(int(-7), reads(&f, " - 7 ", "I64"));
    assert_eq!(Value::Decimal(150, 2), reads(&f, "1.5", "Decimal 2"));
    assert_eq!(Value::Decimal(300, 2), reads(&f, "3", "Decimal 2"));
    assert_eq!(Value::F64(3.0), reads(&f, "3", "F64"));
    assert_eq!(Value::F64(-2.5e-3), reads(&f, "-2.5E-3", "F64"));
    assert_eq!(
        Value::F64(0.0),
        reads(&f, "-0.0", "F64"),
        "-0.0 is never a value"
    );
    assert_eq!(ints([1, 2, 3]), reads(&f, "[3, 1, 2, 1]", "Set I64"));
    assert_eq!(Value::bool(true), reads(&f, "true", "Bool"));
    assert_eq!(
        Value::Ctor(0, vec![("id".into(), key(1)), ("name".into(), text("ann"))]),
        reads(&f, r#"User { name: "ann", id: users.key(1) }"#, "User")
    );
    assert_eq!(
        Value::key(Value::record([("team", text("x")), ("user", key(1))])),
        reads(
            &f,
            r#"members.key(user: users.key(1), team: "x")"#,
            "Key members"
        )
    );
    assert_eq!(key(3), reads(&f, "app::users.key(3)", "Key users"));
    let mut id = vec![0x01; 16];
    id[15] = 0x0a;
    assert_eq!(
        Value::Bytes(id),
        reads(
            &f,
            r#"uuid("01010101-0101-0101-0101-01010101010A")"#,
            "Uuid"
        )
    );
}

#[test]
fn text_reads_in_nfc() {
    let f = fixture();
    assert_eq!(text("\u{e9}"), reads(&f, "\"e\\u{301}\"", "Text"));
    assert_eq!(text("\u{e9}"), reads(&f, "\"e\u{301}\"", "Text"));
    assert_eq!(text("a\tb"), reads(&f, r#""a\tb""#, "Text"));
    refused(&f, r#""\q""#, "Text");
    refused(&f, r#""\u{d800}""#, "Text");
    refused(&f, "\"open", "Text");
    refused(&f, "\"a\nb\"", "Text");
}

#[test]
fn a_reader_refuses_what_is_not_a_value_of_the_type() {
    let f = fixture();
    for (t, ty) in [
        ("128", "I8"),
        ("-129", "I8"),
        ("9223372036854775808", "I64"),
        ("1.5", "I64"),
        ("1.234", "Decimal 2"),
        ("100000000000000000000000000000000000000", "Decimal 0"),
        ("1e39", "F32"),
        ("1e400", "F64"),
        ("nan", "F64"),
        ("12 13", "I64"),
        ("", "I64"),
        ("2026-02-29", "Date"),
        ("2026-13-01", "Date"),
        ("2026-1-01", "Date"),
        ("24:00:00", "Time"),
        ("09:30:00.0000001", "Time"),
        ("2026-10-25T09:30:00", "Instant"),
        ("2026-10-25T09:30:00Z", "DateTime"),
        ("2026-10-25T09:30:00[Nowhere/Special]", "ZonedDateTime"),
        ("P1M", "Period"),
        ("P2147483648M0D", "Period"),
        ("0x0", "Bytes"),
        ("0xzz", "Bytes"),
        ("30", "Duration"),
        ("9223372036854776s", "Duration"),
        ("Maybe", "Bool"),
        ("Sent", "Stage"),
        ("Draft { at: 1 }", "Stage"),
        ("Sent { at: 1, at: 2 }", "Stage"),
        ("Sent { at: 1, ref: \"a\" }", "Stage"),
        (r"User { id: users.key(1) }", "User"),
        ("orders.key(1)", "Key users"),
        ("members.key(user: users.key(1))", "Key members"),
        ("Email(1)", "Email"),
        (r#"Mail("a@b")"#, "Email"),
        (r#"uuid("0190a4b2")"#, "Uuid"),
        ("span(2, 1)", "Span I64"),
        ("Span { start: 1, end: 2 }", "Span I64"),
        ("[1, 2", "Set I64"),
        ("[1,, 2]", "Set I64"),
        ("Unknown", "Tag Stage"),
    ] {
        refused(&f, t, ty);
    }
}

#[test]
fn a_parse_error_says_where() {
    let f = fixture();
    let e = parse(&f, "[1, x]", "Set I64").unwrap_err();
    assert_eq!(4, e.offset);
}

#[test]
fn t_parse_reads_the_textual_syntax_in_queries() {
    let f = fixture();
    let at = days_from_civil(2026, 10, 25) * DAY + 9 * 3_600_000_000 + 30 * 60_000_000;
    assert_eq!(
        Ok(Value::some(Value::Instant(at))),
        f.eval(r#"Instant.parse("2026-10-25T09:30:00.000000Z")"#)
    );
    assert_eq!(
        Ok(Value::some(Value::Period { months: 1, days: 0 })),
        f.eval(r#"Period.parse("P1M0D")"#)
    );
    assert_eq!(
        Ok(Value::some(Value::Bytes(vec![0, 255]))),
        f.eval(r#"Bytes.parse("0x00ff")"#)
    );
    assert_eq!(
        Ok(Value::some(Value::F64(f64::INFINITY))),
        f.eval(r#"F64.parse("inf")"#)
    );
    assert_eq!(
        Ok(Value::some(Value::Ctor(1, vec![]))),
        f.eval(r#"(Tag Login).parse("Social")"#)
    );
    assert_eq!(Ok(Value::none()), f.eval(r#"(Tag Login).parse("Google")"#));
    assert_eq!(
        Ok(Value::some(ints([1, 2]))),
        f.eval(r#"(Set I64).parse("[2, 1]")"#)
    );
    assert_eq!(
        Ok(Value::some(key(3))),
        f.eval(r#"(Key users).parse("users.key(3)")"#)
    );
}

#[test]
fn print_in_queries_gives_the_textual_syntax() {
    let f = fixture();
    assert_eq!(Ok(text("0x68c3a9")), f.eval(r#"print(bytes("hé"))"#));
    assert_eq!(
        Ok(text("Some { value: 1.50 }")),
        f.eval("print(Some { value: 1.50 })")
    );
    assert_eq!(Ok(text("Cents(5)")), f.eval("print(Cents(5))"));
}

const HASH: &str = "c7f2000000000000000000000000000000000000000000000000000000000001";

#[test]
fn a_document_states_its_edition_and_version() {
    let f = fixture();
    let doc = format!(
        "text 2026-10 pins 42 ({HASH})\ndata users\nUser {{ id: users.key(4), name: \"bo\" }}\nUser {{ id: users.key(3), name: \"ann\" }}\n"
    );
    let d = textual::read_document(&f.schema, &doc).unwrap();
    assert_eq!(
        Some(Header {
            edition: "2026-10".into(),
            version: 42,
            hash: HASH.into(),
        }),
        d.header
    );
    assert_eq!(f.rel("users"), d.relation);
    assert_eq!(2, d.rows.len());
    let printed = textual::print_document(&f.schema, &d);
    assert_eq!(
        format!(
            "text 2026-10 pins 42 ({HASH})\ndata users\nUser {{ id: users.key(3), name: \"ann\" }}\nUser {{ id: users.key(4), name: \"bo\" }}\n"
        ),
        printed
    );
    assert_eq!(Ok(d), textual::read_document(&f.schema, &printed));
}

#[test]
fn a_document_without_a_header_is_at_the_readers_version() {
    let f = fixture();
    let d = textual::read_document(&f.schema, "data members\n").unwrap();
    assert_eq!(None, d.header);
    assert_eq!(f.rel("members"), d.relation);
    assert!(d.rows.is_empty());
    let d = Document {
        header: None,
        relation: f.rel("users"),
        rows: BTreeSet::from([Value::Ctor(
            0,
            vec![("id".into(), key(1)), ("name".into(), text("a"))],
        )]),
    };
    let printed = textual::print_document(&f.schema, &d);
    assert_eq!(
        "data users\nUser { id: users.key(1), name: \"a\" }\n",
        printed
    );
    assert_eq!(Ok(d), textual::read_document(&f.schema, &printed));
}

#[test]
fn a_malformed_document_is_refused() {
    let f = fixture();
    for doc in [
        "",
        "data nowhere\n",
        "text 2026-10 pins 42 (c7f2)\ndata users\n",
        "text 2026 pins 42 ({HASH})\ndata users\n",
        "text 2026-10 42 ({HASH})\ndata users\n",
        "data users\nUser { id: users.key(1) }\n",
        "data users\nMember { user: users.key(1), team: \"x\" }\n",
    ] {
        let doc = doc.replace("{HASH}", HASH);
        assert!(textual::read_document(&f.schema, &doc).is_err(), "{doc}");
    }
}

/// Returns an `i64` drawn mostly from small magnitudes and the edges of the range.
fn draw_int(rng: &mut Rng) -> i64 {
    match rng.below(4) {
        0 => *rng.pick(&[0, 1, -1, i64::MAX, i64::MIN]),
        1 => rng.below(1000) as i64 - 500,
        _ => rng.next_u64() as i64,
    }
}

const CHARS: &[&str] = &[
    "a",
    "Z",
    " ",
    "\"",
    "\\",
    "\n",
    "\r",
    "\t",
    "\u{0}",
    "\u{7f}",
    "\u{85}",
    "\u{e9}",
    "e\u{301}",
    "\u{1e0b}\u{323}",
    "\u{1f600}",
    "\u{ffff}",
    "\u{10ffff}",
    "\u{2028}",
    "{",
    "]",
    ",",
    ":",
    "\u{fb03}",
];

const ZONES: &[&str] = &["UTC", "Europe/Paris", "America/New_York", "Asia/Kathmandu"];

fn draw_base(rng: &mut Rng, b: Base) -> Value {
    let i = draw_int(rng);
    match b {
        Base::I8 => int(i64::from(i as i8)),
        Base::I16 => int(i64::from(i as i16)),
        Base::I32 => int(i64::from(i as i32)),
        Base::I64 => int(i),
        Base::Decimal(s) => {
            let bound = 10i128.pow(38);
            let wide = (i128::from(rng.next_u64()) << 64 | i128::from(rng.next_u64())) % bound;
            let d = match rng.below(3) {
                0 => wide,
                1 => *rng.pick(&[0, 1, -1, bound - 1, 1 - bound]),
                _ => i128::from(i),
            };
            Value::Decimal(d, s)
        }
        Base::F64 => {
            let f = match rng.below(3) {
                0 => *rng.pick(&[0.0, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.1]),
                1 => i as f64 / 1000.0,
                _ => f64::from_bits(rng.next_u64()),
            };
            Value::F64(if f.is_nan() {
                f64::NAN
            } else if f == 0.0 {
                0.0
            } else {
                f
            })
        }
        Base::F32 => {
            let f = match rng.below(3) {
                0 => *rng.pick(&[0.0, f32::NAN, f32::INFINITY, f32::MIN_POSITIVE, f32::MAX]),
                1 => i as f32 / 1000.0,
                _ => f32::from_bits(rng.next_u64() as u32),
            };
            Value::F32(if f.is_nan() {
                f32::NAN
            } else if f == 0.0 {
                0.0
            } else {
                f
            })
        }
        Base::Text => {
            let n = rng.below(6);
            let s: String = (0..n).map(|_| *rng.pick(CHARS)).collect();
            Value::Text(s.nfc().collect())
        }
        Base::Bytes | Base::SortKey => {
            let n = rng.below(6);
            let bs = (0..n).map(|_| rng.next_u64() as u8).collect();
            if b == Base::Bytes {
                Value::Bytes(bs)
            } else {
                Value::SortKey(bs)
            }
        }
        Base::Instant => Value::Instant(i),
        Base::DateTime => Value::DateTime(i),
        Base::Date => Value::Date(i.div_euclid(DAY)),
        Base::Time => Value::Time(i.rem_euclid(DAY)),
        Base::ZonedDateTime => Value::Zoned(i, rng.pick(ZONES).to_string()),
        Base::Duration => Value::Duration(i),
        Base::Period => Value::Period {
            months: draw_int(rng) as i32,
            days: i as i32,
        },
    }
}

fn draw_fields(
    rng: &mut Rng,
    schema: &Schema,
    tys: &[(String, Ty)],
    depth: u32,
) -> Vec<(String, Value)> {
    tys.iter()
        .map(|(n, t)| (n.clone(), draw(rng, schema, t, depth + 1)))
        .collect()
}

fn draw_key_field(
    rng: &mut Rng,
    schema: &Schema,
    rel: adtb_lang::types::RelId,
    f: &str,
    depth: u32,
) -> Value {
    let r = schema.relation(rel);
    if textual::self_key(schema, rel, f) {
        return match r.generate {
            Some(Generator::Uuidv7) => {
                Value::Bytes((0..16).map(|_| rng.next_u64() as u8).collect())
            }
            _ => int(draw_int(rng)),
        };
    }
    let ty = schema
        .ty(r.elem_def)
        .field(0, f)
        .expect("a key field")
        .ty
        .subst(r.elem_args());
    draw(rng, schema, &ty, depth + 1)
}

/// Returns a value of `ty` drawn from `rng`.
fn draw(rng: &mut Rng, schema: &Schema, ty: &Ty, depth: u32) -> Value {
    match ty {
        Ty::Base(b) => draw_base(rng, *b),
        Ty::Data { def, args, ctors } => {
            let d = schema.ty(*def);
            match &d.kind {
                TypeDefKind::Newtype { .. } if d.name == "Uuid" => {
                    Value::Bytes((0..16).map(|_| rng.next_u64() as u8).collect())
                }
                TypeDefKind::Newtype { base, .. } => draw(rng, schema, base, depth),
                TypeDefKind::Sum {
                    header, ctors: all, ..
                } => {
                    let allowed: Vec<u32> = match ctors {
                        Some(s) => s.clone(),
                        None => (0..all.len() as u32).collect(),
                    };
                    let c = if depth > 3 {
                        allowed[0]
                    } else {
                        *rng.pick(&allowed)
                    };
                    let tys: Vec<(String, Ty)> = header
                        .iter()
                        .chain(&all[c as usize].fields)
                        .map(|f| (f.name.clone(), f.ty.subst(args)))
                        .collect();
                    let mut fs = draw_fields(rng, schema, &tys, depth);
                    if d.name == "Span" && fs[0].1 > fs[1].1 {
                        let (a, b) = (fs[0].1.clone(), fs[1].1.clone());
                        fs[0].1 = b;
                        fs[1].1 = a;
                    }
                    Value::Ctor(c, fs)
                }
            }
        }
        Ty::Sum { ctors, .. } => {
            let c = rng.below(ctors.len() as u64) as usize;
            Value::Ctor(c as u32, draw_fields(rng, schema, &ctors[c].1, depth))
        }
        Ty::Tag(inner) => {
            let n = match inner.as_ref() {
                Ty::Data { def, .. } => schema.ty(*def).ctors().len(),
                Ty::Sum { ctors, .. } => ctors.len(),
                t => panic!("a tag of {t:?}"),
            };
            Value::Ctor(rng.below(n as u64) as u32, Vec::new())
        }
        Ty::Record(tys) => Value::Record(draw_fields(rng, schema, tys, depth)),
        Ty::Unit => Value::unit(),
        Ty::Key { rel, .. } => {
            let r = schema.relation(*rel);
            match r.key.as_slice() {
                [f] => Value::key(draw_key_field(rng, schema, *rel, f, depth)),
                fs => {
                    let mut names: Vec<&String> = fs.iter().collect();
                    names.sort();
                    Value::key(Value::Record(
                        names
                            .into_iter()
                            .map(|f| (f.clone(), draw_key_field(rng, schema, *rel, f, depth)))
                            .collect(),
                    ))
                }
            }
        }
        Ty::Set(t) | Ty::NonEmpty(t) => {
            let n = if depth > 3 { 0 } else { rng.below(4) };
            Value::Set((0..n).map(|_| draw(rng, schema, t, depth + 1)).collect())
        }
        t => panic!("no values are drawn at {t:?}"),
    }
}

const TYPES: &[&str] = &[
    "I8",
    "I16",
    "I32",
    "I64",
    "Decimal 0",
    "Decimal 2",
    "Decimal 38",
    "F32",
    "F64",
    "Text",
    "Bytes",
    "SortKey",
    "Instant",
    "Date",
    "Time",
    "DateTime",
    "ZonedDateTime",
    "Duration",
    "Period",
    "Bool",
    "Option I64",
    "Option (Option Text)",
    "Result Text I64",
    "Stage",
    "Tag Stage",
    "Tag Login",
    "User",
    "Member",
    "Login",
    "Profile",
    "Pair (Set (Decimal 1))",
    "Email",
    "Cents",
    "Uuid",
    "Span Instant",
    "Span I64",
    "Key users",
    "Key members",
    "Key logins",
    "Key profiles",
    "Set I64",
    "Set (Set Text)",
    "Set User",
    "Set ZonedDateTime",
    "{ a: I64, b: Text }",
    "{}",
    "Okay | Bad { why: Text, at: Date }",
    "Principal",
    "Fault",
    "Rounding",
    "()",
    "Login.Social",
    "Set (Key logins)",
];

#[test]
fn print_then_parse_gives_the_value_back_for_every_type() {
    let f = fixture();
    let mut rng = Rng::new(0x00ad_7b00);
    for name in TYPES {
        let ty = f.ty(name);
        for _ in 0..300 {
            let v = draw(&mut rng, &f.schema, &ty, 0);
            let printed = textual::print(&f.schema, &v, &ty);
            let back = textual::parse(&f.schema, &printed, &ty)
                .unwrap_or_else(|e| panic!("`{printed}` does not read back at {name}: {e}"));
            assert_eq!(v, back, "`{printed}` at {name}");
            assert_eq!(
                printed,
                textual::print(&f.schema, &back, &ty),
                "equal values print identically"
            );
        }
    }
}

#[test]
fn equal_values_print_identically() {
    let f = fixture();
    let a = Value::set([int(2), int(1)]);
    let b = reads(&f, "[1, 2, 2]", "Set I64");
    assert_eq!(a, b);
    assert_eq!(print(&f, &a, "Set I64"), print(&f, &b, "Set I64"));
    let c = reads(&f, "\"\u{e9}\"", "Text");
    let d = reads(&f, "\"e\\u{301}\"", "Text");
    assert_eq!(print(&f, &c, "Text"), print(&f, &d, "Text"));
}
