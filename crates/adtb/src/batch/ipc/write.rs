use std::io::{self, Write};

use super::flat::{self, Table, Val};
use super::{FileCtor, FileSchema, Form, form, schema, to_text, union_kids, width};
use crate::batch::types::{Cell, union_group};
use crate::batch::{Batch, Datum};
use crate::key::{Scalar, Value};

/// A value slot of a column; `None` is a null, or a row under a null.
type Slot<'a> = Option<&'a Datum>;

/// Write `batches` to `out` as one Arrow IPC stream of `file`'s columns.
///
/// Preconditions:
/// - Every batch's schema is `file`'s.
pub fn write<'a>(
    file: &FileSchema,
    batches: impl IntoIterator<Item = &'a Batch>,
    out: &mut impl Write,
) -> io::Result<()> {
    let (schema, dicts) = schema::schema(file);
    message(out, 1, schema, &[])?;
    for (id, cs) in dicts.into_iter().enumerate() {
        let names: Vec<Datum> = cs
            .iter()
            .map(|c| Datum::Scalar(Value::Text(c.name.clone())))
            .collect();
        let mut b = Body::default();
        b.column(
            Form::Scalar(Scalar::Text),
            &names.iter().map(Some).collect::<Vec<_>>(),
            false,
        );
        let d = Table::default()
            .num(0, 8, id as i64, 0)
            .with(1, Val::Table(b.record_batch(names.len())))
            .num(2, 1, 0, 0);
        message(out, 2, d, &b.data)?;
    }
    for batch in batches {
        assert_eq!(
            batch.schema(),
            file.schema(),
            "a stream's batches have its schema"
        );
        let mut b = Body::default();
        for (i, f) in file.fields.iter().enumerate() {
            let ds: Vec<Datum> = (0..batch.len()).map(|r| batch.datum(i, r)).collect();
            b.column(form(&f.ty), &ds.iter().map(Some).collect::<Vec<_>>(), false);
        }
        message(out, 3, b.record_batch(batch.len()), &b.data)?;
    }
    out.write_all(&[0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0])
}

/// Pad the metadata so the body and the next message start at a multiple of 8.
fn message(out: &mut impl Write, kind: i64, header: Table, body: &[u8]) -> io::Result<()> {
    let m = Table::default()
        .num(0, 2, 4, 0)
        .num(1, 1, kind, 0)
        .with(2, Val::Table(header))
        .num(3, 8, body.len() as i64, 0);
    let mut meta = flat::finish(&m);
    meta.resize(meta.len().next_multiple_of(8), 0);
    let len = i32::try_from(meta.len()).expect("a message's metadata is under 2 GiB");
    out.write_all(&u32::MAX.to_le_bytes())?;
    out.write_all(&len.to_le_bytes())?;
    out.write_all(&meta)?;
    out.write_all(body)
}

fn bits(it: impl ExactSizeIterator<Item = bool>) -> Vec<u8> {
    let mut b = vec![0; it.len().div_ceil(8)];
    for (i, x) in it.enumerate() {
        b[i / 8] |= u8::from(x) << (i % 8);
    }
    b
}

fn i32_le(n: usize) -> [u8; 4] {
    i32::try_from(n)
        .expect("a column holds under 2^31 elements or bytes")
        .to_le_bytes()
}

fn fixed(v: &Value) -> Vec<u8> {
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
        v => panic!("value {v:?} is not fixed width"),
    }
}

/// The field nodes, buffers and body bytes of one record batch.
#[derive(Default)]
struct Body {
    nodes: Vec<[i64; 2]>,
    buffers: Vec<[i64; 2]>,
    data: Vec<u8>,
}

impl Body {
    fn record_batch(&self, len: usize) -> Table {
        Table::default()
            .num(0, 8, len as i64, 0)
            .with(1, Val::Pairs(self.nodes.clone()))
            .with(2, Val::Pairs(self.buffers.clone()))
    }

    fn node(&mut self, len: usize, nulls: usize) {
        self.nodes.push([len as i64, nulls as i64]);
    }

    fn buffer(&mut self, bytes: &[u8]) {
        let at = self.data.len();
        self.data.extend(bytes);
        self.data.resize(self.data.len().next_multiple_of(8), 0);
        self.buffers.push([at as i64, bytes.len() as i64]);
    }

