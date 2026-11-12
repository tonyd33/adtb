use std::collections::{BTreeMap, BTreeSet};

use super::Slot;
use super::key::{KeyKind, ObjectKey, Part, Path, PathStep};

/// A path of field and constructor ids an operator replaces, and the path of the schema after it
/// that holds its values.
#[derive(Clone, Debug)]
pub struct Replacement {
    from: Path,
    to: Vec<Slot>,
}

impl Replacement {
    pub fn new(from: Path, to: Vec<Slot>) -> Replacement {
        Replacement { from, to }
    }
}

/// The fields and constructors a schema holds, and what an operator replaced, by which the keys
/// of index elements before the operator are matched with those after it.
pub(super) struct Correspondence {
    maps: Vec<(Path, Path)>,
    before: BTreeSet<PathStep>,
    after: BTreeSet<PathStep>,
    produced: BTreeSet<PathStep>,
}

impl Correspondence {
    /// Returns the correspondence of a schema whose objects took `ids`, after the schema whose
    /// keys `known` holds, an operator replacing `replaced`.
    pub(super) fn new(
        known: &BTreeMap<ObjectKey, u64>,
        ids: &BTreeMap<Slot, u64>,
        replaced: &[Replacement],
    ) -> Correspondence {
        let step = |s: &Slot, id: u64| match s {
            Slot::Ctor(_) | Slot::ViewCtor(_) => Some(PathStep::Ctor(id)),
            Slot::Field(_) | Slot::ViewField(..) => Some(PathStep::Field(id)),
            _ => None,
        };
        let before = known
            .iter()
            .filter_map(|(k, &id)| match k.kind() {
                KeyKind::Ctor => Some(PathStep::Ctor(id)),
                KeyKind::Field => Some(PathStep::Field(id)),
                _ => None,
            })
            .collect();
        let after = ids.iter().filter_map(|(s, &id)| step(s, id)).collect();
        let maps: Vec<(Path, Path)> = replaced
            .iter()
            .filter_map(|r| {
                let to =
                    r.to.iter()
                        .map(|s| ids.get(s).and_then(|&id| step(s, id)))
                        .collect::<Option<Path>>()?;
                Some((r.from.clone(), to))
            })
            .collect();
        let produced = maps.iter().flat_map(|(_, to)| to.iter().copied()).collect();
        Correspondence {
            maps,
            before,
            after,
            produced,
        }
    }

    /// Returns whether keys `old`, before the operator, and `new`, after it, name one object: the
    /// same but for the paths of their index elements, which correspond.
    pub(super) fn same(&self, old: &ObjectKey, new: &ObjectKey) -> bool {
        old.kind() == new.kind()
            && old.owner() == new.owner()
            && old.ordinal() == new.ordinal()
            && old.parts().len() == new.parts().len()
            && old
                .parts()
                .iter()
                .zip(new.parts())
                .all(|(a, b)| match (a, b) {
                    (Part::Reach(a), Part::Reach(b)) => self.corresponds(a, b),
                    _ => a == b,
                })
    }

    /// Returns whether element paths `old` and `new` correspond: leaving out each old path the
    /// operator drops and each new path through an id it creates without replacing anything,
    /// some remain on both sides, each old path has an image among the new and each new path is
    /// the image of an old one.
    fn corresponds(&self, old: &BTreeSet<Path>, new: &BTreeSet<Path>) -> bool {
        let images: Vec<BTreeSet<Path>> = old
            .iter()
            .map(|p| self.images(p))
            .filter(|i| !i.is_empty())
            .collect();
        let new: Vec<&Path> = new.iter().filter(|q| self.counted(q)).collect();
        !images.is_empty()
            && !new.is_empty()
            && images.iter().all(|i| new.iter().any(|q| i.contains(*q)))
            && new.iter().all(|q| images.iter().any(|i| i.contains(*q)))
    }

    /// Returns every path that holds the values of old path `p` after the operator: each step
    /// the operator keeps, or a run of steps it replaces taken to their replacement.
    fn images(&self, p: &[PathStep]) -> BTreeSet<Path> {
        let Some(&first) = p.first() else {
            return BTreeSet::from([Vec::new()]);
        };
        let mut out = BTreeSet::new();
        let mut heads: Vec<(&[PathStep], usize)> = Vec::new();
        if self.before.contains(&first) && self.after.contains(&first) {
            heads.push((std::slice::from_ref(&p[0]), 1));
        }
        for (from, to) in &self.maps {
            if p.starts_with(from) && !from.is_empty() {
                heads.push((to, from.len()));
            }
        }
        for (head, used) in heads {
            for rest in self.images(&p[used..]) {
                let mut path = head.to_vec();
                path.extend(rest);
                out.insert(path);
            }
        }
        out
    }

    /// Returns whether every step of new path `q` was there before the operator or holds what it
    /// replaced.
    fn counted(&self, q: &[PathStep]) -> bool {
        q.iter()
            .all(|s| self.produced.contains(s) || self.before.contains(s))
    }
}
