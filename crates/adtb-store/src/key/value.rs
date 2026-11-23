use adtb_lang::calendar::{date_in_range, time_in_range};
use adtb_lang::types::decimal_in_range;
use jiff::tz::TimeZone;

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
    /// Panics:
    /// - `lo` is not below `hi`.
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
/// - `Text` is in NFC, and a zone name is one the zone rules name.
/// - A `Decimal` coefficient has at most 38 digits.
/// - A `Date` counts days since 1970-01-01, a day some microsecond of which is an `Instant`.
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

impl Value {
    /// Returns the value with every text normalized to NFC.
    pub fn normalized(self) -> Value {
        match self {
            Value::Text(t) if !unicode_normalization::is_nfc(&t) => {
                Value::Text(unicode_normalization::UnicodeNormalization::nfc(t.as_str()).collect())
            }
            Value::Record(fs) => Value::Record(fs.into_iter().map(Value::normalized).collect()),
            Value::Sum { ctor, fields } => Value::Sum {
                ctor,
                fields: fields.into_iter().map(Value::normalized).collect(),
            },
            other => other,
        }
    }

    /// Returns whether the value keeps the preconditions of [`Value`] that its own bits can
    /// break: NFC text, a zone the zone rules name, a coefficient of at most 38 digits, a `Date` within the
    /// range of `Instant` and a `Time` below a day.
    pub fn in_domain(&self) -> bool {
        match self {
            Value::Text(t) => unicode_normalization::is_nfc(t),
            Value::ZonedDateTime { zone: name, .. } => zone(name).is_some(),
            Value::Decimal(c) => decimal_in_range(*c),
            Value::Date(d) => date_in_range(i64::from(*d)),
            Value::Time(t) => time_in_range(*t),
            Value::Record(fs) | Value::Sum { fields: fs, .. } => fs.iter().all(Value::in_domain),
            Value::I8(_)
            | Value::I16(_)
            | Value::I32(_)
            | Value::I64(_)
            | Value::F32(_)
            | Value::F64(_)
            | Value::Bytes(_)
            | Value::SortKey(_)
            | Value::Instant(_)
            | Value::DateTime(_)
            | Value::Duration(_)
            | Value::Period { .. } => true,
        }
    }
}

/// Returns the zone `name` spells exactly.
pub fn zone(name: &str) -> Option<TimeZone> {
    TimeZone::get(name)
        .ok()
        .filter(|tz| tz.iana_name() == Some(name))
}

impl std::fmt::Display for Scalar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scalar::I8 => f.write_str("I8"),
            Scalar::I16 => f.write_str("I16"),
            Scalar::I32 => f.write_str("I32"),
            Scalar::I64 => f.write_str("I64"),
            Scalar::F32 => f.write_str("F32"),
            Scalar::F64 => f.write_str("F64"),
            Scalar::Decimal(scale) => write!(f, "Decimal of scale {scale}"),
            Scalar::Text => f.write_str("Text"),
            Scalar::Bytes => f.write_str("Bytes"),
            Scalar::SortKey => f.write_str("SortKey"),
            Scalar::Instant => f.write_str("Instant"),
            Scalar::Date => f.write_str("Date"),
            Scalar::Time => f.write_str("Time"),
            Scalar::DateTime => f.write_str("DateTime"),
            Scalar::ZonedDateTime => f.write_str("ZonedDateTime"),
            Scalar::Duration => f.write_str("Duration"),
            Scalar::Period => f.write_str("Period"),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::I8(n) => write!(f, "{n}"),
            Value::I16(n) => write!(f, "{n}"),
            Value::I32(n) => write!(f, "{n}"),
            Value::I64(n) => write!(f, "{n}"),
            Value::F32(x) => write!(f, "{}", x.get()),
            Value::F64(x) => write!(f, "{}", x.get()),
            Value::Decimal(n) => write!(f, "decimal {n}"),
            Value::Text(t) => write!(f, "`{t}`"),
            Value::Bytes(b) => write!(f, "bytes {}", Hex(b)),
            Value::SortKey(b) => write!(f, "sort key {}", Hex(b)),
            Value::Instant(t) => write!(f, "instant {t}"),
            Value::Date(d) => write!(f, "date {d}"),
            Value::Time(t) => write!(f, "time {t}"),
            Value::DateTime(t) => write!(f, "date-time {t}"),
            Value::ZonedDateTime { local, zone } => write!(f, "date-time {local} in `{zone}`"),
            Value::Duration(d) => write!(f, "duration {d}"),
            Value::Period { months, days } => write!(f, "period of {months} months {days} days"),
            Value::Record(fields) => write!(f, "{}", Listed(fields)),
            Value::Sum { ctor, fields } => write!(f, "constructor {ctor} of {}", Listed(fields)),
        }
    }
}

/// Returns `values` in parentheses, each as its `Display` writes it.
pub fn listed(values: &[Value]) -> impl std::fmt::Display + '_ {
    Listed(values)
}

struct Listed<'a>(&'a [Value]);

impl std::fmt::Display for Listed<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(")?;
        for (at, v) in self.0.iter().enumerate() {
            if at > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{v}")?;
        }
        f.write_str(")")
    }
}

/// Returns `bytes` in hexadecimal.
pub fn hex(bytes: &[u8]) -> impl std::fmt::Display + '_ {
    Hex(bytes)
}

struct Hex<'a>(&'a [u8]);

impl std::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("0x")?;
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}
