//! The damage evidence a file holds, read from its bytes by the search's own reader: the damage,
//! freed and image records of its retained log, the entries of its damage list and the marks of
//! its marks page, each of those two pages as recovery redoes it from the log.

use std::collections::{BTreeMap, BTreeSet};

use adtb::runtime::OpKind;
use adtb::testing::{PageKind, Pid, Quarantine, page};
use adtb_harness::search::Files;
use adtb_sim::{Planned, Submitted};

use super::world::{self, FILE, PAGE};

const BLOCK_HEADER: usize = 56;
const BLOCK_CHECKSUM: usize = 32;
const HEADER_COPY: usize = 4096;

const CHANGE: u8 = 1;
const IMAGE: u8 = 2;
const CHECKPOINT: u8 = 6;
const DAMAGED: u8 = 9;
const FREED: u8 = 10;

/// The kind of a settled status page's damage record, whose entry an image leaves in the list.
pub const SETTLED: u8 = 4;

/// The kind of a list entry naming a stale log block, with no page.
const LOG_STALE: u8 = 16;
const LIST_FULL: u32 = 1;
const DATABASE: u32 = 1;
const HEAP: u32 = 2;
const INDEX: u32 = 3;

/// A record of the log the search reads, with what of it the judges use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Logged {
    Damaged {
        pid: Pid,
        kind: u8,
    },
    Freed {
        pid: Pid,
    },
    Image {
        pid: Pid,
        page: Vec<u8>,
    },
    Change {
        pid: Pid,
        prev: u64,
        edits: Vec<(usize, Vec<u8>)>,
    },
    Checkpoint {
        redo: u64,
    },
    Other,
}

/// What the file header that verifies with the higher sequence number names.
#[derive(Clone, Copy, Debug)]
struct Header {
    block: usize,
    segment: u64,
    checkpoint: u64,
    marks: Pid,
    list: Pid,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Returns the page checksum of `bytes` with the four bytes at `field` read as zero, seeded with
/// `seed`.
fn sum_skipping(seed: Option<u64>, bytes: &[u8], field: usize) -> u32 {
    let mut c = page::Crc32c::new();
    if let Some(seed) = seed {
        c = c.update(&seed.to_le_bytes());
    }
    c.update(&bytes[..field])
        .update(&[0; 4])
        .update(&bytes[field + 4..])
        .finish()
}

fn header(data: &[u8]) -> Option<Header> {
    let copy = |c: usize| {
        let at = world::DATA.range(c as u64).start;
        let b = data.get(at..at + HEADER_COPY)?;
        let valid = &b[..8] == b"adtbfile" && u32_at(b, 8) == sum_skipping(Some(c as u64), b, 8);
        valid.then(|| {
            let header = Header {
                block: u32_at(b, 44) as usize,
                segment: u64_at(b, 48),
                checkpoint: u64_at(b, 64),
                marks: Pid(u64_at(b, 80)),
                list: Pid(u64_at(b, 1024)),
            };
            (u64_at(b, 16), header)
        })
    };
    [copy(0), copy(1)]
        .into_iter()
        .flatten()
        .max_by_key(|(seq, _)| *seq)
        .map(|(_, h)| h)
}

/// Returns the records of the batch whose records are `data`, its first block at `start`, each
/// with its LSN. Stops at a record that does not decode.
fn records(start: u64, data: &[u8], block: usize) -> Vec<(u64, Logged)> {
    let payload = block - BLOCK_HEADER;
    let lsn = |k: usize| start + ((k / payload) * block + BLOCK_HEADER + k % payload) as u64;
    let mut out = Vec::new();
    let mut k = 0;
    while k + 5 <= data.len() {
        let tag = data[k];
        let len = u32_at(data, k + 1) as usize;
        let Some(body) = data.get(k + 5..k + 5 + len) else {
            break;
        };
        let pid = || Pid(u64_at(body, 0));
        let record = match tag {
            DAMAGED if len >= 9 => Logged::Damaged {
                pid: pid(),
                kind: body[8],
            },
            FREED if len >= 8 => Logged::Freed { pid: pid() },
            IMAGE if len >= 16 => Logged::Image {
                pid: pid(),
                page: body[16..].to_vec(),
            },
            CHANGE if len >= 20 => Logged::Change {
                pid: pid(),
                prev: u64_at(body, 8),
                edits: edits(&body[20..], u32_at(body, 16)),
            },
            CHECKPOINT if len >= 8 => Logged::Checkpoint {
                redo: u64_at(body, 0),
            },
            _ => Logged::Other,
        };
        out.push((lsn(k), record));
        k += 5 + len;
    }
    out
}

fn edits(mut body: &[u8], n: u32) -> Vec<(usize, Vec<u8>)> {
    let mut out = Vec::new();
    for _ in 0..n {
        let Some(head) = body.get(..8) else {
            break;
        };
        let (offset, len) = (u32_at(head, 0) as usize, u32_at(head, 4) as usize);
        let Some(bytes) = body.get(8..8 + len) else {
            break;
        };
        out.push((offset, bytes.to_vec()));
        body = &body[8 + len..];
    }
    out
}

/// One block that verifies at `at`: its batch's sequence number, block count, index and records.
struct Block<'a> {
    seq: u64,
    count: u32,
    index: u32,
    data: &'a [u8],
}

