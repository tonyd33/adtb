use std::fmt::{self, Write};

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

fn uuid(b: &[u8]) -> R<String> {
    if b.len() != 16 {
        return mistyped(format!("a `Uuid` is 16 bytes, found {}", b.len()));
    }
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
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

/// Why a value does not print at a form: the value is not of the form's shape, or the form has
/// no textual syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintError {
    pub message: String,
}

impl fmt::Display for PrintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PrintError {}

type R<T> = Result<T, PrintError>;

fn mistyped<T>(message: impl Into<String>) -> R<T> {
    Err(PrintError {
        message: message.into(),
    })
}

fn scalar<V: Textual>(v: &V) -> R<Scalar<'_>> {
    match v.scalar() {
        Some(s) => Ok(s),
        None => mistyped(format!("a value of a base form is a scalar, found {v:?}")),
    }
}

fn bytes<V: Textual>(v: &V) -> R<&[u8]> {
    match scalar(v)? {
        Scalar::Bytes(b) => Ok(b),
        s => mistyped(format!("a value of a bytes form is bytes, found {s:?}")),
    }
}

fn fields<V: Textual>(v: &V, n: usize) -> R<Vec<&V>> {
    match v.fields().map(Iterator::collect::<Vec<_>>) {
        Some(fs) if fs.len() == n => Ok(fs),
        _ => mistyped(format!(
            "a value of a form with {n} fields has {n} fields, found {v:?}"
        )),
    }
}

fn ctor<'f, T, V: Textual>(v: &V, ctors: &'f [T]) -> R<&'f T> {
    match v.ctor().and_then(|c| ctors.get(c as usize)) {
        Some(c) => Ok(c),
        None => mistyped(format!(
            "a value of a sum is one of its {} constructors, found {v:?}",
            ctors.len()
        )),
    }
}

/// Returns the text of scalar `s` of base type `b`, unquoted and with no `0x`.
fn scalar_text(s: Scalar<'_>, b: Base) -> R<String> {
    use Base as B;
    Ok(match (s, b) {
        (Scalar::Int(x), B::I8 | B::I16 | B::I32 | B::I64) => x.to_string(),
        (Scalar::Decimal(d), B::Decimal(scale)) => decimal(d, scale),
        (Scalar::F64(f), B::F64) => float(f),
        (Scalar::F32(f), B::F32) => float32(f),
        (Scalar::Instant(t), B::Instant) => format!("{}Z", show_datetime(t)),
        (Scalar::Date(d), B::Date) => show_date(d),
        (Scalar::Time(t), B::Time) => show_time(t),
        (Scalar::DateTime(t), B::DateTime) => show_datetime(t),
        (Scalar::Zoned { local, zone }, B::ZonedDateTime) => {
            format!("{}[{zone}]", show_datetime(local))
        }
        (Scalar::Duration(d), B::Duration) => duration(d),
        (Scalar::Period { months, days }, B::Period) => format!("P{months}M{days}D"),
        (Scalar::Text(t), B::Text) => t.to_string(),
        (Scalar::Bytes(x), B::Bytes | B::SortKey) => {
            let mut out = String::new();
            hex(&mut out, x);
            out
        }
        (s, b) => return mistyped(format!("{s:?} is not a value of {b:?}")),
    })
}

/// Returns `text(v)` for `v` of form `form`: a number or time value as its literal, a `Uuid` in
/// its 8-4-4-4-12 form, and a newtype over `Text` as its text. Any other value is printed.
pub fn to_text<V: Textual>(v: &V, form: &Form) -> R<String> {
    match form {
        Form::Uuid => uuid(bytes(v)?),
        Form::Newtype { base, .. } => match base.as_ref() {
            Form::Base(Base::Text) => scalar_text(scalar(v)?, Base::Text),
            f => to_text(v, f),
        },
        Form::Base(Base::Text | Base::Bytes | Base::SortKey) => print(v, form),
        Form::Base(b) => scalar_text(scalar(v)?, *b),
        _ => print(v, form),
    }
}

/// Returns the textual syntax of `v`, a value of form `form`.
///
/// Fails when `v` is not of `form`'s shape, or `form` is `Opaque` or holds it on the path `v`
/// takes.
pub fn print<V: Textual>(v: &V, form: &Form) -> R<String> {
    let mut out = String::new();
    write_val(&mut out, v, form)?;
    Ok(out)
}

fn write_fields<V: Textual>(out: &mut String, names: &[(String, Form)], v: &V) -> R<()> {
    out.push_str("{ ");
    for (i, ((n, f), x)) in names.iter().zip(fields(v, names.len())?).enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(n);
        out.push_str(": ");
        write_val(out, x, f)?;
    }
    out.push_str(" }");
    Ok(())
}

fn write_val<V: Textual>(out: &mut String, v: &V, form: &Form) -> R<()> {
    match form {
        Form::Base(Base::Text) => match scalar(v)? {
            Scalar::Text(t) => quote(out, t),
            s => return mistyped(format!("a value of `Text` is text, found {s:?}")),
        },
        Form::Base(Base::Bytes | Base::SortKey) => hex(out, bytes(v)?),
        Form::Base(b) => out.push_str(&scalar_text(scalar(v)?, *b)?),
        Form::Uuid => {
            write!(out, "uuid(\"{}\")", uuid(bytes(v)?)?).expect("a String accepts every write");
        }
        Form::Newtype { name, base } => {
            out.push_str(name);
            out.push('(');
            write_val(out, v, base)?;
            out.push(')');
        }
        Form::Span(point) => {
            let fs = fields(v, 2)?;
            let (start, end) = (fs[0], fs[1]);
            out.push_str("span(");
            write_val(out, start, point)?;
            out.push_str(", ");
            write_val(out, end, point)?;
            out.push(')');
        }
        Form::Sum { ctors, .. } => {
            let ctor = ctor(v, ctors)?;
            out.push_str(&ctor.name);
            if !ctor.fields.is_empty() {
                out.push(' ');
                write_fields(out, &ctor.fields, v)?;
            }
        }
        Form::Tag(names) => out.push_str(ctor(v, names)?),
        Form::Record(fs) if fs.is_empty() => out.push_str("{}"),
        Form::Record(fs) => write_fields(out, fs, v)?,
        Form::Unit => out.push_str("()"),
        Form::Key { rel, value } => {
            let Some(key) = v.key() else {
                return mistyped(format!("a value of a key form is a key, found {v:?}"));
            };
            out.push_str(rel);
            out.push_str(".key(");
            match value {
                KeyForm::One(f) => write_val(out, key, f)?,
                KeyForm::Fields(fs) => {
                    for (i, ((n, f), x)) in fs.iter().zip(fields(key, fs.len())?).enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(n);
                        out.push_str(": ");
                        write_val(out, x, f)?;
                    }
                }
            }
            out.push(')');
        }
        Form::Set { elem, .. } => {
            let Some(elems) = v.elems() else {
                return mistyped(format!("a value of a set form is a set, found {v:?}"));
            };
            out.push('[');
            for (i, x) in elems.enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_val(out, x, elem)?;
            }
            out.push(']');
        }
        Form::Opaque => return mistyped("this type has no textual syntax"),
    }
    Ok(())
}
