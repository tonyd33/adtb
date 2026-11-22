//! Write and read the flatbuffer tables of Arrow IPC metadata.

use super::IpcError;

type R<T> = Result<T, IpcError>;

fn bad(what: &'static str) -> IpcError {
    IpcError::Malformed(what)
}

/// A field of a table.
#[derive(Clone, Debug)]
pub(super) enum Val {
    /// A scalar of `width` bytes, and the schema's default for its slot.
    Num {
        width: usize,
        v: i64,
        default: i64,
    },
    Str(String),
    Table(Table),
    Tables(Vec<Table>),
    I32s(Vec<i32>),
    /// A vector of structs of two `i64`s.
    Pairs(Vec<[i64; 2]>),
}

/// A table's fields by slot.
#[derive(Clone, Debug, Default)]
pub(super) struct Table(pub Vec<(u16, Val)>);

impl Table {
    pub fn with(mut self, slot: u16, v: Val) -> Table {
        self.0.push((slot, v));
        self
    }

    pub fn num(self, slot: u16, width: usize, v: i64, default: i64) -> Table {
        self.with(slot, Val::Num { width, v, default })
    }
}

/// Returns the flatbuffer whose root table is `root`.
pub(super) fn finish(root: &Table) -> Vec<u8> {
    let mut out = vec![0; 4];
    let at = table(&mut out, root);
    patch(&mut out, 0, at);
    out
}

fn patch(out: &mut [u8], slot: usize, target: usize) {
    let off = u32::try_from(target - slot).expect("a metadata flatbuffer is under 4 GiB");
    out[slot..slot + 4].copy_from_slice(&off.to_le_bytes());
}

fn pad(out: &mut Vec<u8>, align: usize, rem: usize) {
    while out.len() % align != rem {
        out.push(0);
    }
}

fn u16_of(n: usize) -> [u8; 2] {
    u16::try_from(n)
        .expect("a metadata table has under 8192 fields")
        .to_le_bytes()
}

fn u32_of(n: usize) -> [u8; 4] {
    u32::try_from(n)
        .expect("a metadata vector has under 2^32 elements")
        .to_le_bytes()
}

/// Lay out each inline field in 8 bytes at a multiple of 8, so each scalar is aligned to its width.
fn table(out: &mut Vec<u8>, t: &Table) -> usize {
    let slots =
        t.0.iter()
            .map(|(s, _)| usize::from(*s) + 1)
            .max()
            .unwrap_or(0);
    pad(out, 2, 0);
    let vtable = out.len();
    out.extend(u16_of(4 + 2 * slots));
    out.extend(u16_of(4 + 8 * t.0.len()));
    let entries = out.len();
    out.resize(entries + 2 * slots, 0);
    pad(out, 8, 4);
    let at = out.len();
    out.extend(u32_of(at - vtable));
    let mut deferred = Vec::new();
    for (i, (slot, v)) in t.0.iter().enumerate() {
        let field = at + 4 + 8 * i;
        let e = entries + 2 * usize::from(*slot);
        out[e..e + 2].copy_from_slice(&u16_of(4 + 8 * i));
        match v {
            Val::Num { width, v, .. } => out.extend(&v.to_le_bytes()[..*width]),
            v => deferred.push((field, v)),
        }
        out.resize(field + 8, 0);
    }
    for (field, v) in deferred {
        let target = object(out, v);
        patch(out, field, target);
    }
    at
}

fn object(out: &mut Vec<u8>, v: &Val) -> usize {
    match v {
        Val::Table(t) => return table(out, t),
        Val::Pairs(_) => pad(out, 8, 4),
        _ => pad(out, 4, 0),
    }
    let at = out.len();
    match v {
        Val::Str(s) => {
            out.extend(u32_of(s.len()));
            out.extend(s.as_bytes());
            out.push(0);
        }
        Val::I32s(xs) => {
            out.extend(u32_of(xs.len()));
            xs.iter().for_each(|x| out.extend(x.to_le_bytes()));
        }
        Val::Pairs(ps) => {
            out.extend(u32_of(ps.len()));
            ps.iter()
                .flatten()
                .for_each(|x| out.extend(x.to_le_bytes()));
        }
        Val::Tables(ts) => {
            out.extend(u32_of(ts.len()));
            out.resize(at + 4 + 4 * ts.len(), 0);
            for (i, t) in ts.iter().enumerate() {
                let target = table(out, t);
                patch(out, at + 4 + 4 * i, target);
            }
        }
        Val::Num { .. } | Val::Table(_) => unreachable!("a scalar or table is not a vector"),
    }
    at
}

/// A table's position in a flatbuffer.
#[derive(Clone, Copy, Debug)]
pub(super) struct Tab(usize);

