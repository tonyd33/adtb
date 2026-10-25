use std::collections::BTreeMap;

use super::Lsn;
use super::crc::crc32c_skipping;
use super::record::{Malformed, Record};

pub const BLOCK_HEADER: usize = 36;

const CHECKSUM: usize = 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockHeader {
    pub lsn: Lsn,
    pub seq: u64,
    pub count: u32,
    pub index: u32,
    pub epoch: u32,
    pub used: u16,
}

fn payload(block: usize) -> usize {
    block - BLOCK_HEADER
}

/// Returns the LSN of byte `k` of the records of a batch whose first block is at `start`.
pub fn data_lsn(start: Lsn, k: usize, block: usize) -> Lsn {
    let p = payload(block);
    Lsn(start.0 + ((k / p) * block + BLOCK_HEADER + k % p) as u64)
}

/// Returns the blocks of a batch holding `data`, the first at `start`, padded after the last record.
///
/// Panics if `data` is empty.
pub fn frame(start: Lsn, seq: u64, epoch: u32, data: &[u8], block: usize) -> Vec<u8> {
    assert!(!data.is_empty(), "a batch holds at least one record");
    let chunks: Vec<&[u8]> = data.chunks(payload(block)).collect();
    let count = u32::try_from(chunks.len()).expect("batch exceeds u32 blocks");
    let mut out = vec![0; chunks.len() * block];
    for (i, (chunk, b)) in chunks.iter().zip(out.chunks_mut(block)).enumerate() {
        let lsn = start.0 + (i * block) as u64;
        b[0..8].copy_from_slice(&lsn.to_le_bytes());
        b[8..16].copy_from_slice(&seq.to_le_bytes());
        b[16..20].copy_from_slice(&count.to_le_bytes());
        b[20..24].copy_from_slice(&(i as u32).to_le_bytes());
        b[24..28].copy_from_slice(&epoch.to_le_bytes());
        b[32..34].copy_from_slice(&(chunk.len() as u16).to_le_bytes());
        b[BLOCK_HEADER..BLOCK_HEADER + chunk.len()].copy_from_slice(chunk);
        let sum = crc32c_skipping(None, b, CHECKSUM);
        b[CHECKSUM..CHECKSUM + 4].copy_from_slice(&sum.to_le_bytes());
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum BlockState<'a> {
    Zero,
    Valid(BlockHeader, &'a [u8]),
    /// Neither zero nor a block that verifies at this position.
    Invalid,
}

pub fn read_block(at: Lsn, b: &[u8]) -> BlockState<'_> {
    let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    if u32_at(CHECKSUM) != crc32c_skipping(None, b, CHECKSUM) {
        return if b.iter().all(|&x| x == 0) {
            BlockState::Zero
        } else {
            BlockState::Invalid
        };
    }
    let header = BlockHeader {
        lsn: Lsn(u64_at(0)),
        seq: u64_at(8),
        count: u32_at(16),
        index: u32_at(20),
        epoch: u32_at(24),
        used: u16::from_le_bytes([b[32], b[33]]),
    };
    let fits = usize::from(header.used) <= payload(b.len());
    if header.lsn != at || header.count == 0 || header.index >= header.count || !fits {
        return BlockState::Invalid;
    }
    BlockState::Valid(
        header,
        &b[BLOCK_HEADER..BLOCK_HEADER + usize::from(header.used)],
    )
}

/// The log as read from its segment files.
#[derive(Debug)]
pub struct LogImage {
    pub block: usize,
    pub segment: u64,
    /// Segment contents by first LSN, zero-filled to the segment size; `None` where a read failed.
    pub segments: BTreeMap<u64, Option<Vec<u8>>>,
}

#[derive(Debug, PartialEq, Eq)]
enum Slot<'a> {
    Missing,
    Unreadable,
    Bytes(&'a [u8]),
}

impl LogImage {
    fn slot(&self, at: Lsn) -> Slot<'_> {
        let base = at.0 - at.0 % self.segment;
        match self.segments.get(&base) {
            None => Slot::Missing,
            Some(None) => Slot::Unreadable,
            Some(Some(bytes)) => {
                let off = (at.0 - base) as usize;
                Slot::Bytes(&bytes[off..off + self.block])
            }
        }
    }

    /// Returns the first LSN of every block at or after `from`, in every segment present.
    fn blocks_from(&self, from: Lsn) -> impl Iterator<Item = Lsn> + '_ {
        let block = self.block as u64;
        self.segments.keys().flat_map(move |&base| {
            let first = from.0.max(base).next_multiple_of(block);
            (first..base + self.segment).step_by(self.block).map(Lsn)
        })
    }

    pub fn first(&self) -> Option<Lsn> {
        self.segments.keys().next().map(|&b| Lsn(b))
    }

    /// Returns the first LSN of the batch holding the block at `at`, if that block verifies.
    pub fn batch_start(&self, at: Lsn) -> Option<Lsn> {
        let block = Lsn(at.0 - at.0 % self.block as u64);
        match self.slot(block) {
            Slot::Bytes(b) => match read_block(block, b) {
                BlockState::Valid(h, _) => {
                    Some(Lsn(block.0 - u64::from(h.index) * self.block as u64))
                }
                _ => None,
            },
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Batch {
    pub start: Lsn,
    pub seq: u64,
    pub epoch: u32,
    pub blocks: u32,
    pub data: Vec<u8>,
}

impl Batch {
    pub fn end(&self, block: usize) -> Lsn {
        Lsn(self.start.0 + u64::from(self.blocks) * block as u64)
    }

    /// Returns each record of the batch with its LSN.
    pub fn records(&self, block: usize) -> Result<Vec<(Lsn, Record)>, Malformed> {
        let mut out = Vec::new();
        let mut k = 0;
        while k < self.data.len() {
            let (record, n) = Record::decode(&self.data[k..])?;
            out.push((data_lsn(self.start, k, block), record));
            k += n;
        }
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogProblem {
    /// A block neither zero nor valid where it lies.
    Damaged,
    /// A valid block of an older batch past the end of the log.
    Stale,
    /// The log continues past a batch that is not accepted.
    Gap,
    Unreadable,
    /// An accepted batch whose records do not decode.
    Malformed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogFinding {
    pub lsn: Lsn,
    pub problem: LogProblem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Stop at the first batch not accepted; a gap after it is fatal.
    Recovery,
    /// Report a gap and continue at the next batch found after it.
    Verify,
}

#[derive(Debug)]
pub struct Scan {
    pub batches: Vec<Batch>,
    /// The LSN after the last batch accepted before the first gap.
    pub end: Lsn,
    pub last_seq: Option<u64>,
    pub findings: Vec<LogFinding>,
    /// The first finding that stops recovery.
    pub fatal: Option<LogFinding>,
    /// The index into `batches` of each batch that follows a gap.
    pub resumed: Vec<usize>,
}

enum Refusal {
    Unreadable(Lsn),
    NotAccepted,
}

fn accept(img: &LogImage, pos: Lsn, prev: Option<u64>) -> Result<Batch, Refusal> {
    let block = img.block as u64;
    let first = match img.slot(pos) {
        Slot::Bytes(b) => match read_block(pos, b) {
            BlockState::Valid(h, data) => (h, data),
            _ => return Err(Refusal::NotAccepted),
        },
        Slot::Missing => return Err(Refusal::NotAccepted),
        Slot::Unreadable => return Err(Refusal::Unreadable(pos)),
    };
    let (h0, d0) = first;
    if h0.index != 0 || prev.is_some_and(|p| h0.seq != p + 1) {
        return Err(Refusal::NotAccepted);
    }
    let full = payload(img.block);
    let mut data = d0.to_vec();
    let mut last_used = usize::from(h0.used);
    for i in 1..h0.count {
        let at = Lsn(pos.0 + u64::from(i) * block);
        let (h, d) = match img.slot(at) {
            Slot::Bytes(b) => match read_block(at, b) {
                BlockState::Valid(h, d) => (h, d),
                _ => return Err(Refusal::NotAccepted),
            },
            Slot::Missing => return Err(Refusal::NotAccepted),
            Slot::Unreadable => return Err(Refusal::Unreadable(at)),
        };
        let agrees = h.seq == h0.seq && h.count == h0.count && h.epoch == h0.epoch && h.index == i;
        if !agrees || last_used != full {
            return Err(Refusal::NotAccepted);
        }
        data.extend_from_slice(d);
        last_used = usize::from(h.used);
    }
    if last_used == 0 {
        return Err(Refusal::NotAccepted);
    }
    Ok(Batch {
        start: pos,
        seq: h0.seq,
        epoch: h0.epoch,
        blocks: h0.count,
        data,
    })
}

/// Read the batches of `img` from the batch starting at `from`, and classify what lies past them.
pub fn scan(img: &LogImage, from: Lsn, mode: Mode) -> Scan {
    let mut out = Scan {
        batches: Vec::new(),
        end: from,
        last_seq: None,
        findings: Vec::new(),
        fatal: None,
        resumed: Vec::new(),
    };
    let mut pos = from;
    let mut prev = None;
    let mut gapped = false;
    loop {
        loop {
            match accept(img, pos, prev) {
                Ok(b) => {
                    pos = b.end(img.block);
                    prev = Some(b.seq);
                    if b.records(img.block).is_err() {
                        out.findings.push(LogFinding {
                            lsn: b.start,
                            problem: LogProblem::Malformed,
                        });
                    }
                    out.batches.push(b);
                }
                Err(Refusal::Unreadable(at)) => {
                    out.findings.push(LogFinding {
                        lsn: at,
                        problem: LogProblem::Unreadable,
                    });
                    break;
                }
                Err(Refusal::NotAccepted) => break,
            }
        }
        if !gapped {
            out.end = pos;
            out.last_seq = prev;
        }
        let mut resume = None;
        for at in img.blocks_from(pos) {
            match img.slot(at) {
                Slot::Missing => {}
                Slot::Unreadable => out.findings.push(LogFinding {
                    lsn: Lsn(at.0 - at.0 % img.segment),
                    problem: LogProblem::Unreadable,
                }),
                Slot::Bytes(b) => match read_block(at, b) {
                    BlockState::Zero => {}
                    BlockState::Invalid => out.findings.push(LogFinding {
                        lsn: at,
                        problem: LogProblem::Damaged,
                    }),
                    BlockState::Valid(h, _) => match prev {
                        Some(p) if h.seq <= p => out.findings.push(LogFinding {
                            lsn: at,
                            problem: LogProblem::Stale,
                        }),
                        Some(p) if h.seq == p + 1 => {}
                        _ => {
                            if !out
                                .findings
                                .iter()
                                .any(|f| f.problem == LogProblem::Gap && f.lsn == pos)
                            {
                                out.findings.push(LogFinding {
                                    lsn: pos,
                                    problem: LogProblem::Gap,
                                });
                            }
                            if mode == Mode::Verify && h.index == 0 && at > pos {
                                resume = Some(at);
                                break;
                            }
                        }
                    },
                },
            }
        }
        match resume {
            Some(at) => {
                gapped = true;
                pos = at;
                prev = None;
                out.resumed.push(out.batches.len());
            }
            None => break,
        }
    }
    out.findings.sort();
    out.findings.dedup();
    out.fatal = match mode {
        Mode::Recovery => out
            .findings
            .iter()
            .find(|f| {
                matches!(
                    f.problem,
                    LogProblem::Gap | LogProblem::Unreadable | LogProblem::Malformed
                )
            })
            .copied(),
        Mode::Verify => None,
    };
    out
}