fn block(at: u64, b: &[u8]) -> Option<Block<'_>> {
    if b.len() < BLOCK_HEADER || u32_at(b, BLOCK_CHECKSUM) != sum_skipping(None, b, BLOCK_CHECKSUM)
    {
        return None;
    }
    let (count, index, used) = (u32_at(b, 16), u32_at(b, 20), usize::from(u16_at(b, 36)));
    let fits = used <= b.len() - BLOCK_HEADER;
    (u64_at(b, 0) == at && count > 0 && index < count && fits).then(|| Block {
        seq: u64_at(b, 8),
        count,
        index,
        data: &b[BLOCK_HEADER..BLOCK_HEADER + used],
    })
}

/// Returns the batches of `bytes`, whole blocks from LSN `at`, each as its first LSN and the bytes
/// of its records, from the first block of index 0 up to the first break in the run. A batch must
/// follow the one before it by sequence number when `chained`.
fn batches(at: u64, bytes: &[u8], block_size: usize, chained: bool) -> Vec<(u64, Vec<u8>)> {
    let blocks: Vec<&[u8]> = bytes.chunks_exact(block_size).collect();
    let at_block = |i: usize| block(at + (i * block_size) as u64, blocks[i]);
    let mut out = Vec::new();
    let mut prev: Option<u64> = None;
    let mut i = 0;
    while i < blocks.len() {
        let Some(first) = at_block(i).filter(|b| b.index == 0) else {
            if out.is_empty() {
                i += 1;
                continue;
            }
            break;
        };
        if chained && prev.is_some_and(|p| first.seq != p + 1) {
            break;
        }
        let mut data = first.data.to_vec();
        let whole = (1..first.count as usize).all(|n| {
            let next = (i + n < blocks.len()).then(|| at_block(i + n)).flatten();
            next.filter(|b| b.seq == first.seq && b.index as usize == n)
                .map(|b| data.extend_from_slice(b.data))
                .is_some()
        });
        if !whole {
            break;
        }
        out.push((at + (i * block_size) as u64, data));
        prev = Some(first.seq);
        i += first.count as usize;
    }
    out
}

/// Returns each record of the log `files` retain, by its LSN: the accepted batches from the
/// first segment present, up to the first batch recovery would not accept.
fn retained(files: &Files, h: &Header) -> Vec<(u64, Logged)> {
    let dir = format!("{FILE}-log/");
    let segments: BTreeMap<u64, &Vec<u8>> = files
        .iter()
        .filter_map(|(path, bytes)| {
            let base = u64::from_str_radix(path.strip_prefix(&dir)?, 16).ok()?;
            Some((base, bytes))
        })
        .collect();
    let mut joined = Vec::new();
    let Some(&first) = segments.keys().next() else {
        return Vec::new();
    };
    for (n, (base, bytes)) in segments.iter().enumerate() {
        if *base != first + n as u64 * h.segment {
            break;
        }
        joined.extend_from_slice(bytes);
        joined.resize((n + 1) * h.segment as usize, 0);
    }
    batches(first, &joined, h.block, true)
        .into_iter()
        .flat_map(|(start, data)| records(start, &data, h.block))
        .collect()
}

