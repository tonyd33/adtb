use std::cmp::Reverse;

use crate::key::{CtorId, F32, F64, Scalar, Tid, Value};

use super::version::{tid_bits, tid_from_bits};

/// The type of a row field. A newtype is its representation's type and a reference is its target
/// key's type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldType {
    Scalar(Scalar),
    Record(Vec<FieldType>),
    /// A sum's constructors in declaration order.
    Sum(Vec<CtorType>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtorType {
    pub id: CtorId,
    /// The sum's header fields, then the constructor's own fields.
    pub fields: Vec<FieldType>,
}

/// The largest constructor payload a sum stored whole in a row's fixed part may have.
pub const FIXED_SUM_PAYLOAD: usize = 64;

/// The stored length of an out-of-line value's pointer.
pub const POINTER_LEN: usize = 16;

/// Where an out-of-line value lies in its sum's large-value store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pointer {
    pub len: u64,
    pub first: Tid,
}

impl Pointer {
    fn encode(self) -> [u8; POINTER_LEN] {
        let mut out = [0; POINTER_LEN];
        out[..8].copy_from_slice(&self.len.to_le_bytes());
        out[8..].copy_from_slice(&tid_bits(self.first).to_le_bytes());
        out
    }

    fn decode(bytes: &[u8]) -> Pointer {
        assert_eq!(
            POINTER_LEN,
            bytes.len(),
            "an out-of-line pointer is 16 bytes"
        );
        let word = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
        Pointer {
            len: word(0),
            first: tid_from_bits(word(8)).expect("an out-of-line pointer names a chunk"),
        }
    }
}

/// A variable-width field as a row stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stored {
    Inline(Vec<u8>),
    Out(Pointer),
}

