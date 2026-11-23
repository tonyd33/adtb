use crate::mvcc::Xid;
use crate::runtime::Runtime;
use crate::storage::{HEADER_LEN, Lsn, PageError, Pager, Pid};

/// The format of the catalog's root page.
pub const FORMAT: u16 = 2;

/// The stored `changing` of a root page whose catalog is naming its page of generated counters;
/// no transaction has this id.
const NAMING_SEQUENCES: u64 = Xid::SENTINELS[1].0;

/// The stored `changing` of a root page whose catalog has rebuilds of indexes under way; no
/// transaction has this id.
const REINDEXING: u64 = Xid::SENTINELS[0].0;

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
    changing: Changing,
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
    /// Returns the root page of a catalog whose create has not committed.
    pub fn created(commit_log: Pid, next_heap: u32) -> Root {
        Root {
            commit_log,
            large: (0, Pid(0)),
            roots: (0, Pid(0)),
            next_heap,
            changing: Changing::None,
            sequences: Pid(0),
            history_large: (0, Pid(0)),
            record: (0, Pid(0)),
        }
    }

    pub fn changing(&self) -> Changing {
        self.changing
    }

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

/// A change of a catalog begun on its root page.
#[must_use = "a change dropped unended stays on the root page for the next open to free"]
#[derive(Debug)]
pub struct Begun {
    root: Pid,
    kind: Changing,
}

/// Why a change of a catalog did not begin.
#[derive(Debug, PartialEq, Eq)]
pub enum Unbegun {
    Page(PageError),
    /// The root page records this other change in progress.
    Kept(Changing),
}

/// Why a change of a catalog did not end.
#[derive(Debug, PartialEq, Eq)]
pub enum Unended {
    Page(PageError),
    /// The root page records this other change in progress.
    Moved(Changing),
}

/// Record `kind` in progress on the catalog root page `root`.
///
/// Fail with [`Unbegun::Kept`] and the change found, writing nothing, when the page records a
/// change in progress.
pub fn begin<R: Runtime>(pager: &Pager<R>, root: Pid, kind: Changing) -> Result<Begun, Unbegun> {
    assert_ne!(Changing::None, kind, "a change begun is in progress");
    let mut found = Changing::None;
    pager
        .update(&[root], |p| {
            found = Changing::decoded(u64_at(p[0], CHANGING));
            if found == Changing::None {
                p[0][CHANGING..SEQUENCES].copy_from_slice(&kind.encoded().to_le_bytes());
            }
        })
        .map_err(Unbegun::Page)?;
    match found {
        Changing::None => Ok(Begun { root, kind }),
        kept => Err(Unbegun::Kept(kept)),
    }
}

/// Record `kind` in progress as [`begin`] does. Returns `None`, writing nothing, when the page
/// holds a change in progress already: the caller runs under that change.
pub fn begin_or_join<R: Runtime>(
    pager: &Pager<R>,
    root: Pid,
    kind: Changing,
) -> Result<Option<Begun>, PageError> {
    match begin(pager, root, kind) {
        Ok(begun) => Ok(Some(begun)),
        Err(Unbegun::Kept(_)) => Ok(None),
        Err(Unbegun::Page(e)) => Err(e),
    }
}

/// End `marked`, or with none the change in progress on the catalog root page `root` that
/// [`begin_or_join`] joined. Returns the LSN of the write.
///
/// Fail as [`end`] does.
///
/// Precondition: every page no structure of the catalog names is freed.
pub fn end_freed<R: Runtime>(
    pager: &Pager<R>,
    root: Pid,
    marked: Option<Begun>,
) -> Result<Lsn, Unended> {
    match marked {
        Some(begun) => end(pager, begun, |_| {}),
        None => end_found(pager, root).map_err(Unended::Page),
    }
}

/// End the change `begun`, applying `also` to the root page in the same write. Returns the LSN
/// of the write.
///
/// Fail with [`Unended::Moved`] and the change found, writing nothing, when the page records
/// another change in progress than `begun`.
pub fn end<R: Runtime>(
    pager: &Pager<R>,
    begun: Begun,
    also: impl FnOnce(&mut Root),
) -> Result<Lsn, Unended> {
    let mut found = begun.kind;
    let lsn = pager
        .update(&[begun.root], |p| {
            let mut root = Root::read(p[0]);
            found = root.changing;
            if found == begun.kind {
                root.changing = Changing::None;
                also(&mut root);
                root.write(p[0]);
            }
        })
        .map_err(Unended::Page)?;
    if found == begun.kind {
        Ok(lsn)
    } else {
        Err(Unended::Moved(found))
    }
}

/// End the change a crashed run left on the catalog root page `root`, once an open has freed
/// every page it allocated. Returns the LSN of the write.
pub fn end_found<R: Runtime>(pager: &Pager<R>, root: Pid) -> Result<Lsn, PageError> {
    pager.update(&[root], |p| {
        p[0][CHANGING..SEQUENCES].copy_from_slice(&Changing::None.encoded().to_le_bytes());
    })
}

#[cfg(test)]
#[path = "root_tests.rs"]
mod tests;
