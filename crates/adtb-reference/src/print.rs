use adtb_lang::schema::{Generator, TypeDefKind};
use adtb_lang::types::{Base, Ty};

use crate::eval::{Interpreter, R, not_yet};
use crate::prim::text_newtype;
use crate::time::{show_date, show_datetime, show_time};
use crate::value::Value;

fn decimal(d: i128, s: u8) -> String {
    let digits = d.unsigned_abs().to_string();
    let s = s as usize;
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
        float(f as f64)
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

/// Returns `text(v)` for `v` of type `ty`.
pub fn to_text(it: &Interpreter, v: &Value, ty: &Ty) -> R<String> {
    Ok(match v {
        Value::Int(x) => x.to_string(),
        Value::Decimal(d, s) => decimal(*d, *s),
        Value::F64(f) => float(*f),
        Value::F32(f) => float32(*f),
        Value::Instant(t) => format!("{}Z", show_datetime(*t)),
        Value::Date(d) => show_date(*d),
        Value::Time(t) => show_time(*t),
        Value::DateTime(t) => show_datetime(*t),
        Value::Zoned(..) => return not_yet("zone rules"),
        Value::Duration(d) => duration(*d),
        Value::Period { months, days } => format!("P{months}M{days}D"),
        Value::Bytes(b) if it.is_prelude(ty, "Uuid") => uuid(b),
        Value::Text(t) if text_newtype(it, ty) => t.clone(),
        other => panic!("`text` of {other:?} at {ty:?}"),
    })
}

fn quote(t: &str) -> String {
    let mut out = String::from("\"");
    for c in t.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn fields(it: &Interpreter, fs: &[(String, Value)], tys: &[(String, Ty)]) -> R<String> {
    let mut parts = Vec::new();
    for (n, v) in fs {
        let t = tys
            .iter()
            .find(|(m, _)| m == n)
            .map(|(_, t)| t)
            .unwrap_or_else(|| panic!("field `{n}` has a type"));
        parts.push(format!("{n}: {}", print(it, v, t)?));
    }
    Ok(format!("{{ {} }}", parts.join(", ")))
}

/// The textual syntax of `v`, a value of `ty`.
pub fn print(it: &Interpreter, v: &Value, ty: &Ty) -> R<String> {
    let schema = it.schema;
    match (v, ty) {
        (Value::Text(t), Ty::Base(Base::Text)) => Ok(quote(t)),
        (Value::Bytes(b), Ty::Base(Base::Bytes)) => match std::str::from_utf8(b) {
            Ok(t) => Ok(format!("bytes({})", quote(t))),
            Err(_) => not_yet("printing bytes that are not UTF-8"),
        },
        (Value::Bytes(b), _) if it.is_prelude(ty, "Uuid") => Ok(format!("uuid(\"{}\")", uuid(b))),
        (Value::Ctor(c, fs), Ty::Data { def, args, .. }) => {
            let d = schema.ty(*def);
            let TypeDefKind::Sum { header, ctors, .. } = &d.kind else {
                panic!("a constructor value of a newtype")
            };
            let ctor = &ctors[*c as usize];
            if fs.is_empty() {
                return Ok(ctor.name.clone());
            }
            let tys: Vec<(String, Ty)> = header
                .iter()
                .chain(&ctor.fields)
                .map(|f| (f.name.clone(), f.ty.subst(args)))
                .collect();
            Ok(format!("{} {}", ctor.name, fields(it, fs, &tys)?))
        }
        (Value::Ctor(c, fs), Ty::Sum { ctors, .. }) => {
            let (name, tys) = &ctors[*c as usize];
            if fs.is_empty() {
                return Ok(name.clone());
            }
            Ok(format!("{name} {}", fields(it, fs, tys)?))
        }
        (Value::Ctor(c, _), Ty::Tag(inner)) => match inner.as_ref() {
            Ty::Data { def, .. } => Ok(schema.ty(*def).ctors()[*c as usize].name.clone()),
            t => panic!("a tag of {t:?}"),
        },
        (Value::Record(fs), Ty::Record(tys)) if fs.is_empty() && tys.is_empty() => Ok("{}".into()),
        (Value::Record(_), Ty::Unit) => Ok("()".into()),
        (Value::Record(fs), Ty::Record(tys)) => fields(it, fs, tys),
        (Value::Key(k), Ty::Key { rel, .. }) => {
            let r = schema.relation(*rel);
            let field_ty = |f: &str| -> Ty {
                if it.self_key(*rel, f) {
                    match r.generate {
                        Some(Generator::Uuidv7) => Ty::Data {
                            def: schema.prelude_type("Uuid"),
                            args: Vec::new(),
                            ctors: None,
                        },
                        _ => Ty::Base(Base::I64),
                    }
                } else {
                    schema
                        .ty(r.elem_def)
                        .field(0, f)
                        .expect("a key field")
                        .ty
                        .subst(r.elem_args())
                }
            };
            match (r.key.as_slice(), k.as_ref()) {
                ([f], k) => Ok(format!("{}.key({})", r.name, print(it, k, &field_ty(f))?)),
                (_, Value::Record(fs)) => {
                    let mut parts = Vec::new();
                    for (n, v) in fs {
                        parts.push(format!("{n}: {}", print(it, v, &field_ty(n))?));
                    }
                    Ok(format!("{}.key({})", r.name, parts.join(", ")))
                }
                (_, k) => panic!("a composite key value is a record, found {k:?}"),
            }
        }
        (Value::Set(s), Ty::Set(t) | Ty::NonEmpty(t)) => {
            let mut parts = Vec::new();
            for v in s {
                parts.push(print(it, v, t)?);
            }
            Ok(format!("[{}]", parts.join(", ")))
        }
        (_, Ty::Data { def, .. }) => match &schema.ty(*def).kind {
            TypeDefKind::Newtype { base, .. } => {
                Ok(format!("{}({})", schema.ty(*def).name, print(it, v, base)?))
            }
            TypeDefKind::Sum { .. } => panic!("{v:?} is not a value of a sum"),
        },
        (Value::Fn(_), _) => panic!("`print` takes a value, not a function"),
        _ => to_text(it, v, ty),
    }
}
