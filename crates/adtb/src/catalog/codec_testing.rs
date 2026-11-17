//! Rows of catalog relations stored under one schema and read back under another.

use std::fmt::Debug;

use adtb_lang::ids::CatCtorId;
use adtb_lang::schema::{self, ENGINE_SRC, Schema};
use adtb_lang::types::{RelId, Ty};

use crate::heap::{FieldType, Layout};
use crate::key::{F64, Scalar, Value};

use super::super::ids::{ctor_index, stored};
use super::super::{engine_schema, layout, row_type};
use super::{Codec, Decode, Field, Kind, Nested, Payload, Sum, coded};

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

/// Returns each way `T`'s samples fail to read back, or to be written by field handle as the
/// stored row of the fields of their names and read back: under the shipped catalog, the one
/// with its constructors reversed and the one with its fields reversed; each sum constructor its
/// constructors list written at another position than declared; stored by the shipped catalog and
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
        ("shipped", &*shipped),
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
            let at = position(schema, &codec, id);
            let read = codec.decode(at, &row);
            if read.as_ref() != Ok(&typed) {
                out.push(format!(
                    "{name} {id} under {catalog}: {read:?}, not {typed:?}"
                ));
            }
            let written = codec.encode(id, by_handle(schema, &codec, id, &row));
            if written != (at, row.clone()) {
                out.push(format!(
                    "{name} {id} written under {catalog}: {written:?}, not {:?}",
                    (at, &row)
                ));
            }
            let reread = codec.decode(written.0, &written.1);
            if reread.as_ref() != Ok(&typed) {
                out.push(format!(
                    "{name} {id} written then read under {catalog}: {reread:?}, not {typed:?}"
                ));
            }
        }
        out.extend(fields_written(schema, &codec, catalog, &name));
        out.extend(tags_written(schema, &codec, catalog, &name));
        out.extend(nested_written(schema, &codec, catalog, &name));
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
        for sum in k.sums {
            let field = sum.field.fields[sum.field.at];
            let tys = (
                field_ty(&shipped, old.relation(), was, field),
                field_ty(&ctors, by_ctor.relation(), now, field),
            );
            out.extend(unmirrored(
                (&shipped, &ctors),
                (&tys.0, &tys.1),
                sum.ctors,
                &name,
            ));
        }
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

/// Returns each field of constructor `id` of `codec` by its handle, holding the value `row`, a
/// stored row of it in `schema`, holds in the field of its name.
fn by_handle<T: Decode>(
    schema: &Schema,
    codec: &Codec<T>,
    id: CatCtorId,
    row: &[Value],
) -> Vec<(Field, Value)> {
    let declared = layout::row(schema, codec.relation(), position(schema, codec, id));
    let ctor = T::CTORS
        .iter()
        .find(|k| k.id == id)
        .unwrap_or_else(|| panic!("the codec lists constructor {id}"));
    (0..ctor.fields.len())
        .map(|at| {
            let f = Field {
                fields: ctor.fields,
                at,
            };
            let named = declared
                .iter()
                .position(|(n, _)| n == ctor.fields[at])
                .unwrap_or_else(|| panic!("constructor {id} declares `{}`", ctor.fields[at]));
            (f, row[named].clone())
        })
        .collect()
}

/// Returns a value stored as `t` that differs from that of each other field of its row: built
/// from `n`, the field's position in its constructor's list, and `name`, its name.
fn distinct(t: &FieldType, n: usize, name: &str) -> Value {
    let k = i64::try_from(n).expect("a position fits 63 bits") + 1000;
    match t {
        FieldType::Scalar(Scalar::I64) => Value::I64(k),
        FieldType::Scalar(Scalar::I32) => Value::I32(i32::try_from(k).expect("a position fits")),
        FieldType::Scalar(Scalar::Text) => Value::Text(name.into()),
        FieldType::Scalar(Scalar::Bytes) => Value::Bytes(name.as_bytes().to_vec()),
        FieldType::Scalar(Scalar::Instant) => Value::Instant(k),
        FieldType::Scalar(Scalar::Duration) => Value::Duration(k),
        FieldType::Scalar(Scalar::F64) => Value::F64(F64::new(k as f64)),
        FieldType::Record(ts) => Value::Record(ts.iter().map(|t| distinct(t, n, name)).collect()),
        FieldType::Sum(cs) => {
            let c = n % cs.len();
            Value::Sum {
                ctor: u32::try_from(c).expect("a position fits 32 bits"),
                fields: cs[c].fields.iter().map(|t| distinct(t, n, name)).collect(),
            }
        }
        other => panic!("no distinct value of {other:?}"),
    }
}

