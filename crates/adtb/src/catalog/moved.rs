use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use adtb_lang::evolve::{RowFn, RowMap};
use adtb_lang::kernel::{Kind, Terms, Tm, Var};
use adtb_lang::norm::NormalQuery;
use adtb_lang::schema::Schema;
use adtb_lang::textual::key_names;
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::exec::{self, Conv, Scalar, Val, conv, from_stored, to_stored};
use crate::heap::FieldType;
use crate::key::Value;

use super::change::{Data, Failed, Field, Fill, Moved, Route, convert, fields};
use super::hosts::unbound;
use super::ids::{
    CatCtorId, CatRelId, CatTypeId, ctor_id, ctor_index, field_id, rel_id, relation, type_ctor_id,
    type_ctor_index, type_def, type_id,
};
use super::{Broken, Error, layout, oplog};

/// Returns what `maps`, the row maps of one step from schema `a` to schema `b`, give the rows of
/// `data`, which are in `a`'s layout.
///
/// Fails with `Error::Refused` naming each map the data refuses, or with `Error::Internal`.
pub fn moved(
    a: &Schema,
    b: &Schema,
    maps: &[RowMap],
    data: &Data,
    hosts: &Arc<exec::Hosts>,
) -> Result<Moved, Error> {
    let mut m = Mapping {
        a,
        b,
        data,
        hosts,
        out: Moved::default(),
    };
    let mut why = Vec::new();
    for map in maps {
        let got = match map {
            RowMap::Fill {
                def,
                ctors,
                field,
                fill,
            } => m.fills(*def, ctors, field, fill),
            RowMap::Copy { def, ctors, field } => {
                m.copies(*def, ctors, field);
                Ok(())
            }
            RowMap::Route { def, from, routing } => m.route(*def, from, routing),
            RowMap::Merge {
                def,
                from,
                into,
                discriminator,
            } => m.merge(*def, from, into, discriminator),
            RowMap::Nest { def, from, into } => m.nest(*def, from, into),
            RowMap::Unnest { def, from } => m.unnest(*def, *from),
            RowMap::Rekey { relation } => m.rekey(*relation),
        };
        match got {
            Ok(()) => {}
            Err(Failed::Refused(e)) => why.push(e),
            Err(Failed::Internal(e)) => return Err(Error::Internal(e)),
            Err(Failed::Unbound(e)) => return Err(e),
        }
    }
    if why.is_empty() {
        Ok(m.out)
    } else {
        Err(Error::Refused(why))
    }
}

/// One step's data maps from schema `a` to schema `b` over `data`, in `a`'s layout, and what
/// they have given so far.
struct Mapping<'s> {
    a: &'s Schema,
    b: &'s Schema,
    data: &'s Data,
    hosts: &'s Arc<exec::Hosts>,
    out: Moved,
}

/// Each relation of `a` whose element type is `def`, with its catalog id and the same relation
/// of `b`.
fn relations_of(a: &Schema, b: &Schema, def: TypeId) -> Vec<(RelId, CatRelId, RelId)> {
    a.ids
        .relation_ids()
        .filter(|&(r, _)| a.relation(r).elem_def == def)
        .filter_map(|(r, _)| {
            let rid = rel_id(a, r);
            relation(b, rid).map(|rb| (r, rid, rb))
        })
        .collect()
}

/// The conversions of the fields of a row of constructor `c` of relation `r`.
fn row_convs(schema: &Schema, r: RelId, c: u32) -> Vec<Conv> {
    layout::row(schema, r, c)
        .iter()
        .map(|(_, t)| conv(schema, t))
        .collect()
}

fn row_val(c: u32, convs: &[Conv], row: &[Value]) -> Val {
    Val::ctor(
        c,
        convs
            .iter()
            .zip(row)
            .map(|(k, v)| from_stored(k, v))
            .collect(),
    )
}

/// Returns the constructor of relation `r` named `name`.
fn ctor_named(schema: &Schema, r: RelId, name: &str) -> Option<u32> {
    schema.ty(schema.relation(r).elem_def).ctor_index(name)
}

fn ctor_name(schema: &Schema, def: TypeId, c: u32) -> &str {
    &schema.ty(def).ctors()[c as usize].name
}

fn field_named<'f>(fs: &'f [Field], name: &str) -> Result<&'f Field, Failed> {
    fs.iter()
        .find(|f| f.name == name)
        .ok_or_else(|| Failed::from(Broken::NoField(name.to_owned())))
}

fn compiled(f: &RowFn, hosts: &Arc<exec::Hosts>) -> Result<Scalar, Failed> {
    let [var] = f.query.params.as_slice() else {
        return Err(Failed::from(Broken::RowParams(f.query.params.len())));
    };
    let mut scalar = Scalar::new(&f.schema, &f.query.terms, *var, f.query.body).map_err(shown)?;
    scalar.set_hosts(hosts.clone());
    Ok(scalar)
}

