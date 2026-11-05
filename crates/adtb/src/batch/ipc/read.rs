use std::collections::HashMap;

use super::flat::{Reader, Tab};
use super::schema::{Plan, check, kids, resolve};
use super::{
    Budget, FileCtor, FileSchema, Form, IpcError, bad_value, form, of_text, union_kids, width,
};
use crate::batch::types::union_group;
use crate::batch::{Batch, BatchBuilder, Datum};
use crate::key::{F32, F64, Scalar, Value};

type R<T> = Result<T, IpcError>;

fn bad(what: &'static str) -> IpcError {
    IpcError::Malformed(what)
}

/// The datum of a null slot of an `Option`'s present column, which its wrapper drops.
fn dead() -> Datum {
    Datum::Record(Vec::new())
}

fn sum(ctor: usize, fields: Vec<Datum>) -> Datum {
    let ctor = u32::try_from(ctor).expect("a sum has under 2^32 constructors");
    Datum::Sum { ctor, fields }
}

/// Returns the most datums [`Decoder::column`] charges for one row of form `f`, each element of
/// a list counted as a row.
fn cost(f: Form<'_>) -> usize {
    let kids = kids(f).into_iter().map(|(_, k)| cost(k));
    1 + match f {
        Form::Nullable(inner) => cost(form(inner)),
        Form::Union { .. } => kids.max().unwrap_or(0),
        _ => kids.sum(),
    }
}

/// The constructor positions of each enum's names, by dictionary id.
type Enums<'a> = HashMap<i64, HashMap<&'a str, usize>>;

fn enums<'a>(f: Form<'a>, p: &Plan, out: &mut Enums<'a>) {
    let (f, _) = resolve(f, false);
    if let (Form::Enum(cs), Some(id)) = (f, p.dict) {
        out.insert(
            id,
            cs.iter()
                .enumerate()
                .map(|(i, c)| (c.name.as_str(), i))
                .collect(),
        );
    }
    let want = kids(f);
    for (j, kp) in &p.kids {
        enums(want[*j].1, kp, out);
    }
}

/// What a read has materialised against its caps.
///
/// Invariants:
/// - `rows` is at most the budget's rows: the record batch lengths and list lengths read so far.
/// - `datums` is at most `rows × Σ cost` over the columns.
/// - `bytes`, of text and binary values and of dictionary entries, is at most the stream's length.
struct Meter {
    caps: [usize; 3],
    used: [usize; 3],
}

const ROWS: usize = 0;
const DATUMS: usize = 1;
const BYTES: usize = 2;

impl Meter {
    /// Fail unless `n` more of `what` fit.
    fn afford(&self, what: usize, n: usize) -> R<()> {
        if self.used[what].saturating_add(n) > self.caps[what] {
            return Err(IpcError::OverBudget(["rows", "datums", "bytes"][what]));
        }
        Ok(())
    }

    fn charge(&mut self, what: usize, n: usize) -> R<()> {
        self.afford(what, n)?;
        self.used[what] += n;
        Ok(())
    }
}

/// Returns the batches of the Arrow IPC stream `bytes`, whose columns must be `file`'s, and sets
/// `budget`'s count of the datums it materialised.
///
/// Refuse a stream that is malformed, cut short, of another schema, holding a value outside its
/// type's domain or more than `budget` allows; never read past `bytes` or allocate past what the
/// budget and `bytes` bound.
pub fn read(bytes: &[u8], file: &FileSchema, budget: &mut Budget) -> R<Vec<Batch>> {
    let mut msgs = Messages { bytes, at: 0 };
    let (r, kind, header, _) = msgs.next()?.ok_or(bad("a stream with no schema"))?;
    if kind != 1 {
        return Err(bad("a stream that does not start with its schema"));
    }
    if r.num(header, 0, 2, 0)? != 0 {
        return Err(bad("a big-endian stream"));
    }
    let got = r.tables(header, 1)?;
    if got.len() != file.fields.len() {
        let path = "schema".into();
        return Err(IpcError::SchemaDiffers {
            path,
            what: "column count",
        });
    }
    let plans = (file.fields.iter().zip(got))
        .map(|(f, g)| check(r, g, (&f.name, form(&f.ty)), false, ""))
        .collect::<R<Vec<_>>>()?;
    let per_row: usize = file.fields.iter().map(|f| cost(form(&f.ty))).sum();
    let mut meter = Meter {
        caps: [
            budget.rows,
            budget.rows.saturating_mul(per_row),
            bytes.len(),
        ],
        used: [0; 3],
    };
    let mut names = HashMap::new();
    for (f, p) in file.fields.iter().zip(&plans) {
        enums(form(&f.ty), p, &mut names);
    }
    let mut dicts = HashMap::new();
    let mut out = Vec::new();
    let read = (|| {
        while let Some((r, kind, header, body)) = msgs.next()? {
            match kind {
                2 => dictionary(r, (header, body), &names, &mut dicts, &mut meter)?,
                3 => out.push(record_batch(
                    r, header, body, file, &plans, &dicts, &mut meter,
                )?),
                _ => return Err(bad("a message neither a dictionary nor a record batch")),
            }
        }
        Ok(())
    })();
    budget.datums = meter.used[DATUMS];
    budget.bytes = meter.used[BYTES];
    read.map(|()| out)
}

