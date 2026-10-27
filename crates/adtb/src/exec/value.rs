use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use super::Error;

/// An `F32` with `-0.0` replaced by `0.0` and every NaN by one NaN, ordered with NaN greatest.
#[derive(Clone, Copy, Debug)]
pub struct F32(f32);

/// An `F64` with `-0.0` replaced by `0.0` and every NaN by one NaN, ordered with NaN greatest.
#[derive(Clone, Copy, Debug)]
pub struct F64(f64);

impl F32 {
    pub fn new(x: f32) -> F32 {
        if x.is_nan() {
            F32(f32::NAN)
        } else {
            F32(x + 0.0)
        }
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

impl F64 {
    pub fn new(x: f64) -> F64 {
        if x.is_nan() {
            F64(f64::NAN)
        } else {
            F64(x + 0.0)
        }
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

fn float_cmp(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).expect("two numbers compare"),
    }
}

impl PartialEq for F32 {
    fn eq(&self, other: &F32) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for F32 {}

impl PartialOrd for F32 {
    fn partial_cmp(&self, other: &F32) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for F32 {
    fn cmp(&self, other: &F32) -> Ordering {
        float_cmp(f64::from(self.0), f64::from(other.0))
    }
}

impl Hash for F32 {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.0.to_bits().hash(h);
    }
}

impl PartialEq for F64 {
    fn eq(&self, other: &F64) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for F64 {}

impl PartialOrd for F64 {
    fn partial_cmp(&self, other: &F64) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for F64 {
    fn cmp(&self, other: &F64) -> Ordering {
        float_cmp(self.0, other.0)
    }
}

impl Hash for F64 {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.0.to_bits().hash(h);
    }
}

/// A value of the language, as the executor computes with it.
///
/// Invariants:
/// - Every integer type's value is an `Int`, every `Key r.S` the value of its key fields, and
///   every newtype its representation's value.
/// - A `Ctor` is a value of a declared sum, a declared record included, its fields the sum's
///   header fields then the constructor's own, each in declaration order; or of an anonymous sum
///   or a tag type, its constructor in name order and its fields sorted by name.
/// - A `Record` is an anonymous record, its fields sorted by name.
/// - A `Decimal` is its unscaled digits; its scale is its type's.
/// - Values of one ordered type compare by the language's order, except decimals of different
///   scales, which only the comparison primitives compare.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Val {
    Int(i64),
    F32(F32),
    F64(F64),
    Decimal(i128),
    Text(Arc<str>),
    Bytes(Arc<[u8]>),
    SortKey(Arc<[u8]>),
    Instant(i64),
    Date(i32),
    Time(i64),
    DateTime(i64),
    Zoned {
        local: i64,
        zone: Arc<str>,
    },
    Duration(i64),
    Period {
        months: i32,
        days: i32,
    },
    Ctor(u32, Arc<[Val]>),
    Record(Arc<[Val]>),
    Set(Arc<BTreeSet<Val>>),
    /// A final result in its presented order.
    List(Arc<[Val]>),
}

static NO_FIELDS: LazyLock<Arc<[Val]>> = LazyLock::new(|| Arc::from(Vec::new()));
static EMPTY_SET: LazyLock<Arc<BTreeSet<Val>>> = LazyLock::new(|| Arc::new(BTreeSet::new()));

impl Val {
    /// Returns the nullary constructor `ctor`.
    pub fn tag(ctor: u32) -> Val {
        Val::Ctor(ctor, NO_FIELDS.clone())
    }

    pub fn ctor(ctor: u32, fields: Vec<Val>) -> Val {
        if fields.is_empty() {
            return Val::tag(ctor);
        }
        Val::Ctor(ctor, Arc::from(fields))
    }

    pub fn record(fields: Vec<Val>) -> Val {
        Val::Record(Arc::from(fields))
    }

    pub fn bool(b: bool) -> Val {
        Val::tag(u32::from(b))
    }

    pub fn none() -> Val {
        Val::tag(0)
    }

    pub fn some(v: Val) -> Val {
        Val::ctor(1, vec![v])
    }

    pub fn empty_set() -> Val {
        Val::Set(EMPTY_SET.clone())
    }

    pub fn set(elems: BTreeSet<Val>) -> Val {
        Val::Set(Arc::new(elems))
    }

    pub fn text(t: &str) -> Val {
        Val::Text(Arc::from(t))
    }

    /// Returns whether this is `True`.
    ///
    /// Fails unless it is a `Bool`.
    pub fn truth(&self) -> Result<bool, Error> {
        match self {
            Val::Ctor(c, fs) if fs.is_empty() && *c <= 1 => Ok(*c == 1),
            v => Err(mistyped("a Bool", v)),
        }
    }

    /// Returns the constructor of a constructor value.
    ///
    /// Fails unless it is one.
    pub fn ctor_of(&self) -> Result<u32, Error> {
        match self {
            Val::Ctor(c, _) => Ok(*c),
            v => Err(mistyped("a constructor value", v)),
        }
    }

    /// Returns the fields of a constructor value or a record.
    ///
    /// Fails unless it is one.
    pub fn fields(&self) -> Result<&[Val], Error> {
        match self {
            Val::Ctor(_, fs) | Val::Record(fs) => Ok(fs),
            v => Err(mistyped("a value with fields", v)),
        }
    }

    /// Fails unless it is a set.
    pub fn as_set(&self) -> Result<&BTreeSet<Val>, Error> {
        match self {
            Val::Set(s) => Ok(s),
            v => Err(mistyped("a set", v)),
        }
    }

    /// Fails unless it is an integer.
    pub fn int(&self) -> Result<i64, Error> {
        match self {
            Val::Int(i) => Ok(*i),
            v => Err(mistyped("an integer", v)),
        }
    }

    /// Returns an estimate of the bytes the value occupies, its shared parts counted whole.
    pub fn size(&self) -> usize {
        const WORD: usize = size_of::<Val>();
        WORD + match self {
            Val::Text(t) | Val::Zoned { zone: t, .. } => t.len(),
            Val::Bytes(b) | Val::SortKey(b) => b.len(),
            Val::Ctor(_, fs) | Val::Record(fs) | Val::List(fs) => fs.iter().map(Val::size).sum(),
            Val::Set(s) => s.iter().map(|v| v.size() + 2 * WORD).sum(),
            _ => 0,
        }
    }
}

pub(crate) fn mistyped(want: &str, v: &Val) -> Error {
    Error::Internal(format!("expected {want}, found {v:?}"))
}
