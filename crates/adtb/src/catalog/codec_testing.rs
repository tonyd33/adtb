//! Rows of catalog relations stored under one schema and read back under another.

use std::fmt::Debug;

use adtb_lang::ids::CatCtorId;
use adtb_lang::schema::{self, ENGINE_SRC, Schema};
use adtb_lang::types::RelId;

use crate::heap::{FieldType, Layout};
use crate::key::{F64, Scalar, Value};

use super::super::ids::{ctor_index, stored};
use super::super::{engine_schema, layout, row_type};
use super::{Codec, Decode, coded};

/// A row's constructor id, the fields its reader reads by name, and the typed row it reads as.
pub(crate) type Sample<T> = (CatCtorId, Vec<(&'static str, Value)>, T);

/// A codec's typed rows with stored rows that read as them.
pub(crate) trait Sampled: Decode + PartialEq + Debug {
    /// Returns rows of `schema` covering every constructor.
    fn samples(schema: &Schema) -> Vec<Sample<Self>>;
}

/// Returns a value stored as `t`, a sum's first constructor of no fields when it has one.
pub(crate) fn filler(t: &FieldType) -> Value {
    match t {
        FieldType::Scalar(Scalar::I64) => Value::I64(-1),
        FieldType::Scalar(Scalar::I32) => Value::I32(-1),
        FieldType::Scalar(Scalar::Text) => Value::Text("filler".into()),
        FieldType::Scalar(Scalar::Bytes) => Value::Bytes(vec![0xff]),
        FieldType::Scalar(Scalar::Instant) => Value::Instant(-1),
        FieldType::Scalar(Scalar::Duration) => Value::Duration(-1),
        FieldType::Scalar(Scalar::F64) => Value::F64(F64::new(-1.0)),
        FieldType::Record(ts) => Value::Record(ts.iter().map(filler).collect()),
        FieldType::Sum(cs) => {
            let c = cs.iter().position(|c| c.fields.is_empty()).unwrap_or(0);
            Value::Sum {
                ctor: u32::try_from(c).expect("a position fits 32 bits"),
                fields: cs[c].fields.iter().map(filler).collect(),
            }
        }
        other => panic!("no filler of {other:?}"),
    }
}

/// Returns the stored row of constructor `id` of `codec`'s relation in `schema`, each field
/// `given` names holding its value and every other field a filler.
pub(crate) fn filled<T: Decode>(
    schema: &Schema,
    codec: &Codec<T>,
    id: CatCtorId,
    given: &[(&str, Value)],
) -> Vec<Value> {
    let c = position(schema, codec, id);
    for (name, _) in given {
        assert!(
            layout::row(schema, codec.relation(), c)
                .iter()
                .any(|(n, _)| n == name),
            "constructor {id} has field `{name}`"
        );
    }
    layout::row(schema, codec.relation(), c)
        .into_iter()
        .map(|(name, ty)| {
            given.iter().find(|(n, _)| *n == name).map_or_else(
                || filler(&layout::field_type(schema, &ty)),
                |(_, v)| v.clone(),
            )
        })
        .collect()
}

/// Returns each way `T`'s samples fail to read back: under the shipped catalog, the one with its
/// constructors reversed and the one with its fields reversed; stored by the shipped catalog and
/// read by the one with its constructors reversed; or where a reversal leaves a constructor off
/// its mirror position, or a constructor's header fields or own fields unreversed.
pub(crate) fn read_back<T: Sampled>() -> Vec<String> {
    let shipped = engine_schema();
    let ctors = reversed();
    let fields = fields_reversed();
    let mut out = Vec::new();
    let name = format!("{:?}", coded::<T>());
    let mut want: Vec<CatCtorId> = T::CTORS.iter().map(|c| c.id).collect();
    want.sort_unstable();
    for (catalog, schema) in [
        ("shipped", &shipped),
        ("ctors", &ctors),
        ("fields", &fields),
    ] {
        let codec = Codec::<T>::new(schema);
        let samples = T::samples(schema);
        let mut have: Vec<CatCtorId> = samples.iter().map(|(id, _, _)| *id).collect();
        have.sort_unstable();
        have.dedup();
        if have != want {
            out.push(format!("{name}: samples of {have:?}, not {want:?}"));
        }
        for (id, given, typed) in samples {
            let row = filled(schema, &codec, id, &given);
            let read = codec.decode(position(schema, &codec, id), &row);
            if read.as_ref() != Ok(&typed) {
                out.push(format!(
                    "{name} {id} under {catalog}: {read:?}, not {typed:?}"
                ));
            }
        }
    }
    let (old, by_ctor, by_field) = (
        Codec::<T>::new(&shipped),
        Codec::<T>::new(&ctors),
        Codec::<T>::new(&fields),
    );
    for (id, given, typed) in T::samples(&shipped) {
        let (was, now) = (position(&shipped, &old, id), position(&ctors, &by_ctor, id));
        let stored = filled(&shipped, &old, id, &given);
        let row = reread(
            (&shipped, old.relation(), was),
            (&ctors, by_ctor.relation(), now),
            &stored,
        );
        let read = by_ctor.decode(now, &row);
        if read.as_ref() != Ok(&typed) {
            out.push(format!(
                "{name} {id} stored by shipped, read by ctors: {read:?}, not {typed:?}"
            ));
        }
    }
    let n = u32::try_from(T::CTORS.len()).expect("a count fits 32 bits");
    let header_len = shipped
        .ty(shipped.relation(old.relation()).elem_def)
        .header()
        .len();
    let names = |s: &Schema, c: &Codec<T>, id| -> Vec<String> {
        layout::row(s, c.relation(), position(s, c, id))
            .into_iter()
            .map(|(n, _)| n)
            .collect()
    };
    for k in T::CTORS {
        let (was, now) = (
            position(&shipped, &old, k.id),
            position(&ctors, &by_ctor, k.id),
        );
        if now != n - 1 - was {
            out.push(format!(
                "{name} {}: at {now} under ctors, not {}",
                k.id,
                n - 1 - was
            ));
        }
        let declared = names(&shipped, &old, k.id);
        let (header, own) = declared.split_at(header_len);
        let mirrored: Vec<String> = header
            .iter()
            .rev()
            .chain(own.iter().rev())
            .cloned()
            .collect();
        let reversed = names(&fields, &by_field, k.id);
        if mirrored.len() > 1 && reversed != mirrored {
            out.push(format!(
                "{name} {}: fields {reversed:?} under fields, not {mirrored:?}",
                k.id
            ));
        }
    }
    out
}

/// Returns `row`, a stored row of constructor `was` of `from`'s relation `r`, read back by the
/// heap layout of constructor `now` of `to`'s relation `s`.
pub(crate) fn reread(
    (from, r, was): (&Schema, RelId, u32),
    (to, s, now): (&Schema, RelId, u32),
    row: &[Value],
) -> Vec<Value> {
    let (fixed, vars) = Layout::new(row_type(from, r, was)).encode(row);
    Layout::new(row_type(to, s, now)).decode(&fixed, &vars)
}

/// Returns `text` with the constructors of each sum in reverse order.
pub(crate) fn reversed_sums(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("  = ") {
            out.push(one_line_reversed(lines[i]).unwrap_or_else(|| lines[i].to_owned()));
            i += 1;
            continue;
        }
        let mut ctors: Vec<Vec<&str>> = Vec::new();
        while i < lines.len() {
            let line = lines[i];
            if line.starts_with("  = ") || line.starts_with("  | ") {
                ctors.push(vec![&line[4..]]);
            } else if line.starts_with("    ") {
                ctors
                    .last_mut()
                    .expect("a continuation follows a constructor")
                    .push(line);
            } else {
                break;
            }
            i += 1;
        }
        ctors.reverse();
        for (n, ctor) in ctors.iter().enumerate() {
            let lead = if n == 0 { "  = " } else { "  | " };
            out.push(format!("{lead}{}", ctor[0]));
            out.extend(ctor[1..].iter().map(|l| (*l).to_owned()));
        }
    }
    out.join("\n")
}

/// Returns `line` with its constructors reversed when it declares a whole sum, as
/// `type T = A | B { x: X }` does.
fn one_line_reversed(line: &str) -> Option<String> {
    let (head, body) = line.strip_prefix("type ")?.split_once(" = ")?;
    if body.starts_with('{') || head.contains('{') {
        return None;
    }
    let (mut depth, mut start, mut ctors) = (0, 0, Vec::new());
    for (at, c) in body.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            '|' if depth == 0 => {
                ctors.push(body[start..at].trim());
                start = at + 1;
            }
            _ => {}
        }
    }
    ctors.push(body[start..].trim());
    if ctors.len() < 2 {
        return None;
    }
    ctors.reverse();
    Some(format!("type {head} = {}", ctors.join(" | ")))
}

