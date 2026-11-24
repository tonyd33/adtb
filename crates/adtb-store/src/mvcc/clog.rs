use std::collections::BTreeMap;

use crate::runtime::Runtime;
use crate::storage::{
    Damage, FIRST_EDITABLE, HEADER_LEN, Lsn, PageError, PageKind, Pager, Pid, Resolutions, Shape,
    StaleCopy, Unsettled, flags, format, header_edit, kind, kind_fault, lsn,
};

use super::{Status, StatusError, Xid};

/// A status page holding ids in progress: its page, its first id, those ids, and the file's copy
/// with its judgement when the page lost a write.
pub(crate) type Unresolved<'a, R> = (Pid, u64, Vec<u64>, Option<(Vec<u8>, StaleCopy<'a, R>)>);

/// The right to read a status page that lost a write and write it back settled, which only the
/// commit log holds: no code outside this module can make one.
pub(crate) struct Settling(());

/// The format version of commit log pages.
pub const PAGE_FORMAT: u16 = 2;

/// The page header flags of a root and a directory page; a status page has flags 0.
const ROOT_PAGE: u16 = 1;
const DIRECTORY_PAGE: u16 = 2;

const ROOT_RESERVED: usize = HEADER_LEN;
const ROOT_RESOLVED: usize = HEADER_LEN + 8;
const ROOT_BOUND: usize = HEADER_LEN + 16;
const ROOT_FIRST_DIRECTORY: usize = HEADER_LEN + 24;

const STATUS_NUMBER: usize = HEADER_LEN;
const STATUSES: usize = HEADER_LEN + 8;

const DIRECTORY_NEXT: usize = HEADER_LEN;
const DIRECTORY_COUNT: usize = HEADER_LEN + 8;
const DIRECTORY_ENTRIES: usize = HEADER_LEN + 16;
const ENTRY: usize = 16;

fn u64_at(page: &[u8], at: usize) -> u64 {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&page[at..at + 8]);
    u64::from_le_bytes(bytes)
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

/// Returns what `judge` decodes of page `pid`. Refuses the page, from then on, when `judge` finds
/// it out of shape under the exclusive latch too.
fn decode<R: Runtime, T>(
    pager: &Pager<R>,
    pid: Pid,
    judge: impl Fn(&[u8]) -> Result<T, Shape>,
) -> Result<T, PageError> {
    loop {
        let page = pager.read(pid)?;
        if let Ok(decoded) = judge(&page) {
            return Ok(decoded);
        }
        drop(page);
        if let Some(e) = pager.refuse_if(pid, |p| judge(p).err()) {
            return Err(e);
        }
    }
}

/// Returns how `page` is out of shape as a commit log page of `role`.
fn role_fault(page: &[u8], role: u16) -> Option<Shape> {
    if kind(page) != PageKind::CommitLog as u16 || flags(page) != role {
        Some(kind_fault(page))
    } else if format(page) != PAGE_FORMAT {
        Some(Shape::Format)
    } else {
        None
    }
}

/// A page read as status page `n`.
#[derive(Clone, Copy, Debug)]
pub struct StatusPage<'p>(&'p [u8]);