fn shown(e: exec::Error) -> Failed {
    if let Some(e) = unbound(&e) {
        return Failed::Unbound(e);
    }
    match e {
        exec::Error::Fault(f) => Failed::Refused(format!("the map faults with {}", f.name())),
        exec::Error::Rejected(why) => Failed::Refused(format!("the map rejects: {why}")),
        other => Failed::from(Broken::Map(other)),
    }
}

/// A constructor of a type in a routing's schema: its name, its position and fields in the schema
/// after the operator, and the names and conversions of its fields.
struct Routed {
    name: String,
    product: Option<(u32, Vec<Field>)>,
    names: Vec<String>,
    convs: Vec<Conv>,
}

fn data(def: TypeId, args: &[Ty]) -> Ty {
    Ty::Data {
        def,
        args: args.to_vec(),
        ctors: None,
    }
}

/// Returns the fields of constructor `c` of type `def` at arguments `args`, the header's first.
fn instance(schema: &Schema, def: TypeId, args: &[Ty], c: u32) -> Vec<(String, Ty)> {
    layout::ctors(schema, &data(def, args))
        .swap_remove(c as usize)
        .1
}

/// Returns the catalog id of constructor `name` of type `def` of `b`.
fn b_ctor(b: &Schema, def: TypeId, name: &str) -> Option<CatCtorId> {
    b.ty(def).ctor_index(name).map(|k| type_ctor_id(b, def, k))
}

/// Returns the one constructor the routing `query` gives every row of constructor `c`, or `None`
/// when an end of its body gives another or is no constructor.
fn sole_product(query: &NormalQuery, c: u32) -> Option<u32> {
    let mut out = BTreeSet::new();
    let [row] = query.params.as_slice() else {
        return None;
    };
    ends(&query.terms, query.body, *row, c, &mut out).then_some(())?;
    match out.into_iter().collect::<Vec<_>>().as_slice() {
        [p] => Some(*p),
        _ => None,
    }
}

/// Add to `out` the constructor each end of `t` gives for a row `row` of constructor `c`;
/// returns whether every end gives one.
fn ends(terms: &Terms, t: Tm, row: Var, c: u32, out: &mut BTreeSet<u32>) -> bool {
    match terms.kind(t) {
        Kind::Unreachable => true,
        Kind::Let(_, _, w) => ends(terms, *w, row, c, out),
        Kind::If(_, x, y) => ends(terms, *x, row, c, out) && ends(terms, *y, row, c, out),
        Kind::Case { scrutinee, arms } => {
            let on_row = matches!(terms.kind(*scrutinee), Kind::Var(v) if *v == row);
            arms.iter()
                .filter(|(k, _, _)| !on_row || *k == c)
                .all(|(_, _, b)| ends(terms, *b, row, c, out))
        }
        Kind::Ctor { ctor, .. } => {
            out.insert(*ctor);
            true
        }
        _ => false,
    }
}

/// Refuse arguments `args` of type `def` that reach `def`.
fn unreached(schema: &Schema, def: TypeId, args: &[Ty]) -> Result<(), Failed> {
    let (mut defs, mut rels) = (BTreeSet::new(), BTreeSet::new());
    for t in args {
        schema.reached(t, &mut defs, &mut rels);
    }
    if defs.contains(&def) {
        let name = &schema.ty(def).name;
        return Err(Failed::Refused(format!(
            "a stored value of `{name}` holds a value of `{name}` in a type argument"
        )));
    }
    Ok(())
}

fn shaped(record: bool, ctor: u32, fields: Vec<Value>) -> Value {
    if record {
        Value::Record(fields)
    } else {
        Value::Sum { ctor, fields }
    }
}

/// A field of a held value after an operator: a value, a field of the value before converted,
/// or a field of the value it held.
enum Lift {
    Value(Value),
    At(usize, FieldType, FieldType),
    Inner(usize, FieldType),
}

impl Lift {
    fn value(&self, vals: &[Value], inner: &[Val]) -> Result<Value, Failed> {
        match self {
            Lift::Value(v) => Ok(v.clone()),
            Lift::At(at, from, to) => convert(from, to, &vals[*at]).map_err(Failed::Refused),
            Lift::Inner(at, to) => to_stored(to, &inner[*at]).map_err(shown),
        }
    }
}

/// Returns field `name` of type `t` of `b` as lifted from the field of the same name of `from`,
/// fields of `a`.
fn lift(a: &Schema, from: &[(String, Ty)], b: &Schema, name: &str, t: &Ty) -> Result<Lift, Failed> {
    let at = from
        .iter()
        .position(|(n, _)| n == name)
        .ok_or_else(|| Failed::from(Broken::NoCounterpart(name.to_owned())))?;
    Ok(Lift::At(
        at,
        layout::field_type(a, &from[at].1),
        layout::field_type(b, t),
    ))
}

