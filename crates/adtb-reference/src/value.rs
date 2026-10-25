use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::big::Big;
use crate::eval::Closure;

/// Values carry no type. A newtype's value is its base value, an integer of any width is an `Int`,
/// and a key is `Key` of its key value.
#[derive(Clone, Debug)]
pub enum Value {
    Int(i64),
    /// The unscaled digits and the scale.
    Decimal(i128, u8),
    F32(f32),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    SortKey(Vec<u8>),
    /// Microseconds since 1970-01-01T00:00:00Z.
    Instant(i64),
    /// Days since 1970-01-01.
    Date(i64),
    /// Microseconds since midnight.
    Time(i64),
    /// Microseconds since 1970-01-01T00:00:00, no zone.
    DateTime(i64),
    Zoned(i64, String),
    Duration(i64),
    Period {
        months: i32,
        days: i32,
    },
    /// A constructor value: its index in the declaration order of its sum, and its fields, header
    /// fields first, each in declaration order. A record of a declared type is constructor 0; a
    /// tag is a constructor with no fields.
    Ctor(u32, Vec<(String, Value)>),
    /// An anonymous record, fields sorted by name.
    Record(Vec<(String, Value)>),
    Key(Box<Value>),
    Set(BTreeSet<Value>),
    /// A final result delivered in order.
    List(Vec<Value>),
    Fn(Rc<Closure>),
}

impl Value {
    pub fn bool(b: bool) -> Value {
        Value::Ctor(b as u32, Vec::new())
    }

    pub fn unit() -> Value {
        Value::Record(Vec::new())
    }

    pub fn none() -> Value {
        Value::Ctor(0, Vec::new())
    }

    pub fn some(v: Value) -> Value {
        Value::Ctor(1, vec![("value".into(), v)])
    }

    pub fn key(v: Value) -> Value {
        Value::Key(Box::new(v))
    }

    pub fn set(vs: impl IntoIterator<Item = Value>) -> Value {
        Value::Set(vs.into_iter().collect())
    }

    pub fn record<'a>(fs: impl IntoIterator<Item = (&'a str, Value)>) -> Value {
        let mut fs: Vec<(String, Value)> = fs.into_iter().map(|(n, v)| (n.into(), v)).collect();
        fs.sort_by(|a, b| a.0.cmp(&b.0));
        Value::Record(fs)
    }

    pub fn text(t: &str) -> Value {
        Value::Text(t.into())
    }

    /// Returns the constructor index of a constructor value.
    pub fn tag(&self) -> Option<u32> {
        match self {
            Value::Ctor(c, _) => Some(*c),
            _ => None,
        }
    }

    /// Returns field `name` of a record or constructor value.
    pub fn field(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Ctor(_, fs) | Value::Record(fs) => {
                fs.iter().find(|(n, _)| n == name).map(|(_, v)| v)
            }
            _ => None,
        }
    }

    pub fn as_set(&self) -> Option<&BTreeSet<Value>> {
        match self {
            Value::Set(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Ctor(c @ (0 | 1), fs) if fs.is_empty() => Some(*c == 1),
            _ => None,
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Value::Int(_) => 0,
            Value::Decimal(..) => 1,
            Value::F32(_) => 2,
            Value::F64(_) => 3,
            Value::Text(_) => 4,
            Value::Bytes(_) => 5,
            Value::SortKey(_) => 6,
            Value::Instant(_) => 7,
            Value::Date(_) => 8,
            Value::Time(_) => 9,
            Value::DateTime(_) => 10,
            Value::Zoned(..) => 11,
            Value::Duration(_) => 12,
            Value::Period { .. } => 13,
            Value::Ctor(..) => 14,
            Value::Record(_) => 15,
            Value::Key(_) => 16,
            Value::Set(_) => 17,
            Value::List(_) => 18,
            Value::Fn(_) => 19,
        }
    }
}

/// Compares two decimals by their numbers, whatever their scales.
pub fn cmp_decimal(a: i128, s: u8, b: i128, t: u8) -> Ordering {
    if s == t {
        return a.cmp(&b);
    }
    let x = Big::from_i128(a).mul(&Big::pow10(t as u32));
    let y = Big::from_i128(b).mul(&Big::pow10(s as u32));
    x.cmp(&y)
}

fn fields_cmp(a: &[(String, Value)], b: &[(String, Value)]) -> Ordering {
    a.iter()
        .map(|(_, v)| v)
        .cmp(b.iter().map(|(_, v)| v))
        .then_with(|| a.iter().map(|(n, _)| n).cmp(b.iter().map(|(n, _)| n)))
}

/// The model's order on ordered types, and a total order consistent with `==` on the others.
impl Ord for Value {
    fn cmp(&self, o: &Value) -> Ordering {
        use Value::*;
        match (self, o) {
            (Int(a), Int(b)) => a.cmp(b),
            (Decimal(a, s), Decimal(b, t)) => cmp_decimal(*a, *s, *b, *t),
            // A canonical float has one NaN, positive, which the IEEE total order puts last.
            (F32(a), F32(b)) => a.total_cmp(b),
            (F64(a), F64(b)) => a.total_cmp(b),
            (Text(a), Text(b)) => a.cmp(b),
            (Bytes(a), Bytes(b)) | (SortKey(a), SortKey(b)) => a.cmp(b),
            (Instant(a), Instant(b))
            | (Date(a), Date(b))
            | (Time(a), Time(b))
            | (DateTime(a), DateTime(b))
            | (Duration(a), Duration(b)) => a.cmp(b),
            (Zoned(a, z), Zoned(b, y)) => a.cmp(b).then_with(|| z.cmp(y)),
            (Period { months, days }, Period { months: m, days: d }) => {
                months.cmp(m).then_with(|| days.cmp(d))
            }
            (Ctor(c, a), Ctor(d, b)) => c.cmp(d).then_with(|| fields_cmp(a, b)),
            (Record(a), Record(b)) => fields_cmp(a, b),
            (Key(a), Key(b)) => a.cmp(b),
            (Set(a), Set(b)) => a.cmp(b),
            (List(a), List(b)) => a.cmp(b),
            (Fn(a), Fn(b)) => Rc::as_ptr(a).cmp(&Rc::as_ptr(b)),
            _ => self.rank().cmp(&o.rank()),
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, o: &Value) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl PartialEq for Value {
    fn eq(&self, o: &Value) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Value {}

/// Returns `f` with `-0.0` made `0.0` and every NaN made the one NaN.
pub fn canon64(f: f64) -> f64 {
    if f.is_nan() {
        f64::NAN
    } else if f == 0.0 {
        0.0
    } else {
        f
    }
}

pub fn canon32(f: f32) -> f32 {
    if f.is_nan() {
        f32::NAN
    } else if f == 0.0 {
        0.0
    } else {
        f
    }
}
