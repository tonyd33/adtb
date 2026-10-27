use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, btree_set};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use adtb_sys::lock::Lock;

use crate::runtime::{AlignedBuf, Done, FileId, Op, Runtime};
use crate::storage::{Driver, Malformed, Reader};

use super::memory::{Arena, Grant};
use super::value::{F32, F64, Val};
use super::{Error, Exceeded};

/// The unit of spill file I/O.
const BLOCK: usize = 4096;
/// The bytes a writer buffers before it writes them out.
const CHUNK: usize = 2 * BLOCK;
/// The most runs one merge reads at once.
const FAN_IN: usize = 64;
const DIR: &str = "spill";

struct File<R: Runtime> {
    driver: Driver<R>,
    id: FileId,
    path: String,
    end: u64,
}

/// The spill file of one call, made on its first write and removed when the call ends.
///
/// Its buffers are outside the call's grant: each writer and reader holds at most one chunk.
pub struct Spill<R: Runtime> {
    rt: R,
    grant: Arc<Grant>,
    limit: usize,
    file: Lock<Option<File<R>>>,
}

#[derive(Clone, Copy, Debug)]
struct Extent {
    at: u64,
    len: usize,
}

impl<R: Runtime> Spill<R> {
    /// Returns a spill area of at most `limit` bytes for the call holding `grant`.
    pub fn new(rt: R, grant: Arc<Grant>, limit: usize) -> Spill<R> {
        Spill {
            rt,
            grant,
            limit,
            file: Lock::new(None),
        }
    }

    /// Returns the error of a call that cannot stay within its grant by spilling.
    fn full(&self) -> Error {
        Error::StatementLimit(Exceeded::Memory {
            limit: self.grant.limit(),
        })
    }

    fn open(&self) -> Result<File<R>, Error> {
        let mut driver = Driver::new(self.rt.clone());
        let t = driver.submit(Op::CreateDir { dir: DIR.into() });
        driver.wait(t).result.map_err(|_| self.full())?;
        let path = format!("{DIR}/{:016x}", self.rt.next_u64());
        let (id, _) = driver.open(&path, true).map_err(|_| self.full())?;
        Ok(File {
            driver,
            id,
            path,
            end: 0,
        })
    }

    fn write(&self, bytes: &[u8]) -> Result<Extent, Error> {
        let padded = bytes.len().div_ceil(BLOCK) * BLOCK;
        let mut slot = self.file.lock(&self.rt);
        if slot.is_none() {
            if padded > self.limit {
                return Err(self.full());
            }
            *slot = Some(self.open()?);
        }
        let f = slot.as_mut().expect("a spill file is open once written");
        let at = f.end;
        let fits = usize::try_from(at).is_ok_and(|at| at.saturating_add(padded) <= self.limit);
        if !fits {
            return Err(self.full());
        }
        let mut buf = AlignedBuf::zeroed(padded);
        buf[..bytes.len()].copy_from_slice(bytes);
        let t = f.driver.submit(Op::Write {
            file: f.id,
            offset: at,
            buf,
        });
        f.driver.wait(t).result.map_err(|_| self.full())?;
        f.end += padded as u64;
        self.grant.note_spill(padded);
        Ok(Extent {
            at,
            len: bytes.len(),
        })
    }

    fn read(&self, e: Extent) -> Result<Vec<u8>, Error> {
        let padded = e.len.div_ceil(BLOCK) * BLOCK;
        let mut slot = self.file.lock(&self.rt);
        let f = slot
            .as_mut()
            .expect("a spill file is open while its extents are read");
        let t = f.driver.submit(Op::Read {
            file: f.id,
            offset: e.at,
            buf: AlignedBuf::zeroed(padded),
        });
        let c = f.driver.wait(t);
        match (c.result, c.buf) {
            (Ok(Done::Read(n)), Some(buf)) if n >= e.len => Ok(buf[..e.len].to_vec()),
            _ => Err(Error::Corrupt("a spilled extent reads back short".into())),
        }
    }
}

impl<R: Runtime> Drop for Spill<R> {
    fn drop(&mut self) {
        if let Some(mut f) = self.file.lock(&self.rt).take() {
            f.driver.close(f.id);
            f.driver.remove(&f.path);
        }
    }
}

/// Records of values written to a spill file, read back in the order written.
#[derive(Debug, Default)]
pub struct Stream {
    extents: Vec<Extent>,
}

impl Stream {
    pub fn is_empty(&self) -> bool {
        self.extents.is_empty()
    }
}

/// A stream being written: whole records in a buffer of at most one chunk past the last write.
#[derive(Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
    stream: Stream,
}

