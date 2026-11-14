use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::schema::Schema;
use adtb_lang::textual::{key_field_ty, key_names};
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::heap::FieldType;
use crate::key::Value;

use super::ids::{
    CatCtorId, CatFieldId, CatRelId, CatTypeId, ctor_id, ctor_index, field_id, rel_id, relation,
    stored, type_id,
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
    key_names(schema, rel)
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
            for n in key_names(schema, *rel) {
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
    /// The route of each value of a constructor the operator replaces, of a type held as a
    /// value, by type id and constructor id.
    pub routes: BTreeMap<(CatTypeId, CatCtorId), Box<Route>>,
    /// The constructor after the operator that a tag of each constructor it replaces names, by
    /// type id and constructor id; `None` when the values of the constructor go to more than one.
    pub tags: BTreeMap<(CatTypeId, CatCtorId), Option<CatCtorId>>,
}

/// The value of a field for a row, from the fields of the row before the operator.
pub type Fill<'a> = dyn Fn(&[Field], &[Value]) -> Result<Value, Failed> + 'a;

/// Returns the value after an operator of a held value of a constructor it replaces, from the
/// value's fields, given the schemas before and after the operator and the arguments of the
/// value's type in each.
pub type Route = dyn Fn(&Schema, &Schema, &[Ty], &[Ty]) -> Result<Box<Fill<'static>>, Failed>;

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
    let types = Types {
        a,
        b,
        values: &moved.values,
        routes: &moved.routes,
        tags: &moved.tags,
    };
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
            let rows = match ctor_index(a, ra, cid) {
                Some(ca) => {
                    let fill = |fid| moved.fills.get(&(rid, cid, fid));
                    match carried(&types, (ra, ca), (rb, cb), &ctors[&cid], fill) {
                        Ok(rows) => rows,
                        Err(Failed::Refused(e)) => {
                            why.push(format!("{name}: {e}"));
                            Vec::new()
                        }
                        Err(Failed::Internal(e)) => return Err(Error::Internal(e)),
                        Err(Failed::Unbound(e)) => return Err(e),
                    }
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
    for (r, rid) in b.ids().relation_ids() {
        if relation(a, rid).is_none() {
            let ctors = (0..b.ty(b.relation(r).elem_def).ctors().len() as u32)
                .map(|c| {
                    let cid = ctor_id(b, r, c);
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

/// The schemas `a` and `b` before and after one operator, and what it gives the values of types
/// held as values.
struct Types<'m> {
    a: &'m Schema,
    b: &'m Schema,
    values: &'m BTreeMap<(CatTypeId, CatCtorId, CatFieldId), Box<Fill<'static>>>,
    routes: &'m BTreeMap<(CatTypeId, CatCtorId), Box<Route>>,
    tags: &'m BTreeMap<(CatTypeId, CatCtorId), Option<CatCtorId>>,
}

/// How a stored value of a declared type of the schema before an operator becomes a value of a
/// declared type of the schema after it.
enum Plan<'m> {
    Copy,
    Convert(FieldType, FieldType),
    Refused(String),
    /// A value of a declared type, by its constructor before: the constructor after and its
    /// fields, or the route of a constructor the operator replaces.
    Held(Vec<Image<'m>>),
    /// A key of several fields, each field after.
    Key(Vec<Part<'m>>),
    /// A tag, by its constructor before: the constructor after.
    Tag(Vec<Result<u32, String>>),
}

enum Image<'m> {
    Kept(u32, Vec<Part<'m>>),
    Routed(Box<Fill<'static>>),
    Gone(String),
}

/// A field after an operator: its fill, or the field before at a position and its plan.
enum Part<'m> {
    Filled(&'m Fill<'static>),
    At(usize, Plan<'m>),
    Missing(String),
}

/// Returns `rows`, rows of constructor `from.1` of relation `from.0` of `types.a`, as rows of
/// constructor `to.1` of relation `to.0` of `types.b`: each field the fill `fill` gives for its id
/// from the row, each other field the row has by id converted.
fn carried<'m>(
    types: &Types<'m>,
    from: (RelId, u32),
    to: (RelId, u32),
    rows: &[Vec<Value>],
    fill: impl Fn(CatFieldId) -> Option<&'m Box<Fill<'static>>>,
) -> Result<Vec<Vec<Value>>, Failed> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let (a, b) = (types.a, types.b);
    let fields_a = fields(a, from.0, from.1);
    let tys_a = layout::row(a, from.0, from.1);
    let parts = fields(b, to.0, to.1)
        .into_iter()
        .zip(layout::row(b, to.0, to.1))
        .map(|(t, (_, tb))| {
            let id = t.id.ok_or_else(|| Broken::NoFieldId(t.name.clone()))?;
            if let Some(f) = fill(id) {
                return Ok(Part::Filled(f.as_ref()));
            }
            Ok(match fields_a.iter().position(|f| f.id == Some(id)) {
                Some(at) => Part::At(at, plan(types, &tys_a[at].1, &tb)?),
                None => Part::Missing(format!("field `{}` has no value for existing rows", t.name)),
            })
        })
        .collect::<Result<Vec<_>, Failed>>()?;
    rows.iter()
        .map(|row| apply_parts(&parts, &fields_a, row))
        .collect()
}

fn apply_parts(parts: &[Part<'_>], from: &[Field], vals: &[Value]) -> Result<Vec<Value>, Failed> {
    parts
        .iter()
        .map(|p| match p {
            Part::Filled(f) => f(from, vals),
            Part::At(at, plan) => apply(plan, &vals[*at]),
            Part::Missing(why) => Err(Failed::Refused(why.clone())),
        })
        .collect()
}

fn apply(plan: &Plan<'_>, v: &Value) -> Result<Value, Failed> {
    let unshaped = || Failed::from(Broken::Unshaped(v.clone()));
    match plan {
        Plan::Copy => Ok(v.clone()),
        Plan::Convert(fa, fb) => convert(fa, fb, v).map_err(Failed::Refused),
        Plan::Refused(why) => Err(Failed::Refused(why.clone())),
        Plan::Held(images) => {
            let (ca, vs) = match v {
                Value::Record(vs) => (0, vs),
                Value::Sum { ctor, fields } => (*ctor, fields),
                _ => return Err(unshaped()),
            };
            match images.get(ca as usize).ok_or_else(unshaped)? {
                Image::Kept(cb, parts) => {
                    let fields = apply_parts(parts, &[], vs)?;
                    Ok(match v {
                        Value::Record(_) => Value::Record(fields),
                        _ => Value::Sum { ctor: *cb, fields },
                    })
                }
                Image::Routed(route) => route(&[], vs),
                Image::Gone(why) => Err(Failed::Refused(why.clone())),
            }
        }
        Plan::Key(parts) => match v {
            Value::Record(vs) => apply_parts(parts, &[], vs).map(Value::Record),
            _ => Err(unshaped()),
        },
        Plan::Tag(images) => match v {
            Value::Sum { ctor, .. } => match images.get(*ctor as usize).ok_or_else(unshaped)? {
                Ok(k) => Ok(Value::Sum {
                    ctor: *k,
                    fields: Vec::new(),
                }),
                Err(why) => Err(Failed::Refused(why.clone())),
            },
            _ => Err(unshaped()),
        },
    }
}

/// Returns how a value of declared type `ta` of `types.a` becomes a value of declared type `tb`
/// of `types.b`: the fields of a declared type and of a key matched by catalog id.
fn plan<'m>(types: &Types<'m>, ta: &Ty, tb: &Ty) -> Result<Plan<'m>, Failed> {
    let (a, b) = (types.a, types.b);
    let matched = match (layout::repr(a, ta), layout::repr(b, tb)) {
        (Ty::Base(x), Ty::Base(y)) if x == y => Some(Plan::Copy),
        (ra @ Ty::Data { .. }, rb @ Ty::Data { .. }) => held(types, &ra, &rb)?,
        (Ty::Key { rel: x, .. }, Ty::Key { rel: y, .. }) => keyed(types, x, y)?,
        (Ty::Tag(x), Ty::Tag(y)) => tagged(types, &x, &y),
        _ => None,
    };
    Ok(matched
        .unwrap_or_else(|| Plan::Convert(layout::field_type(a, ta), layout::field_type(b, tb))))
}

/// Returns how a stored value of `Key ra` of `types.a` becomes one of `Key rb` of `types.b`,
/// each key field matched by catalog id, refused when it holds no key field of `rb`. Returns
/// `None` when `ra` and `rb` are not one relation or one has a single key field and the other
/// several.
fn keyed<'m>(types: &Types<'m>, ra: RelId, rb: RelId) -> Result<Option<Plan<'m>>, Failed> {
    let (a, b) = (types.a, types.b);
    let id = |s: &Schema, r: RelId| s.ahead_relation(r).is_none().then(|| rel_id(s, r));
    if id(a, ra).is_none() || id(a, ra) != id(b, rb) {
        return Ok(None);
    }
    let key = |s: &Schema, r: RelId| -> Option<Vec<(CatFieldId, Ty)>> {
        key_ids(s, r)
            .into_iter()
            .zip(key_names(s, r))
            .map(|(id, n)| Some((id?, key_field_ty(s, r, &n))))
            .collect()
    };
    let (Some(ka), Some(kb)) = (key(a, ra), key(b, rb)) else {
        return Ok(None);
    };
    let lacks = || "a key field has no value in stored keys".to_owned();
    Ok(match (ka.as_slice(), kb.as_slice()) {
        ([(i, _)], [(j, _)]) if i != j => Some(Plan::Refused(lacks())),
        ([(_, x)], [(_, y)]) => Some(plan(types, x, y)?),
        ([_, _, ..], [_, _, ..]) => Some(Plan::Key(
            kb.iter()
                .map(|(id, t)| {
                    Ok(match ka.iter().position(|(i, _)| i == id) {
                        Some(at) => Part::At(at, plan(types, &ka[at].1, t)?),
                        None => Part::Missing(lacks()),
                    })
                })
                .collect::<Result<_, Failed>>()?,
        )),
        _ => None,
    })
}

/// A declared type and its arguments.
type Instance<'t> = (TypeId, &'t [Ty]);

/// Returns the catalog id of the declared type `ta` of `a`, with the definition and arguments of
/// `ta` and of `tb` of `b`, when both are that declared type.
fn same_type<'t>(
    a: &Schema,
    ta: &'t Ty,
    b: &Schema,
    tb: &'t Ty,
) -> Option<(CatTypeId, Instance<'t>, Instance<'t>)> {
    let (
        Ty::Data {
            def: da, args: aa, ..
        },
        Ty::Data {
            def: db, args: ab, ..
        },
    ) = (ta, tb)
    else {
        return None;
    };
    let tid = type_id(a, *da)?;
    (type_id(b, *db) == Some(tid)).then_some((tid, (*da, aa.as_slice()), (*db, ab.as_slice())))
}

/// Returns how a value of type `ra` of `types.a` becomes one of the same declared type `rb` of
/// `types.b`: each field its constructor keeps by id converted, each field it gains filled by
/// `types.values`, and a value of a constructor it replaces routed by `types.routes`. Returns
/// `None` when `ra` and `rb` are not one declared type.
fn held<'m>(types: &Types<'m>, ra: &Ty, rb: &Ty) -> Result<Option<Plan<'m>>, Failed> {
    let (a, b) = (types.a, types.b);
    let Some((tid, (da, aa), (db, ab))) = same_type(a, ra, b, rb) else {
        return Ok(None);
    };
    let ctors_b = layout::ctors(b, rb);
    let images = layout::ctors(a, ra)
        .into_iter()
        .enumerate()
        .map(|(ca, (cid, fields_a))| {
            let id = CatCtorId::from_raw(u64::from(cid));
            let Some(cb) = ctors_b.iter().position(|(c, _)| *c == cid) else {
                return Ok(match types.routes.get(&(tid, id)) {
                    Some(route) => match route(a, b, aa, ab) {
                        Ok(f) => Image::Routed(f),
                        Err(Failed::Refused(why)) => Image::Gone(why),
                        Err(e) => return Err(e),
                    },
                    None => Image::Gone(format!(
                        "a stored value holds constructor {cid} of a sum that no longer has it"
                    )),
                });
            };
            let parts = ctors_b[cb]
                .1
                .iter()
                .map(|(name, t)| {
                    let fid = field_id(b, db, cb as u32, name);
                    if let Some(f) = fid.and_then(|f| types.values.get(&(tid, id, f))) {
                        return Ok(Part::Filled(f.as_ref()));
                    }
                    let at = fields_a
                        .iter()
                        .position(|(n, _)| fid.is_some() && field_id(a, da, ca as u32, n) == fid);
                    Ok(match at {
                        Some(at) => Part::At(at, plan(types, &fields_a[at].1, t)?),
                        None => {
                            Part::Missing(format!("field `{name}` has no value in stored values"))
                        }
                    })
                })
                .collect::<Result<_, Failed>>()?;
            Ok(Image::Kept(cb as u32, parts))
        })
        .collect::<Result<_, Failed>>()?;
    Ok(Some(Plan::Held(images)))
}

