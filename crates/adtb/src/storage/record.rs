use super::{Lsn, Pid, Position};

/// Bytes written at `offset` of a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub offset: u32,
    pub bytes: Vec<u8>,
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
    Checkpoint {
        redo: Lsn,
        dirty: Vec<(Pid, Lsn)>,
    },
    /// A safe snapshot, as the transactions layer encodes it, and the commit position it covers.
    SafeSnapshot {
        at: Position,
        snapshot: Vec<u8>,
    },
}

const CHANGE: u8 = 1;
const IMAGE: u8 = 2;
const COMMIT: u8 = 3;
const ABORT: u8 = 4;
const XID_ALLOCATION: u8 = 5;
const CHECKPOINT: u8 = 6;
const SAFE_SNAPSHOT: u8 = 7;

/// The bytes before a record's body: its tag and the body's length.
const PREFIX: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if self.bytes.len() < n {
            return Err(Malformed);
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }
    fn u32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, Malformed> {
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
            Record::Checkpoint { redo, dirty } => {
                put_u64(out, redo.0);
                put_u32(out, len32(dirty.len()));
                for (pid, rec) in dirty {
                    put_u64(out, pid.0);
                    put_u64(out, rec.0);
                }
                CHECKPOINT
            }
            Record::SafeSnapshot { at, snapshot } => {
                put_u32(out, at.epoch);
                put_u64(out, at.lsn.0);
                out.extend_from_slice(snapshot);
                SAFE_SNAPSHOT
            }
        };
        let body = len32(out.len() - start - PREFIX);
        out[start] = tag;
        out[start + 1..start + PREFIX].copy_from_slice(&body.to_le_bytes());
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
                Record::Checkpoint { redo, dirty }
            }
            SAFE_SNAPSHOT => {
                let epoch = b.u32()?;
                let lsn = Lsn(b.u64()?);
                Record::SafeSnapshot {
                    at: Position { epoch, lsn },
                    snapshot: std::mem::take(&mut b.bytes).to_vec(),
                }
            }
            _ => return Err(Malformed),
        };
        if !b.bytes.is_empty() {
            return Err(Malformed);
        }
        Ok((record, PREFIX + len))
    }
}
