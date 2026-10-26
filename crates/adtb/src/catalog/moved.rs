use std::collections::BTreeMap;
use std::sync::Arc;

use adtb_lang::evolve::{RowFn, RowMap};
use adtb_lang::schema::Schema;
use adtb_lang::types::{RelId, Ty, TypeId};

use crate::exec::{self, Conv, Scalar, Val, conv, from_stored, to_stored};
use crate::key::Value;

use super::change::{self, Data, Failed, Field, Fill, Moved, convert, fields, relation};
use super::{Error, ctor_id, layout, oplog};

/// Returns what `maps`, the row maps of one step from schema `a` to schema `b`, give the rows of
/// `data`, which are in `a`'s layout.
///
/// Fails with `Error::Refused` naming each map the data refuses, or with `Error::Internal`.
pub fn moved(a: &Schema, b: &Schema, maps: &[RowMap], data: &Data) -> Result<Moved, Error> {
    let mut out = Moved::default();
    let mut why = Vec::new();
    for map in maps {
        let got = match map {
            RowMap::Fill {
                def,
                ctors,
                field,
                fill,
            } => fills(a, b, *def, ctors, field, fill, &mut out),
            RowMap::Copy { def, ctors, field } => {
                copies(a, b, *def, ctors, field, &mut out);
                Ok(())
            }
            RowMap::Route { def, from, routing } => {
                route(a, b, *def, from, routing, data, &mut out)
            }
            RowMap::Merge {
                def,
                from,
                into,
                discriminator,
            } => merge(a, b, *def, from, into, discriminator, data, &mut out),
            RowMap::Nest { def, from, into } => nest(a, b, *def, from, into, data, &mut out),
            RowMap::Unnest { def, from } => unnest(a, b, *def, *from, data, &mut out),
            RowMap::Rekey { relation } => rekey(a, b, *relation, data, &mut out),
        };
        match got {
            Ok(()) => {}
            Err(Failed::Refused(e)) => why.push(e),
            Err(Failed::Internal(e)) => return Err(Error::Internal(e)),
        }
    }
    if why.is_empty() {
        Ok(out)
    } else {
        Err(Error::Refused(why))
    }
}

/// Each relation of `a` whose element type is `def`, with its catalog id and the same relation
/// of `b`.
fn relations_of(a: &Schema, b: &Schema, def: TypeId) -> Vec<(RelId, u64, RelId)> {
    a.ids
        .relations
        .iter()
        .filter(|&(&r, _)| a.relation(r).elem_def == def)
        .filter_map(|(&r, &rid)| relation(b, rid).map(|rb| (r, rid, rb)))
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

fn ctor_named(schema: &Schema, r: RelId, name: &str) -> Option<u32> {
    schema.ty(schema.relation(r).elem_def).ctor_index(name)
}

fn ctor_name(schema: &Schema, def: TypeId, c: u32) -> &str {
    &schema.ty(def).ctors()[c as usize].name
}

fn field_named<'f>(fs: &'f [Field], name: &str) -> Result<&'f Field, Failed> {
    fs.iter()
        .find(|f| f.name == name)
        .ok_or_else(|| format!("no field `{name}`").into())
}

fn compiled(f: &RowFn) -> Result<Scalar, Failed> {
    let [var] = f.query.params.as_slice() else {
        return Err("a fill or routing takes the row".into());
    };
    Scalar::new(&f.schema, &f.query.terms, *var, f.query.body).map_err(shown)
}

fn shown(e: exec::Error) -> Failed {
    match e {
        exec::Error::Fault(f) => Failed::Refused(format!("the map faults with {}", f.name())),
        exec::Error::Rejected(why) => Failed::Refused(format!("the map rejects: {why}")),
        other => Failed::Internal(format!("a data map fails: {other:?}")),
    }
}