impl Writer {
    /// Append one record of `vals`.
    pub fn put<R: Runtime>(&mut self, sp: &Spill<R>, vals: &[&Val]) -> Result<(), Error> {
        let start = self.buf.len();
        self.buf.extend_from_slice(&[0; 4]);
        for v in vals {
            put(v, &mut self.buf);
        }
        let len = u32::try_from(self.buf.len() - start - 4)
            .expect("a spilled record is smaller than 4 GiB");
        self.buf[start..start + 4].copy_from_slice(&len.to_le_bytes());
        if self.buf.len() >= CHUNK {
            self.flush(sp)?;
        }
        Ok(())
    }

    fn flush<R: Runtime>(&mut self, sp: &Spill<R>) -> Result<(), Error> {
        if !self.buf.is_empty() {
            let e = sp.write(&self.buf)?;
            self.stream.extents.push(e);
            self.buf.clear();
        }
        Ok(())
    }

    pub fn finish<R: Runtime>(mut self, sp: &Spill<R>) -> Result<Stream, Error> {
        self.flush(sp)?;
        Ok(self.stream)
    }
}

/// A reader of the records of a stream.
pub struct Records<'a, R: Runtime> {
    sp: &'a Spill<R>,
    extents: std::vec::IntoIter<Extent>,
    buf: Vec<u8>,
    at: usize,
}

impl<'a, R: Runtime> Records<'a, R> {
    pub fn new(sp: &'a Spill<R>, s: &Stream) -> Records<'a, R> {
        Records {
            sp,
            extents: s.extents.clone().into_iter(),
            buf: Vec::new(),
            at: 0,
        }
    }

    /// Returns the next record's values, or `None` past the last.
    pub fn read(&mut self) -> Result<Option<Vec<Val>>, Error> {
        loop {
            let rest = &self.buf[self.at..];
            if rest.len() >= 4 {
                let len = u32::from_le_bytes(rest[..4].try_into().expect("four bytes")) as usize;
                if rest.len() >= 4 + len {
                    let vals = record(&rest[4..4 + len])?;
                    self.at += 4 + len;
                    return Ok(Some(vals));
                }
            }
            let Some(e) = self.extents.next() else {
                if self.at == self.buf.len() {
                    return Ok(None);
                }
                return Err(Error::Corrupt("a spilled record is cut short".into()));
            };
            self.buf.drain(..self.at);
            self.at = 0;
            self.buf.extend(self.sp.read(e)?);
        }
    }
}

pub(super) fn record(bytes: &[u8]) -> Result<Vec<Val>, Error> {
    let mut r = Reader::new(bytes);
    let mut out = Vec::new();
    while !r.is_empty() {
        out.push(get(&mut r).map_err(|Malformed| malformed())?);
    }
    Ok(out)
}

fn malformed() -> Error {
    Error::Corrupt("a spilled value does not decode".into())
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    let n = u32::try_from(b.len()).expect("a spilled value is smaller than 4 GiB");
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(b);
}

fn put_seq<'a>(out: &mut Vec<u8>, vs: impl ExactSizeIterator<Item = &'a Val>) {
    let n = u32::try_from(vs.len()).expect("a spilled value has fewer than 2^32 parts");
    out.extend_from_slice(&n.to_le_bytes());
    for v in vs {
        put(v, out);
    }
}

pub(super) fn put(v: &Val, out: &mut Vec<u8>) {
    match v {
        Val::Int(i) => tagged(out, 0, &i.to_le_bytes()),
        Val::F32(x) => tagged(out, 1, &x.get().to_bits().to_le_bytes()),
        Val::F64(x) => tagged(out, 2, &x.get().to_bits().to_le_bytes()),
        Val::Decimal(d) => tagged(out, 3, &d.to_le_bytes()),
        Val::Text(t) => {
            out.push(4);
            put_bytes(out, t.as_bytes());
        }
        Val::Bytes(b) => {
            out.push(5);
            put_bytes(out, b);
        }
        Val::SortKey(b) => {
            out.push(6);
            put_bytes(out, b);
        }
        Val::Instant(i) => tagged(out, 7, &i.to_le_bytes()),
        Val::Date(d) => tagged(out, 8, &d.to_le_bytes()),
        Val::Time(i) => tagged(out, 9, &i.to_le_bytes()),
        Val::DateTime(i) => tagged(out, 10, &i.to_le_bytes()),
        Val::Zoned { local, zone } => {
            tagged(out, 11, &local.to_le_bytes());
            put_bytes(out, zone.as_bytes());
        }
        Val::Duration(i) => tagged(out, 12, &i.to_le_bytes()),
        Val::Period { months, days } => {
            tagged(out, 13, &months.to_le_bytes());
            out.extend_from_slice(&days.to_le_bytes());
        }
        Val::Ctor(c, fs) => {
            tagged(out, 14, &c.to_le_bytes());
            put_seq(out, fs.iter());
        }
        Val::Record(fs) => {
            out.push(15);
            put_seq(out, fs.iter());
        }
        Val::Set(s) => {
            out.push(16);
            put_seq(out, s.iter());
        }
        Val::List(xs) => {
            out.push(17);
            put_seq(out, xs.iter());
        }
    }
}

