use crate::mvcc::Xid;
use crate::storage::{HEADER_LEN, Pid};

/// The format of the catalog's root page.
pub const FORMAT: u16 = 1;

/// The stored `changing` of a root page whose catalog is naming its page of generated counters;
/// no transaction has this id.
const NAMING_SEQUENCES: u64 = u64::MAX;

/// The stored `changing` of a root page whose catalog has rebuilds of indexes under way; no
/// transaction has this id.
const REINDEXING: u64 = u64::MAX - 1;

/// What a catalog is in the middle of changing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Changing {
    None,
    /// The schema change of this transaction is in progress.
    Schema(Xid),
    /// The write path's page of generated counters is allocated but not named.
    NamingSequences,
    /// Pages of index rebuilds may be allocated and named by nothing.
    Reindexing,
}

impl Changing {
    fn encoded(self) -> u64 {
        match self {
            Changing::None => 0,
            Changing::Schema(x) => x.0,
            Changing::NamingSequences => NAMING_SEQUENCES,
            Changing::Reindexing => REINDEXING,
        }
    }

    fn decoded(at: u64) -> Changing {
        match at {
            0 => Changing::None,
            NAMING_SEQUENCES => Changing::NamingSequences,
            REINDEXING => Changing::Reindexing,
            x => Changing::Schema(Xid(x)),
        }
    }
}

/// What the catalog's root page names: the structures found before any catalog row is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Root {
    pub commit_log: Pid,
    /// The large-value store of every heap, by heap id and root.
    pub large: (u32, Pid),
    /// The heap whose rows name the root of every other heap and index, by heap id and root.
    /// `Pid(0)` until the catalog's create commits.
    pub roots: (u32, Pid),
    /// No heap has this id or a higher one.
    pub next_heap: u32,
    pub changing: Changing,
    /// The write path's page of generated counters, `Pid(0)` until it has one.
    pub sequences: Pid,
    /// The large-value store of every history heap, by heap id and root.
    pub history_large: (u32, Pid),
    /// The record of the retention window, by the owner of its pages and its root.
    pub record: (u32, Pid),
}

const COMMIT_LOG: usize = HEADER_LEN;
const LARGE_ID: usize = COMMIT_LOG + 8;
const ROOTS_ID: usize = LARGE_ID + 4;
const LARGE_ROOT: usize = ROOTS_ID + 4;
const ROOTS_ROOT: usize = LARGE_ROOT + 8;
const NEXT_HEAP: usize = ROOTS_ROOT + 8;
const CHANGING: usize = NEXT_HEAP + 8;
const SEQUENCES: usize = CHANGING + 8;
const HISTORY_LARGE_ID: usize = SEQUENCES + 8;
const HISTORY_LARGE_ROOT: usize = HISTORY_LARGE_ID + 4;
const RECORD_OWNER: usize = HISTORY_LARGE_ROOT + 8;
const RECORD_ROOT: usize = RECORD_OWNER + 4;

fn u32_at(page: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(page[at..at + 4].try_into().unwrap())
}

fn u64_at(page: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(page[at..at + 8].try_into().unwrap())
}

impl Root {
    pub fn write(&self, page: &mut [u8]) {
        page[COMMIT_LOG..LARGE_ID].copy_from_slice(&self.commit_log.0.to_le_bytes());
        page[LARGE_ID..ROOTS_ID].copy_from_slice(&self.large.0.to_le_bytes());
        page[ROOTS_ID..LARGE_ROOT].copy_from_slice(&self.roots.0.to_le_bytes());
        page[LARGE_ROOT..ROOTS_ROOT].copy_from_slice(&self.large.1.0.to_le_bytes());
        page[ROOTS_ROOT..NEXT_HEAP].copy_from_slice(&self.roots.1.0.to_le_bytes());
        page[NEXT_HEAP..NEXT_HEAP + 4].copy_from_slice(&self.next_heap.to_le_bytes());
        page[CHANGING..SEQUENCES].copy_from_slice(&self.changing.encoded().to_le_bytes());
        page[SEQUENCES..HISTORY_LARGE_ID].copy_from_slice(&self.sequences.0.to_le_bytes());
        page[HISTORY_LARGE_ID..HISTORY_LARGE_ROOT]
            .copy_from_slice(&self.history_large.0.to_le_bytes());
        page[HISTORY_LARGE_ROOT..RECORD_OWNER]
            .copy_from_slice(&self.history_large.1.0.to_le_bytes());
        page[RECORD_OWNER..RECORD_ROOT].copy_from_slice(&self.record.0.to_le_bytes());
        page[RECORD_ROOT..RECORD_ROOT + 8].copy_from_slice(&self.record.1.0.to_le_bytes());
    }

    pub fn read(page: &[u8]) -> Root {
        Root {
            commit_log: Pid(u64_at(page, COMMIT_LOG)),
            large: (u32_at(page, LARGE_ID), Pid(u64_at(page, LARGE_ROOT))),
            roots: (u32_at(page, ROOTS_ID), Pid(u64_at(page, ROOTS_ROOT))),
            next_heap: u32_at(page, NEXT_HEAP),
            changing: Changing::decoded(u64_at(page, CHANGING)),
            sequences: Pid(u64_at(page, SEQUENCES)),
            history_large: (
                u32_at(page, HISTORY_LARGE_ID),
                Pid(u64_at(page, HISTORY_LARGE_ROOT)),
            ),
            record: (u32_at(page, RECORD_OWNER), Pid(u64_at(page, RECORD_ROOT))),
        }
    }
}
