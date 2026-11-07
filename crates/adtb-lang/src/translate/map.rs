use std::collections::{BTreeMap, BTreeSet};

use crate::schema::{Schema, TypeDefKind};
use crate::textual::{key_field_ty, key_names};
use crate::types::*;

/// The objects of one schema version as they are at another, matched by catalog id; an object
/// of an engine module without an id is matched by its module and name.
#[derive(Debug)]
pub struct Map<'a> {
    pub from: &'a Schema,
    pub to: &'a Schema,
    types: BTreeMap<u64, TypeId>,
    ctors: BTreeMap<u64, (TypeId, u32)>,
    relations: BTreeMap<u64, RelId>,
    fields: BTreeMap<u64, (TypeId, Option<u32>, String)>,
}

/// Why a type cannot be carried between the versions as a whole value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Loss {
    /// The type of the earlier version has no counterpart at the later one.
    Type(TypeId),
    /// The relation of the earlier version has no counterpart at the later one.
    Relation(RelId),
    /// The constructor of the type of the earlier version has no counterpart at the later one.
    Ctor(TypeId, u32),
    /// The field of the type has no counterpart at the other version, added at the later one
    /// when `added` and else dropped by it, with its catalog id at the version that has it.
    Field {
        ty: String,
        field: String,
        id: Option<u64>,
        added: bool,
    },
    /// The type of the earlier version orders its values otherwise at the later one.
    Order(TypeId),
    /// The composite key of the relation of the earlier version orders its values otherwise at
    /// the later one.
    KeyOrder(RelId),
    /// The object of the earlier version named `name` is named otherwise at the later one.
    Renamed {
        part: Part,
        name: String,
        id: Option<u64>,
    },
}

/// The kind of object a catalog id names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Type,
    Ctor,
    Field,
    Relation,
}

