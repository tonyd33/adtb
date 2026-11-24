use super::*;
use crate::ids::{KeyKind, Space};

/// A declaration, by the id of the object it declares, a stored view's by the id of its
/// function.
#[derive(Clone, Copy, Debug)]
pub(in crate::evolve) enum DeclOf {
    Object(KeyKind, u64),
    Store(u64),
}

impl DeclOf {
    pub(in crate::evolve) fn word(self) -> &'static str {
        match self {
            DeclOf::Object(kind, _) => word(kind),
            DeclOf::Store(_) => "function",
        }
    }

    pub(in crate::evolve) fn mapped(self, again: &Recreated) -> DeclOf {
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
pub(in crate::evolve) fn holding(ids: &BTreeMap<ObjectKey, u64>) -> BTreeSet<(Space, u64)> {
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
pub(in crate::evolve) fn recreate(
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
    pub(in crate::evolve) fn mapped(mut self, again: &Recreated) -> Inverse {
        let id = |space: Space, x: &mut u64| *x = again_id(again, space, *x);
        let shared = |x: &mut u64| id(Space::Shared, x);
        let all = |space: Space, xs: &mut Vec<u64>| xs.iter_mut().for_each(|x| id(space, x));
        let term = |t: &mut Term| *t = std::mem::take(t).mapped(again);
        match &mut self {
            Inverse::Rename { object, .. } => id(object.0, &mut object.1),
            Inverse::DropSet { set, .. } => id(Space::Set, set),
            Inverse::AddSet {
                relation, members, ..
            } => {
                shared(relation);
                all(Space::Shared, members);
            }
            Inverse::States { states, .. } => {
                for (s, _) in states {
                    *s = s.mapped(&|x| again_id(again, Space::Shared, x));
                }
            }
            Inverse::Grants { back } => {
                for h in back {
                    h.privilege = h
                        .privilege
                        .mapped(&mut |kind, x| Some(again_id(again, kind.key_kind().space(), x)))
                        .expect("a map of ids drops no id");
                }
            }
            Inverse::Replay {
                texts, again: own, ..
            } => {
                let mut every = again.clone();
                for (&k, &v) in own.iter() {
                    every.insert(k, again_id(again, k.0, v));
                }
                for t in texts {
                    *t = std::mem::take(t).mapped(&every);
                }
                *own = every;
            }
            Inverse::Restructure {
                drops,
                op,
                adds,
                how,
                checks,
                ..
            } => {
                all(Space::Index, drops);
                term(op);
                term(adds);
                match how {
                    Undo::UpFrom(cs) => all(Space::Shared, cs),
                    Undo::Unnest(ty, products) => {
                        shared(ty);
                        products.iter_mut().for_each(|(c, _)| shared(c));
                    }
                    Undo::Written | Undo::Derived => {}
                }
                for (c, t) in checks {
                    id(Space::Constraint, c);
                    term(t);
                }
            }
            Inverse::Unadd { ctor, record, .. } => {
                shared(ctor);
                if let Some(fs) = record {
                    all(Space::Field, fs);
                }
            }
            Inverse::Readd {
                ty,
                ctor,
                text,
                after,
                joins,
                arms,
                ..
            } => {
                shared(ty);
                shared(ctor);
                term(text);
                all(Space::Shared, after);
                all(Space::Set, joins);
                for (c, _, beside, ts) in arms {
                    id(Space::Constraint, c);
                    if let Some(b) = beside {
                        shared(b);
                    }
                    ts.iter_mut().for_each(term);
                }
            }
            Inverse::Apply { text, .. }
            | Inverse::Restore { text, .. }
            | Inverse::RestoreRelation { text, .. } => term(text),
            Inverse::Redeclare { decl, text, .. } => {
                *decl = decl.mapped(again);
                if let Some(t) = text {
                    term(t);
                }
            }
            Inverse::Module { decls, .. } => decls.iter_mut().for_each(term),
            Inverse::KeyClause { relation, text, .. } => {
                shared(relation);
                term(text);
            }
            Inverse::Reorder {
                ty, ctor, order, ..
            } => {
                shared(ty);
                if let Some(c) = ctor {
                    shared(c);
                }
                all(Space::Field, order);
            }
            Inverse::PlaceCtors {
                ty, order, placed, ..
            } => {
                shared(ty);
                all(Space::Shared, order);
                all(Space::Shared, placed);
            }
        }
        self
    }
}

/// Returns the refusal of the inverse of operator text `of` whose object of kind `word` is gone.
pub(in crate::evolve) fn dropped(of: &str, word: &str) -> String {
    format!("the inverse of `{of}` names a {word} a later migration dropped")
}
