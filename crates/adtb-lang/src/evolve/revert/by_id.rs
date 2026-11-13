use super::*;
use crate::ids::{KeyKind, Owner, Space};

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

impl NamedId {
    /// Returns the name of the type or relation of the id in `src`, qualified by its module,
    /// `None` when `src` holds no type or relation of the id.
    pub(super) fn qualified(self, src: &Source) -> Option<QName> {
        let (kind, id) = match self {
            NamedId::Type(t) => (KeyKind::Type, t.raw()),
            NamedId::Relation(r) => (KeyKind::Relation, r.raw()),
            _ => return None,
        };
        let key = src
            .ids
            .iter()
            .find(|(k, x)| **x == id && k.kind() == kind)?
            .0;
        let Owner::Module(m) = key.owner() else {
            return None;
        };
        let ident = |text: &str| ast::Ident {
            text: text.to_string(),
            span: Span::new(0, 0),
        };
        Some(QName {
            module: Some(ident(m)),
            name: ident(key.name()?),
        })
    }
}

/// A declaration, by the id of the object it declares, a stored view's by the id of its
/// function.
#[derive(Clone, Copy, Debug)]
pub(in crate::evolve) enum DeclOf {
    Object(KeyKind, u64),
    Store(u64),
}

impl DeclOf {
    pub(super) fn word(self) -> &'static str {
        match self {
            DeclOf::Object(kind, _) => spelled::word(kind),
            DeclOf::Store(_) => "function",
        }
    }

    /// Returns the declaration in `src` with its kind, module and name, `None` when `src` holds
    /// no object of the id.
    pub(super) fn find(self, src: &Source) -> Option<(usize, &'static str, String, String)> {
        let (kind, id) = match self {
            DeclOf::Object(kind, id) => (kind, id),
            DeclOf::Store(f) => (KeyKind::Function, f),
        };
        let key = src
            .ids
            .iter()
            .find(|(k, x)| **x == id && k.kind() == kind)?
            .0;
        let name = key.name()?.to_string();
        let (module, name) = match key.owner() {
            Owner::Module(m) => (m.clone(), name),
            Owner::Object(space, rel) => {
                let owner = src
                    .ids
                    .iter()
                    .find(|(k, x)| **x == *rel && k.kind().space() == *space)?
                    .0;
                let Owner::Module(m) = owner.owner() else {
                    return None;
                };
                (m.clone(), format!("{}.{name}", owner.name()?))
            }
        };
        let kinds: &[&'static str] = match (self, kind) {
            (DeclOf::Store(_), _) => &["store"],
            (_, KeyKind::Type) => &["type"],
            (_, KeyKind::Relation) => &["relation"],
            (_, KeyKind::Set) => &["set"],
            (_, KeyKind::Function) => &["fn", "aggregate"],
            (_, KeyKind::Constraint) => &["constraint", "check"],
            _ => &[],
        };
        kinds.iter().find_map(|&k| {
            let i = src.find(k, &module, &name)?;
            Some((i, k, module.clone(), name.clone()))
        })
    }

    fn mapped(self, again: &Recreated) -> DeclOf {
        match self {
            DeclOf::Object(kind, id) => DeclOf::Object(kind, again_id(again, kind.space(), id)),
            DeclOf::Store(f) => DeclOf::Store(again_id(again, Space::Function, f)),
        }
    }
}

/// The id each object an inverse created again takes, by the space and id it had.
pub(in crate::evolve) type Recreated = BTreeMap<(Space, u64), u64>;

pub(super) fn again_id(again: &Recreated, space: Space, id: u64) -> u64 {
    again.get(&(space, id)).copied().unwrap_or(id)
}

/// Returns the space and id of each object of `ids`.
pub(super) fn holding(ids: &BTreeMap<ObjectKey, u64>) -> BTreeSet<(Space, u64)> {
    ids.iter().map(|(k, &id)| (k.kind().space(), id)).collect()
}

/// Returns each object `before` holds and `after` does not, by its key in `before`, with its id.
pub(super) fn removed(before: &Source, after: &Source) -> Vec<(ObjectKey, u64)> {
    let kept = holding(&after.ids);
    before
        .ids
        .iter()
        .filter(|(k, id)| !kept.contains(&(k.kind().space(), **id)))
        .map(|(k, &id)| (k.clone(), id))
        .collect()
}

