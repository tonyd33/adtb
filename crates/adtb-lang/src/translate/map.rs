use std::collections::BTreeSet;

use crate::ids::{CatCtorId, CatFieldId, CatRelId, CatTypeId};
use crate::schema::{CtorRef, Schema, TypeDefKind};
use crate::textual::{key_field_ty, key_names};
use crate::types::*;

/// The objects of one schema version as they are at another, matched by catalog id; an object
/// of an engine module without an id is matched by its module and name.
#[derive(Debug)]
pub struct Map<'a> {
    pub from: &'a Schema,
    pub to: &'a Schema,
    /// What each relation the earlier version takes as added later is at the later version, by
    /// its `AheadId` index.
    ahead: Vec<Later>,
}

/// What a relation the earlier version takes as added later is at the later version.
#[derive(Clone, Debug)]
pub(super) enum Later {
    At(RelId),
    /// The later version lacks the relation of this name.
    Lacked(String),
    /// A key of it breaks this invariant.
    Unlinked(String),
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
    /// The relation of this name, which the earlier version takes as added later, has no
    /// counterpart at the later version.
    Later(String),
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
        Map::over(from, to, Vec::new())
    }

    /// Returns the map from `from` to `to` where the relation `from` takes as added later at
    /// `AheadId::at(i)` is `ahead[i]`.
    pub(super) fn over(from: &'a Schema, to: &'a Schema, ahead: Vec<Later>) -> Map<'a> {
        Map { from, to, ahead }
    }

    /// Returns the relation of the earlier version a key of `rel` reads through.
    ///
    /// Preconditions: `rel` is declared at the earlier version.
    pub(super) fn read_through(&self, rel: RelRef) -> RelId {
        self.from.relation_of(rel).unwrap_or_else(|e| {
            panic!("a checked term reads through no key of a relation added later: {e}")
        })
    }

    /// Returns the type `def` is at the later version.
    pub fn def(&self, def: TypeId) -> Option<TypeId> {
        match self.from.ids().type_id(def) {
            Some(id) => self.to.ids().type_of(id),
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
        match self.from.ids().relation_id(rel) {
            Some(id) => self.to.ids().relation_of(id),
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
        let from = self.from.ty(def).ctor_at(c)?;
        match self.from.ids().ctor_id(from) {
            Some(id) => self
                .to
                .ids()
                .ctor_of(id)
                .filter(|k| k.def() == to)
                .map(CtorRef::index),
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
        let found = t
            .field(c.and_then(|c| t.ctor_at(c)), name)
            .and_then(|f| self.from.ids().field_id(f));
        match found {
            Some(id) => self
                .to
                .ids()
                .field_of(id)
                .filter(|f| {
                    f.def() == to
                        && (f.ctor().is_none() || f.ctor() == c.and_then(|c| self.ctor(def, c)))
                })
                .map(|f| self.to.ty(to).field_def(f).name.clone()),
            None if self.from.ids().type_id(def).is_some() => None,
            None => {
                let k = c.and_then(|c| self.ctor(def, c)).unwrap_or(0);
                self.to.ty(to).field_named(k, name).map(|f| f.name.clone())
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

    /// Returns `ty` at the later version, a row naming the constructors it holds at the earlier
    /// version.
    pub fn ty(&self, ty: &Ty) -> Result<Ty, Loss> {
        self.ty_at(ty, Narrow::Rows)
    }

    /// Returns `ty` at the later version with its constructor sets not narrowed, for a value read
    /// from where a constructor new since the earlier version can lie.
    pub fn ty_unnarrowed(&self, ty: &Ty) -> Result<Ty, Loss> {
        self.ty_at(ty, Narrow::None)
    }

    /// Returns `ty` at the later version, every declared sum in it naming the constructors it
    /// holds at the earlier version, for a value a parse builds.
    pub fn ty_parsed(&self, ty: &Ty) -> Result<Ty, Loss> {
        self.ty_at(ty, Narrow::All)
    }

    fn ty_at(&self, ty: &Ty, pinned: Narrow) -> Result<Ty, Loss> {
        Ok(match ty {
            Ty::Base(_) | Ty::Param(_) | Ty::Unit | Ty::Change => ty.clone(),
            Ty::Data { def, args, ctors } => Ty::Data {
                def: self.def(*def).ok_or(Loss::Type(*def))?,
                args: args
                    .iter()
                    .map(|a| self.ty_at(a, pinned))
                    .collect::<Result<_, _>>()?,
                ctors: self.set(
                    *def,
                    ctors,
                    pinned == Narrow::All || pinned == Narrow::Rows && self.is_row(*def),
                ),
            },
            Ty::Key {
                rel: RelRef::Ahead(a),
                ..
            } => match &self.ahead[a.index()] {
                Later::At(rel) => Ty::Key {
                    rel: RelRef::Declared(*rel),
                    ctors: None,
                },
                Later::Lacked(name) => return Err(Loss::Later(name.clone())),
                Later::Unlinked(rule) => panic!("{rule}"),
            },
            Ty::Key {
                rel: RelRef::Declared(rel),
                ctors,
            } => {
                let rel = *rel;
                let def = self.from.relation(rel).elem_def;
                Ty::Key {
                    rel: RelRef::Declared(self.relation(rel).ok_or(Loss::Relation(rel))?),
                    ctors: self.set(def, ctors, false),
                }
            }
            Ty::Set(t) => Ty::Set(Box::new(self.ty_at(t, pinned)?)),
            Ty::NonEmpty(t) => Ty::NonEmpty(Box::new(self.ty_at(t, pinned)?)),
            Ty::Group(k, t) => Ty::Group(
                Box::new(self.ty_at(k, pinned)?),
                Box::new(self.ty_at(t, pinned)?),
            ),
            Ty::Tag(t) => Ty::Tag(Box::new(self.ty_at(t, pinned)?)),
            Ty::Record(fs) => Ty::Record(
                fs.iter()
                    .map(|(n, t)| Ok((n.clone(), self.ty_at(t, pinned)?)))
                    .collect::<Result<_, Loss>>()?,
            ),
            Ty::Sum { ctors, sel } => Ty::Sum {
                ctors: ctors
                    .iter()
                    .map(|(n, fs)| {
                        let fs = fs
                            .iter()
                            .map(|(f, t)| Ok((f.clone(), self.ty_at(t, pinned)?)))
                            .collect::<Result<_, Loss>>()?;
                        Ok((n.clone(), fs))
                    })
                    .collect::<Result<_, Loss>>()?,
                sel: sel.clone(),
            },
            Ty::Fn(a, b) => Ty::Fn(
                Box::new(self.ty_at(a, pinned)?),
                Box::new(self.ty_at(b, pinned)?),
            ),
        })
    }

    /// Returns whether `def`, a type of the earlier version, is the row type of a relation.
    fn is_row(&self, def: TypeId) -> bool {
        self.from.relations.iter().any(|r| r.elem_def == def)
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
                self.from.ids().relation_id(rel).map(CatRelId::raw),
            ));
        }
        for f in &r.key {
            if r.key.len() > 1 && self.field(r.elem_def, Some(0), f).as_ref() != Some(f) {
                let id = field_id(self.from, r.elem_def, 0, f);
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
        let order = self.key_layout(rel)?;
        Ok(order.filter(|o| o.iter().enumerate().any(|(i, &(p, _))| i != p)))
    }

    /// Returns, for each field of the composite key of `rel` in its layout at the later version,
    /// the position of the field of the same catalog id in the layout at the earlier version and
    /// its name at the later one; `None` when `rel` has no counterpart.
    ///
    /// Fail when a field of the key at either version has no counterpart at the other.
    pub fn key_layout(&self, rel: RelId) -> Result<Option<Vec<(usize, String)>>, Loss> {
        let Some(later) = self.relation(rel) else {
            return Ok(None);
        };
        let def = self.from.relation(rel).elem_def;
        let later_def = self.to.relation(later).elem_def;
        let pinned: Vec<Option<u64>> = key_names(self.from, rel)
            .iter()
            .map(|f| field_id(self.from, def, 0, f))
            .collect();
        let mut kept = vec![false; pinned.len()];
        let mut order = Vec::with_capacity(pinned.len());
        for n in key_names(self.to, later) {
            let id = field_id(self.to, later_def, 0, &n);
            let Some(p) = pinned.iter().position(|i| i.is_some() && *i == id) else {
                return Err(Loss::Field {
                    ty: self.to.ty(later_def).name.clone(),
                    id,
                    field: n,
                    added: true,
                });
            };
            kept[p] = true;
            order.push((p, n));
        }
        if let Some(p) = kept.iter().position(|k| !k) {
            let f = key_names(self.from, rel).swap_remove(p);
            return Err(Loss::Field {
                ty: self.from.ty(def).name.clone(),
                id: pinned[p],
                field: f,
                added: false,
            });
        }
        Ok(Some(order))
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
            Ty::Key {
                rel: RelRef::Declared(rel),
                ..
            } if named => self.key_shown(*rel, seen),
            Ty::Key {
                rel: RelRef::Declared(rel),
                ..
            } => {
                if self.key_order(*rel)?.is_some() && ordered {
                    return Err(Loss::KeyOrder(*rel));
                }
                let r = self.from.relation(*rel);
                r.key
                    .iter()
                    .try_for_each(|f| inner(&key_field_ty(self.from, *rel, f)))
            }
            Ty::Key {
                rel: RelRef::Ahead(_),
                ..
            }
            | Ty::Base(_)
            | Ty::Param(_)
            | Ty::Unit
            | Ty::Change
            | Ty::Fn(..) => Ok(()),
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
            return Err(renamed(
                Part::Type,
                &name(),
                self.from.ids().type_id(def).map(CatTypeId::raw),
            ));
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
                    self.from
                        .ty(def)
                        .ctor_at(c)
                        .and_then(|c| self.from.ids().ctor_id(c))
                        .map(CatCtorId::raw),
                ));
            }
            let fs = Self::fields(self.from, def, args, c);
            let mut images = Vec::with_capacity(fs.len());
            for (f, t) in &fs {
                let image = self.field(def, Some(c), f).ok_or_else(|| Loss::Field {
                    ty: name(),
                    field: f.clone(),
                    id: field_id(self.from, def, c, f),
                    added: false,
                })?;
                if named && image != *f {
                    let id = field_id(self.from, def, c, f);
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
                    id: field_id(self.to, to, k, added),
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

/// Which declared sums a mapped type names at the constructors of the earlier version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Narrow {
    None,
    Rows,
    All,
}

fn renamed(part: Part, name: &str, id: Option<u64>) -> Loss {
    Loss::Renamed {
        part,
        name: name.to_string(),
        id,
    }
}

/// Returns the catalog id of field `name` of constructor `c` of `def`, a header field's when the
/// header has one by that name.
pub fn field_id(schema: &Schema, def: TypeId, c: u32, name: &str) -> Option<u64> {
    let td = schema.ty(def);
    let f = td.field(td.ctor_at(c), name)?;
    schema.ids().field_id(f).map(CatFieldId::raw)
}
