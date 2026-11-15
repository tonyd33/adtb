//! Rows of catalog relations stored under one schema and read back under another.

use adtb_lang::ids::CatCtorId;
use adtb_lang::schema::{self, ENGINE_SRC, Schema};
use adtb_lang::types::RelId;

use crate::heap::{FieldType, Layout};
use crate::key::Value;

use super::super::ids::{ctor_index, stored};
use super::super::{layout, row_type};
use super::{Codec, Decode};

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
