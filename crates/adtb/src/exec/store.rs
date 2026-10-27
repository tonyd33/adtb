use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, ControlFlow};
use std::sync::Arc;

use adtb_lang::schema::Schema;
use adtb_lang::types::Ty;

use crate::batch::{Batch, BatchBuilder, Schema as BatchSchema};
use crate::btree::{self, Index, prefix_end};
use crate::heap::{FieldType, Header, Heap, HeapNo, walk};
use crate::key::{CtorId, Dir, KeySchema, KeyType, Scalar, Tid, Value};
use crate::mvcc::{
    SerialId, SerializationFailure, Serializer, Snapshot, Status, Transactions, Xid,
};
use crate::plan::{HeapId, IndexId};
use crate::runtime::Runtime;
use crate::storage::{PageError, Pager, Pid};
use adtb_sys::lock::Lock;

use super::Error;
use super::types::{ctors, repr};
use super::value::{F32, F64, Val};

/// What a query's reads see and record: the visibility of versions at its snapshot, and the
/// SIREAD locks its transaction takes.
pub trait Reads: Send + Sync {
    /// Returns whether the version with header `h` is visible at the reader's snapshot.
    fn sees(&self, h: &Header) -> Result<bool, PageError>;

    /// Record that every version of heap `heap` was read.
    fn scanned(&self, heap: HeapNo) -> Result<(), SerializationFailure>;

    /// Record that the encoded keys of the index rooted at `index` between `lo` and `hi` were
    /// read, for rows of `ctors`.
    fn probed(
        &self,
        index: Pid,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        ctors: BTreeSet<CtorId>,
    ) -> Result<(), SerializationFailure>;

    /// Record that the version at `tid` of heap `heap` was read.
    fn fetched(&self, heap: HeapNo, tid: Tid) -> Result<(), SerializationFailure>;

    /// Record that what the reader saw depends on `writer`, the inserter of a version it missed
    /// or the deleter of one it saw, when its snapshot does not count `writer` as committed and
    /// `writer` did not abort.
    fn depends(&self, writer: Xid) -> Result<(), Error>;
}

/// Returns the writer the visibility of the version with header `h` depends on, `seen` its
/// visibility: its deleter when seen, its inserter when not; `None` when it has no deleter.
fn writer_of(h: &Header, seen: bool) -> Option<Xid> {
    let x = Xid(if seen { h.xmax } else { h.xmin });
    (x != Xid::NONE).then_some(x)
}

/// Reads at a snapshot of the transactions layer, recording SIREAD locks with `serial` when it
/// is given.
pub struct AtSnapshot<R: Runtime> {
    pub rt: R,
    pub pager: Pager<R>,
    pub txns: Arc<Transactions<R>>,
    pub snapshot: Snapshot,
    pub reader: Option<Xid>,
    pub serial: Option<(Arc<Lock<Serializer>>, SerialId)>,
}

impl<R: Runtime> std::fmt::Debug for AtSnapshot<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AtSnapshot")
            .field("snapshot", &self.snapshot)
            .field("reader", &self.reader)
            .finish_non_exhaustive()
    }
}

impl<R: Runtime> Reads for AtSnapshot<R> {
    fn sees(&self, h: &Header) -> Result<bool, PageError> {
        self.txns
            .sees(&self.pager, &self.snapshot, self.reader, h.xmin, h.xmax)
    }

    fn scanned(&self, heap: HeapNo) -> Result<(), SerializationFailure> {
        match &self.serial {
            Some((s, id)) => s.lock(&self.rt).read_heap(*id, heap.0),
            None => Ok(()),
        }
    }

    fn probed(
        &self,
        index: Pid,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        ctors: BTreeSet<CtorId>,
    ) -> Result<(), SerializationFailure> {
        match &self.serial {
            Some((s, id)) => s.lock(&self.rt).read_range(*id, index, lo, hi, ctors),
            None => Ok(()),
        }
    }

    fn fetched(&self, heap: HeapNo, tid: Tid) -> Result<(), SerializationFailure> {
        match &self.serial {
            Some((s, id)) => s.lock(&self.rt).read_tuple(*id, heap.0, tid),
            None => Ok(()),
        }
    }

    fn depends(&self, writer: Xid) -> Result<(), Error> {
        let Some((s, id)) = &self.serial else {
            return Ok(());
        };
        if Some(writer) == self.reader {
            return Ok(());
        }
        let status = |x| self.txns.status(&self.pager, x);
        if self
            .snapshot
            .counts_committed(writer, status)
            .map_err(Error::Page)?
            || status(writer).map_err(Error::Page)? == Status::Aborted
        {
            return Ok(());
        }
        s.lock(&self.rt)
            .read_from(*id, writer)
            .map_err(Error::Serialization)
    }
}