/// A flatbuffer that every read checks against its bounds.
#[derive(Clone, Copy)]
pub(super) struct Reader<'a>(pub &'a [u8]);

impl<'a> Reader<'a> {
    fn bytes(&self, at: usize, n: usize) -> R<&'a [u8]> {
        at.checked_add(n)
            .filter(|end| *end <= self.0.len())
            .map(|end| &self.0[at..end])
            .ok_or(bad("an offset past the end of the metadata"))
    }

    fn uint(&self, at: usize, width: usize) -> R<usize> {
        let mut b = [0; 8];
        b[..width].copy_from_slice(self.bytes(at, width)?);
        usize::try_from(u64::from_le_bytes(b)).map_err(|_| bad("an offset past usize"))
    }

    fn target(&self, slot: usize) -> R<usize> {
        let off = self.uint(slot, 4)?;
        Ok(slot + off)
    }

    pub fn root(&self) -> R<Tab> {
        Ok(Tab(self.target(0)?))
    }

    fn vtable(&self, t: Tab) -> R<usize> {
        let soff = i32::from_le_bytes(self.bytes(t.0, 4)?.try_into().unwrap());
        let vt = i64::try_from(t.0).unwrap() - i64::from(soff);
        usize::try_from(vt).map_err(|_| bad("a vtable before the metadata"))
    }

    fn field(&self, t: Tab, slot: u16) -> R<Option<usize>> {
        let vt = self.vtable(t)?;
        let e = 4 + 2 * usize::from(slot);
        if e + 2 > self.uint(vt, 2)? {
            return Ok(None);
        }
        Ok(match self.uint(vt + e, 2)? {
            0 => None,
            off => Some(t.0 + off),
        })
    }

    /// Returns how many slots `t`'s vtable has.
    pub fn slots(&self, t: Tab) -> R<u16> {
        let vt = self.vtable(t)?;
        Ok(u16::try_from(self.uint(vt, 2)?.saturating_sub(4) / 2).unwrap())
    }

    pub fn has(&self, t: Tab, slot: u16) -> R<bool> {
        Ok(self.field(t, slot)?.is_some())
    }

    /// Returns the scalar of `width` bytes at `slot`, sign-extended past one byte.
    pub fn num(&self, t: Tab, slot: u16, width: usize, default: i64) -> R<i64> {
        let Some(at) = self.field(t, slot)? else {
            return Ok(default);
        };
        Ok(le(self.bytes(at, width)?))
    }

    pub fn table(&self, t: Tab, slot: u16) -> R<Option<Tab>> {
        self.field(t, slot)?
            .map(|at| self.target(at).map(Tab))
            .transpose()
    }

    fn vector(&self, t: Tab, slot: u16, elem: usize) -> R<Option<(usize, usize)>> {
        let Some(at) = self.field(t, slot)? else {
            return Ok(None);
        };
        let at = self.target(at)?;
        let n = self.uint(at, 4)?;
        self.bytes(at + 4, n * elem)?;
        Ok(Some((at + 4, n)))
    }

    pub fn str(&self, t: Tab, slot: u16) -> R<Option<&'a str>> {
        let Some((at, n)) = self.vector(t, slot, 1)? else {
            return Ok(None);
        };
        let s = std::str::from_utf8(self.bytes(at, n)?).map_err(|_| bad("a name not in UTF-8"))?;
        Ok(Some(s))
    }

    pub fn tables(&self, t: Tab, slot: u16) -> R<Vec<Tab>> {
        let (at, n) = self.vector(t, slot, 4)?.unwrap_or((0, 0));
        (0..n).map(|i| self.target(at + 4 * i).map(Tab)).collect()
    }

    /// Returns the vector of `width`-byte little-endian scalars at `slot`.
    pub fn nums(&self, t: Tab, slot: u16, width: usize) -> R<Vec<i64>> {
        let (at, n) = self.vector(t, slot, width)?.unwrap_or((0, 0));
        let b = self.bytes(at, n * width)?;
        Ok(b.chunks(width).map(le).collect())
    }

    /// Returns the vector of structs of two `i64`s at `slot`, flattened.
    pub fn pairs(&self, t: Tab, slot: u16) -> R<Vec<i64>> {
        let (at, n) = self.vector(t, slot, 16)?.unwrap_or((0, 0));
        Ok(self.bytes(at, n * 16)?.chunks(8).map(le).collect())
    }
}

fn le(b: &[u8]) -> i64 {
    let fill = if b.len() > 1 && b[b.len() - 1] & 0x80 != 0 {
        0xff
    } else {
        0
    };
    let mut x = [fill; 8];
    x[..b.len()].copy_from_slice(b);
    i64::from_le_bytes(x)
}