impl Stored {
    fn len(&self) -> usize {
        match self {
            Stored::Inline(b) => b.len(),
            Stored::Out(_) => POINTER_LEN,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fixed {
    size: usize,
    align: usize,
}

fn align_up(n: usize, a: usize) -> usize {
    n.div_ceil(a) * a
}

fn scalar_fixed(s: Scalar) -> Option<Fixed> {
    use Scalar::*;
    let (size, align) = match s {
        I8 => (1, 1),
        I16 => (2, 2),
        I32 | F32 | Date => (4, 4),
        I64 | F64 | Instant | Time | DateTime | Duration => (8, 8),
        Period => (8, 4),
        Decimal(_) => (16, 16),
        Text | Bytes | SortKey | ZonedDateTime => return None,
    };
    Some(Fixed { size, align })
}

/// Returns the width of a sum's stored tag: none for one constructor, else the fewest of 1, 2 or
/// 4 bytes holding its largest constructor id.
fn tag_width(ctors: &[CtorType]) -> usize {
    if ctors.len() < 2 {
        return 0;
    }
    match ctors.iter().map(|c| c.id.0).max().unwrap() {
        0..=0xff => 1,
        0x100..=0xffff => 2,
        _ => 4,
    }
}

/// Returns the shape of `shapes` placed by decreasing alignment, each one's offset, and the end
/// of the last before padding.
fn pack(shapes: &[Fixed]) -> (Fixed, Vec<usize>, usize) {
    let mut order: Vec<usize> = (0..shapes.len()).collect();
    order.sort_by_key(|&i| Reverse(shapes[i].align));
    let mut offsets = vec![0; shapes.len()];
    let mut end = 0;
    for i in order {
        end = align_up(end, shapes[i].align);
        offsets[i] = end;
        end += shapes[i].size;
    }
    let align = shapes.iter().map(|s| s.align).max().unwrap_or(1);
    (
        Fixed {
            size: align_up(end, align),
            align,
        },
        offsets,
        end,
    )
}

fn fixed_of(t: &FieldType) -> Option<Fixed> {
    match t {
        FieldType::Scalar(s) => scalar_fixed(*s),
        FieldType::Record(fs) => record_fixed(fs).map(|(f, _)| f),
        FieldType::Sum(cs) => sum_fixed(cs).map(|(f, _)| f),
    }
}

fn record_fixed(fields: &[FieldType]) -> Option<(Fixed, Vec<usize>)> {
    let shapes: Vec<Fixed> = fields.iter().map(fixed_of).collect::<Option<_>>()?;
    let (f, offsets, _) = pack(&shapes);
    Some((f, offsets))
}

/// Returns the shape of a sum stored whole in the fixed part and the offset of its payload.
fn sum_fixed(ctors: &[CtorType]) -> Option<(Fixed, usize)> {
    let payloads: Vec<Fixed> = ctors
        .iter()
        .map(|c| record_fixed(&c.fields).map(|(f, _)| f))
        .collect::<Option<_>>()?;
    let size = payloads.iter().map(|p| p.size).max().unwrap_or(0);
    if size > FIXED_SUM_PAYLOAD {
        return None;
    }
    let payload_align = payloads.iter().map(|p| p.align).max().unwrap_or(1);
    let tag = tag_width(ctors);
    let at = align_up(tag, payload_align);
    let align = payload_align.max(tag).max(1);
    Some((
        Fixed {
            size: align_up(at + size, align),
            align,
        },
        at,
    ))
}

fn mismatch(t: &FieldType, v: &Value) -> ! {
    panic!("value {v:?} is not of field type {t:?}")
}

fn put_scalar(s: Scalar, v: &Value, out: &mut [u8]) {
    match (s, v) {
        (Scalar::I8, Value::I8(x)) => out.copy_from_slice(&x.to_le_bytes()),
        (Scalar::I16, Value::I16(x)) => out.copy_from_slice(&x.to_le_bytes()),
        (Scalar::I32, Value::I32(x)) | (Scalar::Date, Value::Date(x)) => {
            out.copy_from_slice(&x.to_le_bytes())
        }
        (Scalar::I64, Value::I64(x))
        | (Scalar::Instant, Value::Instant(x))
        | (Scalar::Time, Value::Time(x))
        | (Scalar::DateTime, Value::DateTime(x))
        | (Scalar::Duration, Value::Duration(x)) => out.copy_from_slice(&x.to_le_bytes()),
        (Scalar::F32, Value::F32(x)) => out.copy_from_slice(&x.get().to_bits().to_le_bytes()),
        (Scalar::F64, Value::F64(x)) => out.copy_from_slice(&x.get().to_bits().to_le_bytes()),
        (Scalar::Decimal(_), Value::Decimal(x)) => out.copy_from_slice(&x.to_le_bytes()),
        (Scalar::Period, Value::Period { months, days }) => {
            out[..4].copy_from_slice(&months.to_le_bytes());
            out[4..].copy_from_slice(&days.to_le_bytes());
        }
        _ => mismatch(&FieldType::Scalar(s), v),
    }
}

fn get_scalar(s: Scalar, b: &[u8]) -> Value {
    let i32_at = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let i64_at = || i64::from_le_bytes(b[..8].try_into().unwrap());
    match s {
        Scalar::I8 => Value::I8(b[0] as i8),
        Scalar::I16 => Value::I16(i16::from_le_bytes([b[0], b[1]])),
        Scalar::I32 => Value::I32(i32_at(0)),
        Scalar::Date => Value::Date(i32_at(0)),
        Scalar::I64 => Value::I64(i64_at()),
        Scalar::Instant => Value::Instant(i64_at()),
        Scalar::Time => Value::Time(i64_at()),
        Scalar::DateTime => Value::DateTime(i64_at()),
        Scalar::Duration => Value::Duration(i64_at()),
        Scalar::F32 => Value::F32(F32::new(f32::from_bits(i32_at(0) as u32))),
        Scalar::F64 => Value::F64(F64::new(f64::from_bits(i64_at() as u64))),
        Scalar::Decimal(_) => Value::Decimal(i128::from_le_bytes(b[..16].try_into().unwrap())),
        Scalar::Period => Value::Period {
            months: i32_at(0),
            days: i32_at(4),
        },
        Scalar::Text | Scalar::Bytes | Scalar::SortKey | Scalar::ZonedDateTime => {
            unreachable!("{s:?} has no fixed width")
        }
    }
}

fn sum_parts<'v>(t: &FieldType, ctors: &[CtorType], v: &'v Value) -> (usize, &'v [Value]) {
    match v {
        Value::Sum { ctor, fields } if (*ctor as usize) < ctors.len() => (*ctor as usize, fields),
        _ => mismatch(t, v),
    }
}

fn record_parts<'v>(t: &FieldType, n: usize, v: &'v Value) -> &'v [Value] {
    match v {
        Value::Record(vs) if vs.len() == n => vs,
        _ => mismatch(t, v),
    }
}

fn ctor_index(ctors: &[CtorType], id: u32) -> u32 {
    let i = ctors
        .iter()
        .position(|c| c.id.0 == id)
        .unwrap_or_else(|| panic!("stored tag {id} names no constructor"));
    i as u32
}

fn put_record_fixed(fields: &[FieldType], vs: &[Value], out: &mut [u8]) {
    let (_, offsets) = record_fixed(fields).expect("a fixed record");
    for ((t, v), at) in fields.iter().zip(vs).zip(offsets) {
        let size = fixed_of(t).unwrap().size;
        put_fixed(t, v, &mut out[at..at + size]);
    }
}

