//! The textual syntax over a value type of its own: every form prints in its literal syntax
//! and reads back as printed.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use adtb_lang::check::{Checker, Context, check_schema};
use adtb_lang::parse::{Options, parse_schema, parse_type};
use adtb_lang::resolve::Visible;
use adtb_lang::schema::{Schema, ValueDef, build};
use adtb_lang::span::FileId;
use adtb_lang::textual::*;
use adtb_lang::types::{Base, RelId, Ty};
use adtb_testkit::Rng;
use adtb_testkit::draw::{ZONES, draw};

const SCHEMA: &str = r"
module app
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
type Money = { amount: Decimal 2, currency: Text }
type Order = { id: Key orders, total: Money, stage: Stage, note: Option Text }
relation orders : Order key id generate sequence
";

/// A float ordered by the IEEE total order, as the model orders floats.
#[derive(Clone, Copy, Debug)]
struct Total<T>(T);

macro_rules! total {
    ($t:ty) => {
        impl Ord for Total<$t> {
            fn cmp(&self, o: &Self) -> Ordering {
                self.0.total_cmp(&o.0)
            }
        }
        impl PartialOrd for Total<$t> {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl PartialEq for Total<$t> {
            fn eq(&self, o: &Self) -> bool {
                self.cmp(o) == Ordering::Equal
            }
        }
        impl Eq for Total<$t> {}
    };
}

total!(f32);
total!(f64);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Atom {
    Int(i64),
    Decimal(i128),
    F32(Total<f32>),
    F64(Total<f64>),
    Text(String),
    Bytes(Vec<u8>),
    Instant(i64),
    Date(i64),
    Time(i64),
    DateTime(i64),
    Zoned(i64, String),
    Duration(i64),
    Period(i32, i32),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Tree {
    Atom(Atom),
    Ctor(u32, Vec<Tree>),
    Record(Vec<Tree>),
    Key(Box<Tree>),
    Set(BTreeSet<Tree>),
}

impl Textual for Tree {
    fn scalar(&self) -> Option<Scalar<'_>> {
        let Tree::Atom(a) = self else {
            return None;
        };
        Some(match a {
            Atom::Int(x) => Scalar::Int(*x),
            Atom::Decimal(d) => Scalar::Decimal(*d),
            Atom::F32(f) => Scalar::F32(f.0),
            Atom::F64(f) => Scalar::F64(f.0),
            Atom::Text(t) => Scalar::Text(t),
            Atom::Bytes(b) => Scalar::Bytes(b),
            Atom::Instant(t) => Scalar::Instant(*t),
            Atom::Date(d) => Scalar::Date(*d),
            Atom::Time(t) => Scalar::Time(*t),
            Atom::DateTime(t) => Scalar::DateTime(*t),
            Atom::Zoned(local, zone) => Scalar::Zoned {
                local: *local,
                zone,
            },
            Atom::Duration(d) => Scalar::Duration(*d),
            Atom::Period(months, days) => Scalar::Period {
                months: *months,
                days: *days,
            },
        })
    }

    fn ctor(&self) -> Option<u32> {
        match self {
            Tree::Ctor(c, _) => Some(*c),
            _ => None,
        }
    }

    fn fields(&self) -> Option<impl Iterator<Item = &Tree>> {
        match self {
            Tree::Ctor(_, fs) | Tree::Record(fs) => Some(fs.iter()),
            _ => None,
        }
    }

    fn key(&self) -> Option<&Tree> {
        match self {
            Tree::Key(k) => Some(k),
            _ => None,
        }
    }

    fn elems(&self) -> Option<impl Iterator<Item = &Tree>> {
        match self {
            Tree::Set(s) => Some(s.iter()),
            _ => None,
        }
    }

    fn of_scalar(s: Scalar<'_>, _: Base) -> Tree {
        Tree::Atom(match s {
            Scalar::Int(x) => Atom::Int(x),
            Scalar::Decimal(d) => Atom::Decimal(d),
            Scalar::F32(f) => Atom::F32(Total(f)),
            Scalar::F64(f) => Atom::F64(Total(f)),
            Scalar::Text(t) => Atom::Text(t.to_string()),
            Scalar::Bytes(b) => Atom::Bytes(b.to_vec()),
            Scalar::Instant(t) => Atom::Instant(t),
            Scalar::Date(d) => Atom::Date(d),
            Scalar::Time(t) => Atom::Time(t),
            Scalar::DateTime(t) => Atom::DateTime(t),
            Scalar::Zoned { local, zone } => Atom::Zoned(local, zone.to_string()),
            Scalar::Duration(d) => Atom::Duration(d),
            Scalar::Period { months, days } => Atom::Period(months, days),
        })
    }

    fn of_ctor(ctor: u32, fields: Vec<(&str, Tree)>) -> Tree {
        Tree::Ctor(ctor, fields.into_iter().map(|(_, v)| v).collect())
    }

    fn of_record(fields: Vec<(&str, Tree)>) -> Tree {
        Tree::Record(fields.into_iter().map(|(_, v)| v).collect())
    }

    fn of_key(value: Tree) -> Tree {
        Tree::Key(Box::new(value))
    }

    fn of_set(elems: Vec<Tree>) -> Tree {
        Tree::Set(elems.into_iter().collect())
    }

    fn zone_exists(name: &str) -> bool {
        ZONES.contains(&name)
    }
}

fn schema() -> Schema {
    let file = parse_schema(SCHEMA, Options::default())
        .ok()
        .unwrap_or_else(|ds| panic!("the schema does not parse: {ds:?}"));
    let schema = build(&[(FileId(0), file)])
        .unwrap_or_else(|ds| panic!("the schema does not build: {ds:?}"));
    check_schema(&schema).unwrap_or_else(|ds| panic!("the schema does not check: {ds:?}"));
    schema
}

fn ty(schema: &Schema, text: &str) -> Ty {
    let parsed = parse_type(text, Options::default())
        .ok()
        .unwrap_or_else(|_| panic!("`{text}` is not a type"));
    Checker::new(schema, Visible::all(schema), Context::Adhoc)
        .resolve_type(&parsed)
        .unwrap_or_else(|d| panic!("`{text}`: {}", d.message))
}

fn form(schema: &Schema, text: &str) -> Form {
    Form::of(schema, &ty(schema, text))
}

fn rel(schema: &Schema, name: &str) -> RelId {
    let m = schema.module_named("app").expect("the module");
    match schema.module(m).values.get(name) {
        Some(&ValueDef::Relation(r)) => r,
        _ => panic!("no relation `{name}`"),
    }
}

const DAY: i64 = 86_400_000_000;

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
    "Order",
    "Pair (Set (Decimal 1))",
    "Pair Money",
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
    "{ inner: { x: Bytes, y: Option Money } }",
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
    let s = schema();
    let mut rng = Rng::new(0x00ad_7b00);
    for name in TYPES {
        let f = form(&s, name);
        for i in 0..300 {
            let v: Tree = draw(&mut rng, &f);
            let printed = print(&v, &f).unwrap_or_else(|e| panic!("draw {i} at {name}: {e}"));
            let back: Tree = parse(&printed, &f).unwrap_or_else(|e| {
                panic!("draw {i}: `{printed}` does not read back at {name}: {e}")
            });
            assert_eq!(v, back, "draw {i}: `{printed}` at {name}");
            assert_eq!(
                Ok(printed),
                print(&back, &f),
                "draw {i}: equal values print identically"
            );
        }
    }
}