/// Returns `vals`, the fields of a value named `names`, as a row of fields `to`.
fn by_name(names: &[String], vals: &[Val], to: &[Field]) -> Result<Vec<Value>, Failed> {
    to.iter()
        .map(|t| {
            let at = names
                .iter()
                .position(|n| *n == t.name)
                .ok_or_else(|| Failed::from(Broken::Unrouted(t.name.clone())))?;
            to_stored(&t.ty, &vals[at]).map_err(shown)
        })
        .collect()
}

/// Returns the field of `row`, with fields `from`, named as `to`, stored as `to` stores it.
fn carried(from: &[Field], row: &[Value], to: &Field) -> Result<Value, Failed> {
    let at = from
        .iter()
        .position(|f| f.name == to.name)
        .ok_or_else(|| Failed::from(Broken::NoCounterpart(to.name.clone())))?;
    convert(&from[at].ty, &to.ty, &row[at]).map_err(Failed::Refused)
}

/// The declared type of the last field of constructor `c` of relation `r`.
fn last_field_def(schema: &Schema, r: RelId, c: u32) -> Result<TypeId, Failed> {
    match layout::row(schema, r, c).last() {
        Some((_, Ty::Data { def, .. })) => Ok(*def),
        _ => Err(Failed::from(Broken::LastNotSum)),
    }
}

/// Where the key of a row of one constructor lies: the position of each key field in the row,
/// in the order a value of `Key` of the relation holds them, with how the row and the key store
/// it.
struct KeyAt(Vec<(usize, FieldType, FieldType)>);

impl KeyAt {
    /// Returns where the key of a row of constructor `c` of relation `r` of `a` lies, as a field
    /// typed `Key` of the relation stores it under the key of `keyed` in schema `k`, `r` there.
    fn new(a: &Schema, r: RelId, c: u32, k: &Schema, keyed: RelId) -> Result<KeyAt, Failed> {
        let from = fields(a, r, c);
        let kc = ctor_index(k, keyed, ctor_id(a, r, c)).ok_or_else(|| {
            Failed::from(Broken::NoCtor(
                ctor_name(a, a.relation(r).elem_def, c).to_owned(),
            ))
        })?;
        let to = fields(k, keyed, kc);
        let at = key_names(k, keyed)
            .iter()
            .map(|n| {
                let t = field_named(&to, n)?;
                let at = from
                    .iter()
                    .position(|f| f.id.is_some() && f.id == t.id)
                    .ok_or_else(|| Failed::from(Broken::NoCounterpart(n.clone())))?;
                Ok((at, from[at].ty.clone(), t.ty.clone()))
            })
            .collect::<Result<_, Failed>>()?;
        Ok(KeyAt(at))
    }

    fn value(&self, row: &[Value]) -> Result<Value, Failed> {
        let mut vs = self
            .0
            .iter()
            .map(|(at, from, to)| convert(from, to, &row[*at]).map_err(Failed::Refused))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(match vs.len() {
            1 => vs.remove(0),
            _ => Value::Record(vs),
        })
    }
}

/// Where a stored value of a type holds values of `Key r` for one relation `r`.
enum Refs {
    Key,
    /// A value of a sum or record: each field of each constructor that holds one.
    Fields(Vec<Vec<Option<Refs>>>),
}

impl Refs {
    /// Returns where a value of type `ty` of `schema` holds values of `Key rel`, directly or
    /// through the key of a relation it references, or `None` when it holds none.
    fn of(schema: &Schema, ty: &Ty, rel: RelId) -> Option<Refs> {
        Refs::within(schema, ty, rel, &mut BTreeSet::new())
    }

    /// Returns `of`, where `seen` holds the relations whose keys the path to `ty` passes through.
    fn within(schema: &Schema, ty: &Ty, rel: RelId, seen: &mut BTreeSet<RelId>) -> Option<Refs> {
        let ctors: Vec<Vec<Ty>> = match layout::repr(schema, ty) {
            Ty::Key { rel: r, .. } if r == rel => return Some(Refs::Key),
            Ty::Key { rel: s, .. } => {
                if !seen.insert(s) {
                    return None;
                }
                let refs = Refs::within(schema, &layout::key_value_ty(schema, s), rel, seen);
                seen.remove(&s);
                return refs;
            }
            Ty::Record(fs) => vec![fs.into_iter().map(|(_, t)| t).collect()],
            t @ (Ty::Data { .. } | Ty::Sum { .. }) => layout::ctors(schema, &t)
                .into_iter()
                .map(|(_, fs)| fs.into_iter().map(|(_, t)| t).collect())
                .collect(),
            _ => return None,
        };
        let refs: Vec<Vec<Option<Refs>>> = ctors
            .iter()
            .map(|fs| {
                fs.iter()
                    .map(|t| Refs::within(schema, t, rel, seen))
                    .collect()
            })
            .collect();
        refs.iter()
            .flatten()
            .any(Option::is_some)
            .then_some(Refs::Fields(refs))
    }