/// Returns each way `codec` writes a constructor's fields, each given a value distinct from the
/// others by its handle, otherwise than `schema` places those values by field name.
fn fields_written<T: Decode>(
    schema: &Schema,
    codec: &Codec<T>,
    catalog: &str,
    name: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for k in T::CTORS {
        let at = position(schema, codec, k.id);
        let declared = layout::row(schema, codec.relation(), at);
        let value = |field: &str| {
            let n = k
                .fields
                .iter()
                .position(|f| *f == field)
                .unwrap_or_else(|| panic!("constructor {} lists `{field}`", k.id));
            let (_, ty) = declared
                .iter()
                .find(|(d, _)| d == field)
                .unwrap_or_else(|| panic!("constructor {} declares `{field}`", k.id));
            distinct(&layout::field_type(schema, ty), n, field)
        };
        let want: Vec<Value> = declared.iter().map(|(d, _)| value(d)).collect();
        let given = (0..k.fields.len()).map(|n| {
            let f = Field {
                fields: k.fields,
                at: n,
            };
            (f, value(k.fields[n]))
        });
        let written = codec.encode(k.id, given);
        if written != (at, want.clone()) {
            out.push(format!(
                "{name} {} written by handle under {catalog}: {written:?}, not {:?}",
                k.id,
                (at, &want)
            ));
        }
    }
    out
}

/// Returns each way `codec` writes a constructor of a sum field its constructors list at another
/// position than `schema` declares it.
fn tags_written<T: Decode>(
    schema: &Schema,
    codec: &Codec<T>,
    catalog: &str,
    name: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for k in T::CTORS {
        let at = (codec.relation(), position(schema, codec, k.id));
        for s in k.sums {
            let field = s.field.fields[s.field.at];
            for p in s.ctors {
                let id = p.id;
                let Value::Sum { ctor: want, .. } = tag(schema, at, field, id, Vec::new()) else {
                    unreachable!("`tag` returns a sum's value");
                };
                let written = codec.tag(*s, id);
                if written != want {
                    out.push(format!(
                        "{name} {} field `{field}` writes {id} at {written} under {catalog}, not {want}",
                        k.id
                    ));
                }
            }
        }
    }
    out
}

