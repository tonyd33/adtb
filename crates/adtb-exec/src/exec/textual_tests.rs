//! The textual syntax over the engine's values: they read back as printed, and print exactly as
//! the reference prints the same values when built directly.

use crate::plan::fixture;

use crate::exec::Val;
use adtb_lang::textual::{self, Form};
use adtb_reference::Value;
use adtb_testkit::Rng;
use adtb_testkit::draw::draw;

use fixture::Fixture;

const SCHEMA: &str = r"
type Money = { amount: Decimal 2, currency: Text }
type Status = Active | Paused { since: Date, why: Text }
type Doc { id: Key docs }
  = Note { body: Text, blob: Bytes, price: Money, status: Status, at: Instant, ratio: F64, note: Option Text }
  | Link { target: Key docs, wait: Duration, zone: ZonedDateTime, every: Period, day: Time }
relation docs : Doc key id generate sequence
type Tagging = { doc: Key docs, tag: Text, weight: I16 }
relation taggings : Tagging key (doc, tag)
type Token { id: Key tokens } = Api { secret: Bytes } | Session { at: DateTime, small: F32 }
relation tokens : Token key id generate uuidv7
type Email = new Text
type Pair a = { left: a, right: a }
";

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
    "Money",
    "Status",
    "Tag Status",
    "Doc",
    "Doc.Link",
    "Tagging",
    "Token",
    "Email",
    "Uuid",
    "Span Instant",
    "Span (Decimal 2)",
    "Pair Money",
    "Pair (Set Text)",
    "Key docs",
    "Key taggings",
    "Key tokens",
    "Set I64",
    "Set (Set Text)",
    "Set Doc",
    "Set (Key taggings)",
    "{ a: I64, b: { c: Bytes, d: Option Money } }",
    "{}",
    "()",
    "Okay | Bad { why: Text, at: Date }",
];

#[test]
fn engine_values_read_back_as_printed_for_every_type() {
    let f = Fixture::new(SCHEMA);
    let mut rng = Rng::new(0x7e47_0001);
    for name in TYPES {
        let form = Form::of(f.schema(), &f.ty(name));
        for i in 0..300 {
            let v: Val = draw(&mut rng, &form);
            let printed =
                textual::print(&v, &form).unwrap_or_else(|e| panic!("draw {i} at {name}: {e}"));
            let back: Val = textual::parse(&printed, &form).unwrap_or_else(|e| {
                panic!("draw {i}: `{printed}` does not read back at {name}: {e}")
            });
            assert_eq!(v, back, "draw {i}: `{printed}` at {name}");
            assert_eq!(
                Ok(printed),
                textual::print(&back, &form),
                "draw {i} at {name}: equal values print identically"
            );
        }
    }
}

#[test]
fn a_value_built_both_ways_prints_as_the_reference_prints_it() {
    let f = Fixture::new(SCHEMA);
    let mut rng = Rng::new(0x7e47_0002);
    for name in TYPES {
        let ty = f.ty(name);
        let form = Form::of(f.schema(), &ty);
        for i in 0..300 {
            let mut again = rng.clone();
            let engine: Val = draw(&mut rng, &form);
            let reference: Value = draw(&mut again, &form);
            let printed = adtb_reference::textual::print(f.schema(), &reference, &ty);
            assert_eq!(
                printed,
                textual::print(&engine, &form),
                "draw {i} at {name}"
            );
        }
    }
}