/// Returns how a value of `Tag ta` of `types.a` becomes one of `Tag tb` of `types.b`: a tag
/// whose constructor `tb` keeps names it by id, and a tag of a constructor the operator replaces
/// names the constructor `types.tags` gives it; `None` when `ta` and `tb` are not one declared
/// type.
fn tagged<'m>(types: &Types<'m>, ta: &Ty, tb: &Ty) -> Option<Plan<'m>> {
    let (a, b) = (types.a, types.b);
    let (ra, rb) = (layout::repr(a, ta), layout::repr(b, tb));
    let (tid, (da, _), _) = same_type(a, &ra, b, &rb)?;
    let ctors_b = layout::ctors(b, &rb);
    let at = |id: u32| ctors_b.iter().position(|(c, _)| *c == id).map(|k| k as u32);
    let images = layout::ctors(a, &ra)
        .into_iter()
        .enumerate()
        .map(|(k, (cid, _))| {
            if let Some(cb) = at(cid) {
                return Ok(cb);
            }
            let d = a.ty(da);
            let name = format!("{}.{}", d.name, d.ctors()[k].name);
            match types.tags.get(&(tid, CatCtorId::from_raw(u64::from(cid)))) {
                Some(Some(to)) => at(stored(*to).0).ok_or_else(|| {
                    format!("a stored tag names `{name}`, which the operator replaces by no one constructor")
                }),
                Some(None) => Err(format!(
                    "a stored tag names `{name}`, whose values the routing sends to no one product"
                )),
                None => Err(format!(
                    "a stored tag names `{name}`, which the operator replaces by no one constructor"
                )),
            }
        })
        .collect();
    Some(Plan::Tag(images))
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
