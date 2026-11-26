use super::*;
use crate::evolve::model::ClauseM;
use crate::evolve::model::op::{Res, refuse};
use crate::ids::{KeyKind, Space};

/// A declaration, by the id of the object it declares, a stored view's by the id of its
/// function.
#[derive(Clone, Copy, Debug)]
pub(in crate::evolve) enum DeclOf {
    Object(KeyKind, u64),
    Store(u64),
}

impl DeclOf {
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
            Inverse::Grants { back, .. } => {
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

    /// Returns each object the step names by id outside its texts, with the text of the operator
    /// inverted.
    pub(super) fn named(&self) -> (Vec<Obj>, &str) {
        let shared = |x: &u64| (Space::Shared, *x);
        match self {
            Inverse::Rename { object, of, .. } => (vec![*object], of),
            Inverse::DropSet { set, of } => (vec![(Space::Set, *set)], of),
            Inverse::AddSet {
                relation,
                members,
                of,
                ..
            } => (
                [relation].into_iter().chain(members).map(shared).collect(),
                of,
            ),
            Inverse::States { states, of } => {
                let mut out = Vec::new();
                for (s, _) in states {
                    match *s {
                        StateKey::Relation(r, c) => out.extend([shared(&r), shared(&c)]),
                        StateKey::Value(c) => out.push(shared(&c)),
                    }
                }
                (out, of)
            }
            Inverse::Grants { back, of } => (
                back.iter()
                    .flat_map(|h| h.privilege.named())
                    .map(|(kind, x)| (kind.key_kind().space(), x))
                    .collect(),
                of,
            ),
            Inverse::Restructure {
                drops,
                how,
                checks,
                of,
                ..
            } => {
                let mut out: Vec<Obj> = checks
                    .iter()
                    .map(|(c, _)| (Space::Constraint, *c))
                    .collect();
                match how {
                    Undo::UpFrom(cs) => out.extend(cs.iter().map(shared)),
                    Undo::Unnest(ty, products) => {
                        out.push(shared(ty));
                        out.extend(products.iter().map(|(c, _)| shared(c)));
                    }
                    Undo::Written | Undo::Derived => {}
                }
                out.extend(drops.iter().map(|&i| (Space::Index, i)));
                (out, of)
            }
            Inverse::Unadd { ctor, of, .. } => (vec![shared(ctor)], of),
            Inverse::Readd {
                ty,
                joins,
                arms,
                of,
                ..
            } => {
                let mut out = vec![shared(ty)];
                out.extend(joins.iter().map(|&s| (Space::Set, s)));
                for (c, _, beside, _) in arms {
                    out.push((Space::Constraint, *c));
                    out.extend(beside.iter().map(shared));
                }
                (out, of)
            }
            Inverse::Apply { of, .. }
            | Inverse::Restore { of, .. }
            | Inverse::RestoreRelation { of, .. }
            | Inverse::Module { of, .. }
            | Inverse::Replay { of, .. } => (Vec::new(), of),
            Inverse::Redeclare { decl, of, .. } => (vec![decl.obj()], of),
            Inverse::KeyClause { relation, of, .. } => (vec![shared(relation)], of),
            Inverse::Reorder {
                ty,
                ctor,
                order,
                of,
            } => {
                let mut out: Vec<Obj> = ctor.iter().map(shared).collect();
                out.extend(order.iter().map(|&f| (Space::Field, f)));
                out.push(shared(ty));
                (out, of)
            }
            Inverse::PlaceCtors { ty, of, .. } => (vec![shared(ty)], of),
        }
    }
}

impl DeclOf {
    /// Returns the object the declaration declares, a stored view's function.
    fn obj(self) -> Obj {
        match self {
            DeclOf::Object(kind, id) => (kind.space(), id),
            DeclOf::Store(f) => (Space::Function, f),
        }
    }
}

impl Model {
    /// Refuse step `inv` of a revert when it names by id an object the model lacks and `late`,
    /// the source after the reverted migration, holds.
    pub(in crate::evolve::model) fn gate(&self, inv: &Inverse, late: &Late) -> Res<()> {
        let (named, of) = inv.named();
        match named
            .into_iter()
            .filter(|&o| !self.holds(o))
            .find_map(|o| late.get(&o))
        {
            Some(&kind) => refuse(dropped(of, word(kind))),
            None => Ok(()),
        }
    }

    fn holds(&self, (space, id): Obj) -> bool {
        let clauses = || self.relations.values().flat_map(|r| &r.clauses);
        match space {
            Space::Shared => {
                self.types.contains_key(&id)
                    || self.ctors.contains_key(&id)
                    || self.relations.contains_key(&id)
                    || self.stores.contains_key(&id)
            }
            Space::Set => self.sets.contains_key(&id),
            Space::Field => self.fields.contains_key(&id),
            Space::Function => self.functions.contains_key(&id),
            Space::Constraint => self.denials.contains_key(&id),
            Space::Index => self.relations.values().any(|r| {
                r.implied.values().any(|&x| x == id)
                    || r.holds_index(id)
                    || r.clauses
                        .iter()
                        .any(|c| matches!(c, ClauseM::Key { id: x, .. } if *x == Some(id)))
            }),
            Space::Computed => {
                clauses().any(|c| matches!(c, ClauseM::Computed { id: Some(x), .. } if *x == id))
            }
            Space::Default => self.default_holder(id).is_some(),
        }
    }
}

/// The fault of a step of a revert naming by id an object the model lacks that the source after
/// the reverted migration lacks too.
pub(in crate::evolve::model) const UNHELD: &str =
    "an inverse names an object the model does not hold";

/// Returns the refusal of the inverse of operator text `of` whose object of kind `word` is gone.
pub(in crate::evolve) fn dropped(of: &str, word: &str) -> String {
    format!("the inverse of `{of}` names a {word} a later migration dropped")
}
