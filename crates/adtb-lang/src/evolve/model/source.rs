use super::*;
use crate::evolve::expected;
use crate::ids::{KeyKind, Owner, keyed};

impl Model {
    /// Returns the source the model holds: its declarations printed, a name of dropped objects as
    /// written, and each object of them by its key with the id the model holds, of each named
    /// one alone while they do not build.
    pub(super) fn source(&self) -> Source {
        self.seeded(Source {
            decls: self.printed().expect("the declarations print"),
            ids: self.ids().unwrap_or_else(|_| self.named_keys()),
            modules: self.modules.clone(),
            installed: self.installed.clone(),
            packages: self.packages.clone(),
            grants: self.grants.clone(),
            states: self
                .states
                .iter()
                .map(|(&(r, c), &s)| ((cat_rel(r), cat_ctor(c)), s))
                .collect(),
            value_states: self
                .value_states
                .iter()
                .map(|(&c, &s)| (cat_ctor(c), s))
                .collect(),
            version: self.version,
            log: self.logged.clone(),
            next: self.next.clone(),
            parsed: self.parsed.clone(),
            ..Source::default()
        })
    }

    /// Returns the source the model holds as [`Model::source`] does, with the relation variables
    /// the operators of the migration being applied add.
    pub(super) fn replay_source(&self) -> Source {
        let ahead = self
            .ahead
            .iter()
            .map(|a| {
                let added = match a.added {
                    Added::Not => expected::Added::Not,
                    Added::Applying => expected::Added::Applying,
                    Added::Refused => expected::Added::Refused,
                    Added::As(r) => expected::Added::As(cat_rel(r)),
                };
                crate::evolve::Expected::added(
                    a.module.clone(),
                    a.name.clone(),
                    a.typed,
                    a.by,
                    added,
                    a.kind,
                )
            })
            .collect();
        self.seeded(Source {
            ahead,
            ..self.source()
        })
    }

    /// Returns the catalog id of each object of the declarations, by its key.
    ///
    /// Fail when the declarations do not build.
    pub(super) fn ids(&self) -> Result<BTreeMap<ObjectKey, u64>, String> {
        let (built, _) = self
            .built(&[], &self.ahead_inputs())
            .map_err(|ds| ds[0].diag.message.clone())?;
        let schema = built.schema();
        let objs = built.objects();
        let ids = schema.ids();
        let keys = keyed(&objs, &|s| ids.slot_id(s))
            .ok_or("an object of the built declarations has no id")?;
        Ok(keys.into_iter().collect())
    }
}

impl Model {
    /// Returns the key and id of each named object, as a source holds them.
    pub(super) fn named_keys(&self) -> BTreeMap<ObjectKey, u64> {
        let mut out = self.held_ids.clone();
        let shared = |id: u64| Owner::Object(Space::Shared, id);
        for d in &self.decls {
            let module = || Owner::Module(d.module.clone());
            match d.item {
                Item::Type(t) => {
                    out.insert(
                        ObjectKey::named(KeyKind::Type, module(), &self.types[&t].name),
                        t,
                    );
                    for c in self.ctors_of(t) {
                        out.insert(
                            ObjectKey::named(KeyKind::Ctor, shared(t), &self.ctors[&c].name),
                            c,
                        );
                        for f in self.ctors[&c].fields.iter().flatten() {
                            out.insert(
                                ObjectKey::named(KeyKind::Field, shared(c), &self.fields[f].name),
                                *f,
                            );
                        }
                    }
                    for f in self.types[&t].header.iter().flatten() {
                        out.insert(
                            ObjectKey::named(KeyKind::Field, shared(t), &self.fields[f].name),
                            *f,
                        );
                    }
                }
                Item::Relation(r) => {
                    let rel = &self.relations[&r];
                    out.insert(ObjectKey::named(KeyKind::Relation, module(), &rel.name), r);
                    for c in &rel.clauses {
                        if let ClauseM::Computed {
                            id: Some(id), name, ..
                        } = c
                        {
                            out.insert(ObjectKey::named(KeyKind::Computed, shared(r), name), *id);
                        }
                    }
                }
                Item::Set(s) => {
                    let set = &self.sets[&s];
                    out.insert(
                        ObjectKey::named(KeyKind::Set, shared(set.relation), &set.name),
                        s,
                    );
                }
                Item::Fn(f) | Item::Aggregate(f) => {
                    out.insert(
                        ObjectKey::named(KeyKind::Function, module(), &self.functions[&f].name),
                        f,
                    );
                }
                Item::Constraint(c) | Item::Check(c) => {
                    out.insert(
                        ObjectKey::named(KeyKind::Constraint, module(), &self.denials[&c].name),
                        c,
                    );
                }
                Item::Store(v) => {
                    let st = &self.stores[&v];
                    let name = self.store_name(v);
                    out.insert(ObjectKey::named(KeyKind::Relation, module(), name), v);
                    if let Some(&t) = st.ids.get(&ViewSlot::Type) {
                        out.insert(ObjectKey::named(KeyKind::Type, shared(v), name), t);
                        if let Some(&c) = st.ids.get(&ViewSlot::Ctor) {
                            out.insert(ObjectKey::named(KeyKind::Ctor, shared(t), name), c);
                            for (slot, &f) in &st.ids {
                                if let ViewSlot::Field(n) = slot {
                                    out.insert(ObjectKey::named(KeyKind::Field, shared(c), n), f);
                                }
                            }
                        }
                    }
                }
                Item::Held(..) => {}
            }
        }
        out
    }
}