/// Returns the engine schema built from catalog.adt with every sum's constructors reversed.
pub(crate) fn reversed() -> Schema {
    schema::build_engine_from(&reversed_sums(ENGINE_SRC)).expect("the reversed catalog builds")
}

/// Returns `text` with the fields of each record, header and constructor its type declarations
/// state in reverse order.
pub(crate) fn reversed_fields(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("type ") {
            out.push(lines[i].to_owned());
            i += 1;
            continue;
        }
        let mut span = lines[i].to_owned();
        i += 1;
        while i < lines.len() && lines[i].starts_with(' ') {
            span.push('\n');
            span.push_str(lines[i]);
            i += 1;
        }
        let mut done = String::new();
        let mut rest = span.as_str();
        while let Some(open) = rest.find('{') {
            let close = rest.find('}').expect("a brace closes");
            let fields: Vec<&str> = rest[open + 1..close]
                .split(',')
                .map(str::trim)
                .rev()
                .collect();
            done.push_str(&rest[..open]);
            done.push_str(&format!("{{ {} }}", fields.join(", ")));
            rest = &rest[close + 1..];
        }
        done.push_str(rest);
        out.push(done);
    }
    out.join("\n")
}

/// Returns the engine schema built from catalog.adt with every record's, header's and
/// constructor's fields reversed.
pub(crate) fn fields_reversed() -> Schema {
    schema::build_engine_from(&reversed_fields(ENGINE_SRC))
        .expect("the field-reversed catalog builds")
}

