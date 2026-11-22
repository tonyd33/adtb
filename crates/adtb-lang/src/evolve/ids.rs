use super::*;
use crate::ids::{
    CatalogIds, FIRST_LOG_ID, KeyKind, Object, ObjectKey, Owner, Space, assign, is_named,
    named_ids, objects,
};

impl Source {
    /// The text of a span of file `file`, its tokens joined by single spaces.
    fn span_text(&self, file: FileId, span: Span) -> String {
        let text = self.file_text(file);
        tokens(&text[span.range()])
            .into_iter()
            .map(|(_, r)| {
                text[span.start as usize + r.start..span.start as usize + r.end].to_string()
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The built schema and every object of its user modules.
    fn objects(&self) -> Option<(Arc<Schema>, Vec<Object>)> {
        let schema = self.built().ok()?;
        let files = self.files().ok()?;
        let out = objects(&schema, &files, &|f, s| self.span_text(f, s));
        Some((schema, out))
    }

    /// Give each object its id: a named object the id the source holds for its key, else the next
    /// of its space; then, when the source builds, each unique, index and default likewise.
    /// Retire the ids and write states of objects that are gone; ids are never reused.
    pub(super) fn settle_ids(&mut self) {
        self.name_ids();
        self.reconcile_clause_ids();
        let held = holding_shared(&self.ids);
        self.states.retain(|(r, c), _| {
            held.contains(&r.raw()) && (held.contains(&c.raw()) || c.raw() < FIRST_LOG_ID)
        });
        self.value_states.retain(|c, _| held.contains(&c.raw()));
    }

    /// Give each named object of the parsed declarations, whether or not they build, the id the
    /// source holds for its key, else the next of its space; forget the ids of named objects
    /// that are gone.
    fn name_ids(&mut self) {
        let Ok(files) = self.files() else {
            return;
        };
        let given = named_ids(&files, &self.ids, &mut self.next, &mut BTreeSet::new());
        self.ids.retain(|k, _| !is_named(k.kind()));
        self.ids.extend(given);
    }

    /// Give each unique, index and default of the built source the id the source holds for its
    /// key, else the next of its space; forget the ids of those that are gone. Leave them as they
    /// are while the source does not build.
    fn reconcile_clause_ids(&mut self) {
        let Some((_, objs)) = self.objects() else {
            return;
        };
        let mut next = self.next.clone();
        let given = assign(
            &objs,
            &self.ids,
            &mut next,
            &[],
            (&BTreeSet::new(), &BTreeSet::new()),
        );
        self.ids.retain(|k, _| is_named(k.kind()));
        self.ids
            .extend(given.into_iter().filter(|(k, _)| !is_named(k.kind())));
        for space in [Space::Index, Space::Default] {
            if let Some(&n) = next.get(&space) {
                self.next.insert(space, n);
            }
        }
    }

    /// Returns the catalog ids of `schema`, which the source built with ids in declaration
    /// order: for each object of its user modules, the id [`Source::settle_ids`] gives it.
    pub(super) fn assigned_ids(&self, schema: &Schema) -> CatalogIds {
        let mut out = schema.ids().clone();
        if let Ok(files) = self.files() {
            let objs = objects(schema, &files, &|f, s| self.span_text(f, s));
            let mut next = self.next.clone();
            let mut known = self.ids.clone();
            let named = named_ids(&files, &self.ids, &mut next, &mut BTreeSet::new());
            known.retain(|k, _| !is_named(k.kind()));
            known.extend(named);
            let ids = assign(
                &objs,
                &known,
                &mut next,
                &[],
                (&BTreeSet::new(), &BTreeSet::new()),
            );
            for o in &objs {
                out.forget(o.slot());
            }
            for (o, (_, id)) in objs.iter().zip(ids) {
                out.set(o.slot(), id);
            }
        }
        out
    }

    /// Returns the ids of the constructors of type `t` of module `m`.
    pub(super) fn ctors_of(&self, m: &str, t: &str) -> BTreeSet<CatCtorId> {
        let Some(Owner::Object(_, ty)) = self.type_owner(m, t) else {
            return BTreeSet::new();
        };
        self.ids
            .iter()
            .filter(|(k, _)| {
                k.kind() == KeyKind::Ctor && *k.owner() == Owner::Object(Space::Shared, ty)
            })
            .map(|(_, &id)| CatCtorId::from_raw(id))
            .collect()
    }

    /// Returns the module and name of the relation variable of id `id`.
    pub(super) fn relation_named(&self, id: CatRelId) -> Option<(String, String)> {
        self.ids
            .iter()
            .find_map(|(k, &v)| match (k.kind(), k.owner(), k.name()) {
                (KeyKind::Relation, Owner::Module(m), Some(n)) if v == id.raw() => {
                    Some((m.clone(), n.to_string()))
                }
                _ => None,
            })
    }

    /// Returns the id of relation variable `r` of module `m`.
    pub(super) fn relation_key_id(&self, m: &str, r: &str) -> Option<CatRelId> {
        self.ids
            .get(&ObjectKey::named(
                KeyKind::Relation,
                Owner::Module(m.to_string()),
                r,
            ))
            .map(|&id| CatRelId::from_raw(id))
    }

    /// Returns type `t` of module `m` as an owner.
    pub(super) fn type_owner(&self, m: &str, t: &str) -> Option<Owner> {
        field_owner(&self.ids, m, t, None)
    }

    /// Returns what holds a field of type `t` of module `m`: constructor `ctor`, else the header.
    pub(super) fn field_owner(&self, m: &str, t: &str, ctor: Option<&str>) -> Option<Owner> {
        field_owner(&self.ids, m, t, ctor)
    }

    /// Returns the id of field `f` of type `t` of module `m`, of constructor `ctor` or the header.
    pub(super) fn field_key_id(
        &self,
        m: &str,
        t: &str,
        ctor: Option<&str>,
        f: &str,
    ) -> Option<u64> {
        field_key_id(&self.ids, m, t, ctor, f)
    }

    /// Returns the id of constructor `c` of type `t` of module `m`.
    pub(super) fn ctor_key_id(&self, m: &str, t: &str, c: &str) -> Option<u64> {
        ctor_key_id(&self.ids, m, t, c)
    }
}

/// Returns what holds a field of type `t` of module `m` among `ids`: constructor `ctor`, else
/// the header.
pub(super) fn field_owner(
    ids: &BTreeMap<ObjectKey, u64>,
    m: &str,
    t: &str,
    ctor: Option<&str>,
) -> Option<Owner> {
    let ty = ids.get(&ObjectKey::named(
        KeyKind::Type,
        Owner::Module(m.to_string()),
        t,
    ))?;
    let ty = Owner::Object(Space::Shared, *ty);
    match ctor {
        Some(c) => ids
            .get(&ObjectKey::named(KeyKind::Ctor, ty, c))
            .map(|&id| Owner::Object(Space::Shared, id)),
        None => Some(ty),
    }
}

/// Returns the id among `ids` of field `f` of type `t` of module `m`, of constructor `ctor` or
/// the header.
pub(super) fn field_key_id(
    ids: &BTreeMap<ObjectKey, u64>,
    m: &str,
    t: &str,
    ctor: Option<&str>,
    f: &str,
) -> Option<u64> {
    let owner = field_owner(ids, m, t, ctor)?;
    ids.get(&ObjectKey::named(KeyKind::Field, owner, f))
        .copied()
}

/// Returns the id among `ids` of constructor `c` of type `t` of module `m`.
pub(super) fn ctor_key_id(
    ids: &BTreeMap<ObjectKey, u64>,
    m: &str,
    t: &str,
    c: &str,
) -> Option<u64> {
    let ty = field_owner(ids, m, t, None)?;
    ids.get(&ObjectKey::named(KeyKind::Ctor, ty, c)).copied()
}

/// Returns the ids of the types, constructors and relation variables of `ids`.
fn holding_shared(ids: &BTreeMap<ObjectKey, u64>) -> BTreeSet<u64> {
    ids.iter()
        .filter(|(k, _)| k.kind().space() == Space::Shared)
        .map(|(_, &id)| id)
        .collect()
}