fn get_record_fixed(fields: &[FieldType], b: &[u8]) -> Vec<Value> {
    let (_, offsets) = record_fixed(fields).expect("a fixed record");
    fields
        .iter()
        .zip(offsets)
        .map(|(t, at)| get_fixed(t, &b[at..at + fixed_of(t).unwrap().size]))
        .collect()
}

fn put_tag(width: usize, id: u32, out: &mut [u8]) {
    out[..width].copy_from_slice(&id.to_le_bytes()[..width]);
}

fn get_tag(width: usize, b: &[u8]) -> u32 {
    let mut word = [0; 4];
    word[..width].copy_from_slice(&b[..width]);
    u32::from_le_bytes(word)
}

fn put_fixed(t: &FieldType, v: &Value, out: &mut [u8]) {
    match t {
        FieldType::Scalar(s) => put_scalar(*s, v, out),
        FieldType::Record(fs) => put_record_fixed(fs, record_parts(t, fs.len(), v), out),
        FieldType::Sum(cs) => {
            let (ctor, fields) = sum_parts(t, cs, v);
            let (_, at) = sum_fixed(cs).expect("a fixed sum");
            put_tag(tag_width(cs), cs[ctor].id.0, out);
            put_record_fixed(&cs[ctor].fields, fields, &mut out[at..]);
        }
    }
}

fn get_fixed(t: &FieldType, b: &[u8]) -> Value {
    match t {
        FieldType::Scalar(s) => get_scalar(*s, b),
        FieldType::Record(fs) => Value::Record(get_record_fixed(fs, b)),
        FieldType::Sum(cs) => {
            let (_, at) = sum_fixed(cs).expect("a fixed sum");
            let ctor = ctor_of(cs, b);
            Value::Sum {
                ctor,
                fields: get_record_fixed(&cs[ctor as usize].fields, &b[at..]),
            }
        }
    }
}

fn ctor_of(ctors: &[CtorType], b: &[u8]) -> u32 {
    match tag_width(ctors) {
        0 => 0,
        w => ctor_index(ctors, get_tag(w, b)),
    }
}

pub(crate) fn put_uvarint(mut n: u64, out: &mut Vec<u8>) {
    while n >= 0x80 {
        out.push(n as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        head
    }

    fn rest(&mut self) -> &'a [u8] {
        self.take(self.bytes.len())
    }

    fn uvarint(&mut self) -> u64 {
        let mut n = 0;
        let mut shift = 0;
        loop {
            let b = self.take(1)[0];
            n |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                return n;
            }
            shift += 7;
        }
    }

    fn sized(&mut self) -> &'a [u8] {
        let n = self.uvarint() as usize;
        self.take(n)
    }
}

/// Append a variable-width scalar; with `prefixed` false its last run of bytes takes the rest of
/// the field, unprefixed.
fn put_var_scalar(s: Scalar, v: &Value, out: &mut Vec<u8>, prefixed: bool) {
    let run = |bytes: &[u8], out: &mut Vec<u8>| {
        if prefixed {
            put_uvarint(bytes.len() as u64, out);
        }
        out.extend_from_slice(bytes);
    };
    match (s, v) {
        (Scalar::Text, Value::Text(x)) => run(x.as_bytes(), out),
        (Scalar::Bytes, Value::Bytes(x)) | (Scalar::SortKey, Value::SortKey(x)) => run(x, out),
        (Scalar::ZonedDateTime, Value::ZonedDateTime { local, zone }) => {
            out.extend_from_slice(&local.to_le_bytes());
            run(zone.as_bytes(), out);
        }
        _ => mismatch(&FieldType::Scalar(s), v),
    }
}

fn run<'a>(r: &mut Reader<'a>, prefixed: bool) -> &'a [u8] {
    if prefixed { r.sized() } else { r.rest() }
}

fn get_var_scalar(s: Scalar, r: &mut Reader, prefixed: bool) -> Value {
    let run = |r: &mut Reader| run(r, prefixed).to_vec();
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).expect("stored text is UTF-8");
    match s {
        Scalar::Text => Value::Text(text(&run(r))),
        Scalar::Bytes => Value::Bytes(run(r)),
        Scalar::SortKey => Value::SortKey(run(r)),
        Scalar::ZonedDateTime => {
            let local = i64::from_le_bytes(r.take(8).try_into().unwrap());
            Value::ZonedDateTime {
                local,
                zone: text(&run(r)),
            }
        }
        _ => unreachable!("{s:?} has a fixed width"),
    }
}

