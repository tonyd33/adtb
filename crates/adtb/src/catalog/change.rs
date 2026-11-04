use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::schema::Schema;
use adtb_lang::textual::key_field_ty;
use adtb_lang::types::{RelId, Ty};

use crate::heap::FieldType;
use crate::key::Value;

use super::ids::{
    CatCtorId, CatFieldId, CatRelId, CatTypeId, ctor_id, ctor_index, field_id, rel_id, relation,
    type_id,
};
use super::{Broken, Error, ctor_relations, layout, row_type, rows};

/// The rows of the relations a migration rewrites, by relation id and then constructor id, each
/// row in the layout of the schema the operators applied so far produce.
pub type Data = BTreeMap<CatRelId, BTreeMap<CatCtorId, Vec<Vec<Value>>>>;

/// A field of a row: its catalog id and how a heap stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub id: Option<CatFieldId>,
    pub name: String,
    pub ty: FieldType,
}

/// Returns the fields of a row of constructor `c` of relation `r`, the header's first.
pub fn fields(schema: &Schema, r: RelId, c: u32) -> Vec<Field> {
    let def = schema.relation(r).elem_def;
    layout::row(schema, r, c)
        .into_iter()
        .zip(row_type(schema, r, c))
        .map(|((name, _), ty)| Field {
            id: field_id(schema, def, c, &name),
            name,
            ty,
        })
        .collect()
}

/// How a relation is stored: each constructor's id and fields, each index's id, whether it is
/// unique, its constructor ids and its fields, and the key field ids, in stored order, of each
/// relation its fields reach through `Key`.
#[derive(Debug, PartialEq, Eq)]
struct Shape {
    ctors: Vec<(CatCtorId, Vec<Field>)>,
    indexes: Vec<(u64, bool, Vec<CatCtorId>, Vec<String>)>,
    keys: KeyIds,
}

/// Returns the key field ids of `rel`, in the order a value of `Key rel` holds them.
fn key_ids(schema: &Schema, rel: RelId) -> Vec<Option<CatFieldId>> {
    layout::key_names(schema, rel)
        .iter()
        .map(|n| field_id(schema, schema.relation(rel).elem_def, 0, n))
        .collect()
}

type KeyIds = BTreeMap<CatRelId, Vec<Option<CatFieldId>>>;

/// Add to `keys` the key field ids of each relation a value of type `ty` reaches through `Key`.
fn reached_keys(schema: &Schema, ty: &Ty, seen: &mut Vec<Ty>, keys: &mut KeyIds) {
    let t = layout::repr(schema, ty);
    if seen.contains(&t) {
        return;
    }
    seen.push(t.clone());
    match &t {
        Ty::Key { rel, .. } if schema.ahead_relation(*rel).is_none() => {
            keys.insert(rel_id(schema, *rel), key_ids(schema, *rel));
            for n in layout::key_names(schema, *rel) {
                reached_keys(schema, &key_field_ty(schema, *rel, &n), seen, keys);
            }
        }
        Ty::Data { .. } => {
            for (_, f) in layout::ctors(schema, &t).into_iter().flat_map(|(_, fs)| fs) {
                reached_keys(schema, &f, seen, keys);
            }
        }
        _ => {}
    }
}

