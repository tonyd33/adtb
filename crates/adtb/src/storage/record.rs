use super::pager::Damage;
use super::{Lsn, Pid, Position};

/// Bytes written at `offset` of a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub offset: u32,
    pub bytes: Vec<u8>,
}

/// The root of the page LSN map, the least LSN it may have, and the tree's levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapRoot {
    pub root: Pid,
    pub lsn: Lsn,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    /// `prev` is the page's LSN before the change.
    Change {
        pid: Pid,
        prev: Lsn,
        edits: Vec<Edit>,
    },
    /// The whole page after the change that dirtied its clean frame; `prev` is its LSN before.
    Image {
        pid: Pid,
        prev: Lsn,
        page: Vec<u8>,
    },
    Commit {
        xid: u64,
        wall_nanos: u64,
    },
    Abort {
        xid: u64,
    },
    XidAllocation {
        next: u64,
    },
    /// `map` is `None` in a record that carries none.
    Checkpoint {
        redo: Lsn,
        dirty: Vec<(Pid, Lsn)>,
        map: Option<MapRoot>,
    },
    /// A new root of the page LSN map of `height` levels, in the batch that made it. The root's
    /// LSN is that of its record before this one in the batch.
    MapRoot {
        root: Pid,
        height: u32,
    },
    /// A safe snapshot, as the transactions layer encodes it, and the commit position it covers.
    SafeSnapshot {
        at: Position,
        snapshot: Vec<u8>,
    },
    /// Page `pid` was found damaged. No page record: it changes no page and raises no page's
    /// expected LSN.
    Damaged {
        pid: Pid,
        damage: Damage,
    },
    /// Page `pid` was freed, so the damage recorded of it no longer holds. No page record.
    Freed {
        pid: Pid,
    },
}

const CHANGE: u8 = 1;
const IMAGE: u8 = 2;
const COMMIT: u8 = 3;
const ABORT: u8 = 4;
const XID_ALLOCATION: u8 = 5;
const CHECKPOINT: u8 = 6;
const SAFE_SNAPSHOT: u8 = 7;
const MAP_ROOT: u8 = 8;
const DAMAGED: u8 = 9;
const FREED: u8 = 10;

const CHECKSUM: u8 = 1;
const STALE: u8 = 2;
const LOG_UNPROVEN: u8 = 3;
const SETTLED: u8 = 4;

/// Returns the kind of `damage` and its two LSNs, zero where it has none.
///
/// Panic on [`Damage::Unreadable`].
pub(crate) fn damage_words(damage: Damage) -> (u8, Lsn, Lsn) {
    match damage {
        Damage::Checksum => (CHECKSUM, Lsn(0), Lsn(0)),
        Damage::Stale { expected, found } => (STALE, expected, found),
        Damage::LogUnproven { at } => (LOG_UNPROVEN, at, Lsn(0)),
        Damage::Settled { expected, found } => (SETTLED, expected, found),
        Damage::Unreadable => panic!("an unreadable page is never recorded as damaged"),
    }
}

/// Returns the damage [`damage_words`] gave `kind` and the two LSNs, `None` for no such kind.
pub(crate) fn damage_of(kind: u8, a: Lsn, b: Lsn) -> Option<Damage> {
    Some(match kind {
        CHECKSUM => Damage::Checksum,
        STALE => Damage::Stale {
            expected: a,
            found: b,
        },
        LOG_UNPROVEN => Damage::LogUnproven { at: a },
        SETTLED => Damage::Settled {
            expected: a,
            found: b,
        },
        _ => return None,
    })
}

fn put_map(out: &mut Vec<u8>, m: &MapRoot) {
    put_u64(out, m.root.0);
    put_u64(out, m.lsn.0);
    put_u32(out, m.height);
}

fn map_root(b: &mut Reader<'_>) -> Result<MapRoot, Malformed> {
    Ok(MapRoot {
        root: Pid(b.u64()?),
        lsn: Lsn(b.u64()?),
        height: b.u32()?,
    })
}

/// The bytes before a record's body: its tag and the body's length.
const PREFIX: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if self.bytes.len() < n {
            return Err(Malformed);
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }
    pub(crate) fn u32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub(crate) fn u64(&mut self) -> Result<u64, Malformed> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn len32(n: usize) -> u32 {
    u32::try_from(n).expect("log record field exceeds 4 GiB")
}