/// Append `v` in the variable-width encoding: fields in declaration order, fixed-width scalars
/// little-endian, other scalars length-prefixed, a sum's tag as an unsigned LEB128 varint.
fn put_packed(t: &FieldType, v: &Value, out: &mut Vec<u8>) {
    match t {
        FieldType::Scalar(s) => match scalar_fixed(*s) {
            Some(f) => {
                let at = out.len();
                out.resize(at + f.size, 0);
                put_scalar(*s, v, &mut out[at..]);
            }
            None => put_var_scalar(*s, v, out, true),
        },
        FieldType::Record(fs) => {
            for (t, v) in fs.iter().zip(record_parts(t, fs.len(), v)) {
                put_packed(t, v, out);
            }
        }
        FieldType::Sum(cs) => {
            let (ctor, fields) = sum_parts(t, cs, v);
            if cs.len() > 1 {
                put_uvarint(u64::from(cs[ctor].id.0), out);
            }
            put_payload(&cs[ctor].fields, fields, out);
        }
    }
}

fn put_payload(fields: &[FieldType], vs: &[Value], out: &mut Vec<u8>) {
    assert_eq!(
        fields.len(),
        vs.len(),
        "payload has the wrong number of fields"
    );
    for (t, v) in fields.iter().zip(vs) {
        put_packed(t, v, out);
    }
}

fn get_packed(t: &FieldType, r: &mut Reader) -> Value {
    match t {
        FieldType::Scalar(s) => match scalar_fixed(*s) {
            Some(f) => get_scalar(*s, r.take(f.size)),
            None => get_var_scalar(*s, r, true),
        },
        FieldType::Record(fs) => Value::Record(fs.iter().map(|t| get_packed(t, r)).collect()),
        FieldType::Sum(cs) => {
            let ctor = if cs.len() > 1 {
                let id = u32::try_from(r.uvarint()).expect("a constructor id fits 32 bits");
                ctor_index(cs, id)
            } else {
                0
            };
            Value::Sum {
                ctor,
                fields: get_payload(&cs[ctor as usize].fields, r),
            }
        }
    }
}

fn get_payload(fields: &[FieldType], r: &mut Reader) -> Vec<Value> {
    fields.iter().map(|t| get_packed(t, r)).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// The whole value, at `at` in the fixed part.
    Fixed {
        at: usize,
        size: usize,
    },
    /// A sum's tag at `at` in the fixed part, its payload variable-width field `var`.
    Split {
        at: usize,
        var: usize,
    },
    Var {
        var: usize,
    },
}

/// Where a row format places each field of its rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    fields: Vec<FieldType>,
    places: Vec<Place>,
    fixed_len: usize,
    align: usize,
    vars: usize,
}

impl Layout {
    pub fn new(fields: Vec<FieldType>) -> Layout {
        let mut shapes = Vec::new();
        let mut placed = Vec::new();
        let mut vars = 0;
        for t in &fields {
            if let Some(f) = fixed_of(t) {
                placed.push((Some(shapes.len()), None));
                shapes.push(f);
                continue;
            }
            let tag = match t {
                FieldType::Sum(cs) => tag_width(cs),
                _ => 0,
            };
            if tag > 0 {
                placed.push((Some(shapes.len()), Some(vars)));
                shapes.push(Fixed {
                    size: tag,
                    align: tag,
                });
            } else {
                placed.push((None, Some(vars)));
            }
            vars += 1;
        }
        let (shape, offsets, end) = pack(&shapes);
        let places = placed
            .into_iter()
            .map(|p| match p {
                (Some(s), None) => Place::Fixed {
                    at: offsets[s],
                    size: shapes[s].size,
                },
                (Some(s), Some(var)) => Place::Split {
                    at: offsets[s],
                    var,
                },
                (None, Some(var)) => Place::Var { var },
                (None, None) => unreachable!(),
            })
            .collect();
        Layout {
            fields,
            places,
            fixed_len: end,
            align: shape.align,
            vars,
        }
    }

    pub fn fields(&self) -> &[FieldType] {
        &self.fields
    }

    /// Returns whether every field lies in the fixed part.
    pub fn is_fixed(&self) -> bool {
        self.vars == 0
    }

    /// Returns the length of the fixed part, padded to its alignment.
    pub fn fixed_width(&self) -> usize {
        align_up(self.fixed_len, self.align)
    }

    fn table_at(&self) -> usize {
        align_up(self.fixed_len, 2)
    }

    fn bitmap_at(&self) -> usize {
        self.table_at() + 2 * self.vars
    }

    fn vars_at(&self) -> usize {
        self.bitmap_at() + self.vars.div_ceil(8)
    }

