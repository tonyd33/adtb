//! The textual syntax of values: one canonical printed form per value, read back at a known
//! type, and the versioned documents that hold rows of one relation variable, over any value
//! type that implements [`Textual`].

mod document;
mod form;
mod print;
mod read;

#[cfg(test)]
mod tests;

pub use document::{Document, Header, print_document, read_document};
pub use form::{Ctor, Form, KeyForm, key_field_ty, self_key};
pub use print::{print, to_text};
pub use read::{ParseError, parse, parse_uuid};

use crate::types::Base;

/// A value of a base type, as the textual syntax reads and prints it.
///
/// Invariants:
/// - Every integer type's value is an `Int`, and a `Decimal` is its unscaled digits.
/// - `Bytes` holds a `Bytes`, `SortKey` or `Uuid` value.
/// - `Date` counts days since 1970-01-01; `Time` microseconds since midnight; `Instant`,
///   `DateTime`, `Duration` and a `Zoned` local time microseconds, the civil ones from
///   1970-01-01T00:00:00.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar<'a> {
    Int(i64),
    Decimal(i128),
    F32(f32),
    F64(f64),
    Text(&'a str),
    Bytes(&'a [u8]),
    Instant(i64),
    Date(i64),
    Time(i64),
    DateTime(i64),
    Zoned { local: i64, zone: &'a str },
    Duration(i64),
    Period { months: i32, days: i32 },
}

/// A value type the textual syntax reads into and prints from.
///
/// The accessors are called only at the form the value was built for: `scalar` at a base form,
/// `Uuid` or a newtype's base; `ctor` at a sum or tag; `fields` at a sum, a record, a span or a
/// composite key's value; `key` at a key; `elems` at a set. Each may panic elsewhere.
///
/// Postconditions:
/// - `fields` gives a constructor's fields, header fields first, each in declaration order, and
///   a record's or a composite key's fields sorted by name.
/// - `elems` gives a set's elements once each, ascending by `Ord`.
/// - `Ord` is the model's order on ordered types.
pub trait Textual: Ord + Sized {
    fn scalar(&self) -> Scalar<'_>;
    /// Returns the constructor index of a constructor value, in declaration order or, for an
    /// anonymous sum, in name order.
    fn ctor(&self) -> u32;
    fn fields(&self) -> impl Iterator<Item = &Self>;
    /// Returns the key value of a key.
    fn key(&self) -> &Self;
    fn elems(&self) -> impl Iterator<Item = &Self>;

    /// Returns the value of base type `at` that `s` holds.
    fn of_scalar(s: Scalar<'_>, at: Base) -> Self;
    /// Returns a constructor value, its fields in the order `fields` gives them.
    fn of_ctor(ctor: u32, fields: Vec<(&str, Self)>) -> Self;
    /// Returns a record, its fields sorted by name; with none it is the unit value.
    fn of_record(fields: Vec<(&str, Self)>) -> Self;
    /// Returns the key whose key value is `value`.
    fn of_key(value: Self) -> Self;
    /// Returns the set of `elems`, which may repeat and come in any order.
    fn of_set(elems: Vec<Self>) -> Self;

    /// Returns whether the zone rules name a zone `name`.
    fn zone_exists(name: &str) -> bool;
}
