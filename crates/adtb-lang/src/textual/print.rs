use std::fmt::Write;

use super::{Form, KeyForm, Scalar, Textual};
use crate::calendar::{MICROS_PER_DAY, civil_from_days};
use crate::types::Base;

fn show_date(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    if (0..=9999).contains(&y) {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        format!("{y:+06}-{m:02}-{d:02}")
    }
}

fn show_time(micros: i64) -> String {
    let s = micros / 1_000_000;
    format!(
        "{:02}:{:02}:{:02}.{:06}",
        s / 3600,
        s / 60 % 60,
        s % 60,
        micros % 1_000_000
    )
}

fn show_datetime(micros: i64) -> String {
    let days = micros.div_euclid(MICROS_PER_DAY);
    let rest = micros.rem_euclid(MICROS_PER_DAY);
    format!("{}T{}", show_date(days), show_time(rest))
}

fn decimal(d: i128, s: u8) -> String {
    let digits = d.unsigned_abs().to_string();
    let s = usize::from(s);
    let digits = format!("{digits:0>width$}", width = s + 1);
    let (whole, frac) = digits.split_at(digits.len() - s);
    let sign = if d < 0 { "-" } else { "" };
    if s == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{frac}")
    }
}

fn float(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "inf" } else { "-inf" }.into()
    } else {
        format!("{f:?}")
    }
}

fn float32(f: f32) -> String {
    if f.is_nan() || f.is_infinite() {
        float(f64::from(f))
    } else {
        format!("{f:?}")
    }
}

fn duration(d: i64) -> String {
    let units = [
        (86_400_000_000, "d"),
        (3_600_000_000, "h"),
        (60_000_000, "min"),
        (1_000_000, "s"),
        (1_000, "ms"),
    ];
    let (per, unit) = units
        .iter()
        .find(|(per, _)| d % per == 0)
        .copied()
        .unwrap_or((1, "us"));
    if d == 0 {
        return "0s".into();
    }
    format!("{}{unit}", d / per)
}

fn uuid(b: &[u8]) -> String {
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn hex(out: &mut String, b: &[u8]) {
    out.push_str("0x");
    for x in b {
        write!(out, "{x:02x}").expect("a String accepts every write");
    }
}

fn quote(out: &mut String, t: &str) {
    out.push('"');
    for c in t.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                write!(out, "\\u{{{:x}}}", c as u32).expect("a String accepts every write");
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn bytes<V: Textual>(v: &V) -> &[u8] {
    match v.scalar() {
        Scalar::Bytes(b) => b,
        s => panic!("a value of a bytes form is bytes, found {s:?}"),
    }
}

/// Returns the text of scalar `s` of base type `b`, unquoted and with no `0x`.
fn scalar_text(s: Scalar<'_>, b: Base) -> String {
    match (s, b) {
        (Scalar::Int(x), _) => x.to_string(),
        (Scalar::Decimal(d), Base::Decimal(scale)) => decimal(d, scale),
        (Scalar::F64(f), _) => float(f),
        (Scalar::F32(f), _) => float32(f),
        (Scalar::Instant(t), _) => format!("{}Z", show_datetime(t)),
        (Scalar::Date(d), _) => show_date(d),
        (Scalar::Time(t), _) => show_time(t),
        (Scalar::DateTime(t), _) => show_datetime(t),
        (Scalar::Zoned { local, zone }, _) => format!("{}[{zone}]", show_datetime(local)),
        (Scalar::Duration(d), _) => duration(d),
        (Scalar::Period { months, days }, _) => format!("P{months}M{days}D"),
        (Scalar::Text(t), _) => t.to_string(),
        (Scalar::Bytes(x), _) => {
            let mut out = String::new();
            hex(&mut out, x);
            out
        }
        (s, b) => panic!("{s:?} is not a value of {b:?}"),
    }
}

/// Returns `text(v)` for `v` of form `form`: a number or time value as its literal, a `Uuid` in
/// its 8-4-4-4-12 form, and a newtype over `Text` as its text. Any other value is printed.
pub fn to_text<V: Textual>(v: &V, form: &Form) -> String {
    match form {
        Form::Uuid => uuid(bytes(v)),
        Form::Newtype { base, .. } => match base.as_ref() {
            Form::Base(Base::Text) => scalar_text(v.scalar(), Base::Text),
            f => to_text(v, f),
        },
        Form::Base(Base::Text | Base::Bytes | Base::SortKey) => print(v, form),
        Form::Base(b) => scalar_text(v.scalar(), *b),
        _ => print(v, form),
    }
}

/// Returns the textual syntax of `v`, a value of form `form`.
///
/// Panics when `form` is `Opaque` or holds it on the path `v` takes.
pub fn print<V: Textual>(v: &V, form: &Form) -> String {
    let mut out = String::new();
    write_val(&mut out, v, form);
    out
}

fn write_fields<'v, V: Textual + 'v>(
    out: &mut String,
    names: &[(String, Form)],
    vs: impl Iterator<Item = &'v V>,
) {
    out.push_str("{ ");
    for (i, ((n, f), v)) in names.iter().zip(vs).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(n);
        out.push_str(": ");
        write_val(out, v, f);
    }
    out.push_str(" }");
}

fn write_val<V: Textual>(out: &mut String, v: &V, form: &Form) {
    match form {
        Form::Base(Base::Text) => match v.scalar() {
            Scalar::Text(t) => quote(out, t),
            s => panic!("a value of `Text` is text, found {s:?}"),
        },
        Form::Base(Base::Bytes | Base::SortKey) => hex(out, bytes(v)),
        Form::Base(b) => out.push_str(&scalar_text(v.scalar(), *b)),
        Form::Uuid => {
            write!(out, "uuid(\"{}\")", uuid(bytes(v))).expect("a String accepts every write");
        }
        Form::Newtype { name, base } => {
            out.push_str(name);
            out.push('(');
            write_val(out, v, base);
            out.push(')');
        }
        Form::Span(point) => {
            let mut fs = v.fields();
            let (Some(start), Some(end)) = (fs.next(), fs.next()) else {
                panic!("a span has a start and an end")
            };
            out.push_str("span(");
            write_val(out, start, point);
            out.push_str(", ");
            write_val(out, end, point);
            out.push(')');
        }
        Form::Sum { ctors, .. } => {
            let ctor = &ctors[v.ctor() as usize];
            out.push_str(&ctor.name);
            if !ctor.fields.is_empty() {
                out.push(' ');
                write_fields(out, &ctor.fields, v.fields());
            }
        }
        Form::Tag(names) => out.push_str(&names[v.ctor() as usize]),
        Form::Record(fs) if fs.is_empty() => out.push_str("{}"),
        Form::Record(fs) => write_fields(out, fs, v.fields()),
        Form::Unit => out.push_str("()"),
        Form::Key { rel, value } => {
            out.push_str(rel);
            out.push_str(".key(");
            match value {
                KeyForm::One(f) => write_val(out, v.key(), f),
                KeyForm::Fields(fs) => {
                    for (i, ((n, f), x)) in fs.iter().zip(v.key().fields()).enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(n);
                        out.push_str(": ");
                        write_val(out, x, f);
                    }
                }
            }
            out.push(')');
        }
        Form::Set { elem, .. } => {
            out.push('[');
            for (i, x) in v.elems().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_val(out, x, elem);
            }
            out.push(']');
        }
        Form::Opaque => panic!("`print` takes a value of a type with a textual syntax"),
    }
}