fn fills(
    a: &Schema,
    b: &Schema,
    def: TypeId,
    ctors: &[u32],
    field: &str,
    fill: &RowFn,
    out: &mut Moved,
) -> Result<(), Failed> {
    let f = Arc::new(compiled(fill)?);
    for (ra, rid, rb) in relations_of(a, b, def) {
        for &c in ctors {
            let name = ctor_name(a, def, c);
            let Some(cb) = ctor_named(b, rb, name) else {
                continue;
            };
            let to = fields(b, rb, cb);
            let Some(id) = field_named(&to, field)?.id else {
                return Err(format!("field `{field}` has no catalog id").into());
            };
            let ty = field_named(&to, field)?.ty.clone();
            let convs = row_convs(&fill.schema, ra, c);
            let f = f.clone();
            let fill: Box<Fill<'static>> = Box::new(move |_, row| {
                let v = f.eval(row_val(c, &convs, row)).map_err(shown)?;
                to_stored(&ty, &v).map_err(shown)
            });
            out.fills.insert((rid, ctor_id(b, rb, cb), id), fill);
        }
    }
    held_fills(a, b, def, ctors, field, fill, &f, out)
}

/// Adds to `out.values` the fill `f`, compiled from `fill`, of field `field` that constructors
/// `ctors` of type `def` gain, for the values of `def` other rows hold.
#[expect(
    clippy::too_many_arguments,
    reason = "the operator's fill, compiled and as written, and where it goes"
)]
fn held_fills(
    a: &Schema,
    b: &Schema,
    def: TypeId,
    ctors: &[u32],
    field: &str,
    fill: &RowFn,
    f: &Arc<Scalar>,
    out: &mut Moved,
) -> Result<(), Failed> {
    let Some(&tid) = a.ids.types.get(&def) else {
        return Ok(());
    };
    let Some((&db, _)) = b.ids.types.iter().find(|&(_, &id)| id == tid) else {
        return Ok(());
    };
    let value = |def| Ty::Data {
        def,
        args: Vec::new(),
        ctors: None,
    };
    if !b.ty(db).params.is_empty() {
        return Ok(());
    }
    let after = layout::ctors(b, &value(db));
    let before = layout::ctors(&fill.schema, &value(def));
    for &c in ctors {
        let Some(cb) = b.ty(db).ctor_index(ctor_name(a, def, c)) else {
            continue;
        };
        let (cid, to) = &after[cb as usize];
        let Some(ty) = to.iter().find(|(n, _)| n == field).map(|(_, t)| t) else {
            continue;
        };
        let Some(&id) = b
            .ids
            .fields
            .get(&(db, None, field.to_string()))
            .or_else(|| b.ids.fields.get(&(db, Some(cb), field.to_string())))
        else {
            return Err(format!("field `{field}` has no catalog id").into());
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
        out.values.insert((tid, u64::from(*cid), id), fill);
    }
    Ok(())
}

fn copies(a: &Schema, b: &Schema, def: TypeId, ctors: &[u32], field: &str, out: &mut Moved) {
    for (_, rid, rb) in relations_of(a, b, def) {
        for &c in ctors {
            let Some(cb) = ctor_named(b, rb, ctor_name(a, def, c)) else {
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
                    .ok_or_else(|| format!("the row has no field `{name}` to move"))?;
                Ok(convert(&from[at].ty, &to.ty, &row[at])?)
            });
            out.fills.insert((rid, ctor_id(b, rb, cb), id), fill);
        }
    }
}

/// Returns `vals`, the fields of a value named `names`, as a row of fields `to`.
fn by_name(names: &[String], vals: &[Val], to: &[Field]) -> Result<Vec<Value>, Failed> {
    to.iter()
        .map(|t| {
            let at = names
                .iter()
                .position(|n| *n == t.name)
                .ok_or_else(|| format!("field `{}` has no value in the routed row", t.name))?;
            to_stored(&t.ty, &vals[at]).map_err(shown)
        })
        .collect()
}

