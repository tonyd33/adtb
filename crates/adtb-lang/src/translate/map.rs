use std::collections::{BTreeMap, BTreeSet};

use crate::schema::Schema;
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
    /// header field's when it is one.
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
                .filter(|(t, _, _)| *t == to)
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

    /// Returns `ty` at the later version.
    pub fn ty(&self, ty: &Ty) -> Result<Ty, Loss> {
        Ok(match ty {
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => ty.clone(),
            Ty::Data { def, args, ctors } => Ty::Data {
                def: self.def(*def).ok_or(Loss::Type(*def))?,
                args: args.iter().map(|a| self.ty(a)).collect::<Result<_, _>>()?,
                ctors: self.set(*def, ctors, false),
            },
            Ty::Key { rel, ctors } => {
                let def = self.from.relation(*rel).elem_def;
                Ty::Key {
                    rel: self.relation(*rel).ok_or(Loss::Relation(*rel))?,
                    ctors: self.set(def, ctors, false),
                }
            }
            Ty::Set(t) => Ty::Set(Box::new(self.ty(t)?)),
            Ty::NonEmpty(t) => Ty::NonEmpty(Box::new(self.ty(t)?)),
            Ty::Group(k, t) => Ty::Group(Box::new(self.ty(k)?), Box::new(self.ty(t)?)),
            Ty::Tag(t) => Ty::Tag(Box::new(self.ty(t)?)),
            Ty::Record(fs) => Ty::Record(
                fs.iter()
                    .map(|(n, t)| Ok((n.clone(), self.ty(t)?)))
                    .collect::<Result<_, Loss>>()?,
            ),
            Ty::Sum { ctors, sel } => Ty::Sum {
                ctors: ctors
                    .iter()
                    .map(|(n, fs)| {
                        let fs = fs
                            .iter()
                            .map(|(f, t)| Ok((f.clone(), self.ty(t)?)))
                            .collect::<Result<_, Loss>>()?;
                        Ok((n.clone(), fs))
                    })
                    .collect::<Result<_, Loss>>()?,
                sel: sel.clone(),
            },
            Ty::Fn(a, b) => Ty::Fn(Box::new(self.ty(a)?), Box::new(self.ty(b)?)),
        })
    }

    /// Refuse a type a value of which does not carry whole to the later version: one reaching a
    /// field the later version lacks, and when `exact`, one the later version adds or a
    /// constructor it lacks, or, when `ordered`, one whose constructors or fields are ordered
    /// otherwise.
    pub fn whole(&self, ty: &Ty, exact: bool, ordered: bool) -> Result<(), Loss> {
        self.whole_in(ty, exact, ordered, &mut BTreeSet::new())
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
        seen: &mut BTreeSet<(TypeId, Vec<u32>)>,
    ) -> Result<(), Loss> {
        match ty {
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change | Ty::Fn(..) | Ty::Key { .. } => {
                Ok(())
            }
            Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => self.whole_in(t, exact, ordered, seen),
            Ty::Group(k, t) => {
                self.whole_in(k, exact, ordered, seen)?;
                self.whole_in(t, exact, ordered, seen)
            }
            Ty::Record(fs) => fs
                .iter()
                .try_for_each(|(_, t)| self.whole_in(t, exact, ordered, seen)),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .flat_map(|(_, fs)| fs)
                .try_for_each(|(_, t)| self.whole_in(t, exact, ordered, seen)),
            Ty::Data { def, args, ctors } => {
                for a in args {
                    self.whole_in(a, exact, ordered, seen)?;
                }
                let set = self.narrowed(*def, ctors);
                if !seen.insert((*def, set.clone())) {
                    return Ok(());
                }
                self.whole_def(*def, args, set, exact, ordered, seen)
            }
        }
    }

    fn whole_def(
        &self,
        def: TypeId,
        args: &[Ty],
        set: Vec<u32>,
        exact: bool,
        ordered: bool,
        seen: &mut BTreeSet<(TypeId, Vec<u32>)>,
    ) -> Result<(), Loss> {
        let name = || self.from.ty(def).name.clone();
        let to = self.def(def).ok_or(Loss::Type(def))?;
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
                images.push(image);
                self.whole_in(t, exact, ordered, seen)?;
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

/// Returns where field `name` of constructor `c` of `def` has its catalog id.
pub fn slot(schema: &Schema, def: TypeId, c: u32, name: &str) -> (TypeId, Option<u32>, String) {
    let header = schema.ty(def).header().iter().any(|f| f.name == name);
    (def, (!header).then_some(c), name.to_string())
}
