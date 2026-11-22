use super::*;
use crate::norm::NormalQuery;

/// A function of one row, normalised against the schema it checks in.
#[derive(Debug)]
pub struct RowFn {
    pub schema: Schema,
    /// Its one parameter is the row.
    pub query: NormalQuery,
}

/// How an operator moves the rows of the relations over type `def` beyond what each field and
/// constructor it keeps carries by id.
///
/// Type and constructor indexes are those of the schema before the operator.
#[derive(Debug)]
pub enum RowMap {
    /// Field `field` of each row of constructors `ctors` takes the value of `fill` at the row.
    Fill {
        def: TypeId,
        ctors: Vec<u32>,
        field: String,
        fill: RowFn,
    },
    /// Field `field` of each row of constructors `ctors` keeps the value it had before.
    Copy {
        def: TypeId,
        ctors: Vec<u32>,
        field: String,
    },
    /// Each row of constructors `from` becomes the value of `routing` at the row, a constructor of
    /// `routing`'s schema named as the constructor of the schema after the operator it goes to.
    Route {
        def: TypeId,
        from: Vec<u32>,
        routing: RowFn,
    },
    /// Each row of constructors `from` becomes a row of constructor `into`, field
    /// `discriminator` holding the constructor of the discriminator's sum named as the row's, and
    /// every other field the row's field of the same name.
    Merge {
        def: TypeId,
        from: Vec<u32>,
        into: String,
        discriminator: String,
    },
    /// Each row of constructors `from` becomes a row of constructor `into`, whose one field holds
    /// the constructor of its sum named as the row's, with the row's fields after the header.
    Nest {
        def: TypeId,
        from: Vec<u32>,
        into: String,
    },
    /// Each row of constructor `from` becomes a row of the constructor named as the constructor
    /// its one field holds, with that value's fields after the header.
    Unnest { def: TypeId, from: u32 },
    /// Each field typed `Key` of relation `relation`, in any type, takes the new key of the row
    /// its old key names.
    Rekey { relation: RelId },
}

/// One step of applying an operator: the source after it, how it moves rows, and the operator
/// it applies when it applies one.
#[derive(Debug)]
pub struct Step {
    pub after: Source,
    pub maps: Vec<RowMap>,
    pub applied: Option<Applied>,
}

/// An operator of a migration of `module` whose text is `text`.
#[derive(Debug)]
pub struct Applied {
    pub(super) module: String,
    pub(super) text: String,
    pub(super) op: Operator,
}
