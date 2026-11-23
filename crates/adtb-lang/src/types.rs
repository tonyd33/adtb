//! Define the resolved types and the ids of types, relations, functions and modules.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RelId(pub u32);

/// A relation variable a later operator of the migration being applied adds, by its position in
/// the schema's list of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AheadId(u32);

impl AheadId {
    pub(crate) fn at(i: usize) -> AheadId {
        AheadId(i as u32)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// The relation variable a key type names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RelRef {
    Declared(RelId),
    Ahead(AheadId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FnId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

/// A constraint of a schema, by its position among the schema's constraints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConstraintIdx(u32);

impl ConstraintIdx {
    pub(crate) fn at(i: usize) -> ConstraintIdx {
        ConstraintIdx(i as u32)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// A row check of a schema, by its position among the schema's checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CheckIdx(u32);

impl CheckIdx {
    pub(crate) fn at(i: usize) -> CheckIdx {
        CheckIdx(i as u32)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// A constraint or a row check of a schema: the two kinds that share the constraint id space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DenialIdx {
    Constraint(ConstraintIdx),
    Check(CheckIdx),
}

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

    /// Returns the least and greatest values of an integer type.
    pub fn int_range(self) -> Option<(i64, i64)> {
        Some(match self {
            Base::I8 => (i64::from(i8::MIN), i64::from(i8::MAX)),
            Base::I16 => (i64::from(i16::MIN), i64::from(i16::MAX)),
            Base::I32 => (i64::from(i32::MIN), i64::from(i32::MAX)),
            Base::I64 => (i64::MIN, i64::MAX),
            _ => return None,
        })
    }

    /// Returns `v` when it is a value of this integer type.
    pub fn fit_int(self, v: i128) -> Option<i64> {
        let (lo, hi) = self.int_range()?;
        i64::try_from(v).ok().filter(|v| (lo..=hi).contains(v))
    }
}

const DECIMAL_LIMIT: u128 = 10u128.pow(38);

/// Returns whether an unscaled coefficient names a `Decimal`.
pub fn decimal_in_range(coefficient: i128) -> bool {
    coefficient.unsigned_abs() < DECIMAL_LIMIT
}

/// Built-in type and bound names; no type or type parameter may take one.
pub const RESERVED_TYPE_NAMES: &[&str] = &["Set", "Key", "Group", "Eq", "Ord", "Hash", "Num"];

pub const BOUNDS: &[&str] = &["Eq", "Ord", "Hash", "Num"];

/// Returns whether `name` names a prelude type: a scalar, a built-in type constructor or a prelude sum.
pub fn is_prelude_type(name: &str) -> bool {
    matches!(name, "Set" | "Key" | "Group" | "Decimal" | "Tag")
        || Base::from_name(name).is_some()
        || crate::ids::TYPES
            .iter()
            .any(|&(module, type_name, _, _)| module == "prelude" && type_name == name)
}

macro_rules! prelude_functions {
    ($($family:ident($ty:ident) { $($v:ident = $name:literal,)* })*) => {
        $(
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub enum $ty {
                $($v,)*
            }

            impl $ty {
                pub fn name(self) -> &'static str {
                    match self {
                        $($ty::$v => $name,)*
                    }
                }
            }
        )*

        /// A prelude function or built-in form written as a call, by the checker's family of it.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum PreludeFn {
            $($family($ty),)*
        }

        impl PreludeFn {
            /// Returns the function a call of `name` with no module names; `None` when none.
            pub fn named(name: &str) -> Option<PreludeFn> {
                match name {
                    $($($name => Some(PreludeFn::$family($ty::$v)),)*)*
                    _ => None,
                }
            }

            pub fn name(self) -> &'static str {
                match self {
                    $(PreludeFn::$family(f) => f.name(),)*
                }
            }
        }

        /// Prelude functions and built-in forms written as calls; no schema or bundle may declare one.
        pub const PRELUDE_FUNCTIONS: &[&str] = &[$($($name,)*)*];
    };
}

prelude_functions! {
    Span(SpanFn) {
        Span = "span",
        Start = "start",
        End = "end",
        Overlaps = "overlaps",
        Covers = "covers",
    }
    Text(TextFn) {
        Uuid = "uuid",
        Text = "text",
        BytesLen = "bytes_len",
        ScalarLen = "scalar_len",
        GraphemeLen = "grapheme_len",
        Utf8 = "utf8",
        Bytes = "bytes",
    }
    Checked(CheckedFn) {
        Add = "checked_add",
        Sub = "checked_sub",
        Mul = "checked_mul",
        Div = "checked_div",
        Rem = "checked_rem",
        Neg = "checked_neg",
    }
    Decimal(DecimalFn) {
        Div = "div",
        Round = "round",
        Rescale = "rescale",
        Abs = "abs",
    }
    Convert(ConvertFn) {
        I8 = "i8",
        I16 = "i16",
        I32 = "i32",
        I64 = "i64",
        CheckedI8 = "checked_i8",
        CheckedI16 = "checked_i16",
        CheckedI32 = "checked_i32",
        CheckedI64 = "checked_i64",
        Decimal = "decimal",
        CheckedDecimal = "checked_decimal",
        F32 = "f32",
        F64 = "f64",
        CheckedF32 = "checked_f32",
        Unwrap = "unwrap",
        Value = "value",
    }
    String(StringFn) {
        StartsWith = "starts_with",
        EndsWith = "ends_with",
        Contains = "contains",
        Matches = "matches",
        Lower = "lower",
        Upper = "upper",
        Fold = "fold",
        Collate = "collate",
    }
    Time(TimeFn) {
        Micros = "micros",
        Millis = "millis",
        Seconds = "seconds",
        Minutes = "minutes",
        Hours = "hours",
        Days = "days",
        Weeks = "weeks",
        Months = "months",
        Years = "years",
        AddPeriod = "add_period",
        Zoned = "zoned",
        InZone = "in_zone",
        ToInstant = "to_instant",
    }
    Change(ChangeFn) {
        Tag = "tag",
        Print = "print",
        Inserted = "inserted",
        Deleted = "deleted",
        Prior = "prior",
    }
    Aggregate(AggregateFn) {
        Count = "count",
        Sum = "sum",
        Min = "min",
        Max = "max",
        Avg = "avg",
    }
    File(FileFn) {
        Csv = "read_csv",
        Ndjson = "read_ndjson",
        Parquet = "read_parquet",
        Arrow = "read_arrow",
    }
}

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

/// The constructors of its relation a key type admits.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum KeyCtors {
    All,
    Only(CtorSet),
    /// A selection, as written, that resolves when the migration being applied ends.
    Late(String),
}

impl KeyCtors {
    /// Returns the constructors a key of a committed schema admits, `None` for every one.
    ///
    /// Preconditions: `self` is a selection of a type of a committed schema.
    pub fn committed(&self) -> Option<&CtorSet> {
        match self {
            KeyCtors::All => None,
            KeyCtors::Only(cs) => Some(cs),
            KeyCtors::Late(_) => panic!("a committed schema holds no late selection"),
        }
    }

