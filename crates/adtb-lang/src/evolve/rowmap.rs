use super::*;
use crate::ids::{CatCtorId, CatFieldId};
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
/// Type and constructor indexes are those of the schema before the operator; catalog ids of
/// fields and constructors the operator gives are those of the schema after it.
#[derive(Debug)]
pub enum RowMap {
    /// Field `field` of each row of constructors `ctors` takes the value of `fill` at the row.
    Fill {
        def: TypeId,
        ctors: Vec<u32>,
        field: CatFieldId,
        fill: RowFn,
    },
    /// Each row of constructor `c` takes into field `to` the value of its field `from`, for each
    /// `(c, from, to)` of `copied`.
    Copy {
        def: TypeId,
        copied: Vec<(u32, CatFieldId, CatFieldId)>,
    },
    /// Each row of constructors `from` becomes the value of `routing` at the row, whose
    /// constructors and fields have the ids of the schema after the operator.
    Route {
        def: TypeId,
        from: Vec<u32>,
        routing: RowFn,
    },
    /// Each row of a constructor of `rows` becomes a row of constructor `into`, field
    /// `discriminator` holding the constructor of the discriminator's sum `rows` pairs it with,
    /// and every other field of `into` the row's field `rows` pairs it with, or of its id.
    Merge {
        def: TypeId,
        into: CatCtorId,
        discriminator: CatFieldId,
        rows: Vec<Merged>,
    },
    /// Each row of a constructor of `from` becomes a row of constructor `into`, whose last field
    /// holds the constructor of its sum `from` pairs it with, with the row's fields after the
    /// header in order, and whose other fields are the row's of the same id.
    Nest {
        def: TypeId,
        into: CatCtorId,
        from: Vec<(u32, CatCtorId)>,
    },
    /// Each row of constructor `from` becomes a row of the constructor `products` pairs with the
    /// constructor its last field holds, that value's fields after the header in order.
    Unnest {
        def: TypeId,
        from: u32,
        products: Vec<(CatCtorId, CatCtorId)>,
    },
    /// Each field typed `Key` of relation `relation`, in any type, takes the new key of the row
    /// its old key names.
    Rekey { relation: RelId },
}

/// The rows of constructor `from` in a merge: the discriminator's constructor `tag` they take,
/// and each field of the merged constructor `fields` pairs with a field of `from`.
#[derive(Clone, Debug)]
pub struct Merged {
    from: u32,
    tag: CatCtorId,
    fields: Vec<(CatFieldId, CatFieldId)>,
}

impl Merged {
    pub(crate) fn new(from: u32, tag: CatCtorId, fields: Vec<(CatFieldId, CatFieldId)>) -> Merged {
        Merged { from, tag, fields }
    }

    pub fn from(&self) -> u32 {
        self.from
    }

    pub fn tag(&self) -> CatCtorId {
        self.tag
    }

    /// Returns each field of the merged constructor with the field of `from` it takes.
    pub fn fields(&self) -> &[(CatFieldId, CatFieldId)] {
        &self.fields
    }
}

/// One step of applying an operator: the source after it, how it moves rows, and the operator
/// it applies, as applying it resolved it, when it applies one.
#[derive(Debug)]
pub struct Step {
    pub after: Source,
    pub maps: Vec<RowMap>,
    pub applied: Option<ResolvedOp>,
}