fn shapes(schema: &Schema) -> BTreeMap<CatRelId, Shape> {
    let mut out: BTreeMap<CatRelId, Shape> = BTreeMap::new();
    for (r, c) in ctor_relations(schema) {
        let rid = rel_id(schema, r);
        let shape = out.entry(rid).or_insert(Shape {
            ctors: Vec::new(),
            indexes: Vec::new(),
            keys: BTreeMap::new(),
        });
        let mut seen = Vec::new();
        for (_, t) in layout::row(schema, r, c) {
            reached_keys(schema, &t, &mut seen, &mut shape.keys);
        }
        shape
            .ctors
            .push((ctor_id(schema, r, c), fields(schema, r, c)));
    }
    for spec in rows::indexes(schema).into_values() {
        let rid = rel_id(schema, spec.relation);
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
/// has, and those whose constructors, fields, indexes or reached keys differ.
pub fn rewritten(old: &Schema, new: &Schema) -> BTreeSet<CatRelId> {
    let (a, b) = (shapes(old), shapes(new));
    a.keys()
        .chain(b.keys())
        .filter(|id| a.get(id) != b.get(id))
        .copied()
        .collect()
}

/// What one operator's data map gives beyond the fields and constructors it keeps.
#[derive(Default)]
pub struct Moved {
    /// The value of each field a kept constructor gains, from the row before the operator, by
    /// relation id, constructor id and field id.
    pub fills: BTreeMap<(CatRelId, CatCtorId, CatFieldId), Box<Fill<'static>>>,
    /// The rows of the constructors the operator creates, by relation id and constructor id.
    pub routed: BTreeMap<(CatRelId, CatCtorId), Vec<Vec<Value>>>,
    /// The constructors, by relation id and constructor id, whose rows `routed` holds.
    pub consumed: BTreeSet<(CatRelId, CatCtorId)>,
    /// The value of each field a constructor of a type held as a value gains, from the fields of
    /// the value before the operator, by type id, constructor id and field id.
    pub values: BTreeMap<(CatTypeId, CatCtorId, CatFieldId), Box<Fill<'static>>>,
    /// The value after the operator of each value of a constructor the operator replaces, of a
    /// type held as a value, from its fields, by type id and constructor id.
    pub routes: BTreeMap<(CatTypeId, CatCtorId), Box<Fill<'static>>>,
}

/// The value of a field for a row, from the fields of the row before the operator.
pub type Fill<'a> = dyn Fn(&[Field], &[Value]) -> Result<Value, Failed> + 'a;

/// Why a data map gives no value.
#[derive(Debug)]
pub enum Failed {
    /// The data refuses the operator; why.
    Refused(String),
    /// A guarantee of the checker or the executor does not hold; the one named.
    Internal(Box<Broken>),
    /// A call reached a host function with no body bound at the version its declaration names.
    Unbound(Error),
}

/// Map `data` from schema `a` to schema `b`, the schemas before and after one operator.
///
/// A row of a constructor both schemas have takes each field `moved.fills` gives from it, and
/// keeps each other field both have, by field id. A constructor only `b` has holds the rows `moved.routed`
/// gives it. Fails naming each constructor relation that `b` lacks, `moved` does not consume and
/// that holds rows, each relation `b` lacks that holds rows, and each field a constructor with
/// rows gains and `moved` does not fill, as `Error::Refused`; or with `Error::Internal`.
pub fn step(a: &Schema, b: &Schema, data: &mut Data, mut moved: Moved) -> Result<(), Error> {
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
        let mut next: BTreeMap<CatCtorId, Vec<Vec<Value>>> = BTreeMap::new();
        for cb in 0..b.ty(b.relation(rb).elem_def).ctors().len() as u32 {
            let cid = ctor_id(b, rb, cb);
            let to = fields(b, rb, cb);
            let rows = match ctor_index(a, ra, cid) {
                Some(ca) => {
                    let from = fields(a, ra, ca);
                    let types = Types {
                        a,
                        b,
                        from: layout::row(a, ra, ca).into_iter().map(|(_, t)| t).collect(),
                        to: layout::row(b, rb, cb).into_iter().map(|(_, t)| t).collect(),
                        values: &moved.values,
                        routes: &moved.routes,
                    };
                    let mut rows = Vec::new();
                    for row in &ctors[&cid] {
                        let fill = |fid| moved.fills.get(&(rid, cid, fid));
                        match carry(&from, &to, &types, row, fill) {
                            Ok(r) => rows.push(r),
                            Err(Failed::Refused(e)) => {
                                why.push(format!("{name}: {e}"));
                                break;
                            }
                            Err(Failed::Internal(e)) => return Err(Error::Internal(e)),
                            Err(Failed::Unbound(e)) => return Err(e),
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
        let rid = rel_id(b, *r);
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
        return Err(Error::Refused(why));
    }
    *data = out;
    Ok(())
}

/// The declared types of the fields of a row before and after one operator, in the schemas
/// `a` and `b`, and the fills of fields that types held as values gain.
struct Types<'s> {
    a: &'s Schema,
    b: &'s Schema,
    from: Vec<Ty>,
    to: Vec<Ty>,
    values: &'s BTreeMap<(CatTypeId, CatCtorId, CatFieldId), Box<Fill<'static>>>,
    routes: &'s BTreeMap<(CatTypeId, CatCtorId), Box<Fill<'static>>>,
}

/// Returns `row`, with fields `from`, as a row with fields `to`: each field the fill `fill` gives
/// for its id from it, each other field `from` has by id converted.
fn carry<'f>(
    from: &[Field],
    to: &[Field],
    types: &Types<'_>,
    row: &[Value],
    fill: impl Fn(CatFieldId) -> Option<&'f Box<Fill<'static>>>,
) -> Result<Vec<Value>, Failed> {
    to.iter()
        .zip(&types.to)
        .map(|(t, tb)| {
            let kept =
                t.id.and_then(|id| from.iter().position(|f| f.id == Some(id)))
                    .or_else(|| {
                        t.id.is_none()
                            .then(|| from.iter().position(|f| f.name == t.name))
                            .flatten()
                    });
            match (t.id.and_then(&fill), kept) {
                (Some(f), _) => f(from, row),
                (None, Some(at)) => stored(types, &types.from[at], tb, &row[at]),
                (None, None) => Err(Failed::Refused(format!(
                    "field `{}` has no value for existing rows",
                    t.name
                ))),
            }
        })
        .collect()
}

/// Returns `v`, a value of declared type `ta` of `types.a`, as a value of declared type `tb` of
/// `types.b`: the fields of a declared type and of a key matched by catalog id.
fn stored(types: &Types<'_>, ta: &Ty, tb: &Ty, v: &Value) -> Result<Value, Failed> {
    let (a, b) = (types.a, types.b);
    let matched = match (layout::repr(a, ta), layout::repr(b, tb)) {
        (Ty::Base(x), Ty::Base(y)) if x == y => return Ok(v.clone()),
        (ra @ Ty::Data { .. }, rb @ Ty::Data { .. }) => held(types, &ra, &rb, v),
        (Ty::Key { rel: x, .. }, Ty::Key { rel: y, .. }) => keyed(types, x, y, v),
        _ => None,
    };
    matched.unwrap_or_else(|| {
        let (fa, fb) = (layout::field_type(a, ta), layout::field_type(b, tb));
        convert(&fa, &fb, v).map_err(Failed::Refused)
    })
}

/// Returns `v`, a stored value of `Key ra` of `types.a`, as `Key rb` of `types.b` stores it,
/// each key field matched by catalog id, refused when `v` holds no key field of `rb`. Returns
/// `None` when `ra` and `rb` are not one relation or one has a single key field and the other
/// several.
fn keyed(types: &Types<'_>, ra: RelId, rb: RelId, v: &Value) -> Option<Result<Value, Failed>> {
    let (a, b) = (types.a, types.b);
    let id = |s: &Schema, r: RelId| s.ahead_relation(r).is_none().then(|| rel_id(s, r));
    if id(a, ra)? != id(b, rb)? {
        return None;
    }
    let key = |s: &Schema, r: RelId| -> Option<Vec<(CatFieldId, Ty)>> {
        key_ids(s, r)
            .into_iter()
            .zip(layout::key_names(s, r))
            .map(|(id, n)| Some((id?, key_field_ty(s, r, &n))))
            .collect()
    };
    let (ka, kb) = (key(a, ra)?, key(b, rb)?);
    let lacks = || Failed::Refused("a key field has no value in stored keys".to_owned());
    match (ka.as_slice(), kb.as_slice(), v) {
        ([(i, _)], [(j, _)], _) if i != j => Some(Err(lacks())),
        ([(_, x)], [(_, y)], _) => Some(stored(types, x, y, v)),
        ([_, _, ..], [_, _, ..], Value::Record(vs)) => Some(
            kb.iter()
                .map(|(id, t)| {
                    let at = ka.iter().position(|(i, _)| i == id).ok_or_else(lacks)?;
                    stored(types, &ka[at].1, t, &vs[at])
                })
                .collect::<Result<_, _>>()
                .map(Value::Record),
        ),
        _ => None,
    }
}

/// Returns `v`, a value of type `ra` of `types.a`, as a value of the same declared type `rb` of
/// `types.b`: each field its constructor keeps by id converted, each field it gains filled by
/// `types.values`, and a value of a constructor it replaces routed by `types.routes`. Returns
/// `None` when `ra` and `rb` are not one declared type.
fn held(types: &Types<'_>, ra: &Ty, rb: &Ty, v: &Value) -> Option<Result<Value, Failed>> {
    let (a, b) = (types.a, types.b);
    let (Ty::Data { def: da, .. }, Ty::Data { def: db, .. }) = (ra, rb) else {
        return None;
    };
    let tid = type_id(a, *da)?;
    if type_id(b, *db) != Some(tid) {
        return None;
    }
    let (ca, vs) = match v {
        Value::Record(vs) => (0, vs),
        Value::Sum { ctor, fields } => (*ctor, fields),
        _ => return None,
    };
    let (ctors_a, ctors_b) = (layout::ctors(a, ra), layout::ctors(b, rb));
    let (cid, fields_a) = &ctors_a[ca as usize];
    let Some(cb) = ctors_b.iter().position(|(id, _)| id == cid) else {
        if let Some(route) = types.routes.get(&(tid, CatCtorId(u64::from(*cid)))) {
            return Some(route(&[], vs));
        }
        return Some(Err(Failed::Refused(format!(
            "a stored value holds constructor {cid} of a sum that no longer has it"
        ))));
    };
    let out = ctors_b[cb]
        .1
        .iter()
        .map(|(name, t)| {
            let id = field_id(b, *db, cb as u32, name);
            let cid = CatCtorId(u64::from(*cid));
            if let Some(f) = id.and_then(|id| types.values.get(&(tid, cid, id))) {
                return f(&[], vs);
            }
            let at = fields_a
                .iter()
                .position(|(n, _)| id.is_some() && field_id(a, *da, ca, n) == id)
                .ok_or_else(|| {
                    Failed::Refused(format!("field `{name}` has no value in stored values"))
                })?;
            stored(types, &fields_a[at].1, t, &vs[at])
        })
        .collect::<Result<Vec<_>, _>>();
    Some(out.map(|fields| match v {
        Value::Record(_) => Value::Record(fields),
        _ => Value::Sum {
            ctor: cb as u32,
            fields,
        },
    }))
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
