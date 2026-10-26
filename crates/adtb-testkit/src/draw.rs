//! Random values of a textual form, built through [`Textual`] so two value types draw equal
//! values from equal seeds.

use adtb_lang::calendar::MICROS_PER_DAY as DAY;
use adtb_lang::textual::{Form, KeyForm, Scalar, Textual};
use adtb_lang::types::Base;

use crate::Rng;

/// The zones a drawn zoned value names.
pub const ZONES: &[&str] = &["UTC", "Europe/Paris", "Asia/Kathmandu"];

/// Returns an `i64` drawn mostly from small magnitudes and the edges of the range.
fn int(rng: &mut Rng) -> i64 {
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
    "\u{1f600}",
    "\u{ffff}",
    "\u{10ffff}",
    "\u{2028}",
    "{",
    "]",
    ",",
    ":",
];

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
    let i = int(rng);
    let text: String;
    let bytes: Vec<u8>;
    let s = match b {
        Base::I8 => Scalar::Int(i64::from(i as i8)),
        Base::I16 => Scalar::Int(i64::from(i as i16)),
        Base::I32 => Scalar::Int(i64::from(i as i32)),
        Base::I64 => Scalar::Int(i),
        Base::Decimal(_) => {
            let bound = 10i128.pow(38);
            let wide = (i128::from(rng.next_u64()) << 64 | i128::from(rng.next_u64())) % bound;
            Scalar::Decimal(match rng.below(3) {
                0 => wide,
                1 => *rng.pick(&[0, 1, -1, bound - 1, 1 - bound]),
                _ => i128::from(i),
            })
        }
        Base::F64 => Scalar::F64(canon64(match rng.below(3) {
            0 => *rng.pick(&[0.0, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.1]),
            1 => i as f64 / 1000.0,
            _ => f64::from_bits(rng.next_u64()),
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
            _ => f32::from_bits(rng.next_u64() as u32),
        })),
        Base::Text => {
            let n = rng.below(6);
            text = (0..n).map(|_| *rng.pick(CHARS)).collect();
            Scalar::Text(&text)
        }
        Base::Bytes | Base::SortKey => {
            let n = rng.below(6);
            bytes = (0..n).map(|_| rng.next_u64() as u8).collect();
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
            months: int(rng) as i32,
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
        .map(|(n, f)| (n.as_str(), draw_at(rng, f, depth + 1)))
        .collect()
}

/// Returns a value of form `form` drawn from `rng`, built as a `V`; the same draws build equal
/// values of any two value types. Its zones are among `ZONES`.
///
/// Panics at an opaque form.
pub fn draw<V: Textual>(rng: &mut Rng, form: &Form) -> V {
    draw_at(rng, form, 0)
}

fn draw_at<V: Textual>(rng: &mut Rng, form: &Form, depth: u32) -> V {
    match form {
        Form::Base(b) => draw_base(rng, *b),
        Form::Uuid => {
            let b: Vec<u8> = (0..16).map(|_| rng.next_u64() as u8).collect();
            V::of_scalar(Scalar::Bytes(&b), Base::Bytes)
        }
        Form::Newtype { base, .. } => draw_at(rng, base, depth),
        Form::Span(point) => {
            let a: V = draw_at(rng, point, depth + 1);
            let b = draw_at(rng, point, depth + 1);
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
            KeyForm::One(f) => draw_at(rng, f, depth + 1),
            KeyForm::Fields(fs) => V::of_record(draw_fields(rng, fs, depth)),
        }),
        Form::Set { elem, nonempty } => {
            let n = if depth > 3 { 0 } else { rng.below(4) } + u64::from(*nonempty);
            V::of_set((0..n).map(|_| draw_at(rng, elem, depth + 1)).collect())
        }
        Form::Opaque => panic!("no values are drawn at an opaque form"),
    }
}