    /// Returns the selection of the keys of rows of a row type selecting `ctors`.
    pub fn of_row(ctors: Option<CtorSet>) -> KeyCtors {
        match ctors {
            Some(cs) => KeyCtors::Only(cs),
            None => KeyCtors::All,
        }
    }
}

/// How an index element keys its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyForm {
    /// One entry, keyed by the element's value.
    Value,
    /// One entry for each element of the set the element gives.
    Entries,
}

/// The fields of an anonymous record or of a constructor of an anonymous sum, in name order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Fields(Vec<(String, Ty)>);

impl Fields {
    /// Returns `fields` in name order.
    pub fn new(fields: impl IntoIterator<Item = (String, Ty)>) -> Fields {
        let mut fields: Vec<(String, Ty)> = fields.into_iter().collect();
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        Fields(fields)
    }

    /// Returns the fields with each type mapped by `f`, in the same order.
    #[must_use]
    pub fn map(&self, mut f: impl FnMut(&Ty) -> Ty) -> Fields {
        Fields(self.0.iter().map(|(n, t)| (n.clone(), f(t))).collect())
    }

    /// Returns the fields with each type mapped by `f`, failing as the first failing type fails.
    pub fn try_map<E>(&self, mut f: impl FnMut(&Ty) -> Result<Ty, E>) -> Result<Fields, E> {
        let out = self
            .0
            .iter()
            .map(|(n, t)| Ok((n.clone(), f(t)?)))
            .collect::<Result<_, E>>()?;
        Ok(Fields(out))
    }
}