fn dictionary(
    r: Reader<'_>,
    (header, body): (Tab, &[u8]),
    names: &Enums<'_>,
    dicts: &mut HashMap<i64, Vec<Option<usize>>>,
    meter: &mut Meter,
) -> R<()> {
    if r.num(header, 2, 1, 0)? != 0 {
        return Err(bad("a delta dictionary"));
    }
    let id = r.num(header, 0, 8, 0)?;
    let names = names
        .get(&id)
        .ok_or(bad("a dictionary no enum field uses"))?;
    let data = r
        .table(header, 1)?
        .ok_or(bad("a dictionary with no data"))?;
    let mut d = Decoder::new(r, data, body, dicts, meter)?;
    let n = d.len;
    d.meter.charge(BYTES, n)?;
    if d.node(n)? != 0 {
        return Err(bad_value("a null dictionary entry"));
    }
    d.buffer()?;
    let entries = d.scalar(Scalar::Text, &(0..n).collect::<Vec<_>>())?;
    d.end()?;
    let ctor = |e: &Datum| match e {
        Datum::Scalar(Value::Text(t)) => names.get(t.as_str()).copied(),
        e => unreachable!("a text column holds {e:?}"),
    };
    let ctors = entries.iter().map(ctor).collect();
    dicts.insert(id, ctors);
    Ok(())
}

fn record_batch(
    r: Reader<'_>,
    header: Tab,
    body: &[u8],
    file: &FileSchema,
    plans: &[Plan],
    dicts: &HashMap<i64, Vec<Option<usize>>>,
    meter: &mut Meter,
) -> R<Batch> {
    let mut d = Decoder::new(r, header, body, dicts, meter)?;
    let n = d.len;
    d.meter.charge(ROWS, n)?;
    let at: Vec<usize> = (0..n).collect();
    let mut cols = Vec::new();
    for (f, p) in file.fields.iter().zip(plans) {
        let named = |e| match e {
            IpcError::BadValue { row, what, .. } => IpcError::BadValue {
                column: f.name.clone(),
                row,
                what,
            },
            e => e,
        };
        let col = d.column(form(&f.ty), p, &at, false).map_err(named)?.0;
        if let Some(row) = col.iter().position(|v| !v.in_domain()) {
            let what = "a value outside its type's domain";
            return Err(IpcError::BadValue {
                column: f.name.clone(),
                row: Some(row),
                what,
            });
        }
        cols.push(col);
    }
    d.end()?;
    let mut b = BatchBuilder::new(file.schema.clone());
    transpose(cols, n).iter().for_each(|row| b.push_datums(row));
    Ok(b.finish())
}

fn transpose(cols: Vec<Vec<Datum>>, n: usize) -> Vec<Vec<Datum>> {
    let mut its: Vec<_> = cols.into_iter().map(Vec::into_iter).collect();
    (0..n)
        .map(|_| {
            its.iter_mut()
                .map(|c| c.next().expect("every column has a datum per row"))
                .collect()
        })
        .collect()
}

/// A message's metadata, kind, header and body.
type Message<'a> = (Reader<'a>, i64, Tab, &'a [u8]);