/// Record in `again`, for each object of `removed` that `now` holds again under an id `held`
/// lacks and `again` gives no other object, the id it takes: the id `now` gives its key with every
/// id of the key that `again` maps replaced.
pub(super) fn recreate(
    again: &mut Recreated,
    removed: &[(ObjectKey, u64)],
    held: &BTreeSet<(Space, u64)>,
    now: &BTreeMap<ObjectKey, u64>,
) {
    loop {
        let mut more = false;
        for (key, old) in removed {
            let space = key.kind().space();
            if again.contains_key(&(space, *old)) {
                continue;
            }
            let k = key.mapped(|s, id| again_id(again, s, id));
            if let Some(&id) = now.get(&k)
                && !held.contains(&(space, id))
                && !again.iter().any(|(&(s, _), &x)| s == space && x == id)
            {
                again.insert((space, *old), id);
                more = true;
            }
        }
        if !more {
            return;
        }
    }
}

/// The write state an inverse sets back: of a constructor relation, by the relation's and the
/// constructor's ids, or of a value-level constructor, by its id.
#[derive(Clone, Copy, Debug)]
pub(in crate::evolve) enum StateOf {
    Relation(CatRelId, CatCtorId),
    Value(CatCtorId),
}

impl StateOf {
    /// Returns the ids of the constructor relation, when `relation`, or of the value-level
    /// constructor whose write state `src` keeps at `key`.
    pub(super) fn of(src: &Source, relation: bool, key: &StateKey) -> Option<StateOf> {
        let (m, owner, c) = key;
        if !relation {
            return Some(StateOf::Value(CatCtorId::from_raw(
                src.ctor_key_id(m, owner, c)?,
            )));
        }
        let at = ObjectKey::named(KeyKind::Relation, Owner::Module(m.clone()), owner);
        let rel = *src.ids.get(&at)?;
        let (tm, t) = src.relation_type(m, owner)?;
        Some(StateOf::Relation(
            CatRelId::from_raw(rel),
            CatCtorId::from_raw(src.ctor_key_id(&tm, &t, c)?),
        ))
    }

    /// Returns whether the state is a constructor relation's and its key in `src`, else the kind
    /// of the object `src` lacks.
    pub(super) fn key(self, src: &Source) -> Result<(bool, StateKey), &'static str> {
        let held = |kind: KeyKind, id: u64| {
            src.ids
                .iter()
                .find(|(k, x)| **x == id && k.kind() == kind)
                .map(|(k, _)| k)
        };
        let named = |k: &ObjectKey| -> Result<(String, String), &'static str> {
            let name = k.name().ok_or("constructor")?.to_string();
            match k.owner() {
                Owner::Module(m) => Ok((m.clone(), name)),
                Owner::Object(..) => Err("constructor"),
            }
        };
        let ctor = |c: CatCtorId| -> Result<(String, String, String), &'static str> {
            let k = held(KeyKind::Ctor, c.raw()).ok_or("constructor")?;
            let &Owner::Object(_, t) = k.owner() else {
                return Err("constructor");
            };
            let (m, ty) = named(held(KeyKind::Type, t).ok_or("type")?)?;
            Ok((m, ty, k.name().ok_or("constructor")?.to_string()))
        };
        match self {
            StateOf::Relation(r, c) => {
                let (m, rel) = named(held(KeyKind::Relation, r.raw()).ok_or("relation")?)?;
                Ok((true, (m, rel, ctor(c)?.2)))
            }
            StateOf::Value(c) => Ok((false, ctor(c)?)),
        }
    }

    fn mapped(self, again: &Recreated) -> StateOf {
        let ctor = |c: CatCtorId| CatCtorId::from_raw(again_id(again, Space::Shared, c.raw()));
        match self {
            StateOf::Relation(r, c) => StateOf::Relation(
                CatRelId::from_raw(again_id(again, Space::Shared, r.raw())),
                ctor(c),
            ),
            StateOf::Value(c) => StateOf::Value(ctor(c)),
        }
    }
}

impl Source {
    /// Returns the module and name of the type relation `r` of module `m` is declared over, a
    /// type written unqualified being of module `m`.
    pub(in crate::evolve) fn relation_type(&self, m: &str, r: &str) -> Option<(String, String)> {
        let Some(Decl::Relation(rd)) = self.parsed(self.find("relation", m, r)?) else {
            return None;
        };
        let ast::TypeKind::Named { name, .. } = rd.ty.kind else {
            return None;
        };
        let module = name.module.map_or_else(|| m.to_string(), |x| x.text);
        Some((module, name.name.text))
    }
}

