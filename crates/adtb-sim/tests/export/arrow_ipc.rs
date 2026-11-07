use std::collections::BTreeSet;
use std::io::Cursor;
use std::sync::Arc;

use adtb::batch::ipc::{
    self, Budget, FileCtor, FileField, FileSchema, FileType, IpcError, SumKind,
};
use adtb::batch::{Batch, BatchBuilder, Datum};
use adtb::key::{CtorId, F32, F64, Scalar, Value};
use adtb_testkit::Rng;
use adtb_testkit::profile::{every_case, sweep};
use arrow_array::{Array, ArrayRef, Int32Array, RecordBatch, StructArray, UnionArray, make_array};
use arrow_buffer::{ArrowNativeType, BooleanBuffer, Buffer, MutableBuffer, NullBuffer, bit_util};
use arrow_data::ArrayData;
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{DataType, Field, Schema, TimeUnit, UnionFields, UnionMode};

const SCALARS: [Scalar; 17] = [
    Scalar::I8,
    Scalar::I16,
    Scalar::I32,
    Scalar::I64,
    Scalar::F32,
    Scalar::F64,
    Scalar::Decimal(4),
    Scalar::Text,
    Scalar::Bytes,
    Scalar::SortKey,
    Scalar::Instant,
    Scalar::Date,
    Scalar::Time,
    Scalar::DateTime,
    Scalar::ZonedDateTime,
    Scalar::Duration,
    Scalar::Period,
];

/// Zoned values and their textual syntax.
const ZONED: [(i64, &str, &str); 2] = [
    (0, "Asia/Tokyo", "1970-01-01T00:00:00.000000[Asia/Tokyo]"),
    (
        86_400_000_001,
        "Europe/Paris",
        "1970-01-02T00:00:00.000001[Europe/Paris]",
    ),
];

