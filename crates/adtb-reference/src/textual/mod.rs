//! The textual syntax of values over the reference's `Value`.

use adtb_lang::schema::Schema;
use adtb_lang::textual::{self as syntax, Elems, Form, PrintError, Scalar, Textual};
use adtb_lang::types::{Base, Ty};

pub use adtb_lang::textual::{Header, ParseError};

use crate::value::Value;
use crate::zone;

pub type Document = syntax::Document<Value>;

/// Returns `text(v)` for `v` of type `ty`.
pub(crate) fn to_text(schema: &Schema, v: &Value, ty: &Ty) -> Result<String, PrintError> {
    syntax::to_text(v, &Form::of(schema, ty))
}

/// Returns the textual syntax of `v`, a value of `ty`.
///
/// Fails when `v` is not a value of `ty`.
pub fn print(schema: &Schema, v: &Value, ty: &Ty) -> Result<String, PrintError> {
    syntax::print(v, &Form::of(schema, ty))
}

/// Reads `text`, the textual syntax of one value, at type `ty`.
pub fn parse(schema: &Schema, text: &str, ty: &Ty) -> Result<Value, ParseError> {
    syntax::parse(text, &Form::of(schema, ty))
}

/// Reads a document: an optional header line `text EDITION pins VERSION (HASH)`, then
/// `data RELATION`, then the rows of the relation in the textual syntax, in any order.
pub fn read_document(schema: &Schema, text: &str) -> Result<Document, ParseError> {
    syntax::read_document(schema, text)
}

/// Returns `doc` as text: its header, its `data` line, and one row per line in ascending order.
pub fn print_document(schema: &Schema, doc: &Document) -> Result<String, PrintError> {
    syntax::print_document(schema, doc)
}

fn values(fs: &[(String, Value)]) -> impl Iterator<Item = &Value> {
    fs.iter().map(|(_, v)| v)
}

fn named(fs: Vec<(&str, Value)>) -> Vec<(String, Value)> {
    fs.into_iter().map(|(n, v)| (n.to_string(), v)).collect()
}

impl Textual for Value {
    fn scalar(&self) -> Option<Scalar<'_>> {
        Some(match self {
            Value::Int(x) => Scalar::Int(*x),
            Value::Decimal(d, _) => Scalar::Decimal(*d),
            Value::F32(f) => Scalar::F32(*f),
            Value::F64(f) => Scalar::F64(*f),
            Value::Text(t) => Scalar::Text(t),
            Value::Bytes(b) | Value::SortKey(b) => Scalar::Bytes(b),
            Value::Instant(t) => Scalar::Instant(*t),
            Value::Date(d) => Scalar::Date(*d),
            Value::Time(t) => Scalar::Time(*t),
            Value::DateTime(t) => Scalar::DateTime(*t),
            Value::Zoned(local, zone) => Scalar::Zoned {
                local: *local,
                zone,
            },
            Value::Duration(d) => Scalar::Duration(*d),
            Value::Period { months, days } => Scalar::Period {
                months: *months,
                days: *days,
            },
            _ => return None,
        })
    }

    fn ctor(&self) -> Option<u32> {
        match self {
            Value::Ctor(c, _) => Some(*c),
            _ => None,
        }
    }

    fn fields(&self) -> Option<impl Iterator<Item = &Value>> {
        match self {
            Value::Ctor(_, fs) | Value::Record(fs) => Some(values(fs)),
            _ => None,
        }
    }

    fn key(&self) -> Option<&Value> {
        match self {
            Value::Key(k) => Some(k),
            _ => None,
        }
    }

    fn elems(&self) -> Option<impl Iterator<Item = &Value>> {
        match self {
            Value::Set(s) => Some(Elems::Set(s.iter())),
            Value::List(xs) => Some(Elems::List(xs.iter())),
            _ => None,
        }
    }

    fn of_scalar(s: Scalar<'_>, at: Base) -> Value {
        match (s, at) {
            (Scalar::Int(x), _) => Value::Int(x),
            (Scalar::Decimal(d), Base::Decimal(scale)) => Value::Decimal(d, scale),
            (Scalar::F32(f), _) => Value::F32(f),
            (Scalar::F64(f), _) => Value::F64(f),
            (Scalar::Text(t), _) => Value::text(t),
            (Scalar::Bytes(b), Base::SortKey) => Value::SortKey(b.to_vec()),
            (Scalar::Bytes(b), _) => Value::Bytes(b.to_vec()),
            (Scalar::Instant(t), _) => Value::Instant(t),
            (Scalar::Date(d), _) => Value::Date(d),
            (Scalar::Time(t), _) => Value::Time(t),
            (Scalar::DateTime(t), _) => Value::DateTime(t),
            (Scalar::Zoned { local, zone }, _) => Value::Zoned(local, zone.to_string()),
            (Scalar::Duration(d), _) => Value::Duration(d),
            (Scalar::Period { months, days }, _) => Value::Period { months, days },
            (s, b) => panic!("{s:?} is not a value of {b:?}"),
        }
    }

    fn of_ctor(ctor: u32, fields: Vec<(&str, Value)>) -> Value {
        Value::Ctor(ctor, named(fields))
    }

    fn of_record(fields: Vec<(&str, Value)>) -> Value {
        Value::Record(named(fields))
    }

    fn of_key(value: Value) -> Value {
        Value::key(value)
    }

    fn of_set(elems: Vec<Value>) -> Value {
        Value::set(elems)
    }

    fn zone_exists(name: &str) -> bool {
        zone::exists(name)
    }
}