impl Inverse {
    /// Returns the step with each id an earlier step of the revert created again replaced by
    /// the id `again` maps it to.
    pub(super) fn mapped(self, again: &Recreated) -> Inverse {
        let shared = |id: u64| again_id(again, Space::Shared, id);
        match self {
            Inverse::Rename { object, to, of } => Inverse::Rename {
                object: match object {
                    NamedId::Type(t) => NamedId::Type(CatTypeId::from_raw(shared(t.raw()))),
                    NamedId::Ctor(c) => NamedId::Ctor(CatCtorId::from_raw(shared(c.raw()))),
                    NamedId::Relation(r) => NamedId::Relation(CatRelId::from_raw(shared(r.raw()))),
                    NamedId::Set(x) => {
                        NamedId::Set(CatSetId::from_raw(again_id(again, Space::Set, x.raw())))
                    }
                    NamedId::Field(f) => {
                        NamedId::Field(CatFieldId::from_raw(again_id(again, Space::Field, f.raw())))
                    }
                },
                to,
                of,
            },
            Inverse::DropSet { set, of } => Inverse::DropSet {
                set: CatSetId::from_raw(again_id(again, Space::Set, set.raw())),
                of,
            },
            Inverse::AddSet {
                relation,
                name,
                members,
                of,
            } => Inverse::AddSet {
                relation: CatRelId::from_raw(shared(relation.raw())),
                name,
                members: members
                    .into_iter()
                    .map(|c| CatCtorId::from_raw(shared(c.raw())))
                    .collect(),
                of,
            },
            Inverse::States { states, of } => Inverse::States {
                states: states
                    .into_iter()
                    .map(|(s, state)| (s.mapped(again), state))
                    .collect(),
                of,
            },
            Inverse::Grants { back } => Inverse::Grants {
                back: back
                    .into_iter()
                    .map(|h| HeldGrant {
                        privilege: h
                            .privilege
                            .mapped(&mut |kind, x| {
                                Some(again_id(again, kind.key_kind().space(), x))
                            })
                            .expect("a map of ids drops no id"),
                        ..h
                    })
                    .collect(),
            },
            Inverse::Replay {
                logged,
                text,
                again: own,
                of,
            } => {
                let mut all = again.clone();
                for (k, v) in own {
                    let space = k.0;
                    all.insert(k, again_id(again, space, v));
                }
                Inverse::Replay {
                    logged,
                    text: text.mapped(&all),
                    again: all,
                    of,
                }
            }
            inv @ (Inverse::Restructure { .. } | Inverse::Unadd { .. } | Inverse::Readd { .. }) => {
                inv.mapped_structural(again)
            }
            Inverse::Apply { text, of } => Inverse::Apply {
                text: text.mapped(again),
                of,
            },
            Inverse::Restore {
                module,
                kind,
                name,
                text,
                of,
            } => Inverse::Restore {
                kind,
                name,
                module,
                text: text.mapped(again),
                of,
            },
            Inverse::Redeclare { decl, text, of } => Inverse::Redeclare {
                decl: decl.mapped(again),
                text: text.map(|t| t.mapped(again)),
                of,
            },
            Inverse::Module {
                name,
                present,
                installed,
                decls,
                of,
            } => Inverse::Module {
                name,
                present,
                installed,
                decls: decls.into_iter().map(|d| d.mapped(again)).collect(),
                of,
            },
            Inverse::KeyClause { relation, text, of } => Inverse::KeyClause {
                relation: CatRelId::from_raw(shared(relation.raw())),
                text: text.mapped(again),
                of,
            },
            Inverse::Reorder {
                ty,
                ctor,
                order,
                of,
            } => Inverse::Reorder {
                ty: CatTypeId::from_raw(shared(ty.raw())),
                ctor: ctor.map(|c| CatCtorId::from_raw(shared(c.raw()))),
                order: order
                    .into_iter()
                    .map(|f| CatFieldId::from_raw(again_id(again, Space::Field, f.raw())))
                    .collect(),
                of,
            },
        }
    }
}

/// Returns the refusal of the inverse of operator text `of` whose object of kind `word` is gone.
pub(super) fn dropped(of: &str, word: &str) -> String {
    format!("the inverse of `{of}` names a {word} a later migration dropped")
}