/// A heap a plan names, with the field types of its rows.
#[derive(Clone, Debug)]
pub struct StoredHeap {
    pub heap: Arc<Heap>,
    pub row: Vec<FieldType>,
}

/// An index a plan names, with the constructor position of each constructor id it holds.
#[derive(Clone, Debug)]
pub struct StoredIndex {
    pub index: Index,
    pub ctors: BTreeMap<CtorId, u32>,
}

/// The heaps and indexes a plan's reads go to, and what those reads see.
///
/// Invariant: `heaps` and `indexes` hold every heap and index of the plan's `Reads`.
pub struct Store<R: Runtime> {
    pub pager: Pager<R>,
    pub reads: Arc<dyn Reads>,
    pub heaps: BTreeMap<HeapId, StoredHeap>,
    pub indexes: BTreeMap<IndexId, StoredIndex>,
}

impl<R: Runtime> std::fmt::Debug for Store<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("heaps", &self.heaps.keys().collect::<Vec<_>>())
            .field("indexes", &self.indexes.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

fn index_error(e: btree::Error) -> Error {
    match e {
        btree::Error::Page(p) => Error::Page(p),
        other => Error::Corrupt(format!("{other:?}")),
    }
}

impl<R: Runtime> Store<R> {
    fn heap(&self, id: HeapId) -> &StoredHeap {
        self.heaps
            .get(&id)
            .unwrap_or_else(|| panic!("the store holds heap {id:?}"))
    }

    fn index(&self, id: IndexId) -> &StoredIndex {
        self.indexes
            .get(&id)
            .unwrap_or_else(|| panic!("the store holds index {id:?}"))
    }

    /// Pass the visible rows of heap `id` to `each` as batches of at most `rows` rows each, as
    /// [`Store::scan_pages`] does.
    pub fn scan(
        &self,
        id: HeapId,
        rows: usize,
        each: impl FnMut(Batch) -> Result<ControlFlow<()>, Error>,
    ) -> Result<(), Error> {
        let pages = self.pages(id)?;
        self.scan_pages(id, &pages, rows, each)
    }

    /// Returns the pages of heap `id` in directory order, recording that every version of the
    /// heap is read.
    pub fn pages(&self, id: HeapId) -> Result<Vec<Pid>, Error> {
        let heap = &self.heap(id).heap;
        self.reads
            .scanned(heap.id())
            .map_err(Error::Serialization)?;
        Ok(heap.pages(&self.pager)?)
    }

    /// Pass the visible rows on `pages`, a range of what [`Store::pages`] returned for heap `id`,
    /// to `each` as batches of at most `rows` rows each, reading a page only once the batches
    /// before it are passed. Stop when `each` breaks.
    ///
    /// Postconditions: the writers recorded with [`Reads::depends`] are those of the versions read
    /// before the last batch passed, and of every version on `pages` when `each` never breaks.
    pub fn scan_pages(
        &self,
        id: HeapId,
        pages: &[Pid],
        rows: usize,
        mut each: impl FnMut(Batch) -> Result<ControlFlow<()>, Error>,
    ) -> Result<(), Error> {
        let h = self.heap(id);
        let schema = BatchSchema::of_heap(&h.row);
        let mut b = BatchBuilder::new(schema.clone());
        let mut writers = BTreeSet::new();
        for pid in pages {
            let mut versions = Vec::new();
            h.heap
                .scan_pages(&self.pager, std::slice::from_ref(pid), |v| versions.push(v))?;
            for v in versions {
                let seen = self.reads.sees(&v.header)?;
                writers.extend(writer_of(&v.header, seen));
                if !seen {
                    continue;
                }
                b.push_row(&v.row);
                if b.len() == rows {
                    let full = std::mem::replace(&mut b, BatchBuilder::new(schema.clone()));
                    self.depend_all(std::mem::take(&mut writers))?;
                    if each(full.finish())?.is_break() {
                        return Ok(());
                    }
                }
            }
        }
        self.depend_all(writers)?;
        if b.is_empty() {
            return Ok(());
        }
        each(b.finish()).map(|_last| ())
    }

    fn depend_all(&self, writers: BTreeSet<Xid>) -> Result<(), Error> {
        writers.into_iter().try_for_each(|w| self.reads.depends(w))
    }

    /// Returns the version of heap `id` visible from the chain starting at `tid`, with its TID.
    pub fn fetch(&self, id: HeapId, tid: Tid) -> Result<Option<(Tid, Vec<Value>)>, Error> {
        let heap = &self.heap(id).heap;
        let mut failed = None;
        let mut writers = BTreeSet::new();
        let link = walk(&self.pager, heap.id(), tid, |l| {
            match self.reads.sees(&l.header) {
                Ok(seen) => {
                    writers.extend(writer_of(&l.header, seen));
                    seen
                }
                Err(e) => {
                    failed = Some(e);
                    true
                }
            }
        })?;
        if let Some(e) = failed {
            return Err(Error::Page(e));
        }
        self.depend_all(writers)?;
        let Some(link) = link.filter(|l| l.heap == heap.id()) else {
            return Ok(None);
        };
        self.reads
            .fetched(link.heap, link.tid)
            .map_err(Error::Serialization)?;
        let v = heap
            .read(&self.pager, link.tid)?
            .expect("a walked version is there");
        Ok(Some((link.tid, v.row)))
    }

    /// Returns the entries of index `id` whose leading key fields are `prefix`, each with its
    /// constructor position, for the constructors of `routes`.
    pub fn probe(
        &self,
        id: IndexId,
        prefix: &[Value],
        routes: &BTreeSet<u32>,
    ) -> Result<Vec<(u32, Tid)>, Error> {
        let bytes = self.encode_prefix(id, prefix);
        let end = prefix_end(&bytes);
        self.entries(id, Bound::Included(bytes), end, routes)
    }

    /// Returns the entries of index `id` whose leading key fields are `prefix` and whose next
    /// field lies within `lo` and `hi` in the language's order, each bound its value and whether
    /// it is included, as [`Store::probe`] does.
    ///
    /// Preconditions: the index has a field after `prefix`, and the field's type is ordered.
    pub fn range(
        &self,
        id: IndexId,
        prefix: &[Value],
        lo: Option<(Value, bool)>,
        hi: Option<(Value, bool)>,
        routes: &BTreeSet<u32>,
    ) -> Result<Vec<(u32, Tid)>, Error> {
        let base = self.encode_prefix(id, prefix);
        let next = self.index(id).index.schema().components()[prefix.len()].clone();
        let dir = next.1;
        let field = KeySchema::new(vec![next]);
        let at = |v: &Value| {
            let mut b = base.clone();
            field.encode_into(std::slice::from_ref(v), &mut b);
            b
        };
        let (lo, hi) = match dir {
            Dir::Asc => (lo, hi),
            Dir::Desc => (hi, lo),
        };
        let start = match lo {
            None => Bound::Included(base.clone()),
            Some((v, true)) => Bound::Included(at(&v)),
            Some((v, false)) => match prefix_end(&at(&v)) {
                Bound::Excluded(end) => Bound::Included(end),
                _ => return Ok(Vec::new()),
            },
        };
        let end = match hi {
            None => prefix_end(&base),
            Some((v, true)) => prefix_end(&at(&v)),
            Some((v, false)) => Bound::Excluded(at(&v)),
        };
        self.entries(id, start, end, routes)
    }

    fn encode_prefix(&self, id: IndexId, prefix: &[Value]) -> Vec<u8> {
        let components = self.index(id).index.schema().components();
        KeySchema::new(components[..prefix.len()].to_vec()).encode(prefix)
    }

    /// Returns the entries of index `id` whose encodings lie between `lo` and `hi`, each with
    /// its constructor position, for the constructors of `routes`.
    fn entries(
        &self,
        id: IndexId,
        lo: Bound<Vec<u8>>,
        hi: Bound<Vec<u8>>,
        routes: &BTreeSet<u32>,
    ) -> Result<Vec<(u32, Tid)>, Error> {
        let ix = self.index(id);
        let ctors: BTreeSet<CtorId> = ix
            .ctors
            .iter()
            .filter(|(_, c)| routes.contains(c))
            .map(|(id, _)| *id)
            .collect();
        self.reads
            .probed(ix.index.tree().root(), lo.clone(), hi.clone(), ctors)
            .map_err(Error::Serialization)?;
        let entries = ix
            .index
            .scan(
                &self.pager,
                lo.as_ref().map(Vec::as_slice),
                hi.as_ref().map(Vec::as_slice),
            )
            .map_err(index_error)?;
        Ok(entries
            .into_iter()
            .filter_map(|e| {
                let c = *ix
                    .ctors
                    .get(&e.ctor)
                    .unwrap_or_else(|| panic!("index {id:?} maps constructor {:?}", e.ctor));
                routes.contains(&c).then_some((c, e.tid))
            })
            .collect())
    }

    /// Returns the key types of index `id`'s fields.
    pub fn key_types(&self, id: IndexId) -> Vec<KeyType> {
        self.index(id)
            .index
            .schema()
            .components()
            .iter()
            .map(|(t, _)| t.clone())
            .collect()
    }
}

/// How a stored value of one kernel type becomes a [`Val`].
#[derive(Clone, Debug)]
pub enum Conv {
    Scalar,
    /// A declared sum or record, by constructor.
    Decl(Vec<Vec<Conv>>),
    AnonRecord(Vec<Conv>),
}

/// Returns how a stored value of type `ty` converts.
pub fn conv(schema: &Schema, ty: &Ty) -> Conv {
    let t = repr(schema, ty);
    match &t {
        Ty::Base(_) => Conv::Scalar,
        Ty::Record(fs) => Conv::AnonRecord(fs.iter().map(|(_, f)| conv(schema, f)).collect()),
        Ty::Key { rel, .. } => {
            let r = schema.relation(*rel);
            let row = ctors(schema, &r.elem).expect("a relation's element type is a sum");
            let field_ty = |n: &str| {
                row[0]
                    .iter()
                    .find(|(f, _)| f == n)
                    .unwrap_or_else(|| panic!("a row of `{}` has key field `{n}`", r.name))
                    .1
                    .clone()
            };
            match r.key.as_slice() {
                [k] => match field_ty(k) {
                    Ty::Key { rel: own, .. } if own == *rel => Conv::Scalar,
                    f => conv(schema, &f),
                },
                ks => {
                    let mut names = ks.to_vec();
                    names.sort();
                    Conv::AnonRecord(names.iter().map(|n| conv(schema, &field_ty(n))).collect())
                }
            }
        }
        _ => match ctors(schema, &t) {
            Some(cs) => Conv::Decl(
                cs.iter()
                    .map(|fs| fs.iter().map(|(_, f)| conv(schema, f)).collect())
                    .collect(),
            ),
            None => panic!("a stored value has no type {t:?}"),
        },
    }
}

/// Returns stored value `v` of a type converting by `c`.
pub fn from_stored(c: &Conv, v: &Value) -> Val {
    let all = |cs: &[Conv], fs: &[Value]| -> Vec<Val> {
        cs.iter().zip(fs).map(|(c, v)| from_stored(c, v)).collect()
    };
    match (c, v) {
        (Conv::Decl(cs), Value::Record(fs)) => Val::ctor(0, all(&cs[0], fs)),
        (Conv::Decl(cs), Value::Sum { ctor, fields }) => {
            Val::ctor(*ctor, all(&cs[*ctor as usize], fields))
        }
        (Conv::AnonRecord(cs), Value::Record(fs)) => Val::record(all(cs, fs)),
        (_, v) => scalar(v),
    }
}

fn scalar(v: &Value) -> Val {
    match v {
        Value::I8(x) => Val::Int(i64::from(*x)),
        Value::I16(x) => Val::Int(i64::from(*x)),
        Value::I32(x) => Val::Int(i64::from(*x)),
        Value::I64(x) => Val::Int(*x),
        Value::F32(x) => Val::F32(F32::new(x.get())),
        Value::F64(x) => Val::F64(F64::new(x.get())),
        Value::Decimal(d) => Val::Decimal(*d),
        Value::Text(t) => Val::text(t),
        Value::Bytes(b) => Val::Bytes(Arc::from(b.as_slice())),
        Value::SortKey(b) => Val::SortKey(Arc::from(b.as_slice())),
        Value::Instant(x) => Val::Instant(*x),
        Value::Date(x) => Val::Date(*x),
        Value::Time(x) => Val::Time(*x),
        Value::DateTime(x) => Val::DateTime(*x),
        Value::ZonedDateTime { local, zone } => Val::Zoned {
            local: *local,
            zone: Arc::from(zone.as_str()),
        },
        Value::Duration(x) => Val::Duration(*x),
        Value::Period { months, days } => Val::Period {
            months: *months,
            days: *days,
        },
        other => panic!("a scalar conversion of {other:?}"),
    }
}

/// Returns `v` as a heap stores a field of type `t`.
pub fn to_stored(t: &FieldType, v: &Val) -> Result<Value, Error> {
    Ok(match (t, v) {
        (FieldType::Scalar(s), v) => scalar_key(*s, v)?,
        (FieldType::Record(ts), Val::Ctor(_, fs) | Val::Record(fs)) => Value::Record(
            ts.iter()
                .zip(fs.iter())
                .map(|(t, v)| to_stored(t, v))
                .collect::<Result<_, _>>()?,
        ),
        (FieldType::Sum(cs), Val::Ctor(c, fs)) => Value::Sum {
            ctor: *c,
            fields: cs
                .get(*c as usize)
                .ok_or_else(|| not_stored(v, t))?
                .fields
                .iter()
                .zip(fs.iter())
                .map(|(t, v)| to_stored(t, v))
                .collect::<Result<_, _>>()?,
        },
        (t, v) => return Err(not_stored(v, t)),
    })
}

/// Returns `v` as a value of key type `t`.
pub fn to_key(t: &KeyType, v: &Val) -> Result<Value, Error> {
    Ok(match (t, v) {
        (KeyType::Scalar(s), v) => scalar_key(*s, v)?,
        (KeyType::Record(ts), Val::Ctor(_, fs) | Val::Record(fs)) => Value::Record(
            ts.iter()
                .zip(fs.iter())
                .map(|(t, v)| to_key(t, v))
                .collect::<Result<_, _>>()?,
        ),
        (KeyType::AnonRecord(r), Val::Record(fs)) => Value::Record(
            r.fields()
                .iter()
                .zip(fs.iter())
                .map(|((_, t), v)| to_key(t, v))
                .collect::<Result<_, _>>()?,
        ),
        (KeyType::Sum(s), Val::Ctor(c, fs)) => Value::Sum {
            ctor: *c,
            fields: s
                .ctors()
                .get(*c as usize)
                .ok_or_else(|| not_stored(v, t))?
                .fields
                .iter()
                .zip(fs.iter())
                .map(|(t, v)| to_key(t, v))
                .collect::<Result<_, _>>()?,
        },
        (KeyType::AnonSum(s), Val::Ctor(c, fs)) => Value::Sum {
            ctor: *c,
            fields: s
                .ctors()
                .get(*c as usize)
                .ok_or_else(|| not_stored(v, t))?
                .1
                .fields()
                .iter()
                .zip(fs.iter())
                .map(|((_, t), v)| to_key(t, v))
                .collect::<Result<_, _>>()?,
        },
        (t, v) => return Err(not_stored(v, t)),
    })
}

fn scalar_key(s: Scalar, v: &Val) -> Result<Value, Error> {
    Ok(match (s, v) {
        (Scalar::I8, Val::Int(x)) => Value::I8(*x as i8),
        (Scalar::I16, Val::Int(x)) => Value::I16(*x as i16),
        (Scalar::I32, Val::Int(x)) => Value::I32(*x as i32),
        (Scalar::I64, Val::Int(x)) => Value::I64(*x),
        (Scalar::F32, Val::F32(x)) => Value::F32(crate::key::F32::new(x.get())),
        (Scalar::F64, Val::F64(x)) => Value::F64(crate::key::F64::new(x.get())),
        (Scalar::Decimal(_), Val::Decimal(d)) => Value::Decimal(*d),
        (Scalar::Text, Val::Text(t)) => Value::Text(t.to_string()),
        (Scalar::Bytes, Val::Bytes(b)) => Value::Bytes(b.to_vec()),
        (Scalar::SortKey, Val::SortKey(b)) => Value::SortKey(b.to_vec()),
        (Scalar::Instant, Val::Instant(x)) => Value::Instant(*x),
        (Scalar::Date, Val::Date(x)) => Value::Date(*x),
        (Scalar::Time, Val::Time(x)) => Value::Time(*x),
        (Scalar::DateTime, Val::DateTime(x)) => Value::DateTime(*x),
        (Scalar::ZonedDateTime, Val::Zoned { local, zone }) => Value::ZonedDateTime {
            local: *local,
            zone: zone.to_string(),
        },
        (Scalar::Duration, Val::Duration(x)) => Value::Duration(*x),
        (Scalar::Period, Val::Period { months, days }) => Value::Period {
            months: *months,
            days: *days,
        },
        (s, v) => return Err(not_stored(v, s)),
    })
}

fn not_stored(v: &Val, t: impl std::fmt::Debug) -> Error {
    Error::Internal(format!("{v:?} is not a value of {t:?}"))
}