/// Returns page `pid` of `files` with the retained log's images and changes of it from its LSN
/// redone, up to a change whose previous LSN is not the page's.
fn redone(files: &Files, log: &[(u64, Logged)], pid: Pid) -> Vec<u8> {
    let mut bytes = world::page(files, pid);
    let mut lsn = page::lsn(&bytes).0;
    for (at, record) in log {
        match record {
            Logged::Image { pid: p, page } if *p == pid && *at > lsn && page.len() == PAGE => {
                bytes.clone_from(page);
                lsn = *at;
            }
            Logged::Change {
                pid: p,
                prev,
                edits,
            } if *p == pid && *at > lsn => {
                if *prev != lsn {
                    break;
                }
                for (offset, change) in edits {
                    if let Some(span) = bytes.get_mut(*offset..*offset + change.len()) {
                        span.copy_from_slice(change);
                    }
                }
                lsn = *at;
            }
            _ => {}
        }
    }
    bytes
}

/// The damage list's pages, `None` for a page that is no list page.
fn list(bytes: &[u8]) -> Option<(BTreeSet<Pid>, bool)> {
    if page::kind(bytes) != PageKind::DamageList as u16 {
        return None;
    }
    let (count, entries) = (page::HEADER_LEN, page::HEADER_LEN + 8);
    let n = u32_at(bytes, count) as usize;
    let full = u32_at(bytes, count + 4) & LIST_FULL != 0;
    let mut pages = BTreeSet::new();
    for i in 0..n {
        let head = u64_at(
            bytes,
            entries.checked_add(i * 24).filter(|a| a + 24 <= PAGE)?,
        );
        if (head >> 56) as u8 != LOG_STALE {
            pages.insert(Pid(head & ((1 << 56) - 1)));
        }
    }
    Some((pages, full))
}

/// The marks page's marks, `None` for a page that is no marks page.
fn marks(bytes: &[u8]) -> Option<BTreeSet<Quarantine>> {
    if page::kind(bytes) != PageKind::Quarantine as u16 {
        return None;
    }
    let (count, entries) = (page::HEADER_LEN, page::HEADER_LEN + 8);
    let n = u64_at(bytes, count) as usize;
    (0..n)
        .map(|i| {
            let at = entries.checked_add(i * 8).filter(|a| a + 8 <= PAGE)?;
            let id = u32_at(bytes, at + 4);
            match u32_at(bytes, at) {
                DATABASE => Some(Quarantine::Database),
                HEAP => Some(Quarantine::Heap(id)),
                INDEX => Some(Quarantine::Index(id)),
                _ => None,
            }
        })
        .collect()
}

/// A damage record: its page, its LSN and the kind of its damage.
pub type Record = (Pid, u64, u8);

/// A free or an image of a page the log held, by page and LSN, an image marked `true`.
pub type Ended = (Pid, u64, bool);

/// What the file held at one instant: after a process of the run ended, or after the consumers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    /// The damage records of the retained log.
    pub records: BTreeSet<Record>,
    /// The redo LSN of the checkpoint the header names, `None` when it is not in the log.
    pub redo: Option<u64>,
    /// The pages the damage list names, `None` when the list page is no list page.
    pub list: Option<BTreeSet<Pid>>,
    /// Whether the list counts the whole file as damaged.
    pub full: bool,
    /// The marks of the marks page, `None` when it is no marks page.
    pub marks: Option<BTreeSet<Quarantine>>,
    /// The frees and images of the retained log.
    pub ended: BTreeSet<Ended>,
}

/// Returns what `files` hold, `None` when no file header verifies.
pub fn evidence(files: &Files) -> Option<Evidence> {
    let h = header(world::DATA.bytes(files))?;
    let log = retained(files, &h);
    let mut out = Evidence {
        redo: log.iter().find_map(|(at, r)| match r {
            Logged::Checkpoint { redo } if *at == h.checkpoint => Some(*redo),
            _ => None,
        }),
        ..Evidence::default()
    };
    for (at, record) in &log {
        match record {
            Logged::Damaged { pid, kind } => {
                out.records.insert((*pid, *at, *kind));
            }
            Logged::Freed { pid } => {
                out.ended.insert((*pid, *at, false));
            }
            Logged::Image { pid, .. } => {
                out.ended.insert((*pid, *at, true));
            }
            _ => {}
        }
    }
    let list_page = redone(files, &log, h.list);
    if let Some((pages, full)) = list(&list_page) {
        out.list = Some(pages);
        out.full = full;
    }
    out.marks = marks(&redone(files, &log, h.marks));
    Some(out)
}

