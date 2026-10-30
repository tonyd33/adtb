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
    AnonRecord(AnonRecordType),
    AnonSum(AnonSumType),
}

/// The fields of an anonymous record, in name order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnonRecordType {
    fields: Vec<(String, KeyType)>,
}

/// The constructors of an anonymous sum, in name order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnonSumType {
    ctors: Vec<(String, AnonRecordType)>,
}

impl AnonRecordType {
    /// Returns `None` when two fields share a name.
    pub fn new(fields: Vec<(String, KeyType)>) -> Option<AnonRecordType> {
        sorted_by_name(fields).map(|fields| AnonRecordType { fields })
    }

    pub fn fields(&self) -> &[(String, KeyType)] {
        &self.fields
    }
}

impl AnonSumType {
    /// Returns `None` when two constructors share a name.
    pub fn new(ctors: Vec<(String, AnonRecordType)>) -> Option<AnonSumType> {
        sorted_by_name(ctors).map(|ctors| AnonSumType { ctors })
    }

    pub fn ctors(&self) -> &[(String, AnonRecordType)] {
        &self.ctors
    }
}

fn sorted_by_name<T>(mut named: Vec<(String, T)>) -> Option<Vec<(String, T)>> {
    named.sort_by(|a, b| a.0.cmp(&b.0));
    named.windows(2).all(|w| w[0].0 != w[1].0).then_some(named)
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

    /// Returns a key above `lo` and below `hi`, a missing bound being no
    /// bound.
    ///
    /// Panics unless `lo` is below `hi`.
    pub fn between(lo: Option<&OrderKey>, hi: Option<&OrderKey>) -> OrderKey {
        let lo = lo.map_or(&[][..], OrderKey::as_bytes);
        let mut hi = hi.map(OrderKey::as_bytes);
        if let Some(h) = hi {
            assert!(lo < h, "order key {lo:x?} is not below {h:x?}");
        }
        let mut out = Vec::new();
        for i in 0.. {
            let a = lo.get(i).map_or(0, |&b| u16::from(b));
            let b = hi.map_or(0x100, |h| u16::from(h[i]));
            if b - a >= 2 {
                match (a + b) / 2 {
                    1 => out.extend([0x01, 0x80]),
                    mid => out.push(mid as u8),
                }
                break;
            }
            out.push(a.max(1) as u8);
            if b == a + 1 && a != 0 {
                hi = None;
            }
        }
        OrderKey(out)
    }

    /// Returns `n` increasing keys, each splitting the room left by the
    /// others.
    pub fn spread(n: usize) -> Vec<OrderKey> {
        let mut out = Vec::with_capacity(n);
        spread_into(None, None, n, &mut out);
        out
    }
}

fn spread_into(lo: Option<&OrderKey>, hi: Option<&OrderKey>, n: usize, out: &mut Vec<OrderKey>) {
    if n == 0 {
        return;
    }
    let mid = OrderKey::between(lo, hi);
    let below = (n - 1) / 2;
    spread_into(lo, Some(&mid), below, out);
    out.push(mid.clone());
    spread_into(Some(&mid), hi, n - 1 - below, out);
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

impl std::hash::Hash for F32 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl std::hash::Hash for F64 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

/// A value of a `KeyType`.
///
/// Preconditions:
/// - `Text` and a zone name are in NFC.
/// - A `Decimal` coefficient has at most 38 digits.
/// - A `Date` counts days since 1970-01-01 and its midnight is an `Instant`.
/// - A `Time` counts microseconds since midnight, below 86 400 000 000.
/// - `Instant`, `DateTime` and `Duration` count microseconds; `DateTime`
///   from 1970-01-01T00:00:00 civil.
/// - A `Sum`'s `ctor` indexes its sum's constructors in declaration order, or
///   an anonymous sum's in name order.
/// - The `Record` of an anonymous record, and the fields of an anonymous
///   sum's constructor, are in name order.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