struct Messages<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Messages<'a> {
    /// Returns the next message, or `None` at the end marker.
    fn next(&mut self) -> R<Option<Message<'a>>> {
        let b = self.bytes;
        let head = b
            .get(self.at..self.at + 8)
            .ok_or(bad("a stream without its end marker"))?;
        if head[..4] != [0xff; 4] {
            return Err(bad("a message without the continuation marker"));
        }
        let len = i32::from_le_bytes(head[4..].try_into().unwrap());
        if len == 0 {
            if self.at + 8 != b.len() {
                return Err(bad("bytes past the end marker"));
            }
            return Ok(None);
        }
        let len = usize::try_from(len).map_err(|_| bad("a negative metadata length"))?;
        if len % 8 != 0 {
            return Err(bad("metadata not padded to a multiple of 8 bytes"));
        }
        let start = self.at + 8 + len;
        let meta = b
            .get(self.at + 8..start)
            .ok_or(bad("metadata past the end of the stream"))?;
        let r = Reader(meta);
        let m = r.root()?;
        if r.num(m, 0, 2, 0)? != 4 {
            return Err(bad("a metadata version other than V5"));
        }
        let header = r.table(m, 2)?.ok_or(bad("a message with no header"))?;
        let body_len =
            usize::try_from(r.num(m, 3, 8, 0)?).map_err(|_| bad("a negative body length"))?;
        let end = start.checked_add(body_len).filter(|e| *e <= b.len());
        let body = &b[start..end.ok_or(bad("a body past the end of the stream"))?];
        self.at = start + body_len;
        Ok(Some((r, r.num(m, 1, 1, 0)?, header, body)))
    }
}

/// Returns one past the largest of `at`.
fn end(at: &[usize]) -> usize {
    at.iter().max().map_or(0, |m| m + 1)
}

/// Read the columns of one record batch, taking its field nodes and buffers in order.
///
/// A field is read only at the rows its parent refers to; its node may be longer, and the rest
/// is padding no buffer is read for.
struct Decoder<'a, 'd> {
    nodes: Vec<i64>,
    buffers: Vec<i64>,
    body: &'a [u8],
    ni: usize,
    bi: usize,
    len: usize,
    dicts: &'d HashMap<i64, Vec<Option<usize>>>,
    meter: &'d mut Meter,
}

impl<'a, 'd> Decoder<'a, 'd> {
    fn new(
        r: Reader<'_>,
        rb: Tab,
        body: &'a [u8],
        dicts: &'d HashMap<i64, Vec<Option<usize>>>,
        meter: &'d mut Meter,
    ) -> R<Decoder<'a, 'd>> {
        if r.has(rb, 3)? {
            return Err(bad("a compressed body"));
        }
        let len =
            usize::try_from(r.num(rb, 0, 8, 0)?).map_err(|_| bad("a negative batch length"))?;
        Ok(Decoder {
            nodes: r.pairs(rb, 1)?,
            buffers: r.pairs(rb, 2)?,
            body,
            ni: 0,
            bi: 0,
            len,
            dicts,
            meter,
        })
    }

    fn end(&self) -> R<()> {
        if self.ni * 2 != self.nodes.len() || self.bi * 2 != self.buffers.len() {
            return Err(bad("field nodes or buffers past the schema's"));
        }
        Ok(())
    }

    /// Take the next field node, which must cover `end` rows, and return its null count.
    fn node(&mut self, end: usize) -> R<i64> {
        let (Some(&len), Some(&nulls)) =
            (self.nodes.get(2 * self.ni), self.nodes.get(2 * self.ni + 1))
        else {
            return Err(bad("fewer field nodes than the schema"));
        };
        self.ni += 1;
        if usize::try_from(len).map_or(true, |len| len < end) {
            return Err(bad("a field node shorter than its parent refers to"));
        }
        if nulls < 0 || nulls > len {
            return Err(bad("a null count past the field's length"));
        }
        Ok(nulls)
    }

