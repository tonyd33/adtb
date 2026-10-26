#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FnId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

/// Constructor indices into a type's declaration order, sorted and distinct.
pub type CtorSet = Vec<u32>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Base {
    I8,
    I16,
    I32,
    I64,
    Decimal(u8),
    F32,
    F64,
    Text,
    Bytes,
    SortKey,
    Instant,
    Date,
    Time,
    DateTime,
    ZonedDateTime,
    Duration,
    Period,
}

impl Base {
    pub fn from_name(name: &str) -> Option<Base> {
        Some(match name {
            "I8" => Base::I8,
            "I16" => Base::I16,
            "I32" => Base::I32,
            "I64" => Base::I64,
            "F32" => Base::F32,
            "F64" => Base::F64,
            "Text" => Base::Text,
            "Bytes" => Base::Bytes,
            "SortKey" => Base::SortKey,
            "Instant" => Base::Instant,
            "Date" => Base::Date,
            "Time" => Base::Time,
            "DateTime" => Base::DateTime,
            "ZonedDateTime" => Base::ZonedDateTime,
            "Duration" => Base::Duration,
            "Period" => Base::Period,
            _ => return None,
        })
    }

    pub fn is_ordered(self) -> bool {
        !matches!(self, Base::ZonedDateTime | Base::Period)
    }

    pub fn is_num(self) -> bool {
        matches!(
            self,
            Base::I8 | Base::I16 | Base::I32 | Base::I64 | Base::Decimal(_) | Base::F32 | Base::F64
        )
    }
}

/// Built-in type and bound names; no type or type parameter may take one.
pub const RESERVED_TYPE_NAMES: &[&str] = &["Set", "Key", "Group", "Eq", "Ord", "Hash", "Num"];

pub const BOUNDS: &[&str] = &["Eq", "Ord", "Hash", "Num"];

/// Prelude functions and built-in forms written as calls; no schema or bundle may declare one.
pub const PRELUDE_FUNCTIONS: &[&str] = &[
    "span",
    "start",
    "end",
    "overlaps",
    "covers",
    "uuid",
    "text",
    "checked_add",
    "checked_sub",
    "checked_mul",
    "checked_div",
    "checked_rem",
    "checked_neg",
    "div",
    "round",
    "rescale",
    "abs",
    "i8",
    "i16",
    "i32",
    "i64",
    "decimal",
    "f32",
    "f64",
    "checked_i8",
    "checked_i16",
    "checked_i32",
    "checked_i64",
    "checked_decimal",
    "checked_f32",
    "unwrap",
    "value",
    "bytes_len",
    "scalar_len",
    "grapheme_len",
    "starts_with",
    "ends_with",
    "contains",
    "matches",
    "lower",
    "upper",
    "fold",
    "collate",
    "utf8",
    "bytes",
    "micros",
    "millis",
    "seconds",
    "minutes",
    "hours",
    "days",
    "weeks",
    "months",
    "years",
    "add_period",
    "zoned",
    "in_zone",
    "to_instant",
    "count",
    "sum",
    "min",
    "max",
    "avg",
    "tag",
    "print",
    "inserted",
    "deleted",
    "prior",
    "read_csv",
    "read_ndjson",
    "read_parquet",
    "read_arrow",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bound {
    Eq,
    Ord,
    Hash,
    Num,
}

impl Bound {
    pub fn from_name(name: &str) -> Option<Bound> {
        Some(match name {
            "Eq" => Bound::Eq,
            "Ord" => Bound::Ord,
            "Hash" => Bound::Hash,
            "Num" => Bound::Num,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Base(Base),
    /// A declared type at arguments; `ctors` restricts it to a constructor-set row type.
    Data {
        def: TypeId,
        args: Vec<Ty>,
        ctors: Option<CtorSet>,
    },
    /// The type parameter at this position of the enclosing declaration or function.
    Param(u32),
    Key {
        rel: RelId,
        ctors: Option<CtorSet>,
    },
    Set(Box<Ty>),
    /// A nonempty set, the rows of a group; a subtype of `Set`.
    NonEmpty(Box<Ty>),
    Group(Box<Ty>, Box<Ty>),
    Tag(Box<Ty>),
    /// Fields sorted by name.
    Record(Vec<(String, Ty)>),
    /// An anonymous sum: constructors sorted by name, each with its fields sorted by name; `sel` restricts it to a constructor-set row type.
    Sum {
        ctors: Vec<(String, Vec<(String, Ty)>)>,
        sel: Option<CtorSet>,
    },
    Fn(Box<Ty>, Box<Ty>),
    Unit,
    Change,
}

impl Ty {
    pub fn subst(&self, args: &[Ty]) -> Ty {
        match self {
            Ty::Param(i) => args.get(*i as usize).cloned().unwrap_or(Ty::Param(*i)),
            Ty::Base(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => self.clone(),
            Ty::Data {
                def,
                args: a,
                ctors,
            } => Ty::Data {
                def: *def,
                args: a.iter().map(|t| t.subst(args)).collect(),
                ctors: ctors.clone(),
            },
            Ty::Set(t) => Ty::Set(Box::new(t.subst(args))),
            Ty::NonEmpty(t) => Ty::NonEmpty(Box::new(t.subst(args))),
            Ty::Group(k, t) => Ty::Group(Box::new(k.subst(args)), Box::new(t.subst(args))),
            Ty::Tag(t) => Ty::Tag(Box::new(t.subst(args))),
            Ty::Record(fs) => {
                Ty::Record(fs.iter().map(|(n, t)| (n.clone(), t.subst(args))).collect())
            }
            Ty::Sum { ctors, sel } => Ty::Sum {
                ctors: ctors
                    .iter()
                    .map(|(c, fs)| {
                        (
                            c.clone(),
                            fs.iter().map(|(n, t)| (n.clone(), t.subst(args))).collect(),
                        )
                    })
                    .collect(),
                sel: sel.clone(),
            },
            Ty::Fn(a, b) => Ty::Fn(Box::new(a.subst(args)), Box::new(b.subst(args))),
        }
    }

    pub fn mentions_fn(&self) -> bool {
        match self {
            Ty::Fn(..) => true,
            Ty::Base(_) | Ty::Param(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => false,
            Ty::Data { args, .. } => args.iter().any(Ty::mentions_fn),
            Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => t.mentions_fn(),
            Ty::Group(k, t) => k.mentions_fn() || t.mentions_fn(),
            Ty::Record(fs) => fs.iter().any(|(_, t)| t.mentions_fn()),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .any(|(_, fs)| fs.iter().any(|(_, t)| t.mentions_fn())),
        }
    }
}