/// Returns each way `codec` writes or reads a value of a sum field otherwise than `schema` places
/// it by constructor id and field name: for each constructor the field lists, a value of it for
/// each choice `n` up to the widest sum below it, each sum below taking its constructor `n`
/// modulo its count and each scalar a value distinct from every other in the value.
fn nested_written<T: Decode>(
    schema: &Schema,
    codec: &Codec<T>,
    catalog: &str,
    name: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    for k in T::CTORS {
        let at = position(schema, codec, k.id);
        for s in k.sums {
            let field = s.field.fields[s.field.at];
            let ty = field_ty(schema, codec.relation(), at, field);
            for p in s.ctors {
                for n in 0..widest(p.kinds) {
                    let mut count = 0;
                    let v = chosen(*p, n, &mut count);
                    let want = by_name(schema, &ty, &v);
                    let written = codec.stored(*s, &v);
                    if written != want {
                        out.push(format!(
                            "{name} {} field `{field}` writes {v:?} under {catalog} as {written:?}, not {want:?}",
                            k.id
                        ));
                    }
                    let read = codec.table(*s).nested(&want);
                    if read != v {
                        out.push(format!(
                            "{name} {} field `{field}` reads {want:?} under {catalog} as {read:?}, not {v:?}",
                            k.id
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Returns each constructor of `ctors`, or of a sum at any depth below them, that the second
/// schema does not hold at the mirror of its position in the first when the engine's text declares
/// its sum, `tys` the sum's type in each.
fn unmirrored(
    (from, to): (&Schema, &Schema),
    tys: (&Ty, &Ty),
    ctors: &[Payload],
    name: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let at = |s: &Schema, ty: &Ty, id: CatCtorId| {
        let FieldType::Sum(cs) = layout::field_type(s, ty) else {
            panic!("{ty:?} is a sum");
        };
        cs.iter()
            .position(|k| k.id == stored(id))
            .unwrap_or_else(|| panic!("{id} is a constructor of {ty:?}"))
    };
    let Ty::Data { def, .. } = tys.0 else {
        panic!("{:?} is a declared sum", tys.0);
    };
    let reversed = from.ty(*def).module != schema::PRELUDE;
    for p in ctors {
        let (was, now) = (at(from, tys.0, p.id), at(to, tys.1, p.id));
        if reversed && now != ctors.len() - 1 - was {
            out.push(format!(
                "{name}: sum constructor {} at {now} under ctors, not {}",
                p.id,
                ctors.len() - 1 - was
            ));
        }
        let fields = |s: &Schema, ty: &Ty, at: usize| layout::ctors(s, ty).swap_remove(at).1;
        let (old, new) = (fields(from, tys.0, was), fields(to, tys.1, now));
        for (f, k) in p.fields().iter().zip(p.kinds) {
            let Kind::Sum(sub) = k else { continue };
            let ty = |fs: &[(String, Ty)]| {
                fs.iter()
                    .find(|(n, _)| n == f)
                    .unwrap_or_else(|| panic!("{} declares `{f}`", p.id))
                    .1
                    .clone()
            };
            out.extend(unmirrored((from, to), (&ty(&old), &ty(&new)), sub, name));
        }
    }
    out
}

/// Returns the most constructors of any sum `kinds` holds at any depth, at least 1.
fn widest(kinds: &[Kind]) -> usize {
    kinds
        .iter()
        .map(|k| match k {
            Kind::Scalar(_) => 1,
            Kind::Sum(ps) => ps
                .iter()
                .map(|p| widest(p.kinds))
                .max()
                .unwrap_or(1)
                .max(ps.len()),
        })
        .max()
        .unwrap_or(1)
}

/// Returns a value of `p`, each sum below it taking its constructor `n` modulo its count and each
/// scalar the next of `count`.
fn chosen(p: Payload, n: usize, count: &mut i64) -> Nested {
    let values = p.fields().iter().zip(p.kinds).map(|(name, k)| match k {
        Kind::Scalar(s) => {
            *count += 1;
            Nested::Scalar(distinct(
                &FieldType::Scalar(*s),
                usize::try_from(*count).expect("a count fits"),
                &format!("{name}{count}"),
            ))
        }
        Kind::Sum(ps) => chosen(ps[n % ps.len()], n, count),
    });
    let given: Vec<(Field, Nested)> = (0..p.fields().len())
        .map(|at| Field {
            fields: p.fields(),
            at,
        })
        .zip(values)
        .collect();
    Nested::of(p, given)
}

/// Returns the type of field `field` of constructor `c` of `r` in `schema`.
pub(crate) fn field_ty(schema: &Schema, r: RelId, c: u32, field: &str) -> Ty {
    layout::row(schema, r, c)
        .into_iter()
        .find(|(n, _)| n == field)
        .unwrap_or_else(|| panic!("the row has field `{field}`"))
        .1
}

/// Returns `v`, a value of `ty`, stored as `schema` declares it: each constructor at the position
/// of its id, each field at the position of its name.
pub(crate) fn by_name(schema: &Schema, ty: &Ty, v: &Nested) -> Value {
    let Nested::Ctor { id, fields, values } = v else {
        let Nested::Scalar(v) = v else {
            unreachable!("a value is a scalar or a constructor")
        };
        return v.clone();
    };
    let FieldType::Sum(cs) = layout::field_type(schema, ty) else {
        panic!("{ty:?} is a sum");
    };
    let at = cs
        .iter()
        .position(|k| k.id == stored(*id))
        .unwrap_or_else(|| panic!("{id} is a constructor of {ty:?}"));
    let declared = layout::ctors(schema, ty).swap_remove(at).1;
    assert_eq!(declared.len(), fields.len(), "{id} is given each field");
    let fields = declared
        .iter()
        .map(|(n, t)| {
            let given = fields
                .iter()
                .position(|f| f == n)
                .unwrap_or_else(|| panic!("{id} is given field `{n}`"));
            by_name(schema, t, &values[given])
        })
        .collect();
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
    Layout::new(row_type(to, s, now))
        .decode(&fixed, &vars)
        .unwrap()
}

/// Returns `text` with the constructors of each sum in reverse order.
pub(crate) fn reversed_sums(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some((line, next)) = aligned_reversed(&lines[i..]) {
            out.push(line);
            i += next;
            continue;
        }
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

/// Returns the declaration `lines` starts with, its constructors reversed on one line, and the
/// count of lines it spans, when it states its constructors aligned under the first, as
/// `type T = A { x: X }` with `       | B` on the next line does.
fn aligned_reversed(lines: &[&str]) -> Option<(String, usize)> {
    let (head, first) = lines[0].strip_prefix("type ")?.split_once(" = ")?;
    let mut ctors = vec![first.trim().to_owned()];
    let mut n = 1;
    while let Some(line) = lines.get(n).filter(|l| l.starts_with(' ')) {
        match line.trim_start().strip_prefix("| ") {
            Some(c) => ctors.push(c.trim().to_owned()),
            None => {
                let last = ctors.last_mut().expect("a constructor comes first");
                last.push(' ');
                last.push_str(line.trim());
            }
        }
        n += 1;
    }
    if ctors.len() < 2 || n < 2 {
        return None;
    }
    ctors.reverse();
    Some((format!("type {head} = {}", ctors.join(" | ")), n))
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

/// Returns the fixed ids of the constructors `s` lists.
pub(crate) fn ids(s: Sum) -> Vec<CatCtorId> {
    s.ctors.iter().map(|p| p.id).collect()
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
