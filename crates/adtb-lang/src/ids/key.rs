use super::Space;
use std::collections::{BTreeMap, BTreeSet};

/// The kind of object an [`ObjectKey`] names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeyKind {
    /// A type, a stored view's row type included.
    Type,
    Ctor,
    /// A relation variable, a stored view's included.
    Relation,
    Field,
    Set,
    /// A function or an aggregate.
    Function,
    /// A constraint or a check.
    Constraint,
    Computed,
    Key,
    Unique,
    Index,
    IntervalIndex,
    Implied,
    Generated,
    Default,
}

impl KeyKind {
    /// Returns the id space of the objects of this kind.
    pub fn space(self) -> Space {
        match self {
            KeyKind::Type | KeyKind::Ctor | KeyKind::Relation => Space::Shared,
            KeyKind::Field => Space::Field,
            KeyKind::Set => Space::Set,
            KeyKind::Function => Space::Function,
            KeyKind::Constraint => Space::Constraint,
            KeyKind::Computed => Space::Computed,
            KeyKind::Key
            | KeyKind::Unique
            | KeyKind::Index
            | KeyKind::IntervalIndex
            | KeyKind::Implied => Space::Index,
            KeyKind::Generated | KeyKind::Default => Space::Default,
        }
    }
}

/// What an object belongs to: a module, by name, or another object, by the space of its catalog
/// id and the id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    Module(String),
    Object(Space, u64),
}

/// One part of what tells an object apart from the others of its kind and owner.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Name(String),
    Field(u64),
    Ctor(u64),
    Set(u64),
    /// A run of an index element naming a field, as every path of field and constructor ids it
    /// reaches over the constructors the index ranges over.
    Reach(BTreeSet<Path>),
    /// A token of an index element that names no field of the row.
    Token(String),
    /// The end of an index element.
    Next,
}

/// A step of a [`Part::Reach`] path: a field, or a constructor of the type of the field before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PathStep {
    Field(u64),
    Ctor(u64),
}

pub type Path = Vec<PathStep>;

/// The identity of an object of a user module from one operator to the next: its kind, its
/// owner and its parts, with `nth` counting the objects before it that agree on all three.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectKey {
    kind: KeyKind,
    owner: Owner,
    parts: Vec<Part>,
    nth: usize,
}

impl ObjectKey {
    /// Returns the key of the object of kind `kind` named `name` that `owner` holds.
    pub fn named(kind: KeyKind, owner: Owner, name: &str) -> ObjectKey {
        ObjectKey::new(kind, owner, vec![Part::Name(name.to_string())])
    }

    pub(super) fn new(kind: KeyKind, owner: Owner, parts: Vec<Part>) -> ObjectKey {
        ObjectKey {
            kind,
            owner,
            parts,
            nth: 0,
        }
    }

    pub(super) fn nth(self, nth: usize) -> ObjectKey {
        ObjectKey { nth, ..self }
    }

    pub(super) fn ordinal(&self) -> usize {
        self.nth
    }

    pub fn kind(&self) -> KeyKind {
        self.kind
    }

    pub fn owner(&self) -> &Owner {
        &self.owner
    }

    pub(super) fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// Returns the name of a named object, `None` for a clause object.
    pub fn name(&self) -> Option<&str> {
        match self.parts.as_slice() {
            [Part::Name(n)] => Some(n),
            _ => None,
        }
    }

    /// Returns the key with its one name `name`.
    pub fn renamed(&self, name: &str) -> ObjectKey {
        ObjectKey {
            parts: vec![Part::Name(name.to_string())],
            ..self.clone()
        }
    }

    /// Returns the key with its owner `owner`.
    pub fn owned_by(&self, owner: Owner) -> ObjectKey {
        ObjectKey {
            owner,
            ..self.clone()
        }
    }

    /// Returns the key with each id it holds, of its owner and of its parts, replaced by the id
    /// `f` gives for its space and id.
    pub fn mapped(&self, f: impl Fn(Space, u64) -> u64) -> ObjectKey {
        let step = |s: &PathStep| match *s {
            PathStep::Field(x) => PathStep::Field(f(Space::Field, x)),
            PathStep::Ctor(x) => PathStep::Ctor(f(Space::Shared, x)),
        };
        let part = |p: &Part| match p {
            Part::Field(x) => Part::Field(f(Space::Field, *x)),
            Part::Ctor(x) => Part::Ctor(f(Space::Shared, *x)),
            Part::Set(x) => Part::Set(f(Space::Set, *x)),
            Part::Reach(paths) => Part::Reach(
                paths
                    .iter()
                    .map(|path| path.iter().map(step).collect())
                    .collect(),
            ),
            Part::Name(_) | Part::Token(_) | Part::Next => p.clone(),
        };
        ObjectKey {
            kind: self.kind,
            owner: match &self.owner {
                Owner::Object(space, x) => Owner::Object(*space, f(*space, *x)),
                Owner::Module(m) => Owner::Module(m.clone()),
            },
            parts: self.parts.iter().map(part).collect(),
            nth: self.nth,
        }
    }
}

/// Remove key `gone` from `ids`, each later key of its kind, owner and parts taking the ordinal
/// before it.
pub fn forget(ids: &mut BTreeMap<ObjectKey, u64>, gone: &ObjectKey) {
    if ids.remove(gone).is_none() {
        return;
    }
    let first = gone.clone().nth(0);
    let later: Vec<ObjectKey> = ids
        .keys()
        .filter(|k| k.nth > gone.nth && (*k).clone().nth(0) == first)
        .cloned()
        .collect();
    for k in later {
        let held = ids.remove(&k).expect("a key of the map");
        let n = k.nth - 1;
        ids.insert(k.nth(n), held);
    }
}
