use std::collections::BTreeSet;

use super::{Schema, TypeDefKind};
use crate::types::{AheadId, RelId, RelRef, Ty, TypeId};

impl Schema {
    /// Returns the refusal of a relation a later operator of the migration adds when a value of
    /// `ty` can hold a key of it.
    pub fn holds_no_ahead_key(&self, ty: &Ty) -> Result<(), String> {
        match self.ahead_held(ty, &mut BTreeSet::new(), &mut BTreeSet::new()) {
            Some(a) => Err(self.added_later(a)),
            None => Ok(()),
        }
    }

    /// Returns a relation a later operator of the migration adds that a key a value of `ty` can
    /// hold names, through the types and relations not in `defs` and `rels`, adding those it
    /// passes. A tag holds only a constructor's name.
    fn ahead_held(
        &self,
        ty: &Ty,
        defs: &mut BTreeSet<TypeId>,
        rels: &mut BTreeSet<RelId>,
    ) -> Option<AheadId> {
        let mut inner = |t: &Ty| self.ahead_held(t, defs, rels);
        match ty {
            Ty::Key {
                rel: RelRef::Ahead(a),
                ..
            } => Some(*a),
            Ty::Key {
                rel: RelRef::Declared(rel),
                ..
            } => {
                if !rels.insert(*rel) {
                    return None;
                }
                let r = self.relation(*rel);
                let elem = self.ty(r.elem_def);
                let keys: Vec<Ty> = r
                    .key
                    .iter()
                    .filter_map(|k| elem.field_named(0, k).map(|f| f.ty.subst(r.elem_args())))
                    .collect();
                keys.iter().find_map(|t| self.ahead_held(t, defs, rels))
            }
            Ty::Data { def, args, .. } => {
                if let Some(a) = args.iter().find_map(&mut inner) {
                    return Some(a);
                }
                if !defs.insert(*def) {
                    return None;
                }
                match &self.ty(*def).kind {
                    TypeDefKind::Sum { header, ctors, .. } => header
                        .iter()
                        .chain(ctors.iter().flat_map(|c| &c.fields))
                        .find_map(|f| self.ahead_held(&f.ty, defs, rels)),
                    TypeDefKind::Newtype { base, .. } => self.ahead_held(base, defs, rels),
                }
            }
            Ty::Set(t) | Ty::NonEmpty(t) => inner(t),
            Ty::Group(a, b) | Ty::Fn(a, b) => inner(a).or_else(|| inner(b)),
            Ty::Record(fs) => fs.iter().find_map(|(_, t)| inner(t)),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .find_map(|(_, fs)| fs.iter().find_map(|(_, t)| inner(t))),
            Ty::Tag(_) | Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => None,
        }
    }
}
