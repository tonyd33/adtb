use super::*;
use crate::ids::{Owner, Space};

/// A named object, by its catalog id.
#[derive(Clone, Copy, Debug)]
pub(in crate::evolve) enum NamedId {
    Type(CatTypeId),
    Ctor(CatCtorId),
    Relation(CatRelId),
    Set(CatSetId),
    Field(CatFieldId),
}

impl NamedId {
    pub(super) fn word(self) -> &'static str {
        match self {
            NamedId::Type(_) => "type",
            NamedId::Ctor(_) => "constructor",
            NamedId::Relation(_) => "relation",
            NamedId::Set(_) => "set",
            NamedId::Field(_) => "field",
        }
    }

    /// Returns the module of the object of the id in `schema` and its name in a rename
    /// operator, `None` when no object of `schema` has the id.
    pub(super) fn spelled(self, schema: &Schema) -> Option<(String, String)> {
        let ty = |def: TypeId| {
            let td = schema.ty(def);
            let m = schema.module(td.module).name.clone();
            let q = format!("{m}::{}", td.name);
            (m, q)
        };
        match self {
            NamedId::Type(id) => Some(ty(schema.ids.type_of(id)?)),
            NamedId::Ctor(id) => {
                let c = schema.ids.ctor_of(id)?;
                let (m, t) = ty(c.def());
                Some((m, format!("{t}.{}", schema.ty(c.def()).ctor_def(c).name)))
            }
            NamedId::Relation(id) => {
                let rd = schema.relation(schema.ids.relation_of(id)?);
                let m = schema.module(rd.module).name.clone();
                Some((m.clone(), format!("{m}::{}", rd.name)))
            }
            NamedId::Set(id) => {
                let (r, name) = schema.ids.set_of(id)?;
                let rd = schema.relation(r);
                let m = schema.module(rd.module).name.clone();
                Some((m.clone(), format!("{name} of {m}::{}", rd.name)))
            }
            NamedId::Field(id) => {
                let f = schema.ids.field_of(id)?;
                let td = schema.ty(f.def());
                let (m, t) = ty(f.def());
                let name = &td.field_def(f).name;
                Some(match f.ctor() {
                    Some(c) if !matches!(td.kind, TypeDefKind::Sum { record: true, .. }) => {
                        (m, format!("{t}.{}.{name}", td.ctors()[c as usize].name))
                    }
                    _ => (m, format!("{t}.{name}")),
                })
            }
        }
    }
}

/// The id each object an inverse created again takes, by the space and id it had.
pub(super) type Renewed = BTreeMap<(Space, u64), u64>;

/// Record in `renewed`, after a step restoring declarations, the id each named object of `now`
/// that `held` lacks takes, when a source of `was` held an object of the same kind and name, with
/// the same owner or one created again as it, under an id `now` does not hold.
pub(super) fn renew(
    renewed: &mut Renewed,
    was: &[Source],
    held: &BTreeMap<ObjectKey, u64>,
    now: &BTreeMap<ObjectKey, u64>,
) {
    let holding: BTreeSet<(Space, u64)> =
        now.iter().map(|(k, &id)| (k.kind().space(), id)).collect();
    let mut fresh: Vec<(&ObjectKey, u64)> = now
        .iter()
        .filter(|(k, _)| !held.contains_key(k))
        .map(|(k, &id)| (k, id))
        .collect();
    fresh.sort_by_key(|(k, id)| (k.kind().space(), *id));
    for (k, id) in fresh {
        let Some(name) = k.name() else {
            continue;
        };
        let space = k.kind().space();
        let old = was.iter().flat_map(|s| &s.ids).find(|(o, oid)| {
            o.kind() == k.kind()
                && o.name() == Some(name)
                && !holding.contains(&(space, **oid))
                && renewed_owner(o.owner(), renewed) == *k.owner()
        });
        if let Some((_, &oid)) = old {
            renewed.insert((space, oid), id);
        }
    }
}

fn renewed_owner(o: &Owner, renewed: &Renewed) -> Owner {
    match o {
        Owner::Object(space, id) => Owner::Object(*space, renewed_id(renewed, *space, *id)),
        Owner::Module(m) => Owner::Module(m.clone()),
    }
}

fn renewed_id(renewed: &Renewed, space: Space, id: u64) -> u64 {
    renewed.get(&(space, id)).copied().unwrap_or(id)
}

impl Inverse {
    /// Returns the step with each id an earlier step of the revert created again replaced by
    /// the id `renewed` maps it to.
    pub(super) fn renewed(self, renewed: &Renewed) -> Inverse {
        let shared = |id: u64| renewed_id(renewed, Space::Shared, id);
        let rel = |r: CatRelId| CatRelId::from_raw(shared(r.raw()));
        match self {
            Inverse::Rename { object, to, of } => Inverse::Rename {
                object: match object {
                    NamedId::Type(t) => NamedId::Type(CatTypeId::from_raw(shared(t.raw()))),
                    NamedId::Ctor(c) => NamedId::Ctor(CatCtorId::from_raw(shared(c.raw()))),
                    NamedId::Relation(r) => NamedId::Relation(rel(r)),
                    NamedId::Set(x) => {
                        NamedId::Set(CatSetId::from_raw(renewed_id(renewed, Space::Set, x.raw())))
                    }
                    NamedId::Field(f) => NamedId::Field(CatFieldId::from_raw(renewed_id(
                        renewed,
                        Space::Field,
                        f.raw(),
                    ))),
                },
                to,
                of,
            },
            Inverse::DropSet { relation, name, of } => Inverse::DropSet {
                relation: rel(relation),
                name,
                of,
            },
            Inverse::AddSet {
                set,
                relation,
                name,
                members,
                of,
            } => Inverse::AddSet {
                set,
                relation: rel(relation),
                name,
                members: members
                    .into_iter()
                    .map(|c| CatCtorId::from_raw(shared(c.raw())))
                    .collect(),
                of,
            },
            other => other,
        }
    }
}

/// Returns the refusal of the inverse of operator text `of` whose object of kind `word` is gone.
pub(super) fn dropped(of: &str, word: &str) -> String {
    format!("the inverse of `{of}` names a {word} a later migration dropped")
}