/// The Arrow layout the external type mapping gives a type.
#[derive(Clone, Copy)]
enum L<'a> {
    Scalar(Scalar),
    Bool,
    Enum(&'a [FileCtor]),
    Struct(&'a [FileField], bool),
    List(&'a FileType),
    Union(&'a [FileCtor], usize),
    Nullable(&'a FileType),
}

fn layout(t: &FileType) -> L<'_> {
    match t {
        FileType::Scalar(s) => L::Scalar(*s),
        FileType::Record(fs) => L::Struct(fs, false),
        FileType::List(e) => L::List(e),
        FileType::Sum(_, cs) if cs.len() == 1 => L::Struct(&cs[0].fields, true),
        FileType::Sum(SumKind::Bool, _) => L::Bool,
        FileType::Sum(_, cs) if cs.iter().all(|c| c.fields.is_empty()) => L::Enum(cs),
        FileType::Sum(SumKind::Option, cs) => {
            let inner = &cs[1].fields[0].ty;
            let nested = matches!(inner, FileType::Sum(SumKind::Option, _));
            if nested || matches!(layout(inner), L::Union(..)) {
                L::Union(cs, 0)
            } else {
                L::Nullable(inner)
            }
        }
        FileType::Sum(_, cs) => L::Union(cs, 0),
    }
}

/// Returns how many constructors each child of a union over `n` covers.
fn group(n: usize) -> usize {
    let mut g = 1;
    while 127 * g < n {
        g *= 127;
    }
    g
}

fn kids(cs: &[FileCtor], base: usize) -> Vec<(String, L<'_>)> {
    let g = group(cs.len());
    if g == 1 {
        return cs
            .iter()
            .map(|c| (c.name.clone(), L::Struct(&c.fields, true)))
            .collect();
    }
    let groups = cs.chunks(g).enumerate();
    groups
        .map(|(j, ch)| (j.to_string(), L::Union(ch, base + j * g)))
        .collect()
}

fn field(name: &str, t: &FileType) -> Field {
    match layout(t) {
        L::Nullable(inner) => field(name, inner).with_nullable(true),
        l => Field::new(name, dtype(l), false),
    }
}

fn dtype(l: L<'_>) -> DataType {
    let micros = TimeUnit::Microsecond;
    match l {
        L::Scalar(s) => match s {
            Scalar::I8 => DataType::Int8,
            Scalar::I16 => DataType::Int16,
            Scalar::I32 => DataType::Int32,
            Scalar::I64 => DataType::Int64,
            Scalar::F32 => DataType::Float32,
            Scalar::F64 => DataType::Float64,
            Scalar::Decimal(s) => DataType::Decimal128(38, i8::try_from(s).unwrap()),
            Scalar::Text | Scalar::Period | Scalar::ZonedDateTime => DataType::Utf8,
            Scalar::Bytes | Scalar::SortKey => DataType::Binary,
            Scalar::Instant => DataType::Timestamp(micros, Some("UTC".into())),
            Scalar::DateTime => DataType::Timestamp(micros, None),
            Scalar::Date => DataType::Date32,
            Scalar::Time => DataType::Time64(micros),
            Scalar::Duration => DataType::Duration(micros),
        },
        L::Bool => DataType::Boolean,
        L::Enum(_) => DataType::Dictionary(Box::new(DataType::Int32), Box::new(DataType::Utf8)),
        L::Struct(fs, _) => DataType::Struct(fs.iter().map(|f| field(&f.name, &f.ty)).collect()),
        L::List(e) => DataType::List(Arc::new(field("item", e))),
        L::Union(cs, base) => {
            let ks = kids(cs, base);
            let fs = ks.iter().map(|(n, l)| Field::new(n, dtype(*l), false));
            DataType::Union(
                UnionFields::try_new(0..ks.len() as i8, fs).unwrap(),
                UnionMode::Dense,
            )
        }
        L::Nullable(_) => unreachable!("a nullable layout is a field's"),
    }
}

pub(crate) fn schema(file: &FileSchema) -> Schema {
    Schema::new(
        file.fields()
            .iter()
            .map(|f| field(&f.name, &f.ty))
            .collect::<Vec<_>>(),
    )
}

fn period(v: &Value) -> Option<String> {
    match v {
        Value::Period { months, days } => Some(format!("P{months}M{days}D")),
        Value::ZonedDateTime { local, zone } => ZONED
            .iter()
            .find(|z| (z.0, z.1) == (*local, zone.as_str()))
            .map(|z| z.2.to_string()),
        _ => None,
    }
}

fn le(v: &Value) -> Vec<u8> {
    match v {
        Value::I8(x) => x.to_le_bytes().to_vec(),
        Value::I16(x) => x.to_le_bytes().to_vec(),
        Value::I32(x) | Value::Date(x) => x.to_le_bytes().to_vec(),
        Value::I64(x)
        | Value::Instant(x)
        | Value::Time(x)
        | Value::DateTime(x)
        | Value::Duration(x) => x.to_le_bytes().to_vec(),
        Value::F32(x) => x.get().to_le_bytes().to_vec(),
        Value::F64(x) => x.get().to_le_bytes().to_vec(),
        Value::Decimal(x) => x.to_le_bytes().to_vec(),
        Value::Text(t) => t.as_bytes().to_vec(),
        Value::Bytes(b) | Value::SortKey(b) => b.clone(),
        v => period(v).expect("a zoned value of the table").into_bytes(),
    }
}

fn width(s: Scalar) -> Option<usize> {
    match s {
        Scalar::I8 => Some(1),
        Scalar::I16 => Some(2),
        Scalar::I32 | Scalar::F32 | Scalar::Date => Some(4),
        Scalar::Decimal(_) => Some(16),
        Scalar::Text | Scalar::Bytes | Scalar::SortKey | Scalar::Period | Scalar::ZonedDateTime => {
            None
        }
        _ => Some(8),
    }
}

fn fields_of(d: &Datum, sum: bool) -> &[Datum] {
    match d {
        Datum::Sum { fields, .. } if sum => fields,
        Datum::Record(fs) => fs,
        d => panic!("{d:?} has no fields"),
    }
}

fn ctor(d: &Datum) -> usize {
    let Datum::Sum { ctor, .. } = d else {
        panic!("{d:?} is not a sum")
    };
    *ctor as usize
}

fn some(d: &Datum) -> Option<&Datum> {
    match d {
        Datum::Sum { ctor: 1, fields } => Some(&fields[0]),
        _ => None,
    }
}

fn list<'a>(v: &Option<&'a Datum>) -> &'a [Datum] {
    match v {
        Some(Datum::List(xs)) => xs,
        _ => &[],
    }
}

/// Returns `xs` in a buffer aligned as Arrow requires.
fn aligned<T: ArrowNativeType>(xs: impl IntoIterator<Item = T>) -> Buffer {
    MutableBuffer::from_iter(xs).into()
}

fn offsets(lens: impl Iterator<Item = usize>) -> Buffer {
    let mut at = 0;
    let os = std::iter::once(0).chain(lens.map(|l| {
        at += l;
        i32::try_from(at).unwrap()
    }));
    aligned(os)
}

/// Returns the Arrow data of `vals` in layout `l`; `None` is a null or a row under one.
fn build(l: L<'_>, vals: &[Option<&Datum>], nullable: bool) -> ArrayData {
    if let L::Nullable(inner) = l {
        let present: Vec<_> = vals.iter().map(|v| v.and_then(some)).collect();
        return build(layout(inner), &present, true);
    }
    let valid: Vec<bool> = vals.iter().map(Option::is_some).collect();
    let nulls = nullable.then(|| NullBuffer::from(valid));
    let b = ArrayData::builder(dtype(l)).len(vals.len());
    let b = if matches!(l, L::Union(..)) {
        b
    } else {
        b.nulls(nulls)
    };
    let scalar = |v: &Option<&Datum>| match v {
        Some(Datum::Scalar(v)) => Some(le(v)),
        _ => None,
    };
    let b = match l {
        L::Scalar(s) => match width(s) {
            Some(w) => b.add_buffer(aligned(
                vals.iter().flat_map(|v| scalar(v).unwrap_or(vec![0; w])),
            )),
            None => {
                let bytes: Vec<Vec<u8>> =
                    vals.iter().map(|v| scalar(v).unwrap_or_default()).collect();
                b.add_buffer(offsets(bytes.iter().map(Vec::len)))
                    .add_buffer(aligned(bytes.concat()))
            }
        },
        L::Bool => b.add_buffer(
            BooleanBuffer::from_iter(vals.iter().map(|v| v.is_some_and(|d| ctor(d) == 1)))
                .into_inner(),
        ),
        L::Enum(cs) => {
            let keys = vals
                .iter()
                .map(|v| v.map_or(0, |d| (cs.len() - 1 - ctor(d)) as i32));
            let names: Vec<&str> = cs.iter().rev().map(|c| c.name.as_str()).collect();
            let dict = arrow_array::StringArray::from(names).into_data();
            b.add_buffer(aligned(keys)).add_child_data(dict)
        }
        L::Struct(fs, sum) => b.child_data(
            (fs.iter().enumerate())
                .map(|(i, f)| {
                    build(
                        layout(&f.ty),
                        &vals
                            .iter()
                            .map(|v| v.map(|d| &fields_of(d, sum)[i]))
                            .collect::<Vec<_>>(),
                        false,
                    )
                })
                .collect(),
        ),
        L::List(e) => {
            let elems: Vec<_> = vals.iter().flat_map(|v| list(v).iter().map(Some)).collect();
            b.add_buffer(offsets(vals.iter().map(|v| list(v).len())))
                .child_data(vec![build(layout(e), &elems, false)])
        }
        L::Union(cs, base) => {
            let ks = kids(cs, base);
            let g = group(cs.len());
            let mut rows: Vec<Vec<Option<&Datum>>> = vec![Vec::new(); ks.len()];
            let (mut ids, mut offs) = (Vec::new(), Vec::new());
            for v in vals {
                let j = v.map_or(0, |d| (ctor(d) - base) / g);
                ids.push(j as i8);
                offs.push(rows[j].len() as i32);
                rows[j].push(*v);
            }
            let children = ks
                .iter()
                .zip(&rows)
                .map(|((_, k), r)| build(*k, r, false))
                .collect();
            b.add_buffer(aligned(ids))
                .add_buffer(aligned(offs))
                .child_data(children)
        }
        L::Nullable(_) => unreachable!("handled above"),
    };
    b.build().expect("arrow-rs accepts the data")
}

fn fixed(s: Scalar, b: &[u8]) -> Value {
    let mut x = [0; 16];
    x[..b.len()].copy_from_slice(b);
    let i = i128::from_le_bytes(x);
    match s {
        Scalar::I8 => Value::I8(i as i8),
        Scalar::I16 => Value::I16(i as i16),
        Scalar::I32 => Value::I32(i as i32),
        Scalar::Date => Value::Date(i as i32),
        Scalar::I64 => Value::I64(i as i64),
        Scalar::Instant => Value::Instant(i as i64),
        Scalar::Time => Value::Time(i as i64),
        Scalar::DateTime => Value::DateTime(i as i64),
        Scalar::Duration => Value::Duration(i as i64),
        Scalar::F32 => Value::F32(F32::new(f32::from_bits(i as u32))),
        Scalar::F64 => Value::F64(F64::new(f64::from_bits(i as u64))),
        _ => Value::Decimal(i),
    }
}

fn i32_at(b: &Buffer, i: usize) -> usize {
    i32::from_le_bytes(b.as_slice()[4 * i..4 * i + 4].try_into().unwrap()) as usize
}

/// Returns row `i` of `d`, Arrow data of type `t`'s layout, as arrow-rs decoded it.
pub(crate) fn get(t: &FileType, d: &ArrayData, i: usize) -> Datum {
    match layout(t) {
        L::Nullable(_) if d.is_null(i) => Datum::Sum {
            ctor: 0,
            fields: vec![],
        },
        L::Nullable(inner) => Datum::Sum {
            ctor: 1,
            fields: vec![get(inner, d, i)],
        },
        l => at(l, d, i),
    }
}

fn at(l: L<'_>, d: &ArrayData, i: usize) -> Datum {
    assert_eq!(d.offset(), 0, "arrow-rs decodes unsliced data");
    let b = d.buffers();
    let sum = |c: usize, fields| Datum::Sum {
        ctor: c as u32,
        fields,
    };
    match l {
        L::Scalar(s) => Datum::Scalar(match width(s) {
            Some(w) => fixed(s, &b[0].as_slice()[w * i..w * (i + 1)]),
            None => {
                let bytes = b[1].as_slice()[i32_at(&b[0], i)..i32_at(&b[0], i + 1)].to_vec();
                let text = String::from_utf8(bytes.clone());
                match s {
                    Scalar::Text => Value::Text(text.unwrap()),
                    Scalar::Bytes => Value::Bytes(bytes),
                    Scalar::SortKey => Value::SortKey(bytes),
                    Scalar::Period => {
                        let t = text.unwrap();
                        let (m, d) = t[1..t.len() - 1].split_once('M').unwrap();
                        Value::Period {
                            months: m.parse().unwrap(),
                            days: d.parse().unwrap(),
                        }
                    }
                    _ => {
                        let z = ZONED
                            .iter()
                            .find(|z| z.2 == text.as_deref().unwrap())
                            .expect("a zoned text of the table");
                        Value::ZonedDateTime {
                            local: z.0,
                            zone: z.1.into(),
                        }
                    }
                }
            }
        }),
        L::Bool => sum(usize::from(bit_util::get_bit(b[0].as_slice(), i)), vec![]),
        L::Enum(cs) => {
            let names = &d.child_data()[0];
            let k = i32_at(&b[0], i);
            let name = &names.buffers()[1].as_slice()
                [i32_at(&names.buffers()[0], k)..i32_at(&names.buffers()[0], k + 1)];
            sum(
                cs.iter().position(|c| c.name.as_bytes() == name).unwrap(),
                vec![],
            )
        }
        L::Struct(fs, is_sum) => {
            let fields = fs
                .iter()
                .zip(d.child_data())
                .map(|(f, c)| get(&f.ty, c, i))
                .collect();
            if is_sum {
                sum(0, fields)
            } else {
                Datum::Record(fields)
            }
        }
        L::List(e) => {
            let c = &d.child_data()[0];
            Datum::List(
                (i32_at(&b[0], i)..i32_at(&b[0], i + 1))
                    .map(|j| get(e, c, j))
                    .collect(),
            )
        }
        L::Union(cs, base) => {
            let j = b[0].as_slice()[i] as usize;
            let (_, k) = kids(cs, base)[j];
            match at(k, &d.child_data()[j], i32_at(&b[1], i)) {
                Datum::Sum { fields, .. } if matches!(k, L::Struct(..)) => sum(base + j, fields),
                v => v,
            }
        }
        L::Nullable(_) => unreachable!("handled by get"),
    }
}

/// Write `rows` through the engine and read them with arrow-rs, then write them with arrow-rs and
/// read them through the engine, asserting both give back `rows`.
fn round_trip(file: &FileSchema, rows: &[Vec<Datum>]) {
    let bytes = engine_stream(file, rows);
    assert_aligned(&bytes);
    let reader = StreamReader::try_new(Cursor::new(&bytes), None).unwrap();
    assert_eq!(*reader.schema(), schema(file));
    let batches: Vec<RecordBatch> = reader.collect::<Result<_, _>>().unwrap();
    let got: Vec<Vec<Datum>> = (batches.iter())
        .flat_map(|rb| {
            (0..rb.num_rows()).map(move |i| {
                (file.fields().iter().zip(rb.columns()))
                    .map(|(f, c)| get(&f.ty, &c.to_data(), i))
                    .collect()
            })
        })
        .collect();
    assert_eq!(got, rows, "arrow-rs reading the engine's stream");
    for (f, c) in file
        .fields()
        .iter()
        .zip(batches.iter().flat_map(RecordBatch::columns))
    {
        if let L::Enum(cs) = layout(&f.ty) {
            let dict = arrow_array::StringArray::from(c.to_data().child_data()[0].clone());
            let names: Vec<&str> = cs.iter().map(|c| c.name.as_str()).collect();
            assert!(
                dict.iter().eq(names.into_iter().map(Some)),
                "the dictionary of {}",
                f.name
            );
        }
    }

    let cols: Vec<ArrayRef> = (file.fields().iter().enumerate())
        .map(|(i, f)| {
            make_array(build(
                layout(&f.ty),
                &rows.iter().map(|r| Some(&r[i])).collect::<Vec<_>>(),
                false,
            ))
        })
        .collect();
    let bytes = arrow_stream(schema(file), cols);
    let units = rows.len() + rows.iter().flatten().map(elements).sum::<usize>();
    assert_eq!(
        read(&bytes, file, units),
        Ok(rows.to_vec()),
        "the engine reading arrow-rs's stream"
    );
    if units > 0 {
        assert_eq!(
            read(&bytes, file, units - 1),
            Err(IpcError::OverBudget("rows"))
        );
    }
}

/// Returns how many list elements `d` holds at any depth.
fn elements(d: &Datum) -> usize {
    match d {
        Datum::Scalar(_) => 0,
        Datum::Record(fs) | Datum::Sum { fields: fs, .. } => fs.iter().map(elements).sum(),
        Datum::List(xs) => xs.len() + xs.iter().map(elements).sum::<usize>(),
    }
}

fn engine_stream(file: &FileSchema, rows: &[Vec<Datum>]) -> Vec<u8> {
    let mut b = BatchBuilder::new(file.schema().clone());
    rows.iter().for_each(|r| b.push_datums(r));
    let mut bytes = Vec::new();
    ipc::write(file, [&b.finish()], &mut bytes).unwrap();
    bytes
}

fn arrow_stream(schema: Schema, cols: Vec<ArrayRef>) -> Vec<u8> {
    let schema = Arc::new(schema);
    let rb = RecordBatch::try_new(schema.clone(), cols).unwrap();
    let mut out = Vec::new();
    let mut w = StreamWriter::try_new(&mut out, &schema).unwrap();
    w.write(&rb).unwrap();
    w.finish().unwrap();
    drop(w);
    out
}

/// Returns the rows the engine reads from `bytes` within a budget of `rows` rows.
fn read(bytes: &[u8], file: &FileSchema, rows: usize) -> Result<Vec<Vec<Datum>>, IpcError> {
    let batches = ipc::read(bytes, file, &mut Budget::new(rows))?;
    Ok(batches
        .iter()
        .flat_map(|b| (0..b.len()).map(|i| b.datums(i)))
        .collect())
}

/// Assert, through arrow-rs's flatbuffer verifier, that every message and body buffer of
/// `bytes` starts at a multiple of 8.
fn assert_aligned(bytes: &[u8]) {
    let mut at = 0;
    loop {
        assert_eq!(at % 8, 0, "a message at {at}");
        let len = i32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if len == 0 {
            return;
        }
        let m = arrow_ipc::root_as_message(&bytes[at + 8..at + 8 + len]).unwrap();
        let rb = m
            .header_as_record_batch()
            .or(m.header_as_dictionary_batch().and_then(|d| d.data()));
        for buf in rb.and_then(|rb| rb.buffers()).into_iter().flatten() {
            assert_eq!(buf.offset() % 8, 0, "a buffer of the message at {at}");
        }
        at += 8 + len + m.bodyLength() as usize;
    }
}

/// The generated shapes, by [`Gen::kind`].
const KINDS: u64 = 13;

struct Gen {
    rng: Rng,
    id: u32,
}

impl Gen {
    fn new(seed: u64) -> Gen {
        Gen {
            rng: Rng::new(seed),
            id: 0,
        }
    }

    fn ctor(&mut self, name: impl Into<String>, fields: Vec<FileField>) -> FileCtor {
        self.id += 1;
        FileCtor {
            id: CtorId(self.id),
            name: name.into(),
            fields,
        }
    }

    fn fields(&mut self, n: u64, depth: u32) -> Vec<FileField> {
        (0..n)
            .map(|i| FileField {
                name: format!("f{i}"),
                ty: self.ty(depth),
            })
            .collect()
    }

    fn sum(&mut self, n: usize, fielded: impl Fn(usize) -> bool, depth: u32) -> FileType {
        let cs = (0..n).map(|i| {
            let fs = if fielded(i) {
                self.fields(1, depth)
            } else {
                vec![]
            };
            self.ctor(format!("C{i}"), fs)
        });
        FileType::Sum(SumKind::Other, cs.collect())
    }

    fn option(&mut self, t: FileType) -> FileType {
        let none = self.ctor("None", vec![]);
        let some = self.ctor(
            "Some",
            vec![FileField {
                name: "value".into(),
                ty: t,
            }],
        );
        FileType::Sum(SumKind::Option, vec![none, some])
    }

    fn ty(&mut self, depth: u32) -> FileType {
        let k = if depth == 0 { 0 } else { self.rng.below(KINDS) };
        self.kind(k, depth.saturating_sub(1))
    }

    fn kind(&mut self, k: u64, d: u32) -> FileType {
        match k {
            0 => FileType::Scalar(*self.rng.pick(&SCALARS)),
            1 => {
                let n = 1 + self.rng.below(2);
                FileType::Record(self.fields(n, d))
            }
            2 => FileType::Sum(
                SumKind::Bool,
                vec![self.ctor("False", vec![]), self.ctor("True", vec![])],
            ),
            3 => {
                let n = 2 + self.rng.below(4) as usize;
                self.sum(n, |_| false, d)
            }
            4 => self.sum(300, |_| false, d),
            5 => {
                let t = self.ty(d);
                self.option(t)
            }
            6 => {
                let t = self.ty(d);
                let t = self.option(t);
                self.option(t)
            }
            7 => {
                let t = self.sum(3, |i| i == 0, d);
                self.option(t)
            }
            8 => {
                let n = 2 + self.rng.below(3) as usize;
                self.sum(n, |i| i == 0 || i == 2, d)
            }
            9 => self.sum(130, |i| i % 50 == 0, d),
            10 => self.sum(1, |_| true, d),
            11 => FileType::List(Box::new(self.ty(d))),
            _ => self.sum(2, |i| i == 1, d),
        }
    }

    fn val(&mut self, t: &FileType) -> Datum {
        match t {
            FileType::Scalar(s) => Datum::Scalar(self.scalar(*s)),
            FileType::Record(fs) => Datum::Record(fs.iter().map(|f| self.val(&f.ty)).collect()),
            FileType::Sum(_, cs) => {
                let c = self.rng.below(cs.len() as u64) as usize;
                Datum::Sum {
                    ctor: c as u32,
                    fields: cs[c].fields.iter().map(|f| self.val(&f.ty)).collect(),
                }
            }
            FileType::List(e) => Datum::List((0..self.rng.below(3)).map(|_| self.val(e)).collect()),
        }
    }

    fn scalar(&mut self, s: Scalar) -> Value {
        let x = self.rng.next_u64();
        let bytes = x.to_le_bytes()[..(x % 9) as usize].to_vec();
        match s {
            Scalar::I8 => Value::I8(x as i8),
            Scalar::I16 => Value::I16(x as i16),
            Scalar::I32 => Value::I32(x as i32),
            Scalar::I64 => Value::I64(x as i64),
            Scalar::F32 => Value::F32(F32::new((x % 1000) as f32 / 4.0 - 100.0)),
            Scalar::F64 => Value::F64(F64::new((x % 1000) as f64 / 8.0 - 60.0)),
            Scalar::Decimal(_) => Value::Decimal(i128::from(x as i64) << 40),
            Scalar::Text => Value::Text(["", "a", "héllo", "日本語"][(x % 4) as usize].into()),
            Scalar::Bytes => Value::Bytes(bytes),
            Scalar::SortKey => Value::SortKey(bytes),
            Scalar::Instant => Value::Instant(x as i64),
            Scalar::Date => Value::Date((x % 200_000_000) as i32 - 100_000_000),
            Scalar::Time => Value::Time((x % 86_400_000_000) as i64),
            Scalar::DateTime => Value::DateTime(x as i64),
            Scalar::Duration => Value::Duration(x as i64),
            Scalar::ZonedDateTime => {
                let z = ZONED[(x % 2) as usize];
                Value::ZonedDateTime {
                    local: z.0,
                    zone: z.1.into(),
                }
            }
            Scalar::Period => Value::Period {
                months: (x % 41) as i32 - 20,
                days: (x >> 8) as i32 % 400,
            },
        }
    }

    fn rows(&mut self, file: &FileSchema, n: u64) -> Vec<Vec<Datum>> {
        (0..n)
            .map(|_| file.fields().iter().map(|f| self.val(&f.ty)).collect())
            .collect()
    }
}

/// Add the name of every shape and scalar `t` holds to `seen`.
fn shapes(t: &FileType, seen: &mut BTreeSet<String>) {
    let name = match (t, layout(t)) {
        (FileType::Scalar(s), _) => format!("{s:?}"),
        (FileType::Sum(SumKind::Option, cs), L::Union(..)) => match &cs[1].fields[0].ty {
            FileType::Sum(SumKind::Option, _) => "nested option".into(),
            _ => "option of a union".into(),
        },
        (FileType::Sum(SumKind::Option, _), _) => "option".into(),
        (FileType::Sum(_, cs), L::Union(..)) if cs.len() > 127 => "union of unions".into(),
        (FileType::Sum(_, cs), L::Union(..)) if cs.len() == 2 && cs[0].fields.is_empty() => {
            "two constructors, one nullary".into()
        }
        (_, L::Union(..)) => "union".into(),
        (FileType::Sum(_, cs), L::Enum(_)) if cs.len() > 256 => "wide enum".into(),
        (_, L::Enum(_)) => "enum".into(),
        (_, L::Bool) => "bool".into(),
        (_, L::Struct(_, true)) => "one constructor".into(),
        (_, L::Struct(..)) => "record".into(),
        (_, _) => "list".into(),
    };
    seen.insert(name);
    match t {
        FileType::Scalar(_) => {}
        FileType::Record(fs) => fs.iter().for_each(|f| shapes(&f.ty, seen)),
        FileType::Sum(_, cs) => cs
            .iter()
            .flat_map(|c| &c.fields)
            .for_each(|f| shapes(&f.ty, seen)),
        FileType::List(e) => shapes(e, seen),
    }
}

fn every_shape() -> BTreeSet<String> {
    let mut all: BTreeSet<String> = SCALARS.iter().map(|s| format!("{s:?}")).collect();
    let names = [
        "nested option",
        "option of a union",
        "option",
        "union of unions",
        "two constructors, one nullary",
    ];
    let more = [
        "union",
        "wide enum",
        "enum",
        "bool",
        "one constructor",
        "record",
        "list",
    ];
    all.extend(names.iter().chain(&more).map(ToString::to_string));
    all
}

/// Returns a file of one column per scalar and per generated shape.
fn every_kind(g: &mut Gen) -> FileSchema {
    let scalars = SCALARS.iter().map(|s| FileType::Scalar(*s));
    let kinds: Vec<FileType> = (1..KINDS).map(|k| g.kind(k, 1)).collect();
    let cols = scalars.chain(kinds).enumerate().map(|(i, ty)| FileField {
        name: format!("c{i}"),
        ty,
    });
    FileSchema::new(cols.collect())
}

#[test]
fn every_type_and_shape_round_trips_through_arrow_rs() {
    let mut g = Gen::new(1);
    let file = every_kind(&mut g);
    let mut seen = BTreeSet::new();
    file.fields().iter().for_each(|f| shapes(&f.ty, &mut seen));
    assert_eq!(seen, every_shape());
    round_trip(&file, &[]);
    let rows = g.rows(&file, 12);
    round_trip(&file, &rows);
}

#[test]
fn generated_schemas_round_trip_through_arrow_rs() {
    let mut seen = BTreeSet::new();
    for seed in sweep(0..4) {
        let mut g = Gen::new(seed);
        for case in 0..60 {
            let n = 1 + g.rng.below(3);
            let file = FileSchema::new(g.fields(n, 3));
            file.fields().iter().for_each(|f| shapes(&f.ty, &mut seen));
            let n = g.rng.below(6);
            let rows = g.rows(&file, n);
            let run = std::panic::catch_unwind(|| round_trip(&file, &rows));
            assert!(run.is_ok(), "seed {seed} case {case}: {file:?}");
        }
    }
    assert_eq!(seen, every_shape());
}

#[test]
fn a_stream_of_another_schema_is_refused_naming_the_first_difference() {
    let col = |name: &str, s| FileField {
        name: name.into(),
        ty: FileType::Record(vec![FileField {
            name: "b".into(),
            ty: FileType::Scalar(s),
        }]),
    };
    let file = FileSchema::new(vec![col("a", Scalar::I32)]);
    let mut bytes = Vec::new();
    ipc::write(&file, [], &mut bytes).unwrap();
    let differs = |path: &str, what| {
        Err(IpcError::SchemaDiffers {
            path: path.into(),
            what,
        })
    };
    assert_eq!(
        read(&bytes, &FileSchema::new(vec![col("a", Scalar::I64)]), 1),
        differs("a.b", "type")
    );
    assert_eq!(
        read(&bytes, &FileSchema::new(vec![col("x", Scalar::I32)]), 1),
        differs("x", "name")
    );
    assert_eq!(read(&bytes, &file, 1), Ok(vec![]));
}

/// Assert that a flip of one bit in each byte of `bytes`, a stream of `file`, is refused with a
/// typed error or reads as rows of `file`'s arity whose every value is in its type's domain, and
/// that a flip in a continuation marker is refused. Outside the nextest profile `deep`, flip
/// every other byte.
fn assert_flips_refused_or_in_domain(bytes: &[u8], file: &FileSchema) {
    let mut starts = BTreeSet::new();
    let mut at = 0;
    while at < bytes.len() {
        starts.insert(at);
        let len = i32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let m = arrow_ipc::root_as_message(&bytes[at + 8..at + 8 + len]);
        at += 8 + len + m.map_or(0, |m| m.bodyLength() as usize);
    }
    assert!(
        starts.len() >= 4,
        "schema, dictionary, batch and end marker: {starts:?}"
    );
    let mut flipped = bytes.to_vec();
    let step = if every_case() { 1 } else { 2 };
    for i in (0..bytes.len()).step_by(step) {
        flipped[i] ^= 1 << (i / step % 8);
        let read = read(&flipped, file, 3);
        flipped[i] = bytes[i];
        if starts.range(i.saturating_sub(3)..=i).next().is_some() {
            assert!(
                matches!(read, Err(IpcError::Malformed(_))),
                "a marker flipped at {i}"
            );
        }
        for row in read.iter().flatten() {
            assert_eq!(row.len(), file.fields().len(), "flip at {i}");
            assert!(row.iter().all(Datum::in_domain), "flip at {i}: {row:?}");
        }
    }
}

#[test]
fn cut_streams_are_refused_and_flipped_ones_refused_or_read_in_domain() {
    let mut g = Gen::new(2);
    let kinds = [8, 3, 5, 9].map(|k| g.kind(k, 0));
    let cols = kinds.into_iter().enumerate().map(|(i, ty)| FileField {
        name: format!("c{i}"),
        ty,
    });
    let file = FileSchema::new(cols.collect());
    let mut seen = BTreeSet::new();
    file.fields().iter().for_each(|f| shapes(&f.ty, &mut seen));
    for shape in ["union", "enum", "option", "union of unions"] {
        assert!(seen.contains(shape), "the stream holds a {shape}");
    }
    let rows = g.rows(&file, 3);
    let bytes = engine_stream(&file, &rows);
    for n in 0..bytes.len() {
        let read = read(&bytes[..n], &file, 3);
        assert!(matches!(read, Err(IpcError::Malformed(_))), "cut at {n}");
    }
    assert_flips_refused_or_in_domain(&bytes, &file);
}

#[test]
fn a_fieldless_type_round_trips_at_any_length_within_the_budget() {
    let mut g = Gen::new(3);
    let unit = g.sum(1, |_| false, 0);
    let col = |ty| {
        FileSchema::new(vec![FileField {
            name: "u".into(),
            ty,
        }])
    };
    let u = Datum::Sum {
        ctor: 0,
        fields: vec![],
    };
    let (units, list) = (col(unit.clone()), col(FileType::List(Box::new(unit))));
    round_trip(&units, &vec![vec![u.clone()]; 5000]);
    round_trip(&list, &[vec![Datum::List(vec![u; 5000])]]);
}

#[test]
fn rows_of_the_costliest_constructor_read_at_their_exact_row_budget() {
    let mut g = Gen::new(5);
    let col = |ty| {
        FileSchema::new(vec![FileField {
            name: "c".into(),
            ty,
        }])
    };
    let i64 = |x| Datum::Scalar(Value::I64(x));
    let some = |d| Datum::Sum {
        ctor: 1,
        fields: vec![d],
    };
    let field = FileField {
        name: "x".into(),
        ty: FileType::Scalar(Scalar::I64),
    };
    let (c0, c1) = (g.ctor("C0", vec![]), g.ctor("C1", vec![field]));
    let fielded = col(FileType::Sum(SumKind::Other, vec![c0, c1]));
    round_trip(&fielded, &vec![vec![some(i64(3))]; 1000]);
    let inner = g.option(FileType::Scalar(Scalar::I64));
    let nested = col(g.option(inner));
    round_trip(&nested, &vec![vec![some(some(i64(3)))]; 1000]);
    let wide = col(g.sum(300, |i| i == 0, 0));
    let c7 = Datum::Sum {
        ctor: 7,
        fields: vec![],
    };
    round_trip(&wide, &vec![vec![c7]; 1000]);
}

#[test]
fn a_dictionary_is_charged_once_however_many_batches_follow_it() {
    let mut g = Gen::new(6);
    let file = FileSchema::new(vec![FileField {
        name: "e".into(),
        ty: g.sum(3, |_| false, 0),
    }]);
    let names: Vec<String> = (0..200_000).map(|i| format!("e{i}")).collect();
    let values = arrow_array::StringArray::from_iter_values(&names);
    let keys = Int32Array::from(Vec::<i32>::new());
    let dict = arrow_array::DictionaryArray::try_new(keys, Arc::new(values)).unwrap();
    let schema = Arc::new(schema(&file));
    let rb = RecordBatch::try_new(schema.clone(), vec![Arc::new(dict)]).unwrap();
    let mut bytes = Vec::new();
    let mut w = StreamWriter::try_new(&mut bytes, &schema).unwrap();
    (0..200).for_each(|_| w.write(&rb).unwrap());
    w.finish().unwrap();
    drop(w);
    let mut budget = Budget::new(0);
    let read = ipc::read(&bytes, &file, &mut budget).unwrap();
    assert_eq!(
        read.iter().map(Batch::len).collect::<Vec<_>>(),
        vec![0; 200]
    );
    let text: usize = names.iter().map(String::len).sum();
    assert_eq!(budget.bytes(), names.len() + text);
}

/// Set every field node of length 0 in `bytes` to `to` rows, and return how many it set.
fn pad_nodes(bytes: &mut [u8], to: i64) -> usize {
    let (mut at, mut empty) = (0, Vec::new());
    while let len @ 1.. = i32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize {
        let meta = &bytes[at + 8..at + 8 + len];
        let m = arrow_ipc::root_as_message(meta).unwrap();
        if let Some(nodes) = m.header_as_record_batch().and_then(|rb| rb.nodes()) {
            let start = at + 8 + (nodes.bytes().as_ptr() as usize - meta.as_ptr() as usize);
            empty.extend((0..nodes.len()).map(|i| start + 16 * i));
        }
        at += 8 + len + m.bodyLength() as usize;
    }
    empty
        .iter()
        .for_each(|&p| bytes[p..p + 8].copy_from_slice(&to.to_le_bytes()));
    empty.len()
}

#[test]
fn padding_a_stream_claims_reads_nothing_past_what_rows_refer_to() {
    let mut g = Gen::new(4);
    let i64 = || FileField {
        name: "x".into(),
        ty: FileType::Scalar(Scalar::I64),
    };
    let mut cols: Vec<FileField> = (0..6)
        .map(|i| FileField {
            name: format!("c{i}"),
            ty: g.sum(127, |i| i == 0, 0),
        })
        .collect();
    cols.push(FileField {
        name: "r".into(),
        ty: FileType::Record(vec![i64(), i64()]),
    });
    cols.push(FileField {
        name: "l".into(),
        ty: FileType::List(Box::new(i64().ty)),
    });
    let file = FileSchema::new(cols);
    let rows = g.rows(&file, 2);
    let units = 2 + rows.iter().flatten().map(elements).sum::<usize>();
    let mut bytes = engine_stream(&file, &rows);
    let (mut plain, mut padded) = (Budget::new(units), Budget::new(units));
    assert_eq!(
        ipc::read(&bytes, &file, &mut plain).map(|b| b[0].len()),
        Ok(2)
    );
    let to = 7 * bytes.len() as i64;
    assert_eq!(
        pad_nodes(&mut bytes, to),
        6 * 129 + 3 + 2,
        "every node padded"
    );
    let read = ipc::read(&bytes, &file, &mut padded).unwrap();
    assert_eq!((0..2).map(|i| read[0].datums(i)).collect::<Vec<_>>(), rows);
    assert_eq!(
        (padded.datums(), padded.bytes()),
        (plain.datums(), plain.bytes())
    );
    let most = 2 * 6 * 3 + 2 * 3 + units;
    assert!(plain.datums() <= most, "{} datums", plain.datums());
}

#[test]
fn a_value_outside_its_types_domain_is_refused_naming_column_and_row() {
    let outside = [
        (Scalar::Time, Value::Time(86_400_000_000)),
        (Scalar::Time, Value::Time(-1)),
        (Scalar::Decimal(2), Value::Decimal(10i128.pow(38))),
        (Scalar::Date, Value::Date(i32::MAX)),
        (Scalar::Text, Value::Text("e\u{301}".into())),
    ];
    let inside = [
        (Scalar::Instant, Value::Instant(i64::MIN)),
        (Scalar::DateTime, Value::DateTime(i64::MAX)),
        (Scalar::Duration, Value::Duration(i64::MIN)),
        (Scalar::Decimal(2), Value::Decimal(10i128.pow(38) - 1)),
        (Scalar::Text, Value::Text("\u{e9}".into())),
    ];
    for (i, (s, v)) in outside.iter().chain(&inside).enumerate() {
        let file = FileSchema::new(vec![FileField {
            name: "c".into(),
            ty: FileType::Scalar(*s),
        }]);
        let mut g = Gen::new(5);
        let rows = vec![
            vec![Datum::Scalar(g.scalar(*s))],
            vec![Datum::Scalar(v.clone())],
        ];
        let col = build(
            L::Scalar(*s),
            &rows.iter().map(|r| Some(&r[0])).collect::<Vec<_>>(),
            false,
        );
        let got = read(
            &arrow_stream(schema(&file), vec![make_array(col)]),
            &file,
            2,
        );
        let what = "a value outside its type's domain";
        let refused = Err(IpcError::BadValue {
            column: "c".into(),
            row: Some(1),
            what,
        });
        assert_eq!(
            got,
            if i < outside.len() { refused } else { Ok(rows) },
            "{v:?}"
        );
    }
}

#[test]
fn union_children_are_read_by_name_and_nulls_per_value() {
    let int = |x: i32| Datum::Scalar(Value::I32(x));
    let mut g = Gen::new(6);
    let i32_field = |n: &str| FileField {
        name: n.into(),
        ty: FileType::Scalar(Scalar::I32),
    };
    let ctors = vec![
        g.ctor("A", vec![i32_field("x")]),
        g.ctor("B", vec![i32_field("y")]),
    ];
    let file = FileSchema::new(vec![FileField {
        name: "u".into(),
        ty: FileType::Sum(SumKind::Other, ctors),
    }]);
    let st = |f: &str, x: i32| -> ArrayRef {
        let kid: ArrayRef = Arc::new(Int32Array::from(vec![x]));
        Arc::new(StructArray::from(vec![(
            Arc::new(Field::new(f, DataType::Int32, false)),
            kid,
        )]))
    };
    let kids = [("B", st("y", 2)), ("A", st("x", 1))];
    let fs = kids
        .iter()
        .map(|(n, c)| Field::new(*n, c.data_type().clone(), false));
    let fields = UnionFields::try_new([0, 1], fs).unwrap();
    let ids = vec![1i8, 0].into();
    let u = UnionArray::try_new(
        fields.clone(),
        ids,
        Some(vec![0, 0].into()),
        kids.map(|k| k.1).to_vec(),
    );
    let col = Field::new("u", DataType::Union(fields, UnionMode::Dense), false);
    let rows = vec![
        vec![Datum::Sum {
            ctor: 0,
            fields: vec![int(1)],
        }],
        vec![Datum::Sum {
            ctor: 1,
            fields: vec![int(2)],
        }],
    ];
    assert_eq!(
        read(
            &arrow_stream(Schema::new(vec![col]), vec![Arc::new(u.unwrap())]),
            &file,
            2
        ),
        Ok(rows)
    );

    let file = FileSchema::new(vec![i32_field("n")]);
    let nullable = Schema::new(vec![Field::new("n", DataType::Int32, true)]);
    let stream =
        |xs: Vec<Option<i32>>| arrow_stream(nullable.clone(), vec![Arc::new(Int32Array::from(xs))]);
    assert_eq!(
        read(&stream(vec![Some(1), Some(2)]), &file, 2),
        Ok(vec![vec![int(1)], vec![int(2)]])
    );
    let what = "a null in a field that is not an Option";
    let refused = Err(IpcError::BadValue {
        column: "n".into(),
        row: None,
        what,
    });
    assert_eq!(read(&stream(vec![Some(1), None]), &file, 2), refused);
}