impl Record {
    /// Returns the page the record changes, if it is a page record.
    pub fn page(&self) -> Option<(Pid, Lsn)> {
        match self {
            Record::Change { pid, prev, .. } | Record::Image { pid, prev, .. } => {
                Some((*pid, *prev))
            }
            _ => None,
        }
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(&[0; PREFIX]);
        let tag = match self {
            Record::Change { pid, prev, edits } => {
                put_u64(out, pid.0);
                put_u64(out, prev.0);
                put_u32(out, len32(edits.len()));
                for e in edits {
                    put_u32(out, e.offset);
                    put_u32(out, len32(e.bytes.len()));
                    out.extend_from_slice(&e.bytes);
                }
                CHANGE
            }
            Record::Image { pid, prev, page } => {
                put_u64(out, pid.0);
                put_u64(out, prev.0);
                out.extend_from_slice(page);
                IMAGE
            }
            Record::Commit { xid, wall_nanos } => {
                put_u64(out, *xid);
                put_u64(out, *wall_nanos);
                COMMIT
            }
            Record::Abort { xid } => {
                put_u64(out, *xid);
                ABORT
            }
            Record::XidAllocation { next } => {
                put_u64(out, *next);
                XID_ALLOCATION
            }
            Record::Checkpoint { redo, dirty, map } => {
                put_u64(out, redo.0);
                put_u32(out, len32(dirty.len()));
                for (pid, rec) in dirty {
                    put_u64(out, pid.0);
                    put_u64(out, rec.0);
                }
                if let Some(m) = map {
                    put_map(out, m);
                }
                CHECKPOINT
            }
            Record::MapRoot { root, height } => {
                put_u64(out, root.0);
                put_u32(out, *height);
                MAP_ROOT
            }
            Record::SafeSnapshot { at, snapshot } => {
                put_u64(out, at.epoch);
                put_u64(out, at.lsn.0);
                out.extend_from_slice(snapshot);
                SAFE_SNAPSHOT
            }
            Record::Damaged { pid, damage } => {
                let (kind, a, b) = damage_words(*damage);
                put_u64(out, pid.0);
                out.push(kind);
                put_u64(out, a.0);
                put_u64(out, b.0);
                DAMAGED
            }
            Record::Freed { pid } => {
                put_u64(out, pid.0);
                FREED
            }
        };
        let body = len32(out.len() - start - PREFIX);
        out[start] = tag;
        out[start + 1..start + PREFIX].copy_from_slice(&body.to_le_bytes());
    }

    /// Returns the page of the record at the start of `bytes`, if it is a page record, and the
    /// number of bytes it takes, decoding nothing else.
    pub fn page_of(bytes: &[u8]) -> Result<(Option<Pid>, usize), Malformed> {
        let mut r = Reader { bytes };
        let tag = r.take(1)?[0];
        let len = r.u32()? as usize;
        let mut b = Reader {
            bytes: r.take(len)?,
        };
        let page = matches!(tag, CHANGE | IMAGE)
            .then(|| b.u64().map(Pid))
            .transpose()?;
        Ok((page, PREFIX + len))
    }

    /// Returns the record at the start of `bytes` and the number of bytes it takes.
    pub fn decode(bytes: &[u8]) -> Result<(Record, usize), Malformed> {
        let mut r = Reader { bytes };
        let tag = r.take(1)?[0];
        let len = r.u32()? as usize;
        let mut b = Reader {
            bytes: r.take(len)?,
        };
        let record = match tag {
            CHANGE => {
                let pid = Pid(b.u64()?);
                let prev = Lsn(b.u64()?);
                let n = b.u32()?;
                let mut edits = Vec::new();
                for _ in 0..n {
                    let offset = b.u32()?;
                    let len = b.u32()? as usize;
                    edits.push(Edit {
                        offset,
                        bytes: b.take(len)?.to_vec(),
                    });
                }
                Record::Change { pid, prev, edits }
            }
            IMAGE => {
                let pid = Pid(b.u64()?);
                let prev = Lsn(b.u64()?);
                let page = std::mem::take(&mut b.bytes).to_vec();
                Record::Image { pid, prev, page }
            }
            COMMIT => Record::Commit {
                xid: b.u64()?,
                wall_nanos: b.u64()?,
            },
            ABORT => Record::Abort { xid: b.u64()? },
            XID_ALLOCATION => Record::XidAllocation { next: b.u64()? },
            CHECKPOINT => {
                let redo = Lsn(b.u64()?);
                let n = b.u32()?;
                let mut dirty = Vec::new();
                for _ in 0..n {
                    dirty.push((Pid(b.u64()?), Lsn(b.u64()?)));
                }
                let map = if b.is_empty() {
                    None
                } else {
                    Some(map_root(&mut b)?)
                };
                Record::Checkpoint { redo, dirty, map }
            }
            MAP_ROOT => Record::MapRoot {
                root: Pid(b.u64()?),
                height: b.u32()?,
            },
            SAFE_SNAPSHOT => {
                let epoch = b.u64()?;
                let lsn = Lsn(b.u64()?);
                Record::SafeSnapshot {
                    at: Position { epoch, lsn },
                    snapshot: std::mem::take(&mut b.bytes).to_vec(),
                }
            }
            DAMAGED => {
                let pid = Pid(b.u64()?);
                let kind = b.take(1)?[0];
                let (a, c) = (Lsn(b.u64()?), Lsn(b.u64()?));
                Record::Damaged {
                    pid,
                    damage: damage_of(kind, a, c).ok_or(Malformed)?,
                }
            }
            FREED => Record::Freed { pid: Pid(b.u64()?) },
            _ => return Err(Malformed),
        };
        if !b.bytes.is_empty() {
            return Err(Malformed);
        }
        Ok((record, PREFIX + len))
    }
}