fn route(
    a: &Schema,
    b: &Schema,
    def: TypeId,
    from: &[u32],
    routing: &RowFn,
    data: &Data,
    out: &mut Moved,
) -> Result<(), Failed> {
    let f = compiled(routing)?;
    for (ra, rid, rb) in relations_of(a, b, def) {
        let Some(rows) = data.get(&rid) else { continue };
        for &c in from {
            let cid = ctor_id(a, ra, c);
            let convs = row_convs(&routing.schema, ra, c);
            for row in rows.get(&cid).into_iter().flatten() {
                let Val::Ctor(k, vals) = f.eval(row_val(c, &convs, row)).map_err(shown)? else {
                    return Err("a routing gives a constructor".into());
                };
                let name = ctor_name(&routing.schema, def, k);
                let cb = ctor_named(b, rb, name)
                    .ok_or_else(|| format!("the routing gives `{name}`, no product"))?;
                let names: Vec<String> = layout::row(&routing.schema, ra, k)
                    .into_iter()
                    .map(|(n, _)| n)
                    .collect();
                let routed = by_name(&names, &vals, &fields(b, rb, cb))?;
                out.routed
                    .entry((rid, ctor_id(b, rb, cb)))
                    .or_default()
                    .push(routed);
            }
            out.consumed.insert((rid, cid));
        }
    }
    Ok(())
}

/// Returns the field of `row`, with fields `from`, named as `to`, stored as `to` stores it.
fn carried(from: &[Field], row: &[Value], to: &Field) -> Result<Value, Failed> {
    let at = from
        .iter()
        .position(|f| f.name == to.name)
        .ok_or_else(|| format!("field `{}` has no counterpart", to.name))?;
    Ok(convert(&from[at].ty, &to.ty, &row[at])?)
}

/// The declared type of the last field of constructor `c` of relation `r`.
fn last_field_def(schema: &Schema, r: RelId, c: u32) -> Result<TypeId, Failed> {
    match layout::row(schema, r, c).last() {
        Some((_, Ty::Data { def, .. })) => Ok(*def),
        _ => Err("the constructor's last field holds no declared sum".into()),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one merge's schemas, sources, product and rows"
)]
fn merge(
    a: &Schema,
    b: &Schema,
    def: TypeId,
    from: &[u32],
    into: &str,
    discriminator: &str,
    data: &Data,
    out: &mut Moved,
) -> Result<(), Failed> {
    for (ra, rid, rb) in relations_of(a, b, def) {
        let Some(rows) = data.get(&rid) else { continue };
        let cb = ctor_named(b, rb, into).ok_or_else(|| format!("no constructor `{into}`"))?;
        let to = fields(b, rb, cb);
        let disc = layout::row(b, rb, cb)
            .into_iter()
            .find(|(n, _)| n == discriminator)
            .and_then(|(_, t)| match t {
                Ty::Data { def, .. } => Some(def),
                _ => None,
            })
            .ok_or_else(|| format!("no discriminator `{discriminator}`"))?;
        for &c in from {
            let cid = ctor_id(a, ra, c);
            let name = ctor_name(a, def, c);
            let tag = b
                .ty(disc)
                .ctor_index(name)
                .ok_or_else(|| format!("the discriminator has no `{name}`"))?;
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
                out.routed
                    .entry((rid, ctor_id(b, rb, cb)))
                    .or_default()
                    .push(merged);
            }
            out.consumed.insert((rid, cid));
        }
    }
    Ok(())
}

fn nest(
    a: &Schema,
    b: &Schema,
    def: TypeId,
    from: &[u32],
    into: &str,
    data: &Data,
    out: &mut Moved,
) -> Result<(), Failed> {
    let header = a.ty(def).header().len();
    for (ra, rid, rb) in relations_of(a, b, def) {
        let Some(rows) = data.get(&rid) else { continue };
        let cb = ctor_named(b, rb, into).ok_or_else(|| format!("no constructor `{into}`"))?;
        let to = fields(b, rb, cb);
        let Some((held, outer)) = to.split_last() else {
            return Err(format!("`{into}` has no field").into());
        };
        let s = last_field_def(b, rb, cb)?;
        for &c in from {
            let cid = ctor_id(a, ra, c);
            let name = ctor_name(a, def, c);
            let k = b
                .ty(s)
                .ctor_index(name)
                .ok_or_else(|| format!("the nested sum has no `{name}`"))?;
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
                out.routed
                    .entry((rid, ctor_id(b, rb, cb)))
                    .or_default()
                    .push(nested);
            }
            out.consumed.insert((rid, cid));
        }
    }
    Ok(())
}