    /// Returns the fixed part of `row` and its variable-width fields, all inline.
    ///
    /// Panics if `row` is not a row of the layout's field types.
    pub fn encode(&self, row: &[Value]) -> (Vec<u8>, Vec<Vec<u8>>) {
        assert_eq!(
            self.fields.len(),
            row.len(),
            "row has the wrong number of fields"
        );
        let mut fixed = vec![0; self.fixed_len];
        let mut vars = vec![Vec::new(); self.vars];
        for ((t, v), place) in self.fields.iter().zip(row).zip(&self.places) {
            match *place {
                Place::Fixed { at, size } => put_fixed(t, v, &mut fixed[at..at + size]),
                Place::Split { at, var } => {
                    let FieldType::Sum(cs) = t else {
                        unreachable!()
                    };
                    let (ctor, fields) = sum_parts(t, cs, v);
                    put_tag(tag_width(cs), cs[ctor].id.0, &mut fixed[at..]);
                    put_payload(&cs[ctor].fields, fields, &mut vars[var]);
                }
                Place::Var { var } => match t {
                    FieldType::Scalar(s) => put_var_scalar(*s, v, &mut vars[var], false),
                    _ => put_packed(t, v, &mut vars[var]),
                },
            }
        }
        (fixed, vars)
    }

    /// Returns the length of a body holding `vars`.
    pub fn body_len(&self, vars: &[Stored]) -> usize {
        if self.vars == 0 {
            return self.fixed_len;
        }
        self.vars_at() + vars.iter().map(Stored::len).sum::<usize>()
    }

    /// Returns a row body: the fixed part, a table of `u16` end offsets of the variable-width
    /// fields, a bitmap of those stored out of line, then their bytes.
    pub fn assemble(&self, fixed: &[u8], vars: &[Stored]) -> Vec<u8> {
        assert_eq!(self.vars, vars.len(), "body has the wrong number of fields");
        let mut out = fixed.to_vec();
        if self.vars == 0 {
            return out;
        }
        out.resize(self.vars_at(), 0);
        for (j, v) in vars.iter().enumerate() {
            match v {
                Stored::Inline(b) => out.extend_from_slice(b),
                Stored::Out(p) => {
                    out.extend_from_slice(&p.encode());
                    out[self.bitmap_at() + j / 8] |= 1 << (j % 8);
                }
            }
            let end = u16::try_from(out.len()).expect("a row body fits 64 KiB");
            let at = self.table_at() + 2 * j;
            out[at..at + 2].copy_from_slice(&end.to_le_bytes());
        }
        out
    }

    /// Returns the fixed part of `body` and its variable-width fields as stored.
    pub fn split<'b>(&self, body: &'b [u8]) -> (&'b [u8], Vec<Stored>) {
        let fixed = &body[..self.fixed_len];
        let mut start = self.vars_at();
        let mut vars = Vec::with_capacity(self.vars);
        for j in 0..self.vars {
            let at = self.table_at() + 2 * j;
            let end = usize::from(u16::from_le_bytes([body[at], body[at + 1]]));
            let bytes = &body[start..end];
            vars.push(if body[self.bitmap_at() + j / 8] >> (j % 8) & 1 == 1 {
                Stored::Out(Pointer::decode(bytes))
            } else {
                Stored::Inline(bytes.to_vec())
            });
            start = end;
        }
        (fixed, vars)
    }

    /// Returns the row whose fixed part is `fixed` and whose variable-width fields are `vars`.
    pub fn decode(&self, fixed: &[u8], vars: &[Vec<u8>]) -> Vec<Value> {
        self.fields
            .iter()
            .zip(&self.places)
            .map(|(t, place)| match *place {
                Place::Fixed { at, size } => get_fixed(t, &fixed[at..at + size]),
                Place::Split { at, var } => {
                    let FieldType::Sum(cs) = t else {
                        unreachable!()
                    };
                    let ctor = ctor_of(cs, &fixed[at..]);
                    let mut r = Reader { bytes: &vars[var] };
                    let fields = get_payload(&cs[ctor as usize].fields, &mut r);
                    assert!(r.bytes.is_empty(), "payload has trailing bytes");
                    Value::Sum { ctor, fields }
                }
                Place::Var { var } => {
                    let mut r = Reader { bytes: &vars[var] };
                    let v = match t {
                        FieldType::Scalar(s) => get_var_scalar(*s, &mut r, false),
                        _ => get_packed(t, &mut r),
                    };
                    assert!(r.bytes.is_empty(), "field has trailing bytes");
                    v
                }
            })
            .collect()
    }
}