impl<'a> Map<'a> {
    pub fn new(from: &'a Schema, to: &'a Schema) -> Map<'a> {
        let ids = &to.ids;
        Map {
            from,
            to,
            types: ids.types.iter().map(|(&t, &id)| (id, t)).collect(),
            ctors: ids.ctors.iter().map(|(&c, &id)| (id, c)).collect(),
            relations: ids.relations.iter().map(|(&r, &id)| (id, r)).collect(),
            fields: ids.fields.iter().map(|(f, &id)| (id, f.clone())).collect(),
        }
    }

    /// Returns the type `def` is at the later version.
    pub fn def(&self, def: TypeId) -> Option<TypeId> {
        match self.from.ids.types.get(&def) {
            Some(id) => self.types.get(id).copied(),
            None => {
                let t = self.from.ty(def);
                let module = &self.from.module(t.module).name;
                self.to
                    .types
                    .iter()
                    .position(|u| u.name == t.name && self.to.module(u.module).name == *module)
                    .map(|i| TypeId(i as u32))
            }
        }
    }

    pub fn relation(&self, rel: RelId) -> Option<RelId> {
        match self.from.ids.relations.get(&rel) {
            Some(id) => self.relations.get(id).copied(),
            None => {
                let r = self.from.relation(rel);
                let module = &self.from.module(r.module).name;
                self.to
                    .relations
                    .iter()
                    .position(|u| u.name == r.name && self.to.module(u.module).name == *module)
                    .map(|i| RelId(i as u32))
            }
        }
    }

    /// Returns the index constructor `c` of `def` has at the later version.
    pub fn ctor(&self, def: TypeId, c: u32) -> Option<u32> {
        let to = self.def(def)?;
        match self.from.ids.ctors.get(&(def, c)) {
            Some(id) => self
                .ctors
                .get(id)
                .filter(|(t, _)| *t == to)
                .map(|&(_, k)| k),
            None => {
                let name = &self.from.ty(def).ctors().get(c as usize)?.name;
                self.to.ty(to).ctor_index(name)
            }
        }
    }

    /// Returns the constructor of `def` at the earlier version that `c` of its image is.
    pub fn ctor_back(&self, def: TypeId, c: u32) -> Option<u32> {
        (0..self.from.ty(def).ctors().len() as u32).find(|&k| self.ctor(def, k) == Some(c))
    }

    /// Returns the name field `name` of constructor `c` of `def` has at the later version, a
    /// header field's when it is one; `None` when it sits in another constructor there.
    pub fn field(&self, def: TypeId, c: Option<u32>, name: &str) -> Option<String> {
        let to = self.def(def)?;
        let t = self.from.ty(def);
        let slot = if t.header().iter().any(|f| f.name == name) {
            None
        } else {
            c
        };
        match self.from.ids.fields.get(&(def, slot, name.to_string())) {
            Some(id) => self
                .fields
                .get(id)
                .filter(|(t, k, _)| {
                    *t == to && (k.is_none() || *k == c.and_then(|c| self.ctor(def, c)))
                })
                .map(|(_, _, n)| n.clone()),
            None if self.from.ids.types.contains_key(&def) => None,
            None => {
                let k = c.and_then(|c| self.ctor(def, c)).unwrap_or(0);
                self.to.ty(to).field(k, name).map(|f| f.name.clone())
            }
        }
    }

    /// Returns the fields of constructor `c` of `def`, header first, with their types.
    pub fn fields(schema: &Schema, def: TypeId, args: &[Ty], c: u32) -> Vec<(String, Ty)> {
        let t = schema.ty(def);
        t.header()
            .iter()
            .chain(
                t.ctors()
                    .get(c as usize)
                    .into_iter()
                    .flat_map(|k| &k.fields),
            )
            .map(|f| (f.name.clone(), f.ty.subst(args)))
            .collect()
    }

    /// Returns the constructors a value of `ty` may hold, `None` for a type that is no declared
    /// sum.
    pub fn ctor_set(schema: &Schema, ty: &Ty) -> Option<Vec<u32>> {
        match ty {
            Ty::Data { def, ctors, .. } => Some(
                ctors
                    .clone()
                    .unwrap_or_else(|| (0..schema.ty(*def).ctors().len() as u32).collect()),
            ),
            Ty::Tag(t) => Self::ctor_set(schema, t),
            _ => None,
        }
    }

    /// Returns the constructor set `set` of `def` at the later version, less the constructors
    /// it no longer has.
    pub fn set(&self, def: TypeId, set: &Option<CtorSet>, hide_new: bool) -> Option<CtorSet> {
        let to = self.def(def)?;
        let set = match set {
            Some(s) => s.clone(),
            None if hide_new => (0..self.from.ty(def).ctors().len() as u32).collect(),
            None => return None,
        };
        let mut out: Vec<u32> = set.iter().filter_map(|&c| self.ctor(def, c)).collect();
        out.sort_unstable();
        out.dedup();
        self.to.normalize_ctors(to, out)
    }

    /// Returns `ty` at the later version, a row or value naming the constructors it holds at the
    /// earlier version, and a key narrowed to them when `hide_new`.
    pub fn ty(&self, ty: &Ty, hide_new: bool) -> Result<Ty, Loss> {
        self.ty_at(ty, hide_new, true)
    }

    /// Returns `ty` at the later version with its constructor sets not narrowed, for a value read
    /// from where a constructor new since the earlier version can lie.
    pub fn ty_unnarrowed(&self, ty: &Ty) -> Result<Ty, Loss> {
        self.ty_at(ty, false, false)
    }

    fn ty_at(&self, ty: &Ty, hide_new: bool, pinned: bool) -> Result<Ty, Loss> {
        Ok(match ty {
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => ty.clone(),
            Ty::Data { def, args, ctors } => Ty::Data {
                def: self.def(*def).ok_or(Loss::Type(*def))?,
                args: args
                    .iter()
                    .map(|a| self.ty_at(a, hide_new, pinned))
                    .collect::<Result<_, _>>()?,
                ctors: self.set(*def, ctors, pinned),
            },
            Ty::Key { rel, ctors } => {
                let def = self.from.relation(*rel).elem_def;
                Ty::Key {
                    rel: self.relation(*rel).ok_or(Loss::Relation(*rel))?,
                    ctors: self.set(def, ctors, hide_new),
                }
            }
            Ty::Set(t) => Ty::Set(Box::new(self.ty_at(t, hide_new, pinned)?)),
            Ty::NonEmpty(t) => Ty::NonEmpty(Box::new(self.ty_at(t, hide_new, pinned)?)),
            Ty::Group(k, t) => Ty::Group(
                Box::new(self.ty_at(k, hide_new, pinned)?),
                Box::new(self.ty_at(t, hide_new, pinned)?),
            ),
            Ty::Tag(t) => Ty::Tag(Box::new(self.ty_at(t, hide_new, pinned)?)),
            Ty::Record(fs) => Ty::Record(
                fs.iter()
                    .map(|(n, t)| Ok((n.clone(), self.ty_at(t, hide_new, pinned)?)))
                    .collect::<Result<_, Loss>>()?,
            ),
            Ty::Sum { ctors, sel } => Ty::Sum {
                ctors: ctors
                    .iter()
                    .map(|(n, fs)| {
                        let fs = fs
                            .iter()
                            .map(|(f, t)| Ok((f.clone(), self.ty_at(t, hide_new, pinned)?)))
                            .collect::<Result<_, Loss>>()?;
                        Ok((n.clone(), fs))
                    })
                    .collect::<Result<_, Loss>>()?,
                sel: sel.clone(),
            },
            Ty::Fn(a, b) => Ty::Fn(
                Box::new(self.ty_at(a, hide_new, pinned)?),
                Box::new(self.ty_at(b, hide_new, pinned)?),
            ),
        })
    }

    /// Refuse a type a value of which does not carry whole to the later version: one reaching a
    /// field the later version lacks, and when `exact`, one the later version adds or a
    /// constructor it lacks, or, when `ordered`, one whose constructors or fields are ordered
    /// otherwise.
    pub fn whole(&self, ty: &Ty, exact: bool, ordered: bool) -> Result<(), Loss> {
        self.whole_in(ty, exact, ordered, false, &mut BTreeSet::new())
    }

    /// Refuse a type whose values print otherwise at the later version: one that does not carry
    /// whole, exactly and ordered, or reaches a constructor, field, newtype or relation renamed.
    pub fn shown(&self, ty: &Ty) -> Result<(), Loss> {
        self.whole_in(ty, true, true, true, &mut BTreeSet::new())
    }

    /// Refuse a key of `rel` whose text differs at the later version: its relation, a field
    /// of a composite key, or a key field's type named otherwise.
    fn key_shown(&self, rel: RelId, seen: &mut BTreeSet<(TypeId, Vec<u32>)>) -> Result<(), Loss> {
        let to = self.relation(rel).ok_or(Loss::Relation(rel))?;
        let r = self.from.relation(rel);
        if r.name != self.to.relation(to).name {
            return Err(renamed(
                Part::Relation,
                &r.name,
                self.from.ids.relations.get(&rel),
            ));
        }
        for f in &r.key {
            if r.key.len() > 1 && self.field(r.elem_def, Some(0), f).as_ref() != Some(f) {
                let id = self.from.ids.fields.get(&slot(self.from, r.elem_def, 0, f));
                return Err(renamed(Part::Field, f, id));
            }
            let ty = key_field_ty(self.from, rel, f);
            self.whole_in(&ty, true, true, true, seen)?;
        }
        Ok(())
    }

    /// Returns, for each field of the composite key of `rel` in its layout at the later version,
    /// the position of the field it is in the layout at the earlier version and its name at the
    /// later one; `None` when both lay the key out alike or `rel` has no counterpart.
    ///
    /// Fail when a field of the key at either version has no counterpart at the other.
    pub fn key_order(&self, rel: RelId) -> Result<Option<Vec<(usize, String)>>, Loss> {
        let Some(later) = self.relation(rel) else {
            return Ok(None);
        };
        let r = self.from.relation(rel);
        let pinned = key_names(self.from, rel);
        let image = |f: &String| {
            self.field(r.elem_def, Some(0), f)
                .ok_or_else(|| Loss::Field {
                    ty: self.from.ty(r.elem_def).name.clone(),
                    field: f.clone(),
                    id: self
                        .from
                        .ids
                        .fields
                        .get(&slot(self.from, r.elem_def, 0, f))
                        .copied(),
                    added: false,
                })
        };
        let images = pinned.iter().map(image).collect::<Result<Vec<_>, _>>()?;
        let order = key_names(self.to, later)
            .into_iter()
            .map(|n| match images.iter().position(|i| *i == n) {
                Some(p) => Ok((p, n)),
                None => Err(Loss::Field {
                    ty: self.to.ty(self.to.relation(later).elem_def).name.clone(),
                    id: self
                        .to
                        .ids
                        .fields
                        .get(&slot(self.to, self.to.relation(later).elem_def, 0, &n))
                        .copied(),
                    field: n,
                    added: true,
                }),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let moved = order.iter().enumerate().any(|(i, &(p, _))| i != p);
        Ok(moved.then_some(order))
    }

    /// Returns the constructors `def` narrowed to `ctors` may hold, at the earlier version.
    fn narrowed(&self, def: TypeId, ctors: &Option<CtorSet>) -> Vec<u32> {
        Self::ctor_set(
            self.from,
            &Ty::Data {
                def,
                args: Vec::new(),
                ctors: ctors.clone(),
            },
        )
        .unwrap_or_default()
    }

    fn whole_in(
        &self,
        ty: &Ty,
        exact: bool,
        ordered: bool,
        named: bool,
        seen: &mut BTreeSet<(TypeId, Vec<u32>)>,
    ) -> Result<(), Loss> {
        let mut inner = |t: &Ty| self.whole_in(t, exact, ordered, named, seen);
        match ty {
            Ty::Key { rel, .. } if named => self.key_shown(*rel, seen),
            Ty::Key { rel, .. } => {
                if self.key_order(*rel)?.is_some() && ordered {
                    return Err(Loss::KeyOrder(*rel));
                }
                let r = self.from.relation(*rel);
                r.key
                    .iter()
                    .try_for_each(|f| inner(&key_field_ty(self.from, *rel, f)))
            }
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change | Ty::Fn(..) => Ok(()),
            Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => inner(t),
            Ty::Group(k, t) => {
                inner(k)?;
                inner(t)
            }
            Ty::Record(fs) => fs.iter().try_for_each(|(_, t)| inner(t)),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .flat_map(|(_, fs)| fs)
                .try_for_each(|(_, t)| inner(t)),
            Ty::Data { def, args, ctors } => {
                for a in args {
                    inner(a)?;
                }
                let set = self.narrowed(*def, ctors);
                if !seen.insert((*def, set.clone())) {
                    return Ok(());
                }
                self.whole_def(*def, args, set, (exact, ordered, named), seen)
            }
        }
    }

    fn whole_def(
        &self,
        def: TypeId,
        args: &[Ty],
        set: Vec<u32>,
        (exact, ordered, named): (bool, bool, bool),
        seen: &mut BTreeSet<(TypeId, Vec<u32>)>,
    ) -> Result<(), Loss> {
        let name = || self.from.ty(def).name.clone();
        let to = self.def(def).ok_or(Loss::Type(def))?;
        let named_by_type = matches!(
            self.from.ty(def).kind,
            TypeDefKind::Newtype { .. } | TypeDefKind::Sum { record: true, .. }
        );
        if named && named_by_type && self.from.ty(def).name != self.to.ty(to).name {
            return Err(renamed(Part::Type, &name(), self.from.ids.types.get(&def)));
        }
        let mut last = None;
        for c in set {
            let Some(k) = self.ctor(def, c) else {
                if exact {
                    return Err(Loss::Ctor(def, c));
                }
                continue;
            };
            if ordered && last.is_some_and(|l| l >= k) {
                return Err(Loss::Order(def));
            }
            last = Some(k);
            let ctor = &self.from.ty(def).ctors()[c as usize].name;
            if named && *ctor != self.to.ty(to).ctors()[k as usize].name {
                return Err(renamed(
                    Part::Ctor,
                    ctor,
                    self.from.ids.ctors.get(&(def, c)),
                ));
            }
            let fs = Self::fields(self.from, def, args, c);
            let mut images = Vec::with_capacity(fs.len());
            for (f, t) in &fs {
                let image = self.field(def, Some(c), f).ok_or_else(|| Loss::Field {
                    ty: name(),
                    field: f.clone(),
                    id: self
                        .from
                        .ids
                        .fields
                        .get(&slot(self.from, def, c, f))
                        .copied(),
                    added: false,
                })?;
                if named && image != *f {
                    let id = self.from.ids.fields.get(&slot(self.from, def, c, f));
                    return Err(renamed(Part::Field, f, id));
                }
                images.push(image);
                self.whole_in(t, exact, ordered, named, seen)?;
            }
            let later: Vec<String> = Self::fields(self.to, to, &[], k)
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            if exact && let Some(added) = later.iter().find(|n| !images.contains(n)) {
                return Err(Loss::Field {
                    ty: name(),
                    field: added.clone(),
                    id: self
                        .to
                        .ids
                        .fields
                        .get(&slot(self.to, to, k, added))
                        .copied(),
                    added: true,
                });
            }
            if ordered && images != later {
                return Err(Loss::Order(def));
            }
        }
        Ok(())
    }
}

fn renamed(part: Part, name: &str, id: Option<&u64>) -> Loss {
    Loss::Renamed {
        part,
        name: name.to_string(),
        id: id.copied(),
    }
}

/// Returns where field `name` of constructor `c` of `def` has its catalog id.
pub fn slot(schema: &Schema, def: TypeId, c: u32, name: &str) -> (TypeId, Option<u32>, String) {
    let header = schema.ty(def).header().iter().any(|f| f.name == name);
    (def, (!header).then_some(c), name.to_string())
}