fn unnest(
    a: &Schema,
    b: &Schema,
    def: TypeId,
    from: u32,
    data: &Data,
    out: &mut Moved,
) -> Result<(), Failed> {
    for (ra, rid, rb) in relations_of(a, b, def) {
        let Some(rows) = data.get(&rid) else { continue };
        let cid = ctor_id(a, ra, from);
        let from_fields = fields(a, ra, from);
        let s = last_field_def(a, ra, from)?;
        let convs = row_convs(a, ra, from);
        let Some(held) = convs.last() else {
            return Err("the unnested constructor has no field".into());
        };
        for row in rows.get(&cid).into_iter().flatten() {
            let Some(Val::Ctor(k, vals)) = row.last().map(|v| from_stored(held, v)) else {
                return Err("the unnested field holds a constructor".into());
            };
            let inner = &a.ty(s).ctors()[k as usize];
            let cb = ctor_named(b, rb, &inner.name)
                .ok_or_else(|| format!("no constructor `{}`", inner.name))?;
            let names: Vec<String> = inner.fields.iter().map(|f| f.name.clone()).collect();
            let lifted = fields(b, rb, cb)
                .iter()
                .map(|t| match names.iter().position(|n| *n == t.name) {
                    Some(at) => to_stored(&t.ty, &vals[at]).map_err(shown),
                    None => carried(&from_fields, row, t),
                })
                .collect::<Result<Vec<_>, _>>()?;
            out.routed
                .entry((rid, ctor_id(b, rb, cb)))
                .or_default()
                .push(lifted);
        }
        out.consumed.insert((rid, cid));
    }
    Ok(())
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
    let kc = ctor_named(k, keyed, ctor_name(a, a.relation(r).elem_def, c))
        .ok_or_else(|| "the keyed constructor is gone".to_string())?;
    let to = fields(k, keyed, kc);
    let mut names = k.relation(keyed).key.clone();
    names.sort();
    let mut vs = names
        .iter()
        .map(|n| carried(&from, row, field_named(&to, n)?))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match vs.len() {
        1 => vs.remove(0),
        _ => Value::Record(vs),
    })
}

fn rekey(a: &Schema, b: &Schema, ra: RelId, data: &Data, out: &mut Moved) -> Result<(), Failed> {
    let rid = a.ids.relations[&ra];
    let rb = relation(b, rid).ok_or_else(|| "the rekeyed relation is gone".to_string())?;
    let old_ty = layout::field_type(a, &layout::key_value_ty(a, ra));
    let mut keys: BTreeMap<Vec<u8>, Value> = BTreeMap::new();
    for (&cid, rows) in data.get(&rid).into_iter().flatten() {
        let c = change::ctor_index(a, ra, cid).expect("a constructor of the schema before");
        for row in rows {
            let old = key_value(a, ra, c, row, a, ra)?;
            let mut at = Vec::new();
            oplog::encode(&old_ty, &old, &mut at);
            keys.insert(at, key_value(a, ra, c, row, b, rb)?);
        }
    }
    let keys = Arc::new(keys);
    for &x in data.keys() {
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
                    let at = from.iter().position(|g| g.id == Some(id)).ok_or_else(|| {
                        format!("field `{}` has no value before the rekey", f.name)
                    })?;
                    let mut enc = Vec::new();
                    oplog::encode(&old_ty, &row[at], &mut enc);
                    keys.get(&enc)
                        .cloned()
                        .ok_or_else(|| format!("field `{}` references no row", f.name).into())
                });
                out.fills.insert((x, ctor_id(b, xb, c), id), fill);
            }
        }
    }
    Ok(())
}
