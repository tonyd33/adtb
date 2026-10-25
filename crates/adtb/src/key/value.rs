#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scalar {
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Decimal(u8),
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

/// The type of a key component. A newtype is its representation's type and
/// `Key r` is the type of `r`'s key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyType {
    Scalar(Scalar),
    Record(Vec<KeyType>),
    Sum(SumType),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SumType {
    ctors: Vec<CtorType>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtorType {
    pub order_key: OrderKey,
    /// The sum's header fields, then the constructor's own fields.
    pub fields: Vec<KeyType>,
}

impl SumType {
    /// Returns `None` unless the order keys strictly increase in the order
    /// `ctors` lists them, which is the declaration order.
    pub fn new(ctors: Vec<CtorType>) -> Option<SumType> {
        ctors
            .windows(2)
            .all(|w| w[0].order_key < w[1].order_key)
            .then_some(SumType { ctors })
    }

    pub fn ctors(&self) -> &[CtorType] {
        &self.ctors
    }

    pub(super) fn ctor_of(&self, order_key: &[u8]) -> Option<u32> {
        let i = self
            .ctors
            .binary_search_by(|c| c.order_key.as_bytes().cmp(order_key))
            .ok()?;
        Some(u32::try_from(i).expect("a sum has fewer than 2^32 constructors"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderKey(Vec<u8>);

impl OrderKey {
    /// Returns `None` for an empty string, one holding a `00` byte, or one
    /// ending in `01`.
    pub fn new(bytes: Vec<u8>) -> Option<OrderKey> {
        let valid = bytes.last().is_some_and(|&b| b != 0x01) && !bytes.contains(&0);
        valid.then_some(OrderKey(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// An `F32` with `-0.0` replaced by `0.0` and every NaN by one NaN.
#[derive(Clone, Copy, Debug)]
pub struct F32(f32);

/// An `F64` with `-0.0` replaced by `0.0` and every NaN by one NaN.
#[derive(Clone, Copy, Debug)]
pub struct F64(f64);

impl F32 {
    pub const NAN_BITS: u32 = 0x7FC0_0000;

    pub fn new(x: f32) -> F32 {
        if x.is_nan() {
            F32(f32::from_bits(F32::NAN_BITS))
        } else {
            F32(x + 0.0)
        }
    }

    pub fn get(self) -> f32 {
        self.0
    }
}

impl F64 {
    pub const NAN_BITS: u64 = 0x7FF8_0000_0000_0000;

    pub fn new(x: f64) -> F64 {
        if x.is_nan() {
            F64(f64::from_bits(F64::NAN_BITS))
        } else {
            F64(x + 0.0)
        }
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl PartialEq for F32 {
    fn eq(&self, other: &F32) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for F32 {}

impl PartialEq for F64 {
    fn eq(&self, other: &F64) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for F64 {}

/// A value of a `KeyType`.
///
/// Preconditions:
/// - `Text` and a zone name are in NFC.
/// - A `Decimal` coefficient has at most 38 digits.
/// - A `Date` counts days since 1970-01-01 and its midnight is an `Instant`.
/// - A `Time` counts microseconds since midnight, below 86 400 000 000.
/// - `Instant`, `DateTime` and `Duration` count microseconds; `DateTime`
///   from 1970-01-01T00:00:00 civil.
/// - A `Sum`'s `ctor` indexes its sum's constructors in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(F32),
    F64(F64),
    Decimal(i128),
    Text(String),
    Bytes(Vec<u8>),
    SortKey(Vec<u8>),
    Instant(i64),
    Date(i32),
    Time(i64),
    DateTime(i64),
    ZonedDateTime { local: i64, zone: String },
    Duration(i64),
    Period { months: i32, days: i32 },
    Record(Vec<Value>),
    Sum { ctor: u32, fields: Vec<Value> },
}
