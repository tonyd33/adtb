use std::collections::BTreeMap;
use std::sync::Arc;

use adtb_lang::evolve::{RowFn, RowMap};
use adtb_lang::schema::Schema;
use adtb_lang::textual::key_names;
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::exec::{self, Conv, Scalar, Val, conv, from_stored, to_stored};
use crate::key::Value;

use super::change::{Data, Failed, Field, Fill, Moved, convert, fields};
use super::hosts::unbound;
use super::ids::{
    CatCtorId, CatRelId, ctor_id, ctor_index, field_id, rel_id, relation, type_ctor_id,
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
        .relations
        .keys()
        .filter(|&&r| a.relation(r).elem_def == def)
        .filter_map(|&r| {
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

fn data(def: TypeId) -> Ty {
    Ty::Data {
        def,
        args: Vec::new(),
        ctors: None,
    }
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

/// Returns the key of `row`, a row of constructor `c` of relation `r` of `a`, as a field typed
/// `Key` of the relation stores it under the key of `keyed` in schema `k`, `r` there.
fn key_value(
    a: &Schema,
    r: RelId,
    c: u32,
    row: &[Value],
    k: &Schema,
    keyed: RelId,
) -> Result<Value, Failed> {
    let from = fields(a, r, c);
    let kc = ctor_index(k, keyed, ctor_id(a, r, c)).ok_or_else(|| {
        Failed::from(Broken::NoCtor(
            ctor_name(a, a.relation(r).elem_def, c).to_owned(),
        ))
    })?;
    let to = fields(k, keyed, kc);
    let mut vs = key_names(k, keyed)
        .iter()
        .map(|n| {
            let t = field_named(&to, n)?;
            let at = from
                .iter()
                .position(|f| f.id.is_some() && f.id == t.id)
                .ok_or_else(|| Failed::from(Broken::NoCounterpart(n.clone())))?;
            convert(&from[at].ty, &t.ty, &row[at]).map_err(Failed::Refused)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match vs.len() {
        1 => vs.remove(0),
        _ => Value::Record(vs),
    })
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
        let after = layout::ctors(b, &data(db));
        let before = layout::ctors(&fill.schema, &data(def));
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
                .1
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
                .insert((tid, CatCtorId(u64::from(*cid)), id), fill);
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

    /// Add to `out.routes` the routing `f`, compiled from `routing`, of each constructor `from`
    /// of type `def`, for the values of `def` other rows hold.
    fn held_routes(&mut self, def: TypeId, from: &[u32], routing: &RowFn, f: &Arc<Scalar>) {
        let (a, b) = (self.a, self.b);
        let (Some(tid), true) = (type_id(a, def), a.ty(def).params.is_empty()) else {
            return;
        };
        let Some(db) = type_def(b, tid) else {
            return;
        };
        let after = layout::ctors(b, &data(db));
        let d = routing.schema.ty(def);
        let routed: Arc<Vec<Routed>> = Arc::new(
            d.ctors()
                .iter()
                .map(|c| {
                    let fs: Vec<_> = d.header().iter().chain(&c.fields).collect();
                    let product = b.ty(db).ctor_index(&c.name).map(|k| {
                        let to = after[k as usize].1.iter().map(|(name, t)| Field {
                            id: None,
                            name: name.clone(),
                            ty: layout::field_type(b, t),
                        });
                        (k, to.collect())
                    });
                    Routed {
                        name: c.name.clone(),
                        product,
                        names: fs.iter().map(|f| f.name.clone()).collect(),
                        convs: fs.iter().map(|f| conv(&routing.schema, &f.ty)).collect(),
                    }
                })
                .collect(),
        );
        for &c in from {
            let (f, routed) = (f.clone(), routed.clone());
            let route: Box<Fill<'static>> = Box::new(move |_, vals| {
                let held = row_val(c, &routed[c as usize].convs, vals);
                let Val::Ctor(k, out) = f.eval(held).map_err(shown)? else {
                    return Err(Failed::from(Broken::RoutedNotCtor));
                };
                let to = &routed[k as usize];
                let (cb, product) = to.product.as_ref().ok_or_else(|| {
                    Failed::Refused(format!("the routing gives `{}`, no product", to.name))
                })?;
                let fields = by_name(&to.names, &out, product)?;
                Ok(Value::Sum { ctor: *cb, fields })
            });
            let cid = CatCtorId(u64::from(layout::ctors(a, &data(def))[c as usize].0));
            self.out.routes.insert((tid, cid), route);
        }
    }

    fn merge(
        &mut self,
        def: TypeId,
        from: &[u32],
        into: &str,
        discriminator: &str,
    ) -> Result<(), Failed> {
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
            for row in rows {
                let old = key_value(a, ra, c, row, a, ra)?;
                let mut at = Vec::new();
                oplog::encode(&old_ty, &old, &mut at);
                keys.insert(at, key_value(a, ra, c, row, b, rb)?);
            }
        }
        let keys = Arc::new(keys);
        for &x in self.data.keys() {
            let Some(xb) = relation(b, x) else { continue };
            let def = b.relation(xb).elem_def;
            for c in 0..b.ty(def).ctors().len() as u32 {
                let named = layout::row(b, xb, c);
                for (f, (_, ty)) in fields(b, xb, c).into_iter().zip(named) {
                    let (Ty::Key { rel, .. }, Some(id)) = (ty, f.id) else {
                        continue;
                    };
                    if rel != rb {
                        continue;
                    }
                    let (keys, old_ty) = (keys.clone(), old_ty.clone());
                    let fill: Box<Fill<'static>> = Box::new(move |from, row| {
                        let at = from
                            .iter()
                            .position(|g| g.id == Some(id))
                            .ok_or_else(|| Failed::from(Broken::NotRekeyed(id)))?;
                        let mut enc = Vec::new();
                        oplog::encode(&old_ty, &row[at], &mut enc);
                        keys.get(&enc).cloned().ok_or_else(|| {
                            Failed::Refused(format!("field `{}` references no row", f.name))
                        })
                    });
                    self.out.fills.insert((x, ctor_id(b, xb, c), id), fill);
                }
            }
        }
        Ok(())
    }
}