    fn column(&mut self, f: Form<'_>, vals: &[Slot<'_>], nullable: bool) {
        match f {
            Form::Nullable(inner) => {
                let present = vals.iter().map(|v| v.and_then(some)).collect::<Vec<_>>();
                return self.column(form(inner), &present, true);
            }
            Form::Union { ctors, base } => return self.union(ctors, base, vals),
            _ => {}
        }
        let nulls = if nullable {
            vals.iter().filter(|v| v.is_none()).count()
        } else {
            0
        };
        self.node(vals.len(), nulls);
        if nulls == 0 {
            self.buffer(&[]);
        } else {
            self.buffer(&bits(vals.iter().map(Option::is_some)));
        }
        match f {
            Form::Scalar(s) => self.scalar(s, vals),
            Form::Bool => self.buffer(&bits(
                vals.iter().map(|v| v.is_some_and(|d| d.as_sum().0 == 1)),
            )),
            Form::Enum(_) => {
                let ixs = vals
                    .iter()
                    .flat_map(|v| i32_le(v.map_or(0, |d| d.as_sum().0 as usize)));
                self.buffer(&ixs.collect::<Vec<_>>());
            }
            Form::Struct { fields, sum } => {
                for (i, x) in fields.iter().enumerate() {
                    let kid: Vec<Slot<'_>> = vals
                        .iter()
                        .map(|v| {
                            v.map(|d| {
                                if sum {
                                    &d.as_sum().1[i]
                                } else {
                                    &d.as_record()[i]
                                }
                            })
                        })
                        .collect();
                    self.column(form(&x.ty), &kid, false);
                }
            }
            Form::List(e) => {
                let mut offsets = i32_le(0).to_vec();
                let mut elems = Vec::new();
                for v in vals {
                    elems.extend(v.map_or(&[][..], |d| d.as_list()).iter().map(Some));
                    offsets.extend(i32_le(elems.len()));
                }
                self.buffer(&offsets);
                self.column(form(e), &elems, false);
            }
            Form::Union { .. } | Form::Nullable(_) => unreachable!("handled above"),
        }
    }

    fn scalar(&mut self, s: Scalar, vals: &[Slot<'_>]) {
        use Scalar::{Bytes, Period, SortKey, Text, ZonedDateTime};
        if !matches!(s, Text | Bytes | SortKey | Period | ZonedDateTime) {
            let w = width(s);
            let data: Vec<u8> = vals
                .iter()
                .flat_map(|v| v.map_or_else(|| vec![0; w], |d| fixed(d.as_scalar())))
                .collect();
            return self.buffer(&data);
        }
        let mut offsets = i32_le(0).to_vec();
        let mut data = Vec::new();
        for d in vals {
            match d.map(Cell::as_scalar) {
                None => {}
                Some(Value::Text(t)) => data.extend(t.as_bytes()),
                Some(Value::Bytes(b) | Value::SortKey(b)) => data.extend(b),
                Some(v) => data.extend(to_text(v, s).as_bytes()),
            }
            offsets.extend(i32_le(data.len()));
        }
        self.buffer(&offsets);
        self.buffer(&data);
    }

    fn union(&mut self, ctors: &[FileCtor], base: usize, vals: &[Slot<'_>]) {
        self.node(vals.len(), 0);
        let kids = union_kids(ctors, base);
        let g = union_group(ctors.len());
        let mut ids = Vec::new();
        let mut offsets = Vec::new();
        let mut rows: Vec<Vec<Slot<'_>>> = vec![Vec::new(); kids.len()];
        for v in vals {
            let j = v.map_or(0, |d| (d.as_sum().0 as usize - base) / g);
            ids.push(u8::try_from(j).expect("a union has at most 127 children"));
            offsets.extend(i32_le(rows[j].len()));
            rows[j].push(*v);
        }
        self.buffer(&ids);
        self.buffer(&offsets);
        for ((_, k), rows) in kids.into_iter().zip(rows) {
            self.column(k, &rows, false);
        }
    }
}

fn some(d: &Datum) -> Option<&Datum> {
    let (ctor, fields) = d.as_sum();
    (ctor == 1).then(|| &fields[0])
}
