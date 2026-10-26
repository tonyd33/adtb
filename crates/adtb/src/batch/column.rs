use crate::key::{F32, F64, Scalar, Value};

use super::buffer::Buffer;
use super::types::{Cell, Ctor, Shape, Type, shape, tag_width, union_group};

/// One column of a batch in Arrow's physical layout.
///
/// Invariants:
/// - The offsets of a `Binary` or `List` are `len + 1` little-endian `i32`s, starting at 0 and
///   never decreasing.
/// - A `Union` has `len` type ids, `i8`s below its child count, and `len` little-endian `i32`
///   offsets that number the rows of each child in turn from 0.
/// - Every child of a `Struct` has the struct's length.
/// - A `Nullable`'s validity holds one bit per row of `inner`, least significant bit first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Column {
    /// Little-endian values of `width` bytes.
    Fixed {
        len: usize,
        width: usize,
        data: Buffer,
    },
    /// Variable-length bytes, row `i` at `data[offsets[i]..offsets[i + 1]]`.
    Binary {
        len: usize,
        offsets: Buffer,
        data: Buffer,
    },
    Struct {
        len: usize,
        children: Vec<Column>,
    },
    /// Row `i` is `child[offsets[i]..offsets[i + 1]]`.
    List {
        len: usize,
        offsets: Buffer,
        child: Box<Column>,
    },
    /// A dense union: row `i` is row `offsets[i]` of child `type_ids[i]`.
    Union {
        len: usize,
        type_ids: Buffer,
        offsets: Buffer,
        children: Vec<Column>,
    },
    /// `inner` with a validity bitmap; a cleared bit marks a row whose `inner` slot holds an
    /// unspecified value.
    Nullable {
        validity: Buffer,
        inner: Box<Column>,
    },
}

fn offsets() -> Buffer {
    let mut b = Buffer::new();
    b.extend(&0i32.to_le_bytes());
    b
}

fn push_offset(offsets: &mut Buffer, at: usize) {
    let at = i32::try_from(at).expect("a batch column holds fewer than 2^31 elements or bytes");
    offsets.extend(&at.to_le_bytes());
}

fn i32_at(b: &Buffer, i: usize) -> usize {
    let at = i32::from_le_bytes(b.as_slice()[4 * i..4 * i + 4].try_into().unwrap());
    usize::try_from(at).expect("an offset is never negative")
}

fn fixed(width: usize) -> Column {
    Column::Fixed {
        len: 0,
        width,
        data: Buffer::new(),
    }
}

fn binary() -> Column {
    Column::Binary {
        len: 0,
        offsets: offsets(),
        data: Buffer::new(),
    }
}

fn structure(children: Vec<Column>) -> Column {
    Column::Struct { len: 0, children }
}

fn scalar_column(s: Scalar) -> Column {
    use Scalar::*;
    match s {
        I8 => fixed(1),
        I16 => fixed(2),
        I32 | F32 | Date => fixed(4),
        I64 | F64 | Instant | Time | DateTime | Duration => fixed(8),
        Decimal(_) => fixed(16),
        Text | Bytes | SortKey => binary(),
        ZonedDateTime => structure(vec![fixed(8), binary()]),
        Period => structure(vec![fixed(4), fixed(4)]),
    }
}

fn payload(c: &Ctor) -> Column {
    structure(c.fields.iter().map(Column::new).collect())
}

fn present(c: &Ctor) -> Column {
    match c.fields.as_slice() {
        [t] => Column::new(t),
        _ => payload(c),
    }
}

fn union(ctors: &[Ctor]) -> Column {
    let group = union_group(ctors.len());
    let children = if group == 1 {
        ctors.iter().map(payload).collect()
    } else {
        ctors.chunks(group).map(union).collect()
    };
    Column::Union {
        len: 0,
        type_ids: Buffer::new(),
        offsets: Buffer::new(),
        children,
    }
}

