use crate::kernel::{Build, Kind, Prim, Terms, Tm};
use crate::schema::Schema;
use crate::types::*;

use super::super::R;
use super::Walk;

impl Walk<'_, '_> {
    /// Returns the key of `rel` built from `x`, a record laid out as the pin lays out the key.
    pub(super) fn key_of(&mut self, out: &mut Terms, rel: RelId, x: Tm) -> R<Tm> {
        let to = self.relation(rel)?;
        let mut x = self.tm(out, x, None)?;
        let order = self.map.key_layout(rel);
        if let Some(order) = order.map_err(|l| self.p.lost(&l, "builds a key of"))?
            && let Ty::Record(fs) = out.ty(x).clone()
            && order
                .iter()
                .enumerate()
                .any(|(i, (p, n))| i != *p || fs[*p].0 != *n)
        {
            let fields: Vec<_> = order
                .into_iter()
                .map(|(p, n)| (fs[p].0.clone(), n, fs[p].1.clone()))
                .collect();
            x = relaid(self.map.to, out, x, &fields);
        }
        Ok(self.b(out).key_of(to, x))
    }

    /// Returns the value of key `k` as a record laid out as the pin lays out the key.
    pub(super) fn key_value(&mut self, out: &mut Terms, src_ty: &Ty, k: Tm) -> R<Tm> {
        let ty = self.ty(src_ty)?;
        let Ty::Key { rel, .. } = *self.src.ty(k) else {
            panic!("the operand of a key's value is a key");
        };
        let k = self.tm(out, k, None)?;
        let order = self.map.key_layout(self.map.read_through(rel));
        let order = order.map_err(|l| self.p.lost(&l, "reads a key of"))?;
        let (Some(order), Ty::Record(fs)) = (order, &ty) else {
            return Ok(self.b(out).prim(Prim::KeyValue, vec![k], ty));
        };
        if order
            .iter()
            .enumerate()
            .all(|(i, (p, n))| i == *p && fs[*p].0 == *n)
        {
            return Ok(self.b(out).prim(Prim::KeyValue, vec![k], ty));
        }
        let mut now: Vec<_> = order
            .iter()
            .map(|(p, n)| (n.clone(), fs[*p].1.clone()))
            .collect();
        now.sort_by(|a, b| a.0.cmp(&b.0));
        let v = self.b(out).prim(Prim::KeyValue, vec![k], Ty::Record(now));
        let mut back = order;
        back.sort();
        let back: Vec<_> = back
            .into_iter()
            .map(|(p, n)| (n, fs[p].0.clone(), fs[p].1.clone()))
            .collect();
        Ok(relaid(self.map.to, out, v, &back))
    }
}

/// Returns a record holding, for each `(from, name, ty)` of `fields`, field `from` of record `x`
/// as its field `name`.
pub(super) fn relaid(
    schema: &Schema,
    out: &mut Terms,
    x: Tm,
    fields: &[(String, String, Ty)],
) -> Tm {
    let mut parts = Vec::with_capacity(fields.len());
    for (f, n, _) in fields {
        let given = match out.kind(x) {
            Kind::Record(vs) => vs.iter().find(|(m, _)| m == f).map(|&(_, v)| v),
            _ => None,
        };
        let v = given.unwrap_or_else(|| Build::new(schema, out).field(x, f.clone()));
        parts.push((n.clone(), v));
    }
    let mut ty: Vec<_> = fields
        .iter()
        .map(|(_, n, t)| (n.clone(), t.clone()))
        .collect();
    ty.sort_by(|a, b| a.0.cmp(&b.0));
    Build::new(schema, out).record(parts, Ty::Record(ty))
}
