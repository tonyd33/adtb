use std::collections::BTreeMap;

use super::{ClauseM, Implied, Item, Model, ViewSlot};
use crate::ids::Slot;
use crate::schema::{FieldRef, ImpliedIndex, PRELUDE, Schema};
use crate::span::FileId;
use crate::types::*;

/// Where each declaration of a file lands in a built schema.
pub(super) struct Found<'a> {
    schema: &'a Schema,
    file: FileId,
}

impl<'a> Found<'a> {
    pub(super) fn new(schema: &'a Schema, file: FileId) -> Found<'a> {
        Found { schema, file }
    }

    pub(super) fn ty(&self) -> Option<TypeId> {
        (0..self.schema.types.len() as u32)
            .map(TypeId)
            .find(|&t| self.schema.ty(t).file == self.file)
    }

    pub(super) fn relation(&self) -> Option<RelId> {
        (0..self.schema.relations.len() as u32)
            .map(RelId)
            .find(|&r| self.schema.relation(r).file == self.file)
    }

    pub(super) fn function(&self) -> Option<FnId> {
        (0..self.schema.functions.len() as u32)
            .map(FnId)
            .find(|&f| self.schema.function(f).file == self.file)
    }

    pub(super) fn aggregate(&self) -> Option<usize> {
        self.schema
            .aggregates
            .iter()
            .position(|a| a.file == self.file)
    }

    pub(super) fn constraint(&self) -> Option<usize> {
        self.schema
            .constraints
            .iter()
            .position(|c| c.file == self.file)
    }

    pub(super) fn check(&self) -> Option<usize> {
        self.schema.checks.iter().position(|c| c.file == self.file)
    }
}

/// Returns the objects type `def` declares, in creation order: the type, its constructors, the
/// header's fields, then each constructor's.
pub(super) fn type_slots(schema: &Schema, def: TypeId) -> Vec<Slot> {
    let td = schema.ty(def);
    let mut out = vec![Slot::Type(def)];
    out.extend(td.ctor_refs().map(Slot::Ctor));
    out.extend(td.field_refs().map(|(f, _)| Slot::Field(f)));
    out
}

impl Model {
    /// Returns the ids `type_slots` lists for type `t`, in its order.
    fn type_ids(&self, t: u64) -> Vec<u64> {
        let mut out = vec![t];
        out.extend(self.ctors_of(t));
        out.extend(self.fields_of(t));
        out
    }

    /// Returns each object of `schema` that a declaration of the model declares, with the id the
    /// model holds for it.
    pub(super) fn placed(&self, schema: &Schema) -> Vec<(Slot, u64)> {
        let mut out = Vec::new();
        let mut fields: BTreeMap<FieldRef, u64> = BTreeMap::new();
        let mut rels: BTreeMap<u64, RelId> = BTreeMap::new();
        let mut fns: BTreeMap<u64, FnId> = BTreeMap::new();
        for (i, d) in self.decls.iter().enumerate() {
            let found = Found::new(schema, FileId(i as u32));
            match d.item {
                Item::Type(t) => {
                    let Some(def) = found.ty() else { continue };
                    let slots = type_slots(schema, def);
                    let ids = self.type_ids(t);
                    if slots.len() != ids.len() {
                        continue;
                    }
                    for (s, id) in slots.into_iter().zip(ids) {
                        if let Slot::Field(f) = s {
                            fields.insert(f, id);
                        }
                        out.push((s, id));
                    }
                }
                Item::Relation(r) => {
                    if let Some(rel) = found.relation() {
                        rels.insert(r, rel);
                        out.push((Slot::Relation(rel), r));
                    }
                }
                Item::Fn(f) => {
                    if let Some(x) = found.function() {
                        fns.insert(f, x);
                        out.push((Slot::Function(x), f));
                    }
                }
                Item::Aggregate(f) => {
                    if let Some(x) = found.aggregate() {
                        out.push((Slot::Aggregate(x), f));
                    }
                }
                Item::Constraint(c) => {
                    if let Some(x) = found.constraint() {
                        out.push((Slot::Constraint(x), c));
                    }
                }
                Item::Check(c) => {
                    if let Some(x) = found.check() {
                        out.push((Slot::Check(x), c));
                    }
                }
                Item::Set(_) | Item::Store(_) | Item::Held(..) => {}
            }
        }
        let field_id = |f: &FieldRef| fields.get(f).copied();
        for (&r, &rel) in &rels {
            out.extend(self.clause_slots(schema, r, rel, &field_id));
        }
        for d in &self.decls {
            match d.item {
                Item::Set(s) => {
                    let set = &self.sets[&s];
                    if let Some(&rel) = rels.get(&set.relation) {
                        out.push((Slot::Set(rel, set.name.clone()), s));
                    }
                }
                Item::Store(v) => {
                    let st = &self.stores[&v];
                    let Some(&f) = fns.get(&st.function) else {
                        continue;
                    };
                    out.push((Slot::View(f), v));
                    for (slot, &id) in &st.ids {
                        out.push((
                            match slot {
                                ViewSlot::Key => Slot::ViewKey(f),
                                ViewSlot::Index(k) => Slot::ViewIndex(f, *k),
                                ViewSlot::Type => Slot::ViewType(f),
                                ViewSlot::Ctor => Slot::ViewCtor(f),
                                ViewSlot::Field(n) => Slot::ViewField(f, n.clone()),
                            },
                            id,
                        ));
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Returns the objects the clauses of relation variable `r`, built as `rel`, declare, and the
    /// indexes its reference fields imply, with their ids.
    fn clause_slots(
        &self,
        schema: &Schema,
        r: u64,
        rel: RelId,
        field_id: &dyn Fn(&FieldRef) -> Option<u64>,
    ) -> Vec<(Slot, u64)> {
        let m = &self.relations[&r];
        let mut out = Vec::new();
        let (mut ix, mut cx) = (0, 0);
        for c in &m.clauses {
            match c {
                ClauseM::Key { id, generate, .. } => {
                    out.extend(id.map(|id| (Slot::Key(rel), id)));
                    if let Some((_, Some(g))) = generate {
                        out.push((Slot::Generated(rel), *g));
                    }
                }
                ClauseM::Index(x) => {
                    out.extend(x.id().map(|id| (Slot::Index(rel, ix), id)));
                    ix += 1;
                }
                ClauseM::Computed { id, .. } => {
                    out.extend(id.map(|id| (Slot::Computed(rel, cx), id)));
                    cx += 1;
                }
                ClauseM::Volatile => {}
            }
        }
        let td = schema.ty(schema.relation(rel).elem_def);
        for (f, _) in td.field_refs() {
            if let Some(d) = field_id(&f).and_then(|x| m.defaults.id_of(x)) {
                out.push((Slot::Default(rel, f), d));
            }
        }
        for i in schema.implied_indexes(rel) {
            if let Some(&id) = implied_key(schema, &i, field_id).and_then(|k| m.implied.get(&k)) {
                out.push((Slot::Implied(rel, i.field_ref()), id));
            }
        }
        out
    }
}

/// Returns the key the model holds implied index `i` by, `field_id` giving a field's id.
pub(super) fn implied_key(
    schema: &Schema,
    i: &ImpliedIndex,
    field_id: impl Fn(&FieldRef) -> Option<u64>,
) -> Option<Implied> {
    let f = i.field_ref();
    let td = schema.ty(f.def());
    let module = schema.module(td.module);
    if td.module == PRELUDE || module.engine {
        return Some(Implied::Builtin(
            format!("{}::{}", module.name, td.name),
            i.field.clone(),
        ));
    }
    field_id(&f).map(Implied::Field)
}
