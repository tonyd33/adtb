use crate::heap::FieldType;
use crate::key::{CtorId, Scalar, Value};

/// The type of a batch column. A newtype is its representation's type and `Key r` is the type
/// of `r`'s key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Scalar(Scalar),
    Record(Vec<Type>),
    /// A sum's constructors in the batch schema's order; a constructor's position in it is the
    /// `ctor` of its values.
    Sum(Vec<Ctor>),
    List(Box<Type>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ctor {
    pub id: CtorId,
    pub fields: Vec<Type>,
}

/// The layout a sum's column takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// One constructor: its payload with no tag.
    Payload,
    /// No constructor has fields: the tag vector alone.
    Tag,
    /// One nullary constructor at `none` and one other: the other's present column with a
    /// validity bitmap.
    Option { none: usize },
    /// A dense union, nested as a union of unions past [`UNION_LIMIT`] constructors.
    Union,
}

/// The most children one union has.
pub const UNION_LIMIT: usize = 127;

/// Returns the layout of a sum with constructors `ctors`.
pub fn shape(ctors: &[Ctor]) -> Shape {
    if ctors.len() == 1 {
        return Shape::Payload;
    }
    if ctors.iter().all(|c| c.fields.is_empty()) {
        return Shape::Tag;
    }
    if let [a, b] = ctors {
        let (none, some) = if a.fields.is_empty() { (0, b) } else { (1, a) };
        if ctors[none].fields.is_empty() && takes_validity(some) {
            return Shape::Option { none };
        }
    }
    Shape::Union
}

fn takes_validity(some: &Ctor) -> bool {
    match some.fields.as_slice() {
        [Type::Sum(cs)] => matches!(shape(cs), Shape::Payload | Shape::Tag),
        _ => true,
    }
}

/// Returns the width in bytes of a tag vector over `n` constructors.
pub fn tag_width(n: usize) -> usize {
    match n {
        0..=0x100 => 1,
        0x101..=0x1_0000 => 2,
        _ => 4,
    }
}

/// Returns how many leaf constructors each child of a union over `n` constructors covers.
pub(super) fn union_group(n: usize) -> usize {
    let mut group = 1;
    while group * UNION_LIMIT < n {
        group *= UNION_LIMIT;
    }
    group
}

impl From<&FieldType> for Type {
    fn from(t: &FieldType) -> Type {
        match t {
            FieldType::Scalar(s) => Type::Scalar(*s),
            FieldType::Record(fs) => Type::Record(fs.iter().map(Type::from).collect()),
            FieldType::Sum(cs) => Type::Sum(
                cs.iter()
                    .map(|c| Ctor {
                        id: c.id,
                        fields: c.fields.iter().map(Type::from).collect(),
                    })
                    .collect(),
            ),
        }
    }
}

/// A value of a [`Type`]: a [`Value`] that may hold lists.
///
/// Preconditions:
/// - A `Scalar` holds a value of a scalar type.
/// - A `Sum`'s `ctor` is a position in its sum's constructors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Datum {
    Scalar(Value),
    Record(Vec<Datum>),
    Sum { ctor: u32, fields: Vec<Datum> },
    List(Vec<Datum>),
}

impl From<Value> for Datum {
    fn from(v: Value) -> Datum {
        match v {
            Value::Record(fs) => Datum::Record(fs.into_iter().map(Datum::from).collect()),
            Value::Sum { ctor, fields } => Datum::Sum {
                ctor,
                fields: fields.into_iter().map(Datum::from).collect(),
            },
            v => Datum::Scalar(v),
        }
    }
}

/// A tree of values a column reads from and writes to.
pub(super) trait Cell: Sized {
    fn as_scalar(&self) -> &Value;
    fn as_record(&self) -> &[Self];
    fn as_sum(&self) -> (u32, &[Self]);
    fn as_list(&self) -> &[Self];
    fn scalar(v: Value) -> Self;
    fn record(fields: Vec<Self>) -> Self;
    fn sum(ctor: u32, fields: Vec<Self>) -> Self;
    fn list(elems: Vec<Self>) -> Self;
}

impl Cell for Value {
    fn as_scalar(&self) -> &Value {
        self
    }

    fn as_record(&self) -> &[Value] {
        match self {
            Value::Record(fs) => fs,
            v => panic!("value {v:?} is not a record"),
        }
    }

    fn as_sum(&self) -> (u32, &[Value]) {
        match self {
            Value::Sum { ctor, fields } => (*ctor, fields),
            v => panic!("value {v:?} is not a sum"),
        }
    }

    fn as_list(&self) -> &[Value] {
        panic!("value {self:?} is not a list: a key value holds none")
    }

    fn scalar(v: Value) -> Value {
        v
    }

    fn record(fields: Vec<Value>) -> Value {
        Value::Record(fields)
    }

    fn sum(ctor: u32, fields: Vec<Value>) -> Value {
        Value::Sum { ctor, fields }
    }

    fn list(_: Vec<Value>) -> Value {
        panic!("a key value holds no list")
    }
}

impl Cell for Datum {
    fn as_scalar(&self) -> &Value {
        match self {
            Datum::Scalar(v) => v,
            d => panic!("datum {d:?} is not a scalar"),
        }
    }

    fn as_record(&self) -> &[Datum] {
        match self {
            Datum::Record(fs) => fs,
            d => panic!("datum {d:?} is not a record"),
        }
    }

    fn as_sum(&self) -> (u32, &[Datum]) {
        match self {
            Datum::Sum { ctor, fields } => (*ctor, fields),
            d => panic!("datum {d:?} is not a sum"),
        }
    }

    fn as_list(&self) -> &[Datum] {
        match self {
            Datum::List(xs) => xs,
            d => panic!("datum {d:?} is not a list"),
        }
    }

    fn scalar(v: Value) -> Datum {
        Datum::Scalar(v)
    }

    fn record(fields: Vec<Datum>) -> Datum {
        Datum::Record(fields)
    }

    fn sum(ctor: u32, fields: Vec<Datum>) -> Datum {
        Datum::Sum { ctor, fields }
    }

    fn list(elems: Vec<Datum>) -> Datum {
        Datum::List(elems)
    }
}