    fn buffer(&mut self) -> R<&'a [u8]> {
        let Some(&[at, len]) = self.buffers.get(2 * self.bi..2 * self.bi + 2) else {
            return Err(bad("fewer buffers than the schema"));
        };
        self.bi += 1;
        let at = usize::try_from(at).map_err(|_| bad("a negative buffer offset"))?;
        if at % 8 != 0 {
            return Err(bad("a buffer not aligned to 8 bytes"));
        }
        let end = usize::try_from(len).ok().and_then(|l| at.checked_add(l));
        let end = end.filter(|e| *e <= self.body.len());
        Ok(&self.body[at..end.ok_or(bad("a buffer past the body"))?])
    }

    /// Returns the next buffer, which must hold at least `need` bytes.
    fn sized(&mut self, need: usize) -> R<&'a [u8]> {
        let b = self.buffer()?;
        if b.len() < need {
            return Err(bad("a buffer shorter than its column"));
        }
        Ok(b)
    }

    /// Returns the `i32` offsets at `at` and one past each, from an offsets buffer covering `end`.
    fn offsets(&mut self, at: &[usize]) -> R<Vec<(usize, usize)>> {
        let b = self.sized(4 * (end(at) + 1))?;
        let o =
            |i: usize| usize::try_from(i32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap()));
        at.iter()
            .map(|&i| match (o(i), o(i + 1)) {
                (Ok(lo), Ok(hi)) if lo <= hi => Ok((lo, hi)),
                _ => Err(bad("a negative or decreasing offset")),
            })
            .collect()
    }

    /// Returns the values of a field of form `f` at its rows `at`, and their validity.
    fn column(
        &mut self,
        f: Form<'_>,
        p: &Plan,
        at: &[usize],
        nullable: bool,
    ) -> R<(Vec<Datum>, Vec<bool>)> {
        self.meter.charge(DATUMS, at.len())?;
        match f {
            Form::Nullable(inner) => {
                let (ds, valid) = self.column(form(inner), p, at, true)?;
                let opt = |(d, v)| {
                    if v {
                        sum(1, vec![d])
                    } else {
                        sum(0, Vec::new())
                    }
                };
                return Ok((
                    ds.into_iter().zip(valid).map(opt).collect(),
                    vec![true; at.len()],
                ));
            }
            Form::Union { ctors, base } => {
                return Ok((self.union(ctors, base, p, at)?, vec![true; at.len()]));
            }
            _ => {}
        }
        let end = end(at);
        let nulls = self.node(end)?;
        let bitmap = self.buffer()?;
        let valid: Vec<bool> = if bitmap.is_empty() {
            if nulls != 0 {
                return Err(bad("a null count with no validity bitmap"));
            }
            vec![true; at.len()]
        } else if bitmap.len() < end.div_ceil(8) {
            return Err(bad("a validity bitmap shorter than its column"));
        } else {
            at.iter()
                .map(|i| bitmap[i / 8] >> (i % 8) & 1 == 1)
                .collect()
        };
        if !nullable && valid.contains(&false) {
            return Err(bad_value("a null in a field that is not an Option"));
        }
        let live: Vec<usize> = at
            .iter()
            .zip(&valid)
            .filter(|(_, v)| **v)
            .map(|(i, _)| *i)
            .collect();
        let mut ds = self.values(f, p, &live)?.into_iter();
        let ds = valid.iter().map(|v| {
            if *v {
                ds.next().expect("a value per valid row")
            } else {
                dead()
            }
        });
        Ok((ds.collect(), valid))
    }

    /// Returns the values of a field of form `f` at its valid rows `at`.
    fn values(&mut self, f: Form<'_>, p: &Plan, at: &[usize]) -> R<Vec<Datum>> {
        let end = end(at);
        Ok(match f {
            Form::Scalar(s) => self.scalar(s, at)?,
            Form::Bool => {
                let b = self.sized(end.div_ceil(8))?;
                at.iter()
                    .map(|i| sum(usize::from(b[i / 8] >> (i % 8) & 1), Vec::new()))
                    .collect()
            }
            Form::Enum(_) => self.dictionary(p, at)?,
            Form::Struct {
                fields,
                sum: is_sum,
            } => {
                let mut cols = Vec::new();
                for (x, (_, kp)) in fields.iter().zip(&p.kids) {
                    cols.push(self.column(form(&x.ty), kp, at, false)?.0);
                }
                let rows = transpose(cols, at.len()).into_iter();
                rows.map(|fs| {
                    if is_sum {
                        sum(0, fs)
                    } else {
                        Datum::Record(fs)
                    }
                })
                .collect()
            }
            Form::List(e) => {
                let ranges = self.offsets(at)?;
                let total = ranges.iter().map(|(lo, hi)| hi - lo).sum();
                self.meter.charge(ROWS, total)?;
                let kid: Vec<usize> = ranges.iter().flat_map(|(lo, hi)| *lo..*hi).collect();
                let mut es = self
                    .column(form(e), &p.kids[0].1, &kid, false)?
                    .0
                    .into_iter();
                let list =
                    |(lo, hi): &(usize, usize)| Datum::List(es.by_ref().take(hi - lo).collect());
                ranges.iter().map(list).collect()
            }
            Form::Union { .. } | Form::Nullable(_) => unreachable!("read by column"),
        })
    }

    fn scalar(&mut self, s: Scalar, at: &[usize]) -> R<Vec<Datum>> {
        use Scalar::{Bytes, Period, SortKey, Text, ZonedDateTime};
        if !matches!(s, Text | Bytes | SortKey | Period | ZonedDateTime) {
            let w = width(s);
            let b = self.sized(w * end(at))?;
            return Ok(at
                .iter()
                .map(|i| Datum::Scalar(fixed(s, &b[w * i..w * (i + 1)])))
                .collect());
        }
        let ranges = self.offsets(at)?;
        let data = self.buffer()?;
        let mut out = Vec::with_capacity(at.len());
        for (lo, hi) in ranges {
            let b = data.get(lo..hi).ok_or(bad("a value past its data"))?;
            self.meter.charge(BYTES, b.len())?;
            let text = || std::str::from_utf8(b).map_err(|_| bad_value("text not in UTF-8"));
            out.push(Datum::Scalar(match s {
                Text => Value::Text(text()?.to_string()),
                Bytes => Value::Bytes(b.to_vec()),
                SortKey => Value::SortKey(b.to_vec()),
                s => of_text(text()?, s)?,
            }));
        }
        Ok(out)
    }

    fn dictionary(&mut self, p: &Plan, at: &[usize]) -> R<Vec<Datum>> {
        let b = self.sized(4 * end(at))?;
        let ctors = p.dict.and_then(|id| self.dicts.get(&id));
        let ctors = ctors.ok_or(bad("an enum column before its dictionary"))?;
        let row = |&i: &usize| {
            let ix = usize::try_from(i32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap()));
            let ctor = ix.ok().and_then(|ix| ctors.get(ix).copied().flatten());
            let ctor = ctor.ok_or(bad_value("a dictionary index naming no constructor"))?;
            Ok(sum(ctor, Vec::new()))
        };
        at.iter().map(row).collect()
    }

    fn union(&mut self, ctors: &[FileCtor], base: usize, p: &Plan, at: &[usize]) -> R<Vec<Datum>> {
        let end = end(at);
        if self.node(end)? != 0 {
            return Err(bad("a null in a union"));
        }
        let ids = self.sized(end)?;
        let offs = self.sized(4 * end)?;
        let mut refs: Vec<(Vec<usize>, Vec<usize>)> = vec![(Vec::new(), Vec::new()); p.kids.len()];
        for (pos, &i) in at.iter().enumerate() {
            let off = i32::from_le_bytes(offs[4 * i..4 * i + 4].try_into().unwrap());
            let off = usize::try_from(off).map_err(|_| bad("a negative union offset"))?;
            let kid = refs
                .get_mut(usize::from(ids[i]))
                .ok_or(bad("a type id past the union's children"))?;
            if kid.1.last().is_some_and(|o| *o > off) {
                return Err(bad("a union child's offsets decreasing"));
            }
            kid.0.push(pos);
            kid.1.push(off);
        }
        let want = union_kids(ctors, base);
        let leaf = union_group(ctors.len()) == 1;
        let mut out: Vec<Option<Datum>> = vec![None; at.len()];
        for ((j, kp), (rows, offs)) in p.kids.iter().zip(refs) {
            let (ds, _) = self.column(want[*j].1, kp, &offs, false)?;
            for (pos, d) in rows.into_iter().zip(ds) {
                out[pos] = Some(match d {
                    Datum::Sum { fields, .. } if leaf => sum(base + j, fields),
                    d => d,
                });
            }
        }
        Ok(out
            .into_iter()
            .map(|d| d.expect("every row's type id names a child"))
            .collect())
    }
}

