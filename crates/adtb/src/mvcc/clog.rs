use std::collections::BTreeMap;

use crate::runtime::Runtime;
use crate::storage::{
    FIRST_EDITABLE, HEADER_LEN, Lsn, PageError, PageKind, Pager, Pid, flags, header_edit, kind,
};

use super::{Status, Xid};

/// The format version of commit log pages.
pub const PAGE_FORMAT: u16 = 1;

/// The page header flags of a root and a directory page; a status page has flags 0.
const ROOT_PAGE: u16 = 1;
const DIRECTORY_PAGE: u16 = 2;

const ROOT_RESERVED: usize = HEADER_LEN;
const ROOT_RESOLVED: usize = HEADER_LEN + 8;
const ROOT_BOUND: usize = HEADER_LEN + 16;
const ROOT_FIRST_DIRECTORY: usize = HEADER_LEN + 24;

const DIRECTORY_NEXT: usize = HEADER_LEN;
const DIRECTORY_COUNT: usize = HEADER_LEN + 8;
const DIRECTORY_ENTRIES: usize = HEADER_LEN + 16;
const ENTRY: usize = 16;

fn u64_at(page: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(page[at..at + 8].try_into().unwrap())
}

fn put_u64(page: &mut [u8], at: usize, v: u64) {
    page[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn set_role(page: &mut [u8], role: u16) {
    page[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&header_edit(
        PAGE_FORMAT,
        PageKind::CommitLog,
        0,
        role,
    ));
}

fn check_role(pid: Pid, page: &[u8], role: u16) {
    assert!(
        kind(page) == PageKind::CommitLog as u16 && flags(page) == role,
        "{pid:?} is not a commit log page of role {role}"
    );
}

/// Returns the number of transaction ids one status page of a `page_size` page holds.
pub(crate) fn per_page(page_size: usize) -> u64 {
    ((page_size - HEADER_LEN) * 4) as u64
}

/// Returns the number of status pages one directory page of a `page_size` page names.
pub(crate) fn directory_capacity(page_size: usize) -> usize {
    (page_size - DIRECTORY_ENTRIES) / ENTRY
}

/// Returns the number of the status page holding `x`, and `x`'s index within it.
pub(crate) fn locate(x: Xid, per_page: u64) -> (u64, usize) {
    (x.0 / per_page, (x.0 % per_page) as usize)
}

pub(crate) fn status_at(page: &[u8], index: usize) -> Status {
    let byte = page[HEADER_LEN + index / 4];
    Status::from_bits(byte >> (index % 4 * 2) & 0b11)
}

/// Record `status` for the id at `index`.
///
/// Panics if the id has resolved already, or if `status` is in progress.
pub(crate) fn set_status(page: &mut [u8], index: usize, status: Status) {
    assert_ne!(
        Status::InProgress,
        status,
        "a status is set only once resolved"
    );
    let old = status_at(page, index);
    assert_eq!(
        Status::InProgress,
        old,
        "status {index} resolved twice, from {old:?} to {status:?}"
    );
    page[HEADER_LEN + index / 4] |= (status as u8) << (index % 4 * 2);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Root {
    /// Every id below it is reserved.
    pub(crate) reserved: Xid,
    /// Every id below it had resolved when the root was last written.
    pub(crate) resolved: Xid,
    /// Every id below it counts as committed; its status pages may be gone.
    pub(crate) bound: Xid,
    pub(crate) first_directory: Pid,
}

impl Root {
    pub(crate) fn read(page: &[u8]) -> Root {
        Root {
            reserved: Xid(u64_at(page, ROOT_RESERVED)),
            resolved: Xid(u64_at(page, ROOT_RESOLVED)),
            bound: Xid(u64_at(page, ROOT_BOUND)),
            first_directory: Pid(u64_at(page, ROOT_FIRST_DIRECTORY)),
        }
    }

    pub(crate) fn write(&self, page: &mut [u8]) {
        set_role(page, ROOT_PAGE);
        put_u64(page, ROOT_RESERVED, self.reserved.0);
        put_u64(page, ROOT_RESOLVED, self.resolved.0);
        put_u64(page, ROOT_BOUND, self.bound.0);
        put_u64(page, ROOT_FIRST_DIRECTORY, self.first_directory.0);
    }
}

/// One directory page: the status pages it names, by page number, and the next directory page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Directory {
    pub(crate) pid: Pid,
    pub(crate) next: Option<Pid>,
    pub(crate) entries: Vec<(u64, Pid)>,
}

impl Directory {
    pub(crate) fn read(pid: Pid, page: &[u8]) -> Directory {
        let next = u64_at(page, DIRECTORY_NEXT);
        let count = u64_at(page, DIRECTORY_COUNT) as usize;
        assert!(
            count <= directory_capacity(page.len()),
            "{pid:?} names {count} status pages"
        );
        let entries = (0..count)
            .map(|i| {
                let at = DIRECTORY_ENTRIES + i * ENTRY;
                (u64_at(page, at), Pid(u64_at(page, at + 8)))
            })
            .collect();
        Directory {
            pid,
            next: (next != 0).then_some(Pid(next)),
            entries,
        }
    }

    pub(crate) fn write(&self, page: &mut [u8]) {
        assert!(self.entries.len() <= directory_capacity(page.len()));
        set_role(page, DIRECTORY_PAGE);
        put_u64(page, DIRECTORY_NEXT, self.next.map_or(0, |p| p.0));
        put_u64(page, DIRECTORY_COUNT, self.entries.len() as u64);
        page[DIRECTORY_ENTRIES..].fill(0);
        for (i, (n, pid)) in self.entries.iter().enumerate() {
            let at = DIRECTORY_ENTRIES + i * ENTRY;
            put_u64(page, at, *n);
            put_u64(page, at + 8, pid.0);
        }
    }
}

/// The commit status of every reserved transaction id, on commit log pages.
///
/// A root page holds the reservation and the truncation bound and names the first of a chain of
/// directory pages; the directories name the status pages, in increasing page number. Status page
/// `n` holds ids `[n * per_page, (n + 1) * per_page)` at two bits each.
///
/// Invariants:
///
/// - Every id below the reservation and not below the bound has its status page.
/// - The last directory is never dropped, so a reservation always has one to extend.
#[derive(Debug)]
pub struct CommitLog {
    root_pid: Pid,
    root: Root,
    per_page: u64,
    directory_capacity: usize,
    directories: Vec<Directory>,
    pages: BTreeMap<u64, Pid>,
}

impl CommitLog {
    /// Allocate an empty commit log. Ids from [`Xid::FIRST`] are unreserved.
    pub fn create<R: Runtime>(pager: &Pager<R>) -> Result<CommitLog, PageError> {
        let mut new = pager.reserve();
        let root_pid = new.page(PAGE_FORMAT, PageKind::CommitLog, 0)?;
        let dir = Directory {
            pid: new.page(PAGE_FORMAT, PageKind::CommitLog, 0)?,
            next: None,
            entries: Vec::new(),
        };
        let root = Root {
            reserved: Xid::FIRST,
            resolved: Xid::FIRST,
            bound: Xid::FIRST,
            first_directory: dir.pid,
        };
        pager.update_with(new, &[root_pid, dir.pid], |p| {
            root.write(p[0]);
            dir.write(p[1]);
        })?;
        Ok(CommitLog {
            root_pid,
            root,
            per_page: per_page(pager.page_size()),
            directory_capacity: directory_capacity(pager.page_size()),
            directories: vec![dir],
            pages: BTreeMap::new(),
        })
    }

    /// Load the commit log rooted at `root` from a recovered pager, and record as aborted every
    /// reserved id still in progress.
    ///
    /// Postcondition: every id below [`CommitLog::reserved`] has resolved.
    pub fn open<R: Runtime>(pager: &Pager<R>, root: Pid) -> Result<CommitLog, PageError> {
        let page = pager.read(root)?;
        check_role(root, &page, ROOT_PAGE);
        let state = Root::read(&page);
        drop(page);
        let mut directories = Vec::new();
        let mut at = Some(state.first_directory);
        while let Some(pid) = at {
            let page = pager.read(pid)?;
            check_role(pid, &page, DIRECTORY_PAGE);
            let dir = Directory::read(pid, &page);
            at = dir.next;
            directories.push(dir);
        }
        let pages = directories
            .iter()
            .flat_map(|d| d.entries.iter().copied())
            .collect();
        let mut clog = CommitLog {
            root_pid: root,
            root: state,
            per_page: per_page(pager.page_size()),
            directory_capacity: directory_capacity(pager.page_size()),
            directories,
            pages,
        };
        clog.abort_unresolved(pager)?;
        Ok(clog)
    }

    fn abort_unresolved<R: Runtime>(&mut self, pager: &Pager<R>) -> Result<(), PageError> {
        let from = self.root.resolved.max(self.root.bound);
        let to = self.root.reserved;
        if from >= to {
            return Ok(());
        }
        let (first, _) = locate(from, self.per_page);
        let (last, _) = locate(Xid(to.0 - 1), self.per_page);
        for n in first..=last {
            let pid = self.page(n);
            let lo = from.0.max(n * self.per_page);
            let hi = to.0.min((n + 1) * self.per_page);
            let page = pager.read(pid)?;
            let open: Vec<usize> = (lo - n * self.per_page..hi - n * self.per_page)
                .map(|i| i as usize)
                .filter(|&i| status_at(&page, i) == Status::InProgress)
                .collect();
            drop(page);
            if !open.is_empty() {
                pager.update(&[pid], |p| {
                    for &i in &open {
                        set_status(p[0], i, Status::Aborted);
                    }
                })?;
            }
        }
        Ok(())
    }

    /// Returns the page id of the root, by which [`CommitLog::open`] finds the commit log.
    pub fn root(&self) -> Pid {
        self.root_pid
    }

    /// Returns the first id not reserved.
    pub fn reserved(&self) -> Xid {
        self.root.reserved
    }

    /// Returns the truncation bound: every id below it counts as committed.
    pub fn bound(&self) -> Xid {
        self.root.bound
    }

    /// Returns the number of transaction ids one status page holds.
    pub fn ids_per_page(&self) -> u64 {
        self.per_page
    }

    /// Returns the status pages, by page number.
    pub fn status_pages(&self) -> &BTreeMap<u64, Pid> {
        &self.pages
    }

    /// Returns the directory pages, first to last.
    pub fn directory_pages(&self) -> Vec<Pid> {
        self.directories.iter().map(|d| d.pid).collect()
    }

    fn page(&self, n: u64) -> Pid {
        *self
            .pages
            .get(&n)
            .unwrap_or_else(|| panic!("status page {n} is missing below the reservation"))
    }

    /// Reserve every id below `upto`, allocating the status pages they need, and log the
    /// reservation and force the log. `resolved` is an id below which every id has resolved.
    ///
    /// Returns the LSN of the xid allocation record, which is durable.
    ///
    /// Panics if `upto` does not extend the reservation by at most one status page of ids, or if
    /// `resolved` lies outside `[resolved, reserved]` of the current reservation.
    pub fn reserve<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
        upto: Xid,
        resolved: Xid,
    ) -> Result<Lsn, PageError> {
        let from = self.root.reserved;
        assert!(
            from < upto && upto.0 - from.0 <= self.per_page,
            "reservation from {from:?} to {upto:?} is not one page of ids"
        );
        assert!(
            (self.root.resolved..=from).contains(&resolved),
            "{resolved:?} moves the resolved bound outside [{:?}, {from:?}]",
            self.root.resolved
        );
        let (first, _) = locate(from, self.per_page);
        let (last, _) = locate(Xid(upto.0 - 1), self.per_page);
        let mut new = pager.reserve();
        let mut fresh = Vec::new();
        for n in first..=last {
            if !self.pages.contains_key(&n) {
                fresh.push((n, new.page(PAGE_FORMAT, PageKind::CommitLog, 0)?));
            }
        }
        let mut tail = vec![self.directories.last().expect("a last directory").clone()];
        for entry in &fresh {
            if tail.last().unwrap().entries.len() == self.directory_capacity {
                let pid = new.page(PAGE_FORMAT, PageKind::CommitLog, 0)?;
                tail.last_mut().unwrap().next = Some(pid);
                tail.push(Directory {
                    pid,
                    next: None,
                    entries: Vec::new(),
                });
            }
            tail.last_mut().unwrap().entries.push(*entry);
        }
        let root = Root {
            reserved: upto,
            resolved,
            ..self.root
        };
        let pids: Vec<Pid> = std::iter::once(self.root_pid)
            .chain(tail.iter().map(|d| d.pid))
            .chain(fresh.iter().map(|(_, pid)| *pid))
            .collect();
        pager.update_with(new, &pids, |p| {
            root.write(p[0]);
            for (d, page) in tail.iter().zip(p[1..].iter_mut()) {
                d.write(page);
            }
        })?;
        self.root = root;
        self.directories.pop();
        self.directories.extend(tail);
        self.pages.extend(fresh);
        let lsn = pager.reserve_xids(upto.0);
        pager.flush_log();
        Ok(lsn)
    }

    /// Returns the status of `x`.
    ///
    /// Panics if `x` is not reserved.
    pub fn status<R: Runtime>(&self, pager: &Pager<R>, x: Xid) -> Result<Status, PageError> {
        assert!(
            Xid::FIRST <= x && x < self.root.reserved,
            "{x:?} is not a reserved id"
        );
        if x < self.root.bound {
            return Ok(Status::Committed);
        }
        let (n, i) = locate(x, self.per_page);
        Ok(status_at(&pager.read(self.page(n))?, i))
    }

    /// Record that `x` committed and log its commit record, in one batch.
    ///
    /// Returns the LSN of the commit record. Fails, recording nothing, if the status page is
    /// damaged.
    ///
    /// Panics if `x` is not reserved or has resolved.
    pub fn commit<R: Runtime>(&mut self, pager: &Pager<R>, x: Xid) -> Result<Lsn, PageError> {
        let (pid, i) = self.slot(x);
        pager.commit(x.0, &[pid], |p| set_status(p[0], i, Status::Committed))
    }

    /// Record that `x` aborted and log its abort record, in one batch.
    pub fn abort<R: Runtime>(&mut self, pager: &Pager<R>, x: Xid) -> Result<Lsn, PageError> {
        let (pid, i) = self.slot(x);
        pager.abort(x.0, &[pid], |p| set_status(p[0], i, Status::Aborted))
    }

    /// Returns the status page holding `x` and its index there.
    fn slot(&self, x: Xid) -> (Pid, usize) {
        assert!(
            self.root.bound <= x && x < self.root.reserved,
            "{x:?} is not a reserved id above the bound"
        );
        let (n, i) = locate(x, self.per_page);
        (self.page(n), i)
    }

    /// Drop the status of every id below `bound`, which then counts as committed, and free the
    /// status and directory pages that held only such ids.
    ///
    /// Preconditions: every id below `bound` has resolved; no version names an aborted id below
    /// it, and none has an aborted deleter below it in `xmax`.
    ///
    /// Panics if `bound` is past the reservation.
    pub fn truncate<R: Runtime>(&mut self, pager: &Pager<R>, bound: Xid) -> Result<(), PageError> {
        assert!(
            bound <= self.root.reserved,
            "truncation to {bound:?} passes the reservation"
        );
        if bound <= self.root.bound {
            return Ok(());
        }
        let (keep, _) = locate(bound, self.per_page);
        let last = self.directories.len() - 1;
        let mut gone_dirs = Vec::new();
        let mut kept = None;
        for (i, d) in self.directories.iter().enumerate() {
            let entries: Vec<(u64, Pid)> = d
                .entries
                .iter()
                .copied()
                .filter(|(n, _)| *n >= keep)
                .collect();
            if entries.is_empty() && i < last {
                gone_dirs.push(d.pid);
                continue;
            }
            kept = Some((
                i,
                Directory {
                    entries,
                    ..d.clone()
                },
            ));
            break;
        }
        let (first, dir) = kept.expect("the last directory is kept");
        let root = Root {
            bound,
            first_directory: dir.pid,
            ..self.root
        };
        let mut gone = pager.reserve();
        for &pid in gone_dirs
            .iter()
            .chain(self.pages.range(..keep).map(|(_, p)| p))
        {
            gone.free(pid);
        }
        pager.update_with(gone, &[self.root_pid, dir.pid], |p| {
            root.write(p[0]);
            dir.write(p[1]);
        })?;
        self.root = root;
        self.directories.drain(..first);
        self.directories[0] = dir;
        self.pages = self.pages.split_off(&keep);
        Ok(())
    }
}
