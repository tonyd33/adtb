//! The textual syntax over the engine's values: they read back as printed, and print exactly as
//! the reference prints the same values, whether built directly or read back from the engine.

mod plan {
    pub use adtb::plan::*;
}

#[path = "../../adtb/src/plan/tests/fixture.rs"]
#[expect(dead_code, reason = "this binary uses part of the planner fixture")]
mod fixture;

mod support;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use adtb::exec::{self, Budget, Call, Options as XOptions, Val};
use adtb::plan::{Options as PlanOptions, Rewrites, plan};
use adtb_lang::textual::{self, Form, KeyForm, Scalar, Textual};
use adtb_lang::types::{Base, Ty};
use adtb_reference::Value;
use adtb_sim::{Config, Outcome, Sim};

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

const ZONES: &[&str] = &["UTC", "Europe/Paris", "Asia/Kathmandu"];

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
    "\u{1f600}",
    "\u{10ffff}",
    "{",
    "]",
    ",",
    ":",
];

const DAY: i64 = 86_400_000_000;

/// A SplitMix64 generator, so every run draws the same values.
#[derive(Clone)]
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }

    /// Returns an `i64` drawn mostly from small magnitudes and the edges of the range.
    fn int(&mut self) -> i64 {
        match self.below(4) {
            0 => *self.pick(&[0, 1, -1, i64::MAX, i64::MIN]),
            1 => self.below(1000) as i64 - 500,
            _ => self.next() as i64,
        }
    }
}

fn canon64(f: f64) -> f64 {
    if f.is_nan() {
        f64::NAN
    } else if f == 0.0 {
        0.0
    } else {
        f
    }
}

fn canon32(f: f32) -> f32 {
    if f.is_nan() {
        f32::NAN
    } else if f == 0.0 {
        0.0
    } else {
        f
    }
}

fn draw_base<V: Textual>(rng: &mut Rng, b: Base) -> V {
    let i = rng.int();
    let text: String;
    let bytes: Vec<u8>;
    let s = match b {
        Base::I8 => Scalar::Int(i64::from(i as i8)),
        Base::I16 => Scalar::Int(i64::from(i as i16)),
        Base::I32 => Scalar::Int(i64::from(i as i32)),
        Base::I64 => Scalar::Int(i),
        Base::Decimal(_) => {
            let bound = 10i128.pow(38);
            let wide = (i128::from(rng.next()) << 64 | i128::from(rng.next())) % bound;
            Scalar::Decimal(match rng.below(3) {
                0 => wide,
                1 => *rng.pick(&[0, 1, -1, bound - 1, 1 - bound]),
                _ => i128::from(i),
            })
        }
        Base::F64 => Scalar::F64(canon64(match rng.below(3) {
            0 => *rng.pick(&[0.0, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.1]),
            1 => i as f64 / 1000.0,
            _ => f64::from_bits(rng.next()),
        })),
        Base::F32 => Scalar::F32(canon32(match rng.below(3) {
            0 => *rng.pick(&[
                0.0,
                f32::NAN,
                f32::NEG_INFINITY,
                f32::MIN_POSITIVE,
                f32::MAX,
            ]),
            1 => i as f32 / 1000.0,
            _ => f32::from_bits(rng.next() as u32),
        })),
        Base::Text => {
            let n = rng.below(6);
            text = (0..n).map(|_| *rng.pick(CHARS)).collect();
            Scalar::Text(&text)
        }
        Base::Bytes | Base::SortKey => {
            let n = rng.below(6);
            bytes = (0..n).map(|_| rng.next() as u8).collect();
            Scalar::Bytes(&bytes)
        }
        Base::Instant => Scalar::Instant(i),
        Base::DateTime => Scalar::DateTime(i),
        Base::Date => Scalar::Date(i.div_euclid(DAY)),
        Base::Time => Scalar::Time(i.rem_euclid(DAY)),
        Base::ZonedDateTime => Scalar::Zoned {
            local: i,
            zone: rng.pick(ZONES),
        },
        Base::Duration => Scalar::Duration(i),
        Base::Period => Scalar::Period {
            months: rng.int() as i32,
            days: i as i32,
        },
    };
    V::of_scalar(s, b)
}