impl<'p> StatusPage<'p> {
    /// Returns `page` as status page `n`, or how it is out of shape as one.
    pub fn of(page: &'p [u8], n: u64) -> Result<StatusPage<'p>, Shape> {
        if let Some(fault) = role_fault(page, 0) {
            return Err(fault);
        }
        if u64_at(page, STATUS_NUMBER) != n {
            return Err(Shape::Frame);
        }
        Ok(StatusPage(page))
    }

    /// Returns the status of the id at `index`.
    pub fn status(self, index: usize) -> Result<Status, Shape> {
        status_at(self.0, index)
    }

    /// Returns the ids from `base` on in `ids` that are in progress, `base` being the page's
    /// first id.
    fn in_progress(self, base: u64, ids: std::ops::Range<u64>) -> Result<Vec<u64>, Shape> {
        let mut open = Vec::new();
        for x in ids {
            if self.status((x - base) as usize)? == Status::InProgress {
                open.push(x);
            }
        }
        Ok(open)
    }
}

/// Returns the number of transaction ids one status page of a `page_size` page holds.
pub(crate) fn per_page(page_size: usize) -> u64 {
    ((page_size - STATUSES) * 4) as u64
}

/// Returns the number of status pages one directory page of a `page_size` page names.
pub(crate) fn directory_capacity(page_size: usize) -> usize {
    (page_size - DIRECTORY_ENTRIES) / ENTRY
}

/// Returns the number of the status page holding `x`, and `x`'s index within it.
pub fn locate(x: Xid, per_page: u64) -> (u64, usize) {
    (x.0 / per_page, (x.0 % per_page) as usize)
}

/// Returns the status the bits of the id at `index` of a status page record, or
/// [`Shape::Frame`] when they name none.
pub fn status_at(page: &[u8], index: usize) -> Result<Status, Shape> {
    let byte = page[STATUSES + index / 4];
    Status::from_bits(byte >> (index % 4 * 2) & 0b11).ok_or(Shape::Frame)
}

/// Record `status` for the id at `index`.
///
/// Preconditions: the id's bits read in progress.
///
/// Panics:
/// - `status` is in progress.
pub(crate) fn set_status(page: &mut [u8], index: usize, status: Status) {
    assert_ne!(
        Status::InProgress,
        status,
        "a status is set only once resolved"
    );
    page[STATUSES + index / 4] |= (status as u8) << (index % 4 * 2);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Root {
    /// Every id below it is reserved.
    pub(crate) reserved: Xid,
    /// Every id below it had resolved when the root was last written.
    pub(crate) resolved: Xid,
    /// Every id below it counts as committed; its status pages may be gone.
    pub(crate) bound: Xid,
    pub(crate) first_directory: Pid,
}

impl Root {
    #[cfg(any(test, feature = "testing"))]
    pub fn resolved(&self) -> Xid {
        self.resolved
    }

    /// Returns the root `page` holds, or [`Shape::Frame`] when its bounds do not lie in order
    /// from the first id to the reservation or it names no directory.
    pub fn read(page: &[u8]) -> Result<Root, Shape> {
        let root = Root {
            reserved: Xid(u64_at(page, ROOT_RESERVED)),
            resolved: Xid(u64_at(page, ROOT_RESOLVED)),
            bound: Xid(u64_at(page, ROOT_BOUND)),
            first_directory: Pid(u64_at(page, ROOT_FIRST_DIRECTORY)),
        };
        let ordered = Xid::FIRST <= root.bound
            && root.bound <= root.reserved
            && Xid::FIRST <= root.resolved
            && root.resolved <= root.reserved;
        if ordered && root.first_directory != Pid(0) {
            Ok(root)
        } else {
            Err(Shape::Frame)
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
    /// Returns directory page `pid` as `page` holds it, or [`Shape::Frame`] when its count passes
    /// its capacity.
    pub(crate) fn read(pid: Pid, page: &[u8]) -> Result<Directory, Shape> {
        let next = u64_at(page, DIRECTORY_NEXT);
        let count = u64_at(page, DIRECTORY_COUNT);
        let count = match usize::try_from(count) {
            Ok(c) if c <= directory_capacity(page.len()) => c,
            _ => return Err(Shape::Frame),
        };
        let entries = (0..count)
            .map(|i| {
                let at = DIRECTORY_ENTRIES + i * ENTRY;
                (u64_at(page, at), Pid(u64_at(page, at + 8)))
            })
            .collect();
        Ok(Directory {
            pid,
            next: (next != 0).then_some(Pid(next)),
            entries,
        })
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

/// Returns the index in `dirs`, the directories of `root` in chain order, of the one out of
/// shape: one holding an entry not past the entry before it or naming a status page outside those
/// from the bound's to the reservation's; else, for the first status page an id below the
/// reservation and not below the bound needs that no entry names, the directory holding the next
/// entry, the one before it when that entry starts its directory, or the last when none follows.
fn misshapen(root: &Root, dirs: &[Directory], per_page: u64) -> Option<usize> {
    let (first, _) = locate(root.bound, per_page);
    let (end, _) = locate(Xid(root.bound.0.max(root.reserved.0 - 1)), per_page);
    let mut named = BTreeMap::new();
    let mut last = None;
    for (i, d) in dirs.iter().enumerate() {
        for &(n, _) in &d.entries {
            if last.is_some_and(|l| n <= l) || !(first..=end).contains(&n) {
                return Some(i);
            }
            last = Some(n);
            named.insert(n, i);
        }
    }
    if root.bound >= root.reserved {
        return None;
    }
    let missing = (first..=end).find(|n| !named.contains_key(n))?;
    Some(match named.range(missing..).next() {
        None => dirs.len() - 1,
        Some((&n, &i)) if i > 0 && dirs[i].entries.first().map(|e| e.0) == Some(n) => i - 1,
        Some((_, &i)) => i,
    })
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

    /// Load the commit log rooted at `root` from a recovered pager, and settle every reserved id
    /// still in progress as [`CommitLog::settle_unresolved`] does.
    #[cfg(any(test, feature = "testing"))]
    pub fn open<R: Runtime>(pager: &Pager<R>, root: Pid) -> Result<CommitLog, PageError> {
        let mut clog = CommitLog::load(pager, root)?;
        clog.settle_unresolved(pager)?;
        Ok(clog)
    }

    /// Returns a commit log of no pages and no ids, for a file whose own cannot be read.
    pub fn unread<R: Runtime>(pager: &Pager<R>) -> CommitLog {
        CommitLog {
            root_pid: Pid(0),
            root: Root {
                reserved: Xid::FIRST,
                resolved: Xid::FIRST,
                bound: Xid::FIRST,
                first_directory: Pid(0),
            },
            per_page: per_page(pager.page_size()),
            directory_capacity: directory_capacity(pager.page_size()),
            directories: Vec::new(),
            pages: BTreeMap::new(),
        }
    }

    /// Read the commit log rooted at `root`: its root and directories, and no status page.
    ///
    /// Refuse, from then on, the root or a directory page out of shape, or the directory that a
    /// chain of directories returning to a page names, that holds an entry not past the one
    /// before it, or that lost the entry of a status page an id below the reservation and not
    /// below the bound needs.
    pub fn load<R: Runtime>(pager: &Pager<R>, root: Pid) -> Result<CommitLog, PageError> {
        let per_page = per_page(pager.page_size());
        loop {
            let state = decode(pager, root, |p| match role_fault(p, ROOT_PAGE) {
                Some(fault) => Err(fault),
                None => Root::read(p),
            })?;
            let mut directories: Vec<Directory> = Vec::new();
            let mut at = Some(state.first_directory);
            let mut looped = false;
            while let Some(pid) = at {
                if directories.iter().any(|d| d.pid == pid) {
                    looped = true;
                    break;
                }
                let dir = decode(pager, pid, |p| match role_fault(p, DIRECTORY_PAGE) {
                    Some(fault) => Err(fault),
                    None => Directory::read(pid, p),
                })?;
                at = dir.next;
                directories.push(dir);
            }
            let blamed = if looped {
                Some(directories.len() - 1)
            } else {
                misshapen(&state, &directories, per_page)
            };
            if let Some(i) = blamed {
                let dir = &directories[i];
                let same = |p: &[u8]| Directory::read(dir.pid, p).as_ref() == Ok(dir);
                match pager.refuse_if(dir.pid, |p| same(p).then_some(Shape::Frame)) {
                    Some(e) => return Err(e),
                    None => continue,
                }
            }
            let pages = directories
                .iter()
                .flat_map(|d| d.entries.iter().copied())
                .collect();
            return Ok(CommitLog {
                root_pid: root,
                root: state,
                per_page,
                directory_capacity: directory_capacity(pager.page_size()),
                directories,
                pages,
            });
        }
    }

    /// Record every reserved id still in progress as committed when the retained log holds its
    /// commit record, and as aborted otherwise, as [`CommitLog::unresolved`] reads them.
    ///
    /// Fail as [`CommitLog::unresolved`] does, settling no page. Otherwise fail at the first status
    /// page that cannot be settled, leaving each page before it settled and refusing each later
    /// one that lost a write:
    ///
    /// - with the page's refusal when it is refused other than as stale, recording nothing more;
    /// - with its stale refusal when it is resident, leaving it unrefused and unrecorded;
    /// - with the failure of its rewrite, a page that lost a write then recorded settled.
    ///
    /// Postcondition: on success every id below [`CommitLog::reserved`] has resolved.
    pub(crate) fn settle_unresolved<R: Runtime>(
        &mut self,
        pager: &Pager<R>,
    ) -> Result<(), PageError> {
        let (open, logged) = self.unresolved(pager)?;
        for (pid, base, ids, copy) in open {
            let settle = |p: &mut [u8]| {
                for &x in &ids {
                    let status = if logged.commits.contains(&x) {
                        Status::Committed
                    } else {
                        Status::Aborted
                    };
                    set_status(p, (x - base) as usize, status);
                }
            };
            match copy {
                Some((copy, stale)) => {
                    let mut settled = copy.clone();
                    settle(&mut settled);
                    stale.settle(&Settling(()), &copy, &settled)?;
                }
                None => drop(pager.update(&[pid], |p| settle(p[0]))?),
            }
        }
        Ok(())
    }

    /// Returns each status page holding a reserved id still in progress, with the page's first id
    /// and those ids, and what the retained log says of them from the oldest LSN of those pages,
    /// past which lies every commit record changing them.
    ///
    /// Fail with the refusal of a status page that cannot be read, other than one that lost a
    /// write, refusing from then on the directory page naming a page that is no status page, or
    /// a status page of another number or holding bits of an id that name no status. Fail with [`Damage::LogUnproven`], naming the
    /// page of that LSN, when the log from there to the end recovery kept is not whole and
    /// readable. Fail with the refusal of a page that lost a write when the file's copy is not
    /// the status page the directory names there or holds bits of an id that name no status, or
    /// when the copy holds an id in progress and the retained log holds no record of the page
    /// continuing from the copy.
    pub fn unresolved<'p, R: Runtime>(
        &self,
        pager: &'p Pager<R>,
    ) -> Result<(Vec<Unresolved<'p, R>>, Resolutions), PageError> {
        let (from, to) = (self.root.bound, self.root.reserved);
        let mut open = Vec::new();
        let mut oldest: Option<(Lsn, Pid)> = None;
        for (&n, &pid) in &self.pages {
            let base = n * self.per_page;
            let range = from.0.max(base)..to.0.min(base + self.per_page);
            let judge = |p: &[u8]| StatusPage::of(p, n)?.in_progress(base, range.clone());
            let (page, refused, ids) = loop {
                let (page, refused) = pager.read_stale(pid, &Settling(()))?;
                match (judge(&page), refused) {
                    (Ok(ids), refused) => break (page, refused, ids),
                    (Err(_), Some(stale)) => return Err(stale.refuse()),
                    (Err(_), None) => {
                        if let Some(e) = self.refuse_status(pager, (n, pid), judge) {
                            return Err(e);
                        }
                    }
                }
            };
            if !ids.is_empty() || refused.is_some() {
                if oldest.is_none_or(|(at, _)| lsn(&page) < at) {
                    oldest = Some((lsn(&page), pid));
                }
                open.push((pid, base, ids, refused.map(|e| (page, e))));
            }
        }
        let Some((since, pid)) = oldest else {
            return Ok((open, Resolutions::default()));
        };
        let stale = open
            .iter()
            .filter_map(|(pid, _, _, copy)| copy.as_ref().map(|_| *pid))
            .collect();
        let logged = pager.logged_resolutions_since(since, &stale).map_err(|e| {
            let at = match e {
                Unsettled::At(at) => at,
                Unsettled::Unread(why) => {
                    pager.keep_unarmed(why);
                    since
                }
            };
            PageError {
                pid,
                damage: Damage::LogUnproven { at },
            }
        })?;
        let unproven = open.iter().position(|(pid, _, ids, copy)| {
            copy.as_ref().is_some_and(|(copy, _)| {
                !ids.is_empty() && !logged.continued.contains(&(*pid, lsn(copy)))
            })
        });
        if let Some((_, stale)) = unproven.and_then(|i| open.swap_remove(i).3) {
            return Err(stale.refuse());
        }
        Ok((open, logged))
    }

    /// Returns the page id of the root, by which [`CommitLog::open`] finds the commit log.
    pub fn root(&self) -> Pid {
        self.root_pid
    }

    /// Returns the first id not reserved.
    pub(crate) fn reserved(&self) -> Xid {
        self.root.reserved
    }

    /// Returns the first id not reserved, as [`CommitLog::reserved`] does.
    #[cfg(any(test, feature = "testing"))]
    pub fn reserved_tested(&self) -> Xid {
        self.reserved()
    }

    /// Returns whether `x` is reserved: a transaction id below the first id not reserved.
    pub fn reserves(&self, x: Xid) -> bool {
        Xid::FIRST <= x && x < self.root.reserved
    }

    /// Returns the number of each status page that would hold an id reserved above the bound and
    /// that the commit log names no page for.
    pub fn unpaged(&self) -> Vec<u64> {
        if self.root.bound >= self.root.reserved {
            return Vec::new();
        }
        let first = self.root.bound.0 / self.per_page;
        let last = (self.root.reserved.0 - 1) / self.per_page;
        (first..=last)
            .filter(|n| !self.pages.contains_key(n))
            .collect()
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

    /// Returns every page of the commit log: its root, its status pages and its directory pages.
    pub fn pages(&self) -> Vec<Pid> {
        let mut pages = vec![self.root_pid];
        pages.extend(self.pages.values());
        pages.extend(self.directory_pages());
        pages
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
    /// Panics:
    /// - `upto` does not extend the reservation by at most one status page of ids.
    /// - `resolved` lies outside `[resolved, reserved]` of the current reservation.
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
            let (dirs, statuses) = p[1..].split_at_mut(tail.len());
            for (d, page) in tail.iter().zip(dirs) {
                d.write(page);
            }
            for (&(n, _), page) in fresh.iter().zip(statuses) {
                put_u64(page, STATUS_NUMBER, n);
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
    /// Fail with [`StatusError::Unreserved`] when `x` is not reserved, and with the damage of
    /// its status page when that fails verification.
    pub fn status<R: Runtime>(&self, pager: &Pager<R>, x: Xid) -> Result<Status, StatusError> {
        if !self.reserves(x) {
            return Err(StatusError::Unreserved(x));
        }
        if x < self.root.bound {
            return Ok(Status::Committed);
        }
        let (n, i) = locate(x, self.per_page);
        Ok(self.read_status(pager, n, |p| p.status(i))?)
    }

    /// Returns what `judge` decodes of status page `n`. Refuses, from then on, the directory
    /// page naming it when the page it names is no status page, else the page when it is out of
    /// shape as status page `n` or `judge` refuses it, under the exclusive latch too.
    fn read_status<R: Runtime, T>(
        &self,
        pager: &Pager<R>,
        n: u64,
        judge: impl Fn(StatusPage<'_>) -> Result<T, Shape>,
    ) -> Result<T, PageError> {
        let pid = self.page(n);
        let judge = |p: &[u8]| judge(StatusPage::of(p, n)?);
        loop {
            let page = pager.read(pid)?;
            if let Ok(decoded) = judge(&page) {
                return Ok(decoded);
            }
            drop(page);
            if let Some(e) = self.refuse_status(pager, (n, pid), judge) {
                return Err(e);
            }
        }
    }

    /// Refuse, from then on, the directory page naming `pid` as status page `n` when `pid` is no
    /// status page under its exclusive latch, else `pid` when `judge` refuses it there. Returns
    /// `None` when neither still holds: the caller reads again.
    fn refuse_status<R: Runtime, T>(
        &self,
        pager: &Pager<R>,
        (n, pid): (u64, Pid),
        judge: impl Fn(&[u8]) -> Result<T, Shape>,
    ) -> Option<PageError> {
        let misnamed = std::cell::Cell::new(false);
        let refused = pager.refuse_if(pid, |p| match role_fault(p, 0) {
            Some(Shape::Kind | Shape::Blank) => {
                misnamed.set(true);
                None
            }
            _ => judge(p).err(),
        });
        if refused.is_some() || !misnamed.get() {
            return refused;
        }
        let Some(dir) = (self.directories.iter()).find(|d| d.entries.contains(&(n, pid))) else {
            return pager.refuse_if(pid, |p| role_fault(p, 0));
        };
        let names = |p: &[u8]| Directory::read(dir.pid, p).is_ok_and(|d| d.entries == dir.entries);
        pager.refuse_if(dir.pid, |p| names(p).then_some(Shape::Frame))
    }

    /// Record that `x` committed and log its commit record, in one batch, in a commit log no
    /// transactions hold.
    ///
    /// Returns the LSN of the commit record.
    ///
    /// Errors:
    /// - The status page is damaged, or does not read `x` in progress, recording nothing.
    ///
    /// Panics:
    /// - `x` is not reserved above the bound.
    #[cfg(any(test, feature = "testing"))]
    pub fn commit<R: Runtime>(&mut self, pager: &Pager<R>, x: Xid) -> Result<Lsn, PageError> {
        let (pid, i) = self.slot(pager, x)?;
        pager.commit(x.0, &[pid], |p| set_status(p[0], i, Status::Committed))
    }

    /// Commit `x` as [`CommitLog::commit`] does, its commit record recording `time`, in
    /// nanoseconds since the Unix epoch, and the pages `also` names changed through its function
    /// in the same batch.
    pub(super) fn commit_at<R: Runtime>(
        &self,
        pager: &Pager<R>,
        x: Xid,
        time: u64,
        (pids, f): (&[Pid], impl FnOnce(&mut [&mut [u8]])),
    ) -> Result<Lsn, PageError> {
        let (pid, i) = self.slot(pager, x)?;
        let pages: Vec<Pid> = std::iter::once(pid).chain(pids.iter().copied()).collect();
        pager.commit_at(x.0, time, &pages, |p| {
            set_status(p[0], i, Status::Committed);
            f(&mut p[1..]);
        })
    }

    /// Record that `x` aborted and log its abort record, in one batch.
    pub(super) fn abort<R: Runtime>(&self, pager: &Pager<R>, x: Xid) -> Result<Lsn, PageError> {
        let (pid, i) = self.slot(pager, x)?;
        pager.abort(x.0, &[pid], |p| set_status(p[0], i, Status::Aborted))
    }

    /// Abort `x` as [`CommitLog::abort`] does, in a commit log no transactions hold.
    #[cfg(any(test, feature = "testing"))]
    pub fn abort_tested<R: Runtime>(&mut self, pager: &Pager<R>, x: Xid) -> Result<Lsn, PageError> {
        self.abort(pager, x)
    }

    /// Returns the status page holding `x` and its index there. Refuses the page, from then on,
    /// when it is not that status page or does not read `x` in progress.
    ///
    /// Panics:
    /// - `x` is not reserved above the bound.
    fn slot<R: Runtime>(&self, pager: &Pager<R>, x: Xid) -> Result<(Pid, usize), PageError> {
        assert!(
            self.root.bound <= x && x < self.root.reserved,
            "{x:?} is not a reserved id above the bound"
        );
        let (n, i) = locate(x, self.per_page);
        self.read_status(pager, n, |p| match p.status(i)? {
            Status::InProgress => Ok(()),
            Status::Committed | Status::Aborted => Err(Shape::Frame),
        })?;
        Ok((self.page(n), i))
    }

    /// Drop the status of every id below `bound`, which then counts as committed, and free the
    /// status and directory pages that held only such ids.
    ///
    /// Preconditions: every id below `bound` has resolved; no version names an aborted id below
    /// it, and none has an aborted deleter below it in `xmax`.
    ///
    /// Panics:
    /// - `bound` is past the reservation.
    #[cfg(any(test, feature = "testing"))]
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