fn tagged(out: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
    out.push(tag);
    out.extend_from_slice(bytes);
}

fn fixed<const N: usize>(r: &mut Reader<'_>) -> Result<[u8; N], Malformed> {
    Ok(r.take(N)?.try_into().expect("a slice of the length taken"))
}

fn get_bytes<'a>(r: &mut Reader<'a>) -> Result<&'a [u8], Malformed> {
    let n = r.u32()? as usize;
    r.take(n)
}

fn get_text(r: &mut Reader<'_>) -> Result<Arc<str>, Malformed> {
    std::str::from_utf8(get_bytes(r)?)
        .map(Arc::from)
        .map_err(|_| Malformed)
}

fn get_seq(r: &mut Reader<'_>) -> Result<Vec<Val>, Malformed> {
    let n = r.u32()?;
    (0..n).map(|_| get(r)).collect()
}

fn get(r: &mut Reader<'_>) -> Result<Val, Malformed> {
    let i64_of = |r: &mut Reader<'_>| Ok::<_, Malformed>(i64::from_le_bytes(fixed(r)?));
    Ok(match fixed::<1>(r)?[0] {
        0 => Val::Int(i64_of(r)?),
        1 => Val::F32(F32::new(f32::from_bits(r.u32()?))),
        2 => Val::F64(F64::new(f64::from_bits(r.u64()?))),
        3 => Val::Decimal(i128::from_le_bytes(fixed(r)?)),
        4 => Val::Text(get_text(r)?),
        5 => Val::Bytes(Arc::from(get_bytes(r)?)),
        6 => Val::SortKey(Arc::from(get_bytes(r)?)),
        7 => Val::Instant(i64_of(r)?),
        8 => Val::Date(i32::from_le_bytes(fixed(r)?)),
        9 => Val::Time(i64_of(r)?),
        10 => Val::DateTime(i64_of(r)?),
        11 => Val::Zoned {
            local: i64_of(r)?,
            zone: get_text(r)?,
        },
        12 => Val::Duration(i64_of(r)?),
        13 => Val::Period {
            months: i32::from_le_bytes(fixed(r)?),
            days: i32::from_le_bytes(fixed(r)?),
        },
        14 => {
            let c = r.u32()?;
            Val::ctor(c, get_seq(r)?)
        }
        15 => Val::record(get_seq(r)?),
        16 => Val::set(get_seq(r)?.into_iter().collect()),
        17 => Val::List(Arc::from(get_seq(r)?)),
        _ => return Err(Malformed),
    })
}

/// A set of distinct values held in memory up to an eighth of its grant, the rest in sorted
/// runs of the spill file.
pub struct Distinct {
    set: BTreeSet<Val>,
    arena: Arena,
    cap: usize,
    runs: Vec<Stream>,
    /// Set on the first value inserted.
    stop: Option<Arc<AtomicBool>>,
}

impl Distinct {
    pub fn new(grant: &Arc<Grant>) -> Result<Distinct, Error> {
        Ok(Distinct {
            set: BTreeSet::new(),
            arena: grant.take(0)?,
            cap: grant.limit() / 8,
            runs: Vec::new(),
            stop: None,
        })
    }

    /// Returns an empty set that sets `stop`, if any, on its first insert.
    pub fn stopping(grant: &Arc<Grant>, stop: Option<Arc<AtomicBool>>) -> Result<Distinct, Error> {
        Ok(Distinct {
            stop,
            ..Distinct::new(grant)?
        })
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty() && self.runs.is_empty()
    }