/// Returns the position of constructor `id` of `codec`'s relation in `schema`.
pub(crate) fn position<T: Decode>(schema: &Schema, codec: &Codec<T>, id: CatCtorId) -> u32 {
    ctor_index(schema, codec.relation(), id)
        .unwrap_or_else(|| panic!("{id} is a constructor of the relation"))
}

/// Returns the stored row of constructor `c` of `r` in `schema` whose fields `fields` gives by
/// name.
pub(crate) fn row(schema: &Schema, r: RelId, c: u32, fields: &[(&str, Value)]) -> Vec<Value> {
    let names = layout::row(schema, r, c);
    assert_eq!(names.len(), fields.len(), "the row gives every field");
    names
        .iter()
        .map(|(n, _)| {
            fields
                .iter()
                .find(|(f, _)| f == n)
                .unwrap_or_else(|| panic!("the row gives field `{n}`"))
                .1
                .clone()
        })
        .collect()
}

/// Returns the value of constructor `id` of the type of field `field` of constructor `c` of `r`
/// in `schema`, holding `fields`.
pub(crate) fn tag(
    schema: &Schema,
    (r, c): (RelId, u32),
    field: &str,
    id: CatCtorId,
    fields: Vec<Value>,
) -> Value {
    let (_, ty) = layout::row(schema, r, c)
        .into_iter()
        .find(|(n, _)| n == field)
        .unwrap_or_else(|| panic!("the row has field `{field}`"));
    let FieldType::Sum(cs) = layout::field_type(schema, &ty) else {
        panic!("field `{field}` is a sum");
    };
    let at = cs
        .iter()
        .position(|k| k.id == stored(id))
        .unwrap_or_else(|| panic!("{id} is a constructor of field `{field}`"));
    Value::Sum {
        ctor: u32::try_from(at).expect("a position fits 32 bits"),
        fields,
    }
}