/// What one write to the log carried, by the search's reader of its blocks.
#[derive(Clone, Debug)]
pub struct LogWrite {
    pub path: String,
    pub offset: u64,
    /// The LSN it starts at.
    pub lsn: u64,
    /// Its place among the writes to the log and the write-outs, in submission order.
    pub submitted: usize,
    /// The write-outs submitted before it.
    pub outs: u64,
    /// Whether a later read of the log covers its first block.
    pub read_later: bool,
    /// The pages of the write-outs submitted before it.
    pub after: usize,
    /// The frees and images it logged.
    pub ended: Vec<Ended>,
    /// Whether a batch of it holds a checkpoint record and a change of the damage list's page.
    pub folds: bool,
    /// Whether a plan acted at it.
    pub planned: bool,
    /// Whether a plan lost it or failed it.
    pub dropped: bool,
}

/// Returns what each write to the log of `writes` carried, by the header of `files`. `writes`
/// holds the writes to the log and the write-outs of data pages in submission order, and
/// `pages` gives the number of pages the first `n` write-outs wrote.
pub fn log_writes(
    files: &Files,
    writes: &[Submitted],
    pages: impl Fn(usize) -> usize,
) -> Vec<LogWrite> {
    let Some(h) = header(world::DATA.bytes(files)) else {
        return Vec::new();
    };
    let (block, list) = (h.block, h.list);
    let mut out = Vec::new();
    let dir = format!("{FILE}-log/");
    let mut outs = 0;
    for (submitted, w) in writes.iter().enumerate() {
        let (Some(path), Some(offset), Some(bytes)) = (w.path(), w.offset(), w.bytes()) else {
            continue;
        };
        if !path.starts_with(&dir) {
            outs += 1;
            continue;
        }
        let Some(base) = path
            .rsplit('/')
            .next()
            .and_then(|n| u64::from_str_radix(n, 16).ok())
        else {
            continue;
        };
        let mut ended = Vec::new();
        let mut folds = false;
        for (start, data) in batches(base + offset, bytes, block, false) {
            let rs = records(start, &data, block);
            let checkpoint = rs
                .iter()
                .any(|(_, r)| matches!(r, Logged::Checkpoint { .. }));
            let changes_list = rs.iter().any(|(_, r)| match r {
                Logged::Change { pid, .. } | Logged::Image { pid, .. } => *pid == list,
                _ => false,
            });
            folds |= checkpoint && changes_list;
            for (at, r) in rs {
                match r {
                    Logged::Freed { pid } => ended.push((pid, at, false)),
                    Logged::Image { pid, .. } => ended.push((pid, at, true)),
                    _ => {}
                }
            }
        }
        out.push(LogWrite {
            path: path.to_owned(),
            offset,
            lsn: base + offset,
            submitted,
            outs: outs as u64,
            read_later: writes[submitted + 1..].iter().any(|r| {
                r.kind() == OpKind::Read
                    && r.path() == Some(path)
                    && r.offset()
                        .zip(r.length())
                        .is_some_and(|(at, len)| (at..at + len).contains(&offset))
            }),
            after: pages(outs),
            ended,
            folds,
            planned: w.planned().is_some(),
            dropped: matches!(w.planned(), Some(Planned::Lose | Planned::Fail(_))),
        });
    }
    out
}

/// Returns the damage list's page the header of `files` names.
pub fn list_page(files: &Files) -> Option<Pid> {
    header(world::DATA.bytes(files)).map(|h| h.list)
}

/// Returns the LSN of the last change or image of page `pid` the log of `files` retains.
pub fn last_logged(files: &Files, pid: Pid) -> Option<u64> {
    let h = header(world::DATA.bytes(files))?;
    retained(files, &h)
        .into_iter()
        .filter(|(_, r)| {
            matches!(r, Logged::Change { pid: p, .. } | Logged::Image { pid: p, .. } if *p == pid)
        })
        .map(|(at, _)| at)
        .max()
}

/// Returns the LSN of the checkpoint the file header names.
pub fn checkpoint(files: &Files) -> Option<u64> {
    header(world::DATA.bytes(files)).map(|h| h.checkpoint)
}
