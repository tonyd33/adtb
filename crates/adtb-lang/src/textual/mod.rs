//! The textual syntax of values: one canonical printed form per value, read back at a known
//! type, and the versioned documents that hold rows of one relation variable, over any value
//! type that implements [`Textual`].

mod document;
mod form;
mod print;
mod read;

pub use document::{Document, Header, print_document, read_document};
pub use form::{Ctor, Form, KeyForm, key_field_ty};
pub use print::{PrintError, print, to_text};
pub use read::{ParseError, parse, parse_uuid};

use std::collections::btree_set;
use std::slice;

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
/// Each accessor returns `None` at a value of another shape: `scalar` at anything but a scalar,
/// `ctor` at anything but a constructor value, `fields` at anything but a constructor value or a
/// record, `key` at anything but a key when keys are values of their own, `elems` at anything
/// but a set or an ordered result.
///
/// Postconditions:
/// - `fields` gives a constructor's fields, header fields first, each in declaration order, and
///   a record's or a composite key's fields sorted by name.
/// - `elems` gives a set's elements once each, ascending by `Ord`, and an ordered result's
///   elements in its order.
/// - `Ord` is the model's order on ordered types.
pub trait Textual: Ord + Sized + std::fmt::Debug {
    fn scalar(&self) -> Option<Scalar<'_>>;
    /// Returns the constructor index of a constructor value, in declaration order or, for an
    /// anonymous sum, in name order.
    fn ctor(&self) -> Option<u32>;
    fn fields(&self) -> Option<impl Iterator<Item = &Self>>;
    /// Returns the key value of a key.
    fn key(&self) -> Option<&Self>;
    fn elems(&self) -> Option<impl Iterator<Item = &Self>>;

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

/// The elements of a set or of an ordered result, as [`Textual::elems`] gives them.
#[derive(Debug)]
pub enum Elems<'v, V> {
    Set(btree_set::Iter<'v, V>),
    List(slice::Iter<'v, V>),
}

impl<'v, V> Iterator for Elems<'v, V> {
    type Item = &'v V;

    fn next(&mut self) -> Option<&'v V> {
        match self {
            Elems::Set(i) => i.next(),
            Elems::List(i) => i.next(),
        }
    }
}
