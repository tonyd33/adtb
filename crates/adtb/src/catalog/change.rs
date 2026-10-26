use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::schema::Schema;
use adtb_lang::types::RelId;

use crate::heap::FieldType;
use crate::key::Value;

use super::{ctor_id, ctor_relations, indexes, layout, row_type};

/// The rows of the relations a migration rewrites, by relation id and then constructor id, each
/// row in the layout of the schema the operators applied so far produce.
pub type Data = BTreeMap<u64, BTreeMap<u64, Vec<Vec<Value>>>>;

/// A field of a row: its catalog id and how a heap stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub id: Option<u64>,
    pub name: String,
    pub ty: FieldType,
}

/// Returns the fields of a row of constructor `c` of relation `r`, the header's first.
pub fn fields(schema: &Schema, r: RelId, c: u32) -> Vec<Field> {
    let def = schema.relation(r).elem_def;
    layout::row(schema, r, c)
        .into_iter()
        .zip(row_type(schema, r, c))
        .map(|((name, _), ty)| {
            let id = schema
                .ids
                .fields
                .get(&(def, None, name.clone()))
                .or_else(|| schema.ids.fields.get(&(def, Some(c), name.clone())))
                .copied();
            Field { id, name, ty }
        })
        .collect()
}

/// How a relation is stored: each constructor's id and fields, and each index's id, whether it is
/// unique, its constructor ids and its fields.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    ctors: Vec<(u64, Vec<Field>)>,
    indexes: Vec<(u64, bool, Vec<u64>, Vec<String>)>,
}

fn shapes(schema: &Schema) -> BTreeMap<u64, Shape> {
    let mut out: BTreeMap<u64, Shape> = BTreeMap::new();
    for (r, c) in ctor_relations(schema) {
        let rid = schema.ids.relations[&r];
        let shape = out.entry(rid).or_insert(Shape {
            ctors: Vec::new(),
            indexes: Vec::new(),
        });
        shape
            .ctors
            .push((ctor_id(schema, r, c), fields(schema, r, c)));
    }
    for spec in indexes(schema).into_values() {
        let rid = schema.ids.relations[&spec.relation];
        let ctors = spec
            .ctors
            .iter()
            .map(|&c| ctor_id(schema, spec.relation, c))
            .collect();
        if let Some(shape) = out.get_mut(&rid) {
            shape
                .indexes
                .push((spec.id, spec.unique, ctors, spec.fields.clone()));
        }
    }
    out
}

/// Returns the ids of the relations that `old` and `new` store otherwise: those only one of them
/// has, and those whose constructors, fields or indexes differ.
pub fn rewritten(old: &Schema, new: &Schema) -> BTreeSet<u64> {
    let (a, b) = (shapes(old), shapes(new));
    a.keys()
        .chain(b.keys())
        .filter(|id| a.get(id) != b.get(id))
        .copied()
        .collect()
}

/// Returns the relation of `schema` whose catalog id is `id`.
pub fn relation(schema: &Schema, id: u64) -> Option<RelId> {
    schema
        .ids
        .relations
        .iter()
        .find(|&(_, &x)| x == id)
        .map(|(&r, _)| r)
}

/// Returns the constructor index of the constructor with catalog id `id` of relation `r`.
pub fn ctor_index(schema: &Schema, r: RelId, id: u64) -> Option<u32> {
    let def = schema.relation(r).elem_def;
    schema
        .ids
        .ctors
        .iter()
        .find(|&(&(d, _), &x)| d == def && x == id)
        .map(|(&(_, k), _)| k)
}

/// What one operator's data map gives beyond the fields and constructors it keeps.
#[derive(Default)]
pub struct Moved {
    /// The value of each field a kept constructor gains, from the row before the operator, by
    /// relation id, constructor id and field id.
    pub fills: BTreeMap<(u64, u64, u64), Box<Fill<'static>>>,
    /// The rows of the constructors the operator creates, by relation id and constructor id.
    pub routed: BTreeMap<(u64, u64), Vec<Vec<Value>>>,
    /// The constructors, by relation id and constructor id, whose rows `routed` holds.
    pub consumed: BTreeSet<(u64, u64)>,
}

/// The value of a field for a row, from the fields of the row before the operator.
pub type Fill<'a> = dyn Fn(&[Field], &[Value]) -> Result<Value, String> + 'a;

