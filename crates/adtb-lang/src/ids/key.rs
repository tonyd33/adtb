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

/// What an object belongs to: a module, by name, or another object, by its catalog id.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Owner {
    Module(String),
    Object(u64),
}

/// One part of what tells an object apart from the others of its kind and owner.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    Name(String),
    Field(u64),
    Ctor(u64),
    Set(u64),
    /// A token of an index element that names no field of the row.
    Token(String),
    /// The end of an index element.
    Next,
}

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

    pub fn kind(&self) -> KeyKind {
        self.kind
    }

    pub fn owner(&self) -> &Owner {
        &self.owner
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
}