impl Column {
    /// Returns an empty column of type `t`.
    pub(super) fn new(t: &Type) -> Column {
        match t {
            Type::Scalar(s) => scalar_column(*s),
            Type::Record(fs) => structure(fs.iter().map(Column::new).collect()),
            Type::List(e) => Column::List {
                len: 0,
                offsets: offsets(),
                child: Box::new(Column::new(e)),
            },
            Type::Sum(cs) => match shape(cs) {
                Shape::Payload => payload(&cs[0]),
                Shape::Tag => fixed(tag_width(cs.len())),
                Shape::Option { none } => Column::Nullable {
                    validity: Buffer::new(),
                    inner: Box::new(present(&cs[1 - none])),
                },
                Shape::Union => union(cs),
            },
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Column::Fixed { len, .. }
            | Column::Binary { len, .. }
            | Column::Struct { len, .. }
            | Column::List { len, .. }
            | Column::Union { len, .. } => *len,
            Column::Nullable { inner, .. } => inner.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn push_fixed(&mut self, bytes: &[u8]) {
        let Column::Fixed { len, width, data } = self else {
            panic!("column {self:?} is not fixed width");
        };
        assert_eq!(*width, bytes.len(), "a value of the wrong width");
        data.extend(bytes);
        *len += 1;
    }

    fn push_binary(&mut self, bytes: &[u8]) {
        let Column::Binary { len, offsets, data } = self else {
            panic!("column {self:?} is not binary");
        };
        data.extend(bytes);
        push_offset(offsets, data.len());
        *len += 1;
    }

    fn children_mut(&mut self) -> &mut [Column] {
        let Column::Struct { len, children } = self else {
            panic!("column {self:?} is not a struct");
        };
        *len += 1;
        children
    }

    fn push_fields<C: Cell>(&mut self, types: &[Type], fields: &[C]) {
        assert_eq!(types.len(), fields.len(), "a record of the wrong arity");
        for ((col, t), v) in self.children_mut().iter_mut().zip(types).zip(fields) {
            col.push(t, v);
        }
    }

    fn push_scalar(&mut self, s: Scalar, v: &Value) {
        match (s, v) {
            (Scalar::I8, Value::I8(x)) => self.push_fixed(&x.to_le_bytes()),
            (Scalar::I16, Value::I16(x)) => self.push_fixed(&x.to_le_bytes()),
            (Scalar::I32, Value::I32(x)) | (Scalar::Date, Value::Date(x)) => {
                self.push_fixed(&x.to_le_bytes());
            }
            (Scalar::I64, Value::I64(x))
            | (Scalar::Instant, Value::Instant(x))
            | (Scalar::Time, Value::Time(x))
            | (Scalar::DateTime, Value::DateTime(x))
            | (Scalar::Duration, Value::Duration(x)) => self.push_fixed(&x.to_le_bytes()),
            (Scalar::F32, Value::F32(x)) => self.push_fixed(&x.get().to_bits().to_le_bytes()),
            (Scalar::F64, Value::F64(x)) => self.push_fixed(&x.get().to_bits().to_le_bytes()),
            (Scalar::Decimal(_), Value::Decimal(x)) => self.push_fixed(&x.to_le_bytes()),
            (Scalar::Text, Value::Text(x)) => self.push_binary(x.as_bytes()),
            (Scalar::Bytes, Value::Bytes(x)) | (Scalar::SortKey, Value::SortKey(x)) => {
                self.push_binary(x);
            }
            (Scalar::ZonedDateTime, Value::ZonedDateTime { local, zone }) => {
                let [l, z] = self.children_mut() else {
                    unreachable!()
                };
                l.push_fixed(&local.to_le_bytes());
                z.push_binary(zone.as_bytes());
            }
            (Scalar::Period, Value::Period { months, days }) => {
                let [m, d] = self.children_mut() else {
                    unreachable!()
                };
                m.push_fixed(&months.to_le_bytes());
                d.push_fixed(&days.to_le_bytes());
            }
            _ => panic!("value {v:?} is not of type {s:?}"),
        }
    }

    /// Append `v`, a value of `t`, the type this column was made for.
    pub(super) fn push<C: Cell>(&mut self, t: &Type, v: &C) {
        match t {
            Type::Scalar(s) => self.push_scalar(*s, v.as_scalar()),
            Type::Record(ts) => self.push_fields(ts, v.as_record()),
            Type::List(e) => {
                let Column::List {
                    len,
                    offsets,
                    child,
                } = self
                else {
                    panic!("column {self:?} is not a list");
                };
                for x in v.as_list() {
                    child.push(e, x);
                }
                push_offset(offsets, child.len());
                *len += 1;
            }
            Type::Sum(cs) => {
                let (ctor, fields) = v.as_sum();
                let at = ctor as usize;
                assert!(at < cs.len(), "constructor {ctor} of a sum of {}", cs.len());
                self.push_sum(cs, at, fields);
            }
        }
    }

    fn push_sum<C: Cell>(&mut self, cs: &[Ctor], at: usize, fields: &[C]) {
        match shape(cs) {
            Shape::Payload => self.push_fields(&cs[0].fields, fields),
            Shape::Tag => {
                let width = tag_width(cs.len());
                self.push_fixed(&at.to_le_bytes()[..width]);
            }
            Shape::Option { none } => {
                let Column::Nullable { validity, inner } = self else {
                    panic!("column {self:?} is not nullable");
                };
                let row = inner.len();
                if row.is_multiple_of(8) {
                    validity.extend(&[0]);
                }
                if at == none {
                    inner.push_default();
                } else {
                    *validity.last_mut() |= 1 << (row % 8);
                    match cs[at].fields.as_slice() {
                        [t] => inner.push(t, &fields[0]),
                        ts => inner.push_fields(ts, fields),
                    }
                }
            }
            Shape::Union => self.push_union(cs, at, fields),
        }
    }

    fn push_union<C: Cell>(&mut self, cs: &[Ctor], at: usize, fields: &[C]) {
        let Column::Union {
            len,
            type_ids,
            offsets,
            children,
        } = self
        else {
            panic!("column {self:?} is not a union");
        };
        let group = union_group(cs.len());
        let id = at / group;
        type_ids.extend(&[id as u8]);
        push_offset(offsets, children[id].len());
        *len += 1;
        let child = &mut children[id];
        if group == 1 {
            child.push_fields(&cs[at].fields, fields);
        } else {
            let first = id * group;
            let members = &cs[first..cs.len().min(first + group)];
            child.push_union(members, at - first, fields);
        }
    }

    fn push_default(&mut self) {
        match self {
            Column::Fixed { width, .. } => {
                let zeros = vec![0; *width];
                self.push_fixed(&zeros);
            }
            Column::Binary { .. } => self.push_binary(&[]),
            Column::Struct { .. } => self
                .children_mut()
                .iter_mut()
                .for_each(Column::push_default),
            Column::List {
                len,
                offsets,
                child,
            } => {
                push_offset(offsets, child.len());
                *len += 1;
            }
            Column::Union {
                len,
                type_ids,
                offsets,
                children,
            } => {
                type_ids.extend(&[0]);
                push_offset(offsets, children[0].len());
                *len += 1;
                children[0].push_default();
            }
            Column::Nullable { validity, inner } => {
                if inner.len().is_multiple_of(8) {
                    validity.extend(&[0]);
                }
                inner.push_default();
            }
        }
    }

    fn fixed_at(&self, row: usize) -> &[u8] {
        let Column::Fixed { width, data, .. } = self else {
            panic!("column {self:?} is not fixed width");
        };
        &data.as_slice()[row * width..(row + 1) * width]
    }

    fn binary_at(&self, row: usize) -> &[u8] {
        let Column::Binary { offsets, data, .. } = self else {
            panic!("column {self:?} is not binary");
        };
        &data.as_slice()[i32_at(offsets, row)..i32_at(offsets, row + 1)]
    }

    fn children(&self) -> &[Column] {
        let Column::Struct { children, .. } = self else {
            panic!("column {self:?} is not a struct");
        };
        children
    }

    fn fields_at<C: Cell>(&self, types: &[Type], row: usize) -> Vec<C> {
        self.children()
            .iter()
            .zip(types)
            .map(|(col, t)| col.get(t, row))
            .collect()
    }

    fn scalar_at(&self, s: Scalar, row: usize) -> Value {
        let b = || self.fixed_at(row);
        let i32_of = |b: &[u8]| i32::from_le_bytes(b.try_into().unwrap());
        let i64_of = |b: &[u8]| i64::from_le_bytes(b.try_into().unwrap());
        let text = |b: &[u8]| String::from_utf8(b.to_vec()).expect("a text column holds UTF-8");
        match s {
            Scalar::I8 => Value::I8(i8::from_le_bytes(b().try_into().unwrap())),
            Scalar::I16 => Value::I16(i16::from_le_bytes(b().try_into().unwrap())),
            Scalar::I32 => Value::I32(i32_of(b())),
            Scalar::Date => Value::Date(i32_of(b())),
            Scalar::I64 => Value::I64(i64_of(b())),
            Scalar::Instant => Value::Instant(i64_of(b())),
            Scalar::Time => Value::Time(i64_of(b())),
            Scalar::DateTime => Value::DateTime(i64_of(b())),
            Scalar::Duration => Value::Duration(i64_of(b())),
            Scalar::F32 => Value::F32(F32::new(f32::from_bits(u32::from_le_bytes(
                b().try_into().unwrap(),
            )))),
            Scalar::F64 => Value::F64(F64::new(f64::from_bits(u64::from_le_bytes(
                b().try_into().unwrap(),
            )))),
            Scalar::Decimal(_) => Value::Decimal(i128::from_le_bytes(b().try_into().unwrap())),
            Scalar::Text => Value::Text(text(self.binary_at(row))),
            Scalar::Bytes => Value::Bytes(self.binary_at(row).to_vec()),
            Scalar::SortKey => Value::SortKey(self.binary_at(row).to_vec()),
            Scalar::ZonedDateTime => {
                let [l, z] = self.children() else {
                    unreachable!()
                };
                Value::ZonedDateTime {
                    local: i64_of(l.fixed_at(row)),
                    zone: text(z.binary_at(row)),
                }
            }
            Scalar::Period => {
                let [m, d] = self.children() else {
                    unreachable!()
                };
                Value::Period {
                    months: i32_of(m.fixed_at(row)),
                    days: i32_of(d.fixed_at(row)),
                }
            }
        }
    }

    /// Returns row `row` as a value of `t`, the type this column was made for.
    pub(super) fn get<C: Cell>(&self, t: &Type, row: usize) -> C {
        assert!(row < self.len(), "row {row} of a column of {}", self.len());
        match t {
            Type::Scalar(s) => C::scalar(self.scalar_at(*s, row)),
            Type::Record(ts) => C::record(self.fields_at(ts, row)),
            Type::List(e) => {
                let Column::List { offsets, child, .. } = self else {
                    panic!("column {self:?} is not a list");
                };
                let elems = (i32_at(offsets, row)..i32_at(offsets, row + 1))
                    .map(|i| child.get(e, i))
                    .collect();
                C::list(elems)
            }
            Type::Sum(cs) => self.sum_at(cs, row),
        }
    }

    fn sum_at<C: Cell>(&self, cs: &[Ctor], row: usize) -> C {
        let (at, fields) = match shape(cs) {
            Shape::Payload => (0, self.fields_at(&cs[0].fields, row)),
            Shape::Tag => {
                let mut tag = [0; 8];
                let b = self.fixed_at(row);
                tag[..b.len()].copy_from_slice(b);
                (usize::from_le_bytes(tag), Vec::new())
            }
            Shape::Option { none } => {
                let Column::Nullable { validity, inner } = self else {
                    panic!("column {self:?} is not nullable");
                };
                if validity.as_slice()[row / 8] & (1 << (row % 8)) == 0 {
                    (none, Vec::new())
                } else {
                    let some = 1 - none;
                    match cs[some].fields.as_slice() {
                        [t] => (some, vec![inner.get(t, row)]),
                        ts => (some, inner.fields_at(ts, row)),
                    }
                }
            }
            Shape::Union => self.union_at(cs, row),
        };
        C::sum(u32::try_from(at).unwrap(), fields)
    }

    fn union_at<C: Cell>(&self, cs: &[Ctor], row: usize) -> (usize, Vec<C>) {
        let Column::Union {
            type_ids,
            offsets,
            children,
            ..
        } = self
        else {
            panic!("column {self:?} is not a union");
        };
        let id = usize::from(type_ids.as_slice()[row]);
        let child = &children[id];
        let at = i32_at(offsets, row);
        let group = union_group(cs.len());
        if group == 1 {
            (id, child.fields_at(&cs[id].fields, at))
        } else {
            let first = id * group;
            let members = &cs[first..cs.len().min(first + group)];
            let (inner, fields) = child.union_at(members, at);
            (first + inner, fields)
        }
    }
}