/// Map `data` from schema `a` to schema `b`, the schemas before and after one operator.
///
/// A row of a constructor both schemas have takes each field `moved.fills` gives from it, and
/// keeps each other field both have, by field id. A constructor only `b` has holds the rows `moved.routed`
/// gives it. Fails naming each constructor relation that `b` lacks, `moved` does not consume and
/// that holds rows, each relation `b` lacks that holds rows, and each field a constructor with
/// rows gains and `moved` does not fill.
pub fn step(a: &Schema, b: &Schema, data: &mut Data, mut moved: Moved) -> Result<(), Vec<String>> {
    let mut why = Vec::new();
    let mut out = Data::new();
    for (&rid, ctors) in data.iter() {
        let ra = relation(a, rid).expect("data holds relations of the schema before");
        let name = &a.relation(ra).name;
        let Some(rb) = relation(b, rid) else {
            if ctors.values().any(|rows| !rows.is_empty()) {
                why.push(format!("relation {name} is not empty"));
            }
            continue;
        };
        let mut next: BTreeMap<u64, Vec<Vec<Value>>> = BTreeMap::new();
        for cb in 0..b.ty(b.relation(rb).elem_def).ctors().len() as u32 {
            let cid = ctor_id(b, rb, cb);
            let to = fields(b, rb, cb);
            let rows = match ctor_index(a, ra, cid) {
                Some(ca) => {
                    let from = fields(a, ra, ca);
                    let mut rows = Vec::new();
                    for row in &ctors[&cid] {
                        match carry(&from, &to, row, |fid| moved.fills.get(&(rid, cid, fid))) {
                            Ok(r) => rows.push(r),
                            Err(e) => {
                                why.push(format!("{name}: {e}"));
                                break;
                            }
                        }
                    }
                    rows
                }
                None => moved.routed.remove(&(rid, cid)).unwrap_or_default(),
            };
            next.insert(cid, rows);
        }
        for (cid, rows) in ctors {
            if next.contains_key(cid) || moved.consumed.contains(&(rid, *cid)) || rows.is_empty() {
                continue;
            }
            let c = ctor_index(a, ra, *cid).expect("a constructor of the schema before");
            let ctor = &a.ty(a.relation(ra).elem_def).ctors()[c as usize].name;
            why.push(format!("constructor relation {name}.{ctor} is not empty"));
        }
        out.insert(rid, next);
    }
    for r in b.ids.relations.keys() {
        let rid = b.ids.relations[r];
        if relation(a, rid).is_none() {
            let ctors = (0..b.ty(b.relation(*r).elem_def).ctors().len() as u32)
                .map(|c| {
                    let cid = ctor_id(b, *r, c);
                    (cid, moved.routed.remove(&(rid, cid)).unwrap_or_default())
                })
                .collect();
            out.insert(rid, ctors);
        }
    }
    if !why.is_empty() {
        return Err(why);
    }
    *data = out;
    Ok(())
}

/// Returns `row`, with fields `from`, as a row with fields `to`: each field the fill `fill` gives
/// for its id from it, each other field `from` has by id converted.
fn carry<'f>(
    from: &[Field],
    to: &[Field],
    row: &[Value],
    fill: impl Fn(u64) -> Option<&'f Box<Fill<'static>>>,
) -> Result<Vec<Value>, String> {
    to.iter()
        .map(|t| {
            let kept =
                t.id.and_then(|id| from.iter().position(|f| f.id == Some(id)))
                    .or_else(|| {
                        t.id.is_none()
                            .then(|| from.iter().position(|f| f.name == t.name))
                            .flatten()
                    });
            match (t.id.and_then(&fill), kept) {
                (Some(f), _) => f(from, row),
                (None, Some(at)) => convert(&from[at].ty, &t.ty, &row[at]),
                (None, None) => Err(format!("field `{}` has no value for existing rows", t.name)),
            }
        })
        .collect()
}

/// Returns `v`, stored as `from`, as a field of type `to` stores it: a constructor of a sum by
/// its id.
pub fn convert(from: &FieldType, to: &FieldType, v: &Value) -> Result<Value, String> {
    if from == to {
        return Ok(v.clone());
    }
    let refuse = || format!("a value stored as {from:?} does not convert to {to:?}");
    match (from, to, v) {
        (FieldType::Record(fs), FieldType::Record(ts), Value::Record(vs))
            if fs.len() == ts.len() =>
        {
            Ok(Value::Record(
                fs.iter()
                    .zip(ts)
                    .zip(vs)
                    .map(|((f, t), v)| convert(f, t, v))
                    .collect::<Result<_, _>>()?,
            ))
        }
        (FieldType::Sum(fs), FieldType::Sum(ts), Value::Sum { ctor, fields }) => {
            let f = fs.get(*ctor as usize).ok_or_else(refuse)?;
            let Some(k) = ts.iter().position(|t| t.id == f.id) else {
                return Err(format!(
                    "a stored value holds constructor {} of a sum that no longer has it",
                    f.id.0
                ));
            };
            let t = &ts[k];
            if t.fields.len() != f.fields.len() {
                return Err(refuse());
            }
            Ok(Value::Sum {
                ctor: k as u32,
                fields: f
                    .fields
                    .iter()
                    .zip(&t.fields)
                    .zip(fields)
                    .map(|((f, t), v)| convert(f, t, v))
                    .collect::<Result<_, _>>()?,
            })
        }
        _ => Err(refuse()),
    }
}
