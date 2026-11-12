use crate::types::TypeId;

/// A constructor of a declared type, by its position among the type's constructors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CtorRef {
    def: TypeId,
    ctor: u32,
}

impl CtorRef {
    pub(super) fn new(def: TypeId, ctor: u32) -> CtorRef {
        CtorRef { def, ctor }
    }

    pub fn def(self) -> TypeId {
        self.def
    }

    /// Returns the constructor's position among its type's constructors, the index a value of it
    /// carries.
    pub fn index(self) -> u32 {
        self.ctor
    }
}

/// Where a field of a declared type sits: the header, or one constructor's own fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FieldPart {
    Header,
    Ctor(u32),
}

/// A field of a declared type, by its part and its position among that part's fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldRef {
    def: TypeId,
    part: FieldPart,
    pos: u32,
}

impl FieldRef {
    pub(super) fn new(def: TypeId, part: FieldPart, pos: u32) -> FieldRef {
        FieldRef { def, part, pos }
    }

    pub fn def(self) -> TypeId {
        self.def
    }

    pub fn part(self) -> FieldPart {
        self.part
    }

    /// Returns the position of the constructor whose own field this is, `None` for a header field.
    pub fn ctor(self) -> Option<u32> {
        match self.part {
            FieldPart::Header => None,
            FieldPart::Ctor(c) => Some(c),
        }
    }

    pub fn pos(self) -> u32 {
        self.pos
    }
}