    /// Returns `v` with each value of `Key` it holds where `self` places one replaced by `key`
    /// of it.
    fn rekeyed(
        &self,
        v: &Value,
        key: &dyn Fn(&Value) -> Result<Value, Failed>,
    ) -> Result<Value, Failed> {
        let Refs::Fields(ctors) = self else {
            return key(v);
        };
        let each = |refs: &[Option<Refs>], vs: &[Value]| -> Result<Vec<Value>, Failed> {
            refs.iter()
                .zip(vs)
                .map(|(r, v)| match r {
                    Some(r) => r.rekeyed(v, key),
                    None => Ok(v.clone()),
                })
                .collect()
        };
        let shape = || Failed::from(Broken::Unshaped(v.clone()));
        match v {
            Value::Record(vs) => Ok(Value::Record(each(ctors.first().ok_or_else(shape)?, vs)?)),
            Value::Sum { ctor, fields } => Ok(Value::Sum {
                ctor: *ctor,
                fields: each(ctors.get(*ctor as usize).ok_or_else(shape)?, fields)?,
            }),
            _ => Err(shape()),
        }
    }
}

impl Mapping<'_> {
    fn fills(
        &mut self,
        def: TypeId,
        ctors: &[u32],
        field: &str,
        fill: &RowFn,
    ) -> Result<(), Failed> {
        let (a, b) = (self.a, self.b);
        let f = Arc::new(compiled(fill, self.hosts)?);
        for (ra, rid, rb) in relations_of(a, b, def) {
            for &c in ctors {
                let Some(cb) = ctor_index(b, rb, ctor_id(a, ra, c)) else {
                    continue;
                };
                let to = fields(b, rb, cb);
                let Some(id) = field_named(&to, field)?.id else {
                    return Err(Failed::from(Broken::NoFieldId(field.to_owned())));
                };
                let ty = field_named(&to, field)?.ty.clone();
                let convs = row_convs(&fill.schema, ra, c);
                let f = f.clone();
                let fill: Box<Fill<'static>> = Box::new(move |_, row| {
                    let v = f.eval(row_val(c, &convs, row)).map_err(shown)?;
                    to_stored(&ty, &v).map_err(shown)
                });
                self.out.fills.insert((rid, ctor_id(b, rb, cb), id), fill);
            }
        }
        self.held_fills(def, ctors, field, fill, &f)
    }

    /// Adds to `out.values` the fill `f`, compiled from `fill`, of field `field` that
    /// constructors `ctors` of type `def` gain, for the values of `def` other rows hold.
    fn held_fills(
        &mut self,
        def: TypeId,
        ctors: &[u32],
        field: &str,
        fill: &RowFn,
        f: &Arc<Scalar>,
    ) -> Result<(), Failed> {
        let (a, b) = (self.a, self.b);
        let Some(tid) = type_id(a, def) else {
            return Ok(());
        };
        let Some(db) = type_def(b, tid) else {
            return Ok(());
        };
        if !b.ty(db).params.is_empty() {
            return Ok(());
        }
        let after = layout::ctors(b, &data(db, &[]));
        let before = layout::ctor_fields(&fill.schema, &data(def, &[]));
        for &c in ctors {
            let Some(cb) = type_ctor_index(b, db, type_ctor_id(a, def, c)) else {
                continue;
            };
            let (cid, to) = &after[cb as usize];
            let Some(ty) = to.iter().find(|(n, _)| n == field).map(|(_, t)| t) else {
                continue;
            };
            let Some(id) = field_id(b, db, cb, field) else {
                return Err(Failed::from(Broken::NoFieldId(field.to_owned())));
            };
            let ty = layout::field_type(b, ty);
            let convs: Vec<Conv> = before[c as usize]
                .iter()
                .map(|(_, t)| conv(&fill.schema, t))
                .collect();
            let f = f.clone();
            let fill: Box<Fill<'static>> = Box::new(move |_, vals| {
                let v = f.eval(row_val(c, &convs, vals)).map_err(shown)?;
                to_stored(&ty, &v).map_err(shown)
            });
            self.out
                .values
                .insert((tid, CatCtorId::from_raw(u64::from(*cid)), id), fill);
        }
        Ok(())
    }

    fn copies(&mut self, def: TypeId, ctors: &[u32], field: &str) {
        let (a, b) = (self.a, self.b);
        for (ra, rid, rb) in relations_of(a, b, def) {
            for &c in ctors {
                let Some(cb) = ctor_index(b, rb, ctor_id(a, ra, c)) else {
                    continue;
                };
                let Some(to) = fields(b, rb, cb).into_iter().find(|f| f.name == field) else {
                    continue;
                };
                let Some(id) = to.id else { continue };
                let name = field.to_string();
                let fill: Box<Fill<'static>> = Box::new(move |from, row| {
                    let at = from
                        .iter()
                        .position(|f| f.name == name)
                        .ok_or_else(|| Failed::from(Broken::NotCopied(name.clone())))?;
                    convert(&from[at].ty, &to.ty, &row[at]).map_err(Failed::Refused)
                });
                self.out.fills.insert((rid, ctor_id(b, rb, cb), id), fill);
            }
        }
    }

    fn route(&mut self, def: TypeId, from: &[u32], routing: &RowFn) -> Result<(), Failed> {
        let (a, b) = (self.a, self.b);
        let f = Arc::new(compiled(routing, self.hosts)?);
        self.held_routes(def, from, routing, &f);
        for (ra, rid, rb) in relations_of(a, b, def) {
            let Some(rows) = self.data.get(&rid) else {
                continue;
            };
            for &c in from {
                let cid = ctor_id(a, ra, c);
                let convs = row_convs(&routing.schema, ra, c);
                for row in rows.get(&cid).into_iter().flatten() {
                    let Val::Ctor(k, vals) = f.eval(row_val(c, &convs, row)).map_err(shown)? else {
                        return Err(Failed::from(Broken::RoutedNotCtor));
                    };
                    let name = ctor_name(&routing.schema, def, k);
                    let cb = ctor_named(b, rb, name).ok_or_else(|| {
                        Failed::Refused(format!("the routing gives `{name}`, no product"))
                    })?;
                    let names: Vec<String> = layout::row(&routing.schema, ra, k)
                        .into_iter()
                        .map(|(n, _)| n)
                        .collect();
                    let routed = by_name(&names, &vals, &fields(b, rb, cb))?;
                    self.out
                        .routed
                        .entry((rid, ctor_id(b, rb, cb)))
                        .or_default()
                        .push(routed);
                }
                self.out.consumed.insert((rid, cid));
            }
        }
        Ok(())
    }

    /// Returns the catalog id of type `def` and the same type of `b`.
    fn held_type(&self, def: TypeId) -> Option<(CatTypeId, TypeId)> {
        let tid = type_id(self.a, def)?;
        Some((tid, type_def(self.b, tid)?))
    }

    /// Add to `out.routes` the routing `f`, compiled from `routing`, of each constructor `from`
    /// of type `def`, for the values of `def` other rows hold.
    fn held_routes(&mut self, def: TypeId, from: &[u32], routing: &RowFn, f: &Arc<Scalar>) {
        let Some((tid, db)) = self.held_type(def) else {
            return;
        };
        let schema = Arc::new(routing.schema.clone());
        let products: Vec<Option<u32>> = from
            .iter()
            .map(|&c| sole_product(&routing.query, c))
            .collect();
        for (&c, p) in from.iter().zip(&products) {
            let tag = match p {
                Some(p) if products.iter().filter(|q| *q == &Some(*p)).count() > 1 => continue,
                Some(p) => {
                    let name = &schema.ty(def).ctors()[*p as usize].name;
                    b_ctor(self.b, db, name)
                }
                None => None,
            };
            self.out
                .tags
                .insert((tid, type_ctor_id(self.a, def, c)), tag);
        }
        for &c in from {
            let (f, schema) = (f.clone(), schema.clone());
            let route: Box<Route> = Box::new(move |a, b, aa, ab| {
                unreached(a, def, aa)?;
                let d = schema.ty(def);
                let routed: Vec<Routed> = d
                    .ctors()
                    .iter()
                    .map(|k| {
                        let fs: Vec<_> = d.header().iter().chain(&k.fields).collect();
                        let product = b.ty(db).ctor_index(&k.name).map(|p| {
                            let to = instance(b, db, ab, p).into_iter().map(|(name, t)| Field {
                                id: None,
                                name,
                                ty: layout::field_type(b, &t),
                            });
                            (p, to.collect())
                        });
                        Routed {
                            name: k.name.clone(),
                            product,
                            names: fs.iter().map(|f| f.name.clone()).collect(),
                            convs: fs.iter().map(|f| conv(&schema, &f.ty.subst(aa))).collect(),
                        }
                    })
                    .collect();
                let record = layout::is_record(b, &data(db, ab));
                let f = f.clone();
                Ok(Box::new(move |_, vals| {
                    let held = row_val(c, &routed[c as usize].convs, vals);
                    let Val::Ctor(k, out) = f.eval(held).map_err(shown)? else {
                        return Err(Failed::from(Broken::RoutedNotCtor));
                    };
                    let to = &routed[k as usize];
                    let (cb, product) = to.product.as_ref().ok_or_else(|| {
                        Failed::Refused(format!("the routing gives `{}`, no product", to.name))
                    })?;
                    Ok(shaped(record, *cb, by_name(&to.names, &out, product)?))
                }))
            });
            self.out
                .routes
                .insert((tid, type_ctor_id(self.a, def, c)), route);
        }
    }

    /// Add to `out.routes` the merge of each constructor `from` of type `def` into `into`, for
    /// the values of `def` other rows hold.
    fn held_merges(&mut self, def: TypeId, from: &[u32], into: &str, discriminator: &str) {
        let Some((tid, db)) = self.held_type(def) else {
            return;
        };
        for &c in from {
            let name = ctor_name(self.a, def, c).to_owned();
            let (into, discriminator) = (into.to_owned(), discriminator.to_owned());
            let route: Box<Route> = Box::new(move |a, b, aa, ab| {
                unreached(a, def, aa)?;
                let m = b
                    .ty(db)
                    .ctor_index(&into)
                    .ok_or_else(|| Failed::from(Broken::NoCtor(into.clone())))?;
                let fa = instance(a, def, aa, c);
                let parts = instance(b, db, ab, m)
                    .into_iter()
                    .map(|(n, t)| {
                        if n == discriminator {
                            let Ty::Data { def: disc, .. } = layout::repr(b, &t) else {
                                return Err(Failed::from(Broken::NoDiscriminator(n)));
                            };
                            let tag = b.ty(disc).ctor_index(&name).ok_or_else(|| {
                                Failed::from(Broken::NotDiscriminated(name.clone()))
                            })?;
                            let v = to_stored(&layout::field_type(b, &t), &Val::tag(tag));
                            return Ok(Lift::Value(v.map_err(shown)?));
                        }
                        lift(a, &fa, b, &n, &t)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let record = layout::is_record(b, &data(db, ab));
                Ok(Box::new(move |_, vals| {
                    let fields = parts
                        .iter()
                        .map(|p| p.value(vals, &[]))
                        .collect::<Result<_, _>>()?;
                    Ok(shaped(record, m, fields))
                }))
            });
            self.out
                .routes
                .insert((tid, type_ctor_id(self.a, def, c)), route);
        }
    }

    /// Add to `out.routes` the nest of each constructor `from` of type `def` into `into`, for
    /// the values of `def` other rows hold.
    fn held_nests(&mut self, def: TypeId, from: &[u32], into: &str) {
        let Some((tid, db)) = self.held_type(def) else {
            return;
        };
        let header = self.a.ty(def).header().len();
        for &c in from {
            let name = ctor_name(self.a, def, c).to_owned();
            let into = into.to_owned();
            let route: Box<Route> = Box::new(move |a, b, aa, ab| {
                unreached(a, def, aa)?;
                let n = b
                    .ty(db)
                    .ctor_index(&into)
                    .ok_or_else(|| Failed::from(Broken::NoCtor(into.clone())))?;
                let fa = instance(a, def, aa, c);
                let fb = instance(b, db, ab, n);
                let Some(((_, held), outer)) = fb.split_last() else {
                    return Err(Failed::from(Broken::Fieldless(into.clone())));
                };
                let Ty::Data { def: s, .. } = layout::repr(b, held) else {
                    return Err(Failed::from(Broken::LastNotSum));
                };
                let k = b
                    .ty(s)
                    .ctor_index(&name)
                    .ok_or_else(|| Failed::from(Broken::NotNested(name.clone())))?;
                let parts = outer
                    .iter()
                    .map(|(n, t)| lift(a, &fa[..header], b, n, t))
                    .collect::<Result<Vec<_>, _>>()?;
                let held = layout::field_type(b, held);
                let convs: Vec<Conv> = fa[header..].iter().map(|(_, t)| conv(a, t)).collect();
                let record = layout::is_record(b, &data(db, ab));
                Ok(Box::new(move |_, vals| {
                    let mut fields = parts
                        .iter()
                        .map(|p| p.value(vals, &[]))
                        .collect::<Result<Vec<_>, _>>()?;
                    let inner = convs
                        .iter()
                        .zip(&vals[header..])
                        .map(|(k, v)| from_stored(k, v));
                    fields.push(to_stored(&held, &Val::ctor(k, inner.collect())).map_err(shown)?);
                    Ok(shaped(record, n, fields))
                }))
            });
            self.out
                .routes
                .insert((tid, type_ctor_id(self.a, def, c)), route);
        }
    }

    /// Add to `out.routes` the unnest of constructor `from` of type `def`, for the values of
    /// `def` other rows hold.
    fn held_unnests(&mut self, def: TypeId, from: u32) {
        let Some((tid, db)) = self.held_type(def) else {
            return;
        };
        let header = self.a.ty(def).header().len();
        let outer = ctor_name(self.a, def, from).to_owned();
        let route: Box<Route> = Box::new(move |a, b, aa, ab| {
            unreached(a, def, aa)?;
            let fa = instance(a, def, aa, from);
            let Some((_, held)) = fa.last() else {
                return Err(Failed::from(Broken::Fieldless(outer.clone())));
            };
            let s = layout::repr(a, held);
            let lifted = layout::ctors(a, &s)
                .into_iter()
                .enumerate()
                .map(|(k, _)| {
                    let Ty::Data { def: sd, .. } = &s else {
                        return Err(Failed::from(Broken::LastNotSum));
                    };
                    let inner = &a.ty(*sd).ctors()[k];
                    let p = b
                        .ty(db)
                        .ctor_index(&inner.name)
                        .ok_or_else(|| Failed::from(Broken::NoCtor(inner.name.clone())))?;
                    let parts = instance(b, db, ab, p)
                        .into_iter()
                        .map(
                            |(n, t)| match inner.fields.iter().position(|f| f.name == n) {
                                Some(at) => Ok(Lift::Inner(at, layout::field_type(b, &t))),
                                None => lift(a, &fa[..header], b, &n, &t),
                            },
                        )
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok((p, parts))
                })
                .collect::<Result<Vec<_>, Failed>>()?;
            let held = conv(a, held);
            let record = layout::is_record(b, &data(db, ab));
            Ok(Box::new(move |_, vals| {
                let Some(Val::Ctor(k, inner)) = vals.last().map(|v| from_stored(&held, v)) else {
                    return Err(Failed::from(Broken::UnnestedNotCtor));
                };
                let (p, parts) = &lifted[k as usize];
                let fields = parts
                    .iter()
                    .map(|l| l.value(vals, &inner))
                    .collect::<Result<_, _>>()?;
                Ok(shaped(record, *p, fields))
            }))
        });
        self.out
            .routes
            .insert((tid, type_ctor_id(self.a, def, from)), route);
    }

    fn merge(
        &mut self,
        def: TypeId,
        from: &[u32],
        into: &str,
        discriminator: &str,
    ) -> Result<(), Failed> {
        self.held_merges(def, from, into, discriminator);
        let (a, b) = (self.a, self.b);
        for (ra, rid, rb) in relations_of(a, b, def) {
            let Some(rows) = self.data.get(&rid) else {
                continue;
            };
            let cb = ctor_named(b, rb, into)
                .ok_or_else(|| Failed::from(Broken::NoCtor(into.to_owned())))?;
            let to = fields(b, rb, cb);
            let disc = layout::row(b, rb, cb)
                .into_iter()
                .find(|(n, _)| n == discriminator)
                .and_then(|(_, t)| match t {
                    Ty::Data { def, .. } => Some(def),
                    _ => None,
                })
                .ok_or_else(|| Failed::from(Broken::NoDiscriminator(discriminator.to_owned())))?;
            for &c in from {
                let cid = ctor_id(a, ra, c);
                let name = ctor_name(a, def, c);
                let tag = b
                    .ty(disc)
                    .ctor_index(name)
                    .ok_or_else(|| Failed::from(Broken::NotDiscriminated(name.to_owned())))?;
                let from_fields = fields(a, ra, c);
                for row in rows.get(&cid).into_iter().flatten() {
                    let merged = to
                        .iter()
                        .map(|t| {
                            if t.name == discriminator {
                                to_stored(&t.ty, &Val::tag(tag)).map_err(shown)
                            } else {
                                carried(&from_fields, row, t)
                            }
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    self.out
                        .routed
                        .entry((rid, ctor_id(b, rb, cb)))
                        .or_default()
                        .push(merged);
                }
                self.out.consumed.insert((rid, cid));
            }
        }
        Ok(())
    }

    fn nest(&mut self, def: TypeId, from: &[u32], into: &str) -> Result<(), Failed> {
        self.held_nests(def, from, into);
        let (a, b) = (self.a, self.b);
        let header = a.ty(def).header().len();
        for (ra, rid, rb) in relations_of(a, b, def) {
            let Some(rows) = self.data.get(&rid) else {
                continue;
            };
            let cb = ctor_named(b, rb, into)
                .ok_or_else(|| Failed::from(Broken::NoCtor(into.to_owned())))?;
            let to = fields(b, rb, cb);
            let Some((held, outer)) = to.split_last() else {
                return Err(Failed::from(Broken::Fieldless(into.to_owned())));
            };
            let s = last_field_def(b, rb, cb)?;
            for &c in from {
                let cid = ctor_id(a, ra, c);
                let name = ctor_name(a, def, c);
                let k = b
                    .ty(s)
                    .ctor_index(name)
                    .ok_or_else(|| Failed::from(Broken::NotNested(name.to_owned())))?;
                let from_fields = fields(a, ra, c);
                let convs = row_convs(a, ra, c);
                for row in rows.get(&cid).into_iter().flatten() {
                    let mut nested = outer
                        .iter()
                        .map(|t| carried(&from_fields, row, t))
                        .collect::<Result<Vec<_>, _>>()?;
                    let inner = convs[header..]
                        .iter()
                        .zip(&row[header..])
                        .map(|(k, v)| from_stored(k, v))
                        .collect();
                    nested.push(to_stored(&held.ty, &Val::ctor(k, inner)).map_err(shown)?);
                    self.out
                        .routed
                        .entry((rid, ctor_id(b, rb, cb)))
                        .or_default()
                        .push(nested);
                }
                self.out.consumed.insert((rid, cid));
            }
        }
        Ok(())
    }

    fn unnest(&mut self, def: TypeId, from: u32) -> Result<(), Failed> {
        self.held_unnests(def, from);
        let (a, b) = (self.a, self.b);
        for (ra, rid, rb) in relations_of(a, b, def) {
            let Some(rows) = self.data.get(&rid) else {
                continue;
            };
            let cid = ctor_id(a, ra, from);
            let from_fields = fields(a, ra, from);
            let s = last_field_def(a, ra, from)?;
            let convs = row_convs(a, ra, from);
            let Some(held) = convs.last() else {
                return Err(Failed::from(Broken::Fieldless(
                    ctor_name(a, def, from).to_owned(),
                )));
            };
            for row in rows.get(&cid).into_iter().flatten() {
                let Some(Val::Ctor(k, vals)) = row.last().map(|v| from_stored(held, v)) else {
                    return Err(Failed::from(Broken::UnnestedNotCtor));
                };
                let inner = &a.ty(s).ctors()[k as usize];
                let cb = ctor_named(b, rb, &inner.name)
                    .ok_or_else(|| Failed::from(Broken::NoCtor(inner.name.clone())))?;
                let names: Vec<String> = inner.fields.iter().map(|f| f.name.clone()).collect();
                let lifted = fields(b, rb, cb)
                    .iter()
                    .map(|t| match names.iter().position(|n| *n == t.name) {
                        Some(at) => to_stored(&t.ty, &vals[at]).map_err(shown),
                        None => carried(&from_fields, row, t),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.out
                    .routed
                    .entry((rid, ctor_id(b, rb, cb)))
                    .or_default()
                    .push(lifted);
            }
            self.out.consumed.insert((rid, cid));
        }
        Ok(())
    }

    fn rekey(&mut self, ra: RelId) -> Result<(), Failed> {
        let (a, b) = (self.a, self.b);
        let rid = rel_id(a, ra);
        let rb = relation(b, rid).ok_or_else(|| Failed::from(Broken::RekeyedGone(rid)))?;
        let old_ty = layout::field_type(a, &layout::key_value_ty(a, ra));
        let mut keys: BTreeMap<Vec<u8>, Value> = BTreeMap::new();
        for (&cid, rows) in self.data.get(&rid).into_iter().flatten() {
            let c = ctor_index(a, ra, cid).ok_or_else(|| Failed::from(Broken::ForeignCtor(cid)))?;
            let (old, new) = (KeyAt::new(a, ra, c, a, ra)?, KeyAt::new(a, ra, c, b, rb)?);
            for row in rows {
                let mut at = Vec::new();
                oplog::encode(&old_ty, &old.value(row)?, &mut at);
                keys.insert(at, new.value(row)?);
            }
        }
        let keys = Arc::new(keys);
        for &x in self.data.keys() {
            let Some(xb) = relation(b, x) else { continue };
            let def = b.relation(xb).elem_def;
            for c in 0..b.ty(def).ctors().len() as u32 {
                let named = layout::row(b, xb, c);
                for (f, (_, ty)) in fields(b, xb, c).into_iter().zip(named) {
                    let (Some(refs), Some(id)) = (Refs::of(b, &ty, rb), f.id) else {
                        continue;
                    };
                    let (keys, old_ty) = (keys.clone(), old_ty.clone());
                    let fill: Box<Fill<'static>> = Box::new(move |from, row| {
                        let at = from
                            .iter()
                            .position(|g| g.id == Some(id))
                            .ok_or_else(|| Failed::from(Broken::NotRekeyed(id)))?;
                        let key = |v: &Value| {
                            let mut enc = Vec::new();
                            oplog::encode(&old_ty, v, &mut enc);
                            keys.get(&enc).cloned().ok_or_else(|| {
                                Failed::Refused(format!("field `{}` references no row", f.name))
                            })
                        };
                        refs.rekeyed(&row[at], &key)
                    });
                    self.out.fills.insert((x, ctor_id(b, xb, c), id), fill);
                }
            }
        }
        Ok(())
    }
}