impl std::ops::Deref for Fields {
    type Target = [(String, Ty)];

    fn deref(&self) -> &[(String, Ty)] {
        &self.0
    }
}

impl FromIterator<(String, Ty)> for Fields {
    fn from_iter<I: IntoIterator<Item = (String, Ty)>>(fields: I) -> Fields {
        Fields::new(fields)
    }
}

impl IntoIterator for Fields {
    type Item = (String, Ty);
    type IntoIter = std::vec::IntoIter<(String, Ty)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Fields {
    type Item = &'a (String, Ty);
    type IntoIter = std::slice::Iter<'a, (String, Ty)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// A sum the checker is still inferring: the anonymous constructors one statement joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SumClass(u32);

impl SumClass {
    pub(crate) fn new(i: u32) -> SumClass {
        SumClass(i)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// The constructors of an anonymous sum, in name order, each with its fields.
///
/// While a statement is checked, `class` names the sum its constructors belong to, and the
/// constructors are those the class held when the type was formed; a checked term never holds
/// one with a class.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Ctors {
    ctors: Vec<(String, Fields)>,
    class: Option<SumClass>,
}

impl Ctors {
    /// Returns `ctors` in name order.
    pub fn new(ctors: impl IntoIterator<Item = (String, Fields)>) -> Ctors {
        Ctors::open(ctors, None)
    }

    /// Returns `ctors` in name order, of sum `class`.
    pub fn open(
        ctors: impl IntoIterator<Item = (String, Fields)>,
        class: Option<SumClass>,
    ) -> Ctors {
        let mut ctors: Vec<(String, Fields)> = ctors.into_iter().collect();
        ctors.sort_by(|a, b| a.0.cmp(&b.0));
        Ctors { ctors, class }
    }

    pub fn class(&self) -> Option<SumClass> {
        self.class
    }

    /// Returns the constructors with each field type mapped by `f`, in the same order.
    #[must_use]
    pub fn map(&self, mut f: impl FnMut(&Ty) -> Ty) -> Ctors {
        Ctors {
            ctors: self
                .ctors
                .iter()
                .map(|(c, fs)| (c.clone(), fs.map(&mut f)))
                .collect(),
            class: self.class,
        }
    }

    /// Returns the constructors with each field type mapped by `f`, failing as the first failing
    /// type fails.
    pub fn try_map<E>(&self, mut f: impl FnMut(&Ty) -> Result<Ty, E>) -> Result<Ctors, E> {
        let ctors = self
            .ctors
            .iter()
            .map(|(c, fs)| Ok((c.clone(), fs.try_map(&mut f)?)))
            .collect::<Result<_, E>>()?;
        Ok(Ctors {
            ctors,
            class: self.class,
        })
    }
}

impl std::ops::Deref for Ctors {
    type Target = [(String, Fields)];

    fn deref(&self) -> &[(String, Fields)] {
        &self.ctors
    }
}

impl IntoIterator for Ctors {
    type Item = (String, Fields);
    type IntoIter = std::vec::IntoIter<(String, Fields)>;

    fn into_iter(self) -> Self::IntoIter {
        self.ctors.into_iter()
    }
}

impl From<Fields> for Vec<(String, Ty)> {
    fn from(fields: Fields) -> Vec<(String, Ty)> {
        fields.0
    }
}

impl FromIterator<(String, Fields)> for Ctors {
    fn from_iter<I: IntoIterator<Item = (String, Fields)>>(ctors: I) -> Ctors {
        Ctors::new(ctors)
    }
}

impl<'a> IntoIterator for &'a Ctors {
    type Item = &'a (String, Fields);
    type IntoIter = std::slice::Iter<'a, (String, Fields)>;

    fn into_iter(self) -> Self::IntoIter {
        self.ctors.iter()
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
        rel: RelRef,
        ctors: KeyCtors,
    },
    Set(Box<Ty>),
    /// A nonempty set, the rows of a group; a subtype of `Set`.
    NonEmpty(Box<Ty>),
    Group(Box<Ty>, Box<Ty>),
    Tag(Box<Ty>),
    Record(Fields),
    /// An anonymous sum; `sel` restricts it to a constructor-set row type.
    Sum {
        ctors: Ctors,
        sel: Option<CtorSet>,
    },
    Fn(Box<Ty>, Box<Ty>),
    Unit,
    Change,
}

impl Ty {
    /// Returns how an index element of the type keys its row, `None` when it has no key form.
    pub fn key_form(&self) -> Option<KeyForm> {
        match self {
            Ty::Set(t) | Ty::NonEmpty(t) if t.keyed_whole() => Some(KeyForm::Entries),
            t if t.keyed_whole() => Some(KeyForm::Value),
            _ => None,
        }
    }

    /// Returns whether no set, group or function type occurs in the type at any depth, and no
    /// type parameter or write.
    fn keyed_whole(&self) -> bool {
        match self {
            Ty::Base(_) | Ty::Key { .. } | Ty::Tag(_) | Ty::Unit => true,
            Ty::Data { args, .. } => args.iter().all(Ty::keyed_whole),
            Ty::Record(fields) => fields.iter().all(|(_, f)| f.keyed_whole()),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .flat_map(|(_, fs)| fs)
                .all(|(_, f)| f.keyed_whole()),
            Ty::Set(_)
            | Ty::NonEmpty(_)
            | Ty::Group(..)
            | Ty::Fn(..)
            | Ty::Param(_)
            | Ty::Change => false,
        }
    }

    /// Returns the number of nodes of the type's tree.
    pub fn nodes(&self) -> usize {
        let mut nodes = 0;
        let mut stack = vec![self];
        while let Some(t) = stack.pop() {
            nodes += 1;
            match t {
                Ty::Base(_) | Ty::Param(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => {}
                Ty::Data { args, .. } => stack.extend(args),
                Ty::Set(x) | Ty::NonEmpty(x) | Ty::Tag(x) => stack.push(x),
                Ty::Group(a, b) | Ty::Fn(a, b) => stack.extend([&**a, &**b]),
                Ty::Record(fields) => stack.extend(fields.iter().map(|(_, f)| f)),
                Ty::Sum { ctors, .. } => {
                    stack.extend(ctors.iter().flat_map(|(_, fs)| fs.iter().map(|(_, f)| f)));
                }
            }
        }
        nodes
    }

    /// Returns whether the type's tree has at most `limit` nodes and `height` levels, visiting at
    /// most `limit` nodes.
    pub fn within(&self, limit: usize, height: u32) -> bool {
        let mut left = limit;
        let mut stack = vec![(self, 0)];
        while let Some((t, level)) = stack.pop() {
            if level > height {
                return false;
            }
            let Some(rest) = left.checked_sub(1) else {
                return false;
            };
            left = rest;
            let under: Vec<&Ty> = match t {
                Ty::Base(_) | Ty::Param(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => Vec::new(),
                Ty::Data { args, .. } => args.iter().collect(),
                Ty::Set(x) | Ty::NonEmpty(x) | Ty::Tag(x) => vec![&**x],
                Ty::Group(a, b) | Ty::Fn(a, b) => vec![&**a, &**b],
                Ty::Record(fields) => fields.iter().map(|(_, f)| f).collect(),
                Ty::Sum { ctors, .. } => ctors
                    .iter()
                    .flat_map(|(_, fs)| fs.iter().map(|(_, f)| f))
                    .collect(),
            };
            stack.extend(under.into_iter().map(|x| (x, level + 1)));
        }
        true
    }

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
            Ty::Record(fs) => Ty::Record(fs.map(|t| t.subst(args))),
            Ty::Sum { ctors, sel } => Ty::Sum {
                ctors: ctors.map(|t| t.subst(args)),
                sel: sel.clone(),
            },
            Ty::Fn(a, b) => Ty::Fn(Box::new(a.subst(args)), Box::new(b.subst(args))),
        }
    }

    /// Returns the first key type in `self` whose selection resolves when the migration being
    /// applied ends.
    pub fn late_key(&self) -> Option<&Ty> {
        match self {
            Ty::Key {
                ctors: KeyCtors::Late(_),
                ..
            } => Some(self),
            Ty::Key {
                ctors: KeyCtors::All | KeyCtors::Only(_),
                ..
            }
            | Ty::Base(_)
            | Ty::Param(_)
            | Ty::Unit
            | Ty::Change => None,
            Ty::Data { args, .. } => args.iter().find_map(Ty::late_key),
            Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => t.late_key(),
            Ty::Group(a, b) | Ty::Fn(a, b) => a.late_key().or_else(|| b.late_key()),
            Ty::Record(fs) => fs.iter().find_map(|(_, t)| t.late_key()),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .find_map(|(_, fs)| fs.iter().find_map(|(_, t)| t.late_key())),
        }
    }

    /// Returns whether `self` and `other` are equal when a late key selection counts as every
    /// constructor.
    pub fn alike(&self, other: &Ty) -> bool {
        let all =
            |a: &[Ty], b: &[Ty]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.alike(y));
        let fields = |a: &[(String, Ty)], b: &[(String, Ty)]| {
            a.len() == b.len() && a.iter().zip(b).all(|((m, x), (n, y))| m == n && x.alike(y))
        };
        match (self, other) {
            (Ty::Key { rel: r1, ctors: c1 }, Ty::Key { rel: r2, ctors: c2 }) => {
                r1 == r2
                    && match (c1, c2) {
                        (KeyCtors::Only(a), KeyCtors::Only(b)) => a == b,
                        (KeyCtors::All | KeyCtors::Late(_), KeyCtors::All | KeyCtors::Late(_)) => {
                            true
                        }
                        (KeyCtors::Only(_), KeyCtors::All | KeyCtors::Late(_))
                        | (KeyCtors::All | KeyCtors::Late(_), KeyCtors::Only(_)) => false,
                    }
            }
            (
                Ty::Data {
                    def: d1,
                    args: a1,
                    ctors: c1,
                },
                Ty::Data {
                    def: d2,
                    args: a2,
                    ctors: c2,
                },
            ) => d1 == d2 && c1 == c2 && all(a1, a2),
            (Ty::Set(x), Ty::Set(y))
            | (Ty::NonEmpty(x), Ty::NonEmpty(y))
            | (Ty::Tag(x), Ty::Tag(y)) => x.alike(y),
            (Ty::Group(k1, t1), Ty::Group(k2, t2)) | (Ty::Fn(k1, t1), Ty::Fn(k2, t2)) => {
                k1.alike(k2) && t1.alike(t2)
            }
            (Ty::Record(x), Ty::Record(y)) => fields(x, y),
            (Ty::Sum { ctors: x, sel: s1 }, Ty::Sum { ctors: y, sel: s2 }) => {
                s1 == s2
                    && x.len() == y.len()
                    && x.iter()
                        .zip(y)
                        .all(|((c, f), (d, g))| c == d && fields(f, g))
            }
            _ => self == other,
        }
    }

    /// Returns one past the greatest type parameter the type names, 0 when it names none.
    pub fn params_named(&self) -> u32 {
        match self {
            Ty::Param(i) => i + 1,
            Ty::Base(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => 0,
            Ty::Data { args, .. } => args.iter().map(Ty::params_named).max().unwrap_or(0),
            Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => t.params_named(),
            Ty::Group(k, t) | Ty::Fn(k, t) => k.params_named().max(t.params_named()),
            Ty::Record(fs) => fs.iter().map(|(_, t)| t.params_named()).max().unwrap_or(0),
            Ty::Sum { ctors, .. } => ctors
                .iter()
                .flat_map(|(_, fs)| fs.iter().map(|(_, t)| t.params_named()))
                .max()
                .unwrap_or(0),
        }
    }

    pub fn mentions_fn(&self) -> bool {
        self.mentions(&|t| matches!(t, Ty::Fn(..)))
    }

    /// Returns whether `part` holds of this type or of a type it holds.
    pub fn mentions(&self, part: &impl Fn(&Ty) -> bool) -> bool {
        part(self)
            || match self {
                Ty::Base(_) | Ty::Param(_) | Ty::Key { .. } | Ty::Unit | Ty::Change => false,
                Ty::Data { args, .. } => args.iter().any(|t| t.mentions(part)),
                Ty::Fn(p, r) => p.mentions(part) || r.mentions(part),
                Ty::Set(t) | Ty::NonEmpty(t) | Ty::Tag(t) => t.mentions(part),
                Ty::Group(k, t) => k.mentions(part) || t.mentions(part),
                Ty::Record(fs) => fs.iter().any(|(_, t)| t.mentions(part)),
                Ty::Sum { ctors, .. } => ctors
                    .iter()
                    .any(|(_, fs)| fs.iter().any(|(_, t)| t.mentions(part))),
            }
    }
}
