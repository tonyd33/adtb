//! The textual syntax over the engine's values: they read back as printed, and print exactly as
//! the reference prints the same values, whether built directly or read back from the engine.

mod plan {
    pub use adtb::plan::*;
}

#[path = "../../adtb/src/plan/tests/fixture.rs"]
#[expect(dead_code, reason = "this binary uses part of the planner fixture")]
mod fixture;

#[expect(
    dead_code,
    unused_imports,
    reason = "this binary uses part of the shared support"
)]
mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use adtb::exec::{self, Budget, Call, Options as XOptions, Val};
use adtb::plan::{Options as PlanOptions, Rewrites, plan};
use adtb_lang::textual::{self, Form};
use adtb_lang::types::Ty;
use adtb_reference::Value;
use adtb_sim::{Config, Outcome, Sim};
use adtb_testkit::Rng;
use adtb_testkit::draw::draw;

use fixture::Fixture;
use support::*;

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

const DAY: i64 = 86_400_000_000;

#[test]
fn engine_values_read_back_as_printed_for_every_type() {
    let f = Fixture::new(SCHEMA);
    let mut rng = Rng::new(0x7e47_0001);
    for name in TYPES {
        let form = Form::of(&f.schema, &f.ty(name));
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
        let form = Form::of(&f.schema, &ty);
        for i in 0..300 {
            let mut again = rng.clone();
            let engine: Val = draw(&mut rng, &form);
            let reference: Value = draw(&mut again, &form);
            let printed = adtb_reference::textual::print(&f.schema, &reference, &ty);
            assert_eq!(
                printed,
                textual::print(&engine, &form),
                "draw {i} at {name}"
            );
        }
    }
}

/// Returns the value the engine computes for ad hoc `text`, with its type.
fn engine(f: &Fixture, text: &str) -> (Ty, Val) {
    let (n, t) = f.normal(text, &[]);
    let p = plan(
        &f.schema,
        &f.catalog,
        &n.terms,
        t,
        &[],
        PlanOptions {
            rewrites: Rewrites::ALL,
            hiding: false,
        },
    )
    .unwrap_or_else(|e| panic!("`{text}` has no plan: {e}"));
    let program = Arc::new(
        exec::compile(&f.schema, &f.catalog, &p, None)
            .unwrap_or_else(|e| panic!("`{text}` does not compile: {e:?}")),
    );
    let world = World {
        schema: &f.schema,
        catalog: &f.catalog,
        db: &f.db,
    };
    let mut out = None;
    let mut sim = Sim::new(1, Config::default());
    let result = sim.run(|rt| {
        let store = Db::load(&rt, &world).store(&rt);
        let budget = Budget::new(1 << 30);
        let call = Call {
            store,
            grant: budget.admit(64 << 20).expect("a grant fits"),
            pool: None,
            options: XOptions::default(),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        out = Some(exec::execute(rt, program.clone(), call, &[]));
    });
    assert_eq!(Outcome::Finished, result);
    let v = out
        .expect("the run finished")
        .unwrap_or_else(|e| panic!("`{text}` fails in the engine: {e:?}"));
    (n.terms.ty(t).clone(), v)
}

fn stored() -> Fixture {
    let mut f = Fixture::new(SCHEMA);
    let at = Value::Instant(20_000 * DAY + 3_723_000_456);
    let since = Value::Date(-800_000);
    let zone = Value::Zoned(-5 * DAY + 1, "Europe/Paris".into());
    let every = Value::Period {
        months: 13,
        days: -2,
    };
    let day = Value::Time(DAY - 1);
    f.rows(
        "docs",
        r#"[Note { id: docs.key(1), body: "a \"q\"\n\u{7}é", blob: bytes("h1"),
                   price: { amount: 12.50, currency: "EUR" }, status: Active,
                   at: at, ratio: 0.25, note: None },
            Note { id: docs.key(2), body: "", blob: bytes(""),
                   price: { amount: -0.05, currency: "" }, status: Paused { since: since, why: "x" },
                   at: at, ratio: -1e300, note: Some { value: "n" } },
            Link { id: docs.key(3), target: docs.key(1), wait: 90s, zone: zone,
                   every: every, day: day }]"#,
        &[
            ("at", "Instant", at),
            ("since", "Date", since),
            ("zone", "ZonedDateTime", zone),
            ("every", "Period", every),
            ("day", "Time", day),
        ],
    );
    f.rows(
        "taggings",
        r#"[Tagging { doc: docs.key(1), tag: "b", weight: -3 },
            Tagging { doc: docs.key(1), tag: "a", weight: 32767 },
            Tagging { doc: docs.key(3), tag: "a", weight: 0 }]"#,
        &[],
    );
    let session = Value::DateTime(-DAY * 400_000);
    f.rows(
        "tokens",
        r#"[Api { id: tokens.key(uuid("0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b")), secret: bytes("s") },
            Session { id: tokens.key(uuid("0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5c")), at: session, small: 0.1 }]"#,
        &[("session", "DateTime", session)],
    );
    f
}

#[test]
fn rows_read_back_from_the_engine_print_as_the_reference_prints_them() {
    let f = stored();
    for text in [
        "docs",
        "taggings",
        "tokens",
        "from t in taggings select t.doc",
        "from t in taggings select { doc: t.doc, w: { tag: t.tag, weight: t.weight } }",
        "from d in docs select d.id",
    ] {
        let (ty, v) = engine(&f, text);
        let want = f
            .reference(text, &[])
            .unwrap_or_else(|e| panic!("`{text}` fails in the reference: {e:?}"));
        let form = Form::of(&f.schema, &ty);
        let printed = textual::print(&v, &form).unwrap_or_else(|e| panic!("`{text}`: {e}"));
        assert_eq!(
            adtb_reference::textual::print(&f.schema, &want, &ty),
            Ok(printed.clone()),
            "`{text}`"
        );
        let back: Val = textual::parse(&printed, &form)
            .unwrap_or_else(|e| panic!("`{printed}` does not read back: {e}"));
        assert_eq!(v, back, "`{text}`");
    }
}