fn draw_fields<'f, V: Textual>(
    rng: &mut Rng,
    fs: &'f [(String, Form)],
    depth: u32,
) -> Vec<(&'f str, V)> {
    fs.iter()
        .map(|(n, f)| (n.as_str(), draw(rng, f, depth + 1)))
        .collect()
}

/// Returns a value of form `form` drawn from `rng`, built as a `V`; the same draws build equal
/// values of any two value types.
fn draw<V: Textual>(rng: &mut Rng, form: &Form, depth: u32) -> V {
    match form {
        Form::Base(b) => draw_base(rng, *b),
        Form::Uuid => {
            let b: Vec<u8> = (0..16).map(|_| rng.next() as u8).collect();
            V::of_scalar(Scalar::Bytes(&b), Base::Bytes)
        }
        Form::Newtype { base, .. } => draw(rng, base, depth),
        Form::Span(point) => {
            let a: V = draw(rng, point, depth + 1);
            let b = draw(rng, point, depth + 1);
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            V::of_ctor(0, vec![("start", start), ("end", end)])
        }
        Form::Sum { ctors, allowed } => {
            let allowed: Vec<u32> = match allowed {
                Some(s) => s.clone(),
                None => (0..ctors.len() as u32).collect(),
            };
            let c = if depth > 3 {
                allowed[0]
            } else {
                *rng.pick(&allowed)
            };
            V::of_ctor(c, draw_fields(rng, &ctors[c as usize].fields, depth))
        }
        Form::Tag(names) => V::of_ctor(rng.below(names.len() as u64) as u32, Vec::new()),
        Form::Record(fs) => V::of_record(draw_fields(rng, fs, depth)),
        Form::Unit => V::of_record(Vec::new()),
        Form::Key { value, .. } => V::of_key(match value {
            KeyForm::One(f) => draw(rng, f, depth + 1),
            KeyForm::Fields(fs) => V::of_record(draw_fields(rng, fs, depth)),
        }),
        Form::Set { elem, nonempty } => {
            let n = if depth > 3 { 0 } else { rng.below(4) } + u64::from(*nonempty);
            V::of_set((0..n).map(|_| draw(rng, elem, depth + 1)).collect())
        }
        Form::Opaque => panic!("no values are drawn at an opaque form"),
    }
}

#[test]
fn engine_values_read_back_as_printed_for_every_type() {
    let f = Fixture::new(SCHEMA);
    let mut rng = Rng(0x7e47_0001);
    for name in TYPES {
        let form = Form::of(&f.schema, &f.ty(name));
        for i in 0..300 {
            let v: Val = draw(&mut rng, &form, 0);
            let printed = textual::print(&v, &form);
            let back: Val = textual::parse(&printed, &form).unwrap_or_else(|e| {
                panic!("draw {i}: `{printed}` does not read back at {name}: {e}")
            });
            assert_eq!(v, back, "draw {i}: `{printed}` at {name}");
            assert_eq!(
                printed,
                textual::print(&back, &form),
                "draw {i} at {name}: equal values print identically"
            );
        }
    }
}

#[test]
fn a_value_built_both_ways_prints_as_the_reference_prints_it() {
    let f = Fixture::new(SCHEMA);
    let mut rng = Rng(0x7e47_0002);
    for name in TYPES {
        let ty = f.ty(name);
        let form = Form::of(&f.schema, &ty);
        for i in 0..300 {
            let mut again = rng.clone();
            let engine: Val = draw(&mut rng, &form, 0);
            let reference: Value = draw(&mut again, &form, 0);
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
        let printed = textual::print(&v, &form);
        assert_eq!(
            adtb_reference::textual::print(&f.schema, &want, &ty),
            printed,
            "`{text}`"
        );
        let back: Val = textual::parse(&printed, &form)
            .unwrap_or_else(|e| panic!("`{printed}` does not read back: {e}"));
        assert_eq!(v, back, "`{text}`");
    }
}