fn fixed(s: Scalar, b: &[u8]) -> Value {
    fn arr<const N: usize>(b: &[u8]) -> [u8; N] {
        b.try_into().expect("a chunk of the scalar's width")
    }
    match s {
        Scalar::I8 => Value::I8(i8::from_le_bytes(arr(b))),
        Scalar::I16 => Value::I16(i16::from_le_bytes(arr(b))),
        Scalar::I32 => Value::I32(i32::from_le_bytes(arr(b))),
        Scalar::Date => Value::Date(i32::from_le_bytes(arr(b))),
        Scalar::I64 => Value::I64(i64::from_le_bytes(arr(b))),
        Scalar::Instant => Value::Instant(i64::from_le_bytes(arr(b))),
        Scalar::Time => Value::Time(i64::from_le_bytes(arr(b))),
        Scalar::DateTime => Value::DateTime(i64::from_le_bytes(arr(b))),
        Scalar::Duration => Value::Duration(i64::from_le_bytes(arr(b))),
        Scalar::F32 => Value::F32(F32::new(f32::from_le_bytes(arr(b)))),
        Scalar::F64 => Value::F64(F64::new(f64::from_le_bytes(arr(b)))),
        Scalar::Decimal(_) => Value::Decimal(i128::from_le_bytes(arr(b))),
        s => unreachable!("{s:?} is not fixed width"),
    }
}
