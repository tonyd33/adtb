use adtb_lang::schema::{Generator, Schema, TypeDefKind};
use adtb_lang::types::{Base, RelId, Ty};

use crate::time::{show_date, show_datetime, show_time};
use crate::value::Value;

pub(super) fn is_prelude(schema: &Schema, ty: &Ty, name: &str) -> bool {
    matches!(ty, Ty::Data { def, .. } if *def == schema.prelude_type(name))
}

/// Returns whether key field `f` of `rel` is a `Key` of `rel` itself.
pub fn self_key(schema: &Schema, rel: RelId, f: &str) -> bool {
    let r = schema.relation(rel);
    matches!(
        schema.ty(r.elem_def).field(0, f).map(|d| &d.ty),
        Some(Ty::Key { rel: own, .. }) if *own == rel
    )
}

/// Returns the type of key field `f` of `rel`, where a key field that is a key of `rel` itself
/// has the type of the value its generator draws.
pub(super) fn key_field_ty(schema: &Schema, rel: RelId, f: &str) -> Ty {
    let r = schema.relation(rel);
    if self_key(schema, rel, f) {
        return match r.generate {
            Some(Generator::Uuidv7) => Ty::Data {
                def: schema.prelude_type("Uuid"),
                args: Vec::new(),
                ctors: None,
            },
            _ => Ty::Base(Base::I64),
        };
    }
    schema
        .ty(r.elem_def)
        .field(0, f)
        .expect("a key field")
        .ty
        .subst(r.elem_args())
}

fn text_newtype(schema: &Schema, ty: &Ty) -> bool {
    matches!(ty, Ty::Data { def, .. } if matches!(
        &schema.ty(*def).kind,
        TypeDefKind::Newtype { base: Ty::Base(Base::Text), .. }
    ))
}

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

fn hex(b: &[u8]) -> String {
    let digits: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("0x{digits}")
}

/// Returns `text(v)` for `v` of type `ty`.
pub fn to_text(schema: &Schema, v: &Value, ty: &Ty) -> String {
    match v {
        Value::Int(x) => x.to_string(),
        Value::Decimal(d, s) => decimal(*d, *s),
        Value::F64(f) => float(*f),
        Value::F32(f) => float32(*f),
        Value::Instant(t) => format!("{}Z", show_datetime(*t)),
        Value::Date(d) => show_date(*d),
        Value::Time(t) => show_time(*t),
        Value::DateTime(t) => show_datetime(*t),
        Value::Zoned(t, zone) => format!("{}[{zone}]", show_datetime(*t)),
        Value::Duration(d) => duration(*d),
        Value::Period { months, days } => format!("P{months}M{days}D"),
        Value::Bytes(b) if is_prelude(schema, ty, "Uuid") => uuid(b),
        Value::Text(t) if text_newtype(schema, ty) => t.clone(),
        other => panic!("`text` of {other:?} at {ty:?}"),
    }
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

fn fields(schema: &Schema, fs: &[(String, Value)], tys: &[(String, Ty)]) -> String {
    let parts: Vec<String> = fs
        .iter()
        .map(|(n, v)| {
            let t = tys
                .iter()
                .find(|(m, _)| m == n)
                .map(|(_, t)| t)
                .unwrap_or_else(|| panic!("field `{n}` has a type"));
            format!("{n}: {}", print(schema, v, t))
        })
        .collect();
    format!("{{ {} }}", parts.join(", "))
}

/// Returns the textual syntax of `v`, a value of `ty`.
pub fn print(schema: &Schema, v: &Value, ty: &Ty) -> String {
    match (v, ty) {
        (Value::Text(t), Ty::Base(Base::Text)) => quote(t),
        (Value::Bytes(b) | Value::SortKey(b), Ty::Base(Base::Bytes | Base::SortKey)) => hex(b),
        (Value::Bytes(b), _) if is_prelude(schema, ty, "Uuid") => {
            format!("uuid(\"{}\")", uuid(b))
        }
        (Value::Ctor(_, fs), Ty::Data { args, .. }) if is_prelude(schema, ty, "Span") => format!(
            "span({}, {})",
            print(schema, &fs[0].1, &args[0]),
            print(schema, &fs[1].1, &args[0])
        ),
        (Value::Ctor(c, fs), Ty::Data { def, args, .. }) => {
            let d = schema.ty(*def);
            let TypeDefKind::Sum { header, ctors, .. } = &d.kind else {
                panic!("a constructor value of a newtype")
            };
            let ctor = &ctors[*c as usize];
            if fs.is_empty() {
                return ctor.name.clone();
            }
            let tys: Vec<(String, Ty)> = header
                .iter()
                .chain(&ctor.fields)
                .map(|f| (f.name.clone(), f.ty.subst(args)))
                .collect();
            format!("{} {}", ctor.name, fields(schema, fs, &tys))
        }
        (Value::Ctor(c, fs), Ty::Sum { ctors, .. }) => {
            let (name, tys) = &ctors[*c as usize];
            if fs.is_empty() {
                return name.clone();
            }
            format!("{name} {}", fields(schema, fs, tys))
        }
        (Value::Ctor(c, _), Ty::Tag(inner)) => match inner.as_ref() {
            Ty::Data { def, .. } => schema.ty(*def).ctors()[*c as usize].name.clone(),
            Ty::Sum { ctors, .. } => ctors[*c as usize].0.clone(),
            t => panic!("a tag of {t:?}"),
        },
        (Value::Record(_), Ty::Unit) => "()".into(),
        (Value::Record(fs), Ty::Record(_)) if fs.is_empty() => "{}".into(),
        (Value::Record(fs), Ty::Record(tys)) => fields(schema, fs, tys),
        (Value::Key(k), Ty::Key { rel, .. }) => {
            let r = schema.relation(*rel);
            match (r.key.as_slice(), k.as_ref()) {
                ([f], k) => format!(
                    "{}.key({})",
                    r.name,
                    print(schema, k, &key_field_ty(schema, *rel, f))
                ),
                (_, Value::Record(fs)) => {
                    let parts: Vec<String> = fs
                        .iter()
                        .map(|(n, v)| {
                            format!("{n}: {}", print(schema, v, &key_field_ty(schema, *rel, n)))
                        })
                        .collect();
                    format!("{}.key({})", r.name, parts.join(", "))
                }
                (_, k) => panic!("a composite key value is a record, found {k:?}"),
            }
        }
        (Value::Set(s), Ty::Set(t) | Ty::NonEmpty(t)) => {
            let parts: Vec<String> = s.iter().map(|v| print(schema, v, t)).collect();
            format!("[{}]", parts.join(", "))
        }
        (_, Ty::Data { def, .. }) => match &schema.ty(*def).kind {
            TypeDefKind::Newtype { base, .. } => {
                format!("{}({})", schema.ty(*def).name, print(schema, v, base))
            }
            TypeDefKind::Sum { .. } => panic!("{v:?} is not a value of a sum"),
        },
        (Value::Fn(_), _) => panic!("`print` takes a value, not a function"),
        _ => to_text(schema, v, ty),
    }
}