    /// Add `v`, spilling what memory holds past the cap or when the grant has no room for it.
    pub fn insert<R: Runtime>(&mut self, v: Val, sp: &Spill<R>) -> Result<(), Error> {
        if let Some(s) = &self.stop {
            s.store(true, Ordering::Release);
        }
        let n = v.size();
        if self.set.insert(v) {
            if self.arena.bytes() + n > self.cap {
                return self.spill(sp);
            }
            match self.arena.grow(n) {
                Ok(()) => {}
                Err(Error::StatementLimit(Exceeded::Memory { .. })) => self.spill(sp)?,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn spill<R: Runtime>(&mut self, sp: &Spill<R>) -> Result<(), Error> {
        let mut w = Writer::default();
        for v in &self.set {
            w.put(sp, &[v])?;
        }
        self.runs.push(w.finish(sp)?);
        self.set.clear();
        self.arena.release(usize::MAX);
        Ok(())
    }

    /// Add every value of `other`, spilling what memory holds past the cap.
    pub fn absorb<R: Runtime>(&mut self, other: Distinct, sp: &Spill<R>) -> Result<(), Error> {
        if self.set.is_empty() {
            self.set = other.set;
        } else {
            self.set.extend(other.set);
        }
        self.arena.absorb(other.arena);
        self.runs.extend(other.runs);
        if self.arena.bytes() > self.cap {
            self.spill(sp)?;
        }
        Ok(())
    }

    /// Returns the values as one set in memory, which must fit the grant.
    pub fn into_set<R: Runtime>(self, sp: &Spill<R>) -> Result<BTreeSet<Val>, Error> {
        if self.runs.is_empty() {
            return Ok(self.set);
        }
        let mut held = self.arena.grant().take(0)?;
        let mut m = self.merge(sp)?;
        let mut out = BTreeSet::new();
        while let Some(v) = m.pop()? {
            held.grow(v.size())?;
            out.insert(v);
        }
        Ok(out)
    }

    /// Returns the values in order, read back from memory and the spill file, each memory
    /// holds returned to the grant as it is read.
    pub fn merge<R: Runtime>(mut self, sp: &Spill<R>) -> Result<Merge<'_, R>, Error> {
        while self.runs.len() > FAN_IN {
            let group: Vec<Stream> = self.runs.drain(..FAN_IN).collect();
            let mut m = Merge::of(sp, None, group)?;
            let mut w = Writer::default();
            while let Some(v) = m.pop()? {
                w.put(sp, &[&v])?;
            }
            self.runs.push(w.finish(sp)?);
        }
        Merge::of(sp, Some((self.set, self.arena)), self.runs)
    }
}

enum Source<'a, R: Runtime> {
    Mem(btree_set::IntoIter<Val>, Arena),
    Run(Records<'a, R>),
}

impl<R: Runtime> Source<'_, R> {
    fn pop(&mut self) -> Result<Option<Val>, Error> {
        match self {
            Source::Mem(it, arena) => Ok(it.next().inspect(|v| arena.release(v.size()))),
            Source::Run(r) => Ok(r.read()?.and_then(|mut vs| vs.pop())),
        }
    }
}

/// The distinct values of several sorted sources, in order.
pub struct Merge<'a, R: Runtime> {
    sources: Vec<Source<'a, R>>,
    heads: BinaryHeap<Reverse<(Val, usize)>>,
    last: Option<Val>,
}

impl<'a, R: Runtime> Merge<'a, R> {
    fn of(
        sp: &'a Spill<R>,
        mem: Option<(BTreeSet<Val>, Arena)>,
        runs: Vec<Stream>,
    ) -> Result<Merge<'a, R>, Error> {
        let mut sources: Vec<Source<'a, R>> = Vec::with_capacity(runs.len() + 1);
        if let Some((set, arena)) = mem {
            sources.push(Source::Mem(set.into_iter(), arena));
        }
        sources.extend(runs.iter().map(|s| Source::Run(Records::new(sp, s))));
        let mut heads = BinaryHeap::with_capacity(sources.len());
        for (i, s) in sources.iter_mut().enumerate() {
            if let Some(v) = s.pop()? {
                heads.push(Reverse((v, i)));
            }
        }
        Ok(Merge {
            sources,
            heads,
            last: None,
        })
    }

    /// Returns the next value, or `None` past the last.
    pub fn pop(&mut self) -> Result<Option<Val>, Error> {
        while let Some(Reverse((v, i))) = self.heads.pop() {
            if let Some(n) = self.sources[i].pop()? {
                self.heads.push(Reverse((n, i)));
            }
            if self.last.as_ref() != Some(&v) {
                self.last = Some(v.clone());
                return Ok(Some(v));
            }
        }
        Ok(None)
    }

    /// Returns up to `n` next values.
    pub fn chunk(&mut self, n: usize) -> Result<Vec<Val>, Error> {
        let mut out = Vec::new();
        while out.len() < n {
            match self.pop()? {
                Some(v) => out.push(v),
                None => break,
            }
        }
        Ok(out)
    }
}