#[test]
fn every_value_form_prints_in_its_literal_syntax() {
    let s = schema();
    let int = |x| Tree::Atom(Atom::Int(x));
    let text = |t: &str| Tree::Atom(Atom::Text(t.to_string()));
    let cases = [
        ("I64", int(-12), "-12"),
        ("Decimal 2", Tree::Atom(Atom::Decimal(-5)), "-0.05"),
        ("Text", text("a\"b\n\u{7}"), r#""a\"b\n\u{7}""#),
        ("Bytes", Tree::Atom(Atom::Bytes(vec![0, 255])), "0x00ff"),
        (
            "Duration",
            Tree::Atom(Atom::Duration(-5_400_000_000)),
            "-90min",
        ),
        ("Period", Tree::Atom(Atom::Period(1, -15)), "P1M-15D"),
        ("Date", Tree::Atom(Atom::Date(-800_000)), "-00221-09-04"),
        ("Bool", Tree::Ctor(1, Vec::new()), "True"),
        (
            "Option I64",
            Tree::Ctor(1, vec![int(3)]),
            "Some { value: 3 }",
        ),
        ("Email", text("a@b"), "Email(\"a@b\")"),
        ("Tag Stage", Tree::Ctor(2, Vec::new()), "Paid"),
        ("()", Tree::Record(Vec::new()), "()"),
        ("{}", Tree::Record(Vec::new()), "{}"),
        (
            "Span I64",
            Tree::Ctor(0, vec![int(3), int(7)]),
            "span(3, 7)",
        ),
        ("Key users", Tree::Key(Box::new(int(3))), "users.key(3)"),
        (
            "Key members",
            Tree::Key(Box::new(Tree::Record(vec![
                text("red"),
                Tree::Key(Box::new(int(1))),
            ]))),
            r#"members.key(team: "red", user: users.key(1))"#,
        ),
        (
            "Key logins",
            Tree::Key(Box::new(Tree::Atom(Atom::Bytes(vec![0xab; 16])))),
            r#"logins.key(uuid("abababab-abab-abab-abab-abababababab"))"#,
        ),
        (
            "Login",
            Tree::Ctor(
                0,
                vec![
                    Tree::Key(Box::new(Tree::Atom(Atom::Bytes(vec![0; 16])))),
                    Tree::Key(Box::new(int(2))),
                    Tree::Atom(Atom::Bytes(vec![1])),
                ],
            ),
            r#"Password { id: logins.key(uuid("00000000-0000-0000-0000-000000000000")), user: users.key(2), hash: 0x01 }"#,
        ),
        (
            "Set I64",
            Tree::Set([int(2), int(-1)].into_iter().collect()),
            "[-1, 2]",
        ),
    ];
    for (name, v, want) in cases {
        assert_eq!(
            Ok(want.to_string()),
            print(&v, &form(&s, name)),
            "at {name}"
        );
    }
}

#[test]
fn reading_accepts_any_field_order_and_merges_set_elements() {
    let s = schema();
    let f = form(&s, "Set Member");
    let v: Tree = parse(
        r#"[Member { team: "a", user: users.key(1) }, Member { user: users.key(1), team: "a" }]"#,
        &f,
    )
    .expect("the set reads");
    assert_eq!(Some(1), v.elems().map(Iterator::count));
    assert_eq!(
        Ok(r#"[Member { user: users.key(1), team: "a" }]"#.to_string()),
        print(&v, &f)
    );
}

#[test]
fn reading_refuses_text_outside_the_form() {
    let s = schema();
    let password = r#"Password { id: logins.key(uuid("00000000-0000-0000-0000-000000000000")), user: users.key(1), hash: 0x }"#;
    for (name, t) in [
        ("I8", "128"),
        ("Decimal 2", "1.234"),
        ("Login.Social", password),
        ("Span I64", "span(7, 3)"),
        ("ZonedDateTime", "2026-10-25T09:30:00[Mars/Olympus]"),
        ("Date", "2026-02-30"),
        ("Uuid", r#"uuid("0000")"#),
        ("Key users", "members.key(1)"),
        ("User", "User { id: users.key(1) }"),
        ("Set I64", "[1, 2"),
        ("I64", "1 2"),
    ] {
        let got: Result<Tree, ParseError> = parse(t, &form(&s, name));
        assert!(got.is_err(), "`{t}` reads at {name} as {got:?}");
    }
}

#[test]
fn a_type_with_no_syntax_has_an_opaque_form() {
    let s = schema();
    let f = form(&s, "{ f: I64 -> I64 }");
    assert!(!f.has_syntax());
    assert!(form(&s, "Set (Option Order)").has_syntax());
    let got: Result<Tree, ParseError> = parse("{ f: 1 }", &f);
    assert_eq!(
        Err(ParseError {
            offset: 5,
            message: "this type has no textual syntax".into(),
        }),
        got
    );
}

#[test]
fn to_text_gives_the_bare_form_of_scalars_uuids_and_text_newtypes() {
    let s = schema();
    let uuid = Tree::Atom(Atom::Bytes(vec![0xab; 16]));
    assert_eq!(
        Ok("abababab-abab-abab-abab-abababababab".to_string()),
        to_text(&uuid, &form(&s, "Uuid"))
    );
    let email = Tree::Atom(Atom::Text("a@b".into()));
    assert_eq!(Ok("a@b".to_string()), to_text(&email, &form(&s, "Email")));
    assert_eq!(
        Ok("12".to_string()),
        to_text(&Tree::Atom(Atom::Int(12)), &form(&s, "Cents"))
    );
    assert_eq!(
        Ok("1970-01-02T00:00:00.000000Z".to_string()),
        to_text(&Tree::Atom(Atom::Instant(DAY)), &form(&s, "Instant"))
    );
}

#[test]
fn parse_uuid_reads_either_case_in_the_grouped_form() {
    assert_eq!(
        Some(vec![0xab; 16]),
        parse_uuid("ABABABAB-abab-ABAB-abab-ABABABABABAB")
    );
    for t in [
        "",
        "abababab-abab-abab-abab-ababababababab",
        "abababababababababababababababababab",
        "abababab-abab-abab-abab-abababababag",
    ] {
        assert_eq!(None, parse_uuid(t), "`{t}`");
    }
}

#[test]
fn a_document_reads_back_as_printed() {
    let s = schema();
    let hash = "ab".repeat(32);
    let text = format!(
        "text 2026-10 pins 3 ({hash})\ndata app::members\n{}\n{}\n",
        r#"Member { user: users.key(2), team: "b" }"#,
        r#"Member { team: "a", user: users.key(1) }"#,
    );
    let doc: Document<Tree> = read_document(&s, &text).expect("the document reads");
    assert_eq!(rel(&s, "members"), doc.relation);
    assert_eq!(2, doc.rows.len());
    assert_eq!(
        Some(Header {
            edition: "2026-10".into(),
            version: 3,
            hash: hash.clone(),
        }),
        doc.header
    );
    let printed = print_document(&s, &doc).expect("the document prints");
    assert_eq!(
        format!(
            "text 2026-10 pins 3 ({hash})\ndata members\n{}\n{}\n",
            r#"Member { user: users.key(1), team: "a" }"#,
            r#"Member { user: users.key(2), team: "b" }"#,
        ),
        printed
    );
    assert_eq!(Ok(doc), read_document(&s, &printed));
}

#[test]
fn a_value_not_of_its_form_does_not_print() {
    let s = schema();
    let int = || Tree::Atom(Atom::Int(1));
    let text = || Tree::Atom(Atom::Text("a".into()));
    let cases = [
        ("Text", int()),
        ("F64", int()),
        ("Decimal 2", int()),
        ("Bytes", text()),
        ("Uuid", Tree::Atom(Atom::Bytes(vec![0; 3]))),
        ("Uuid", int()),
        ("Email", int()),
        ("Bool", int()),
        ("Bool", Tree::Ctor(2, Vec::new())),
        ("Tag Stage", Tree::Ctor(3, Vec::new())),
        ("Option I64", Tree::Ctor(1, Vec::new())),
        ("User", Tree::Record(vec![int()])),
        ("{ a: I64, b: Text }", int()),
        ("Span I64", Tree::Ctor(0, vec![int()])),
        ("Key users", int()),
        ("Key members", Tree::Key(Box::new(int()))),
        ("Set I64", int()),
        ("Set I64", Tree::Set([text()].into_iter().collect())),
        ("{ f: I64 -> I64 }", Tree::Record(Vec::new())),
    ];
    for (name, v) in cases {
        let f = form(&s, name);
        assert!(print(&v, &f).is_err(), "{v:?} prints at {name}");
        assert!(to_text(&v, &f).is_err(), "{v:?} is text at {name}");
    }
}
