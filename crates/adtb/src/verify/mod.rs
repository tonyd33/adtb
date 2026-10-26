use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use crate::btree::{self, BTree, Index};
use crate::heap::{Flags, Heap, LargeStore, header_at};
use crate::key::{CtorId, Tid};
use crate::mvcc::{CommitLog, Status, Xid};
use crate::runtime::Runtime;
use crate::storage::{self, Damage, LogFinding, PageError, PageKind, Pager, Pid};

/// The heaps and indexes of one sum, as the caller knows them.
#[derive(Debug)]
pub struct Sum<'a> {
    /// Each constructor's heap.
    pub heaps: Vec<(CtorId, &'a Heap)>,
    /// Every index over constructors of the sum.
    pub indexes: Vec<&'a Index>,
}

/// Every structure of a database file, as the caller names them. A sum's large-value store is
/// the one its heaps name.
///
/// Precondition: every allocated page belongs to exactly one structure listed, or is in `pages`.
#[derive(Debug, Default)]
pub struct Inventory<'a> {
    pub commit_log: Option<&'a CommitLog>,
    pub sums: Vec<Sum<'a>>,
    /// B-link trees outside any index.
    pub trees: Vec<BTree>,
    /// Pages the caller refers to outside every structure listed.
    pub pages: Vec<Pid>,
}

/// What is wrong with one page.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Problem {
    /// The pager refuses the page.
    Damaged(Damage),
    /// A valid page that no record of a log retained from its start wrote.
    Unlogged,
    /// Allocated, and no structure refers to it.
    Leaked,
    /// A structure refers to it, and it is free.
    Unallocated,
    /// Two structures refer to it, the two named.
    Shared(String, String),
    /// A structure's own check refuses it.
    Structure(String),
    /// The head of an out-of-line value in `slot` counts `count` versions and only `versions`
    /// point at it.
    Overcounted {
        slot: u16,
        count: u32,
        versions: u32,
    },
}

impl Problem {
    /// Returns whether [`repair`] mends the problem.
    pub fn repairable(&self) -> bool {
        matches!(self, Problem::Leaked | Problem::Overcounted { .. })
    }
}

/// How much [`verify_with`] checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Depth {
    /// Check every page and the whole retained log as the pager does. Without it, only the pages
    /// the structures read are verified, as every read verifies them.
    pub log: bool,
}

impl Depth {
    pub const FULL: Depth = Depth { log: true };
    pub const STRUCTURE: Depth = Depth { log: false };
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub pid: Pid,
    pub problem: Problem,
}

/// What [`verify`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Each problem found, by page.
    pub findings: Vec<Finding>,
    pub log: Vec<LogFinding>,
    /// The transaction of every commit record in the retained log, in log order.
    pub commits: Vec<u64>,
    /// Every structure was read whole. Pages are found leaked only then.
    pub complete: bool,
}

impl Report {
    /// Returns whether nothing is wrong.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty() && self.log.is_empty()
    }

    /// Returns the pages found leaked.
    pub fn leaked(&self) -> Vec<Pid> {
        self.findings
            .iter()
            .filter(|f| f.problem == Problem::Leaked)
            .map(|f| f.pid)
            .collect()
    }

    /// Returns the findings [`repair`] does not mend.
    pub fn unrepairable(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| !f.problem.repairable())
            .collect()
    }
}

struct Walk<'p, R: Runtime> {
    pager: &'p Pager<R>,
    findings: Vec<Finding>,
    owners: BTreeMap<Pid, String>,
    complete: bool,
}

impl<R: Runtime> Walk<'_, R> {
    fn found(&mut self, pid: Pid, problem: Problem) {
        self.findings.push(Finding { pid, problem });
    }

    fn structure(&mut self, pid: Pid, what: impl Into<String>) {
        self.found(pid, Problem::Structure(what.into()));
    }

    fn damaged(&mut self, e: PageError) {
        self.found(e.pid, Problem::Damaged(e.damage));
        self.complete = false;
    }

    fn own(&mut self, name: &str, pages: impl IntoIterator<Item = Pid>) {
        for pid in pages {
            if let Some(other) = self.owners.insert(pid, name.to_owned()) {
                self.found(pid, Problem::Shared(other, name.to_owned()));
            }
        }
    }

    fn tree_error(&mut self, name: &str, root: Pid, e: btree::Error) {
        match e {
            btree::Error::Page(e) => self.damaged(e),
            btree::Error::Malformed(pid) => {
                self.complete = false;
                self.structure(pid, format!("{name}: breaks the tree's structure"));
            }
            e => self.structure(root, format!("{name}: {e:?}")),
        }
    }

    fn commit_log(&mut self, clog: &CommitLog) {
        if clog.bound() < clog.reserved() {
            let per = clog.ids_per_page();
            let first = clog.bound().0 / per;
            let last = (clog.reserved().0 - 1) / per;
            for n in first..=last {
                if !clog.status_pages().contains_key(&n) {
                    self.structure(
                        clog.root(),
                        format!("commit log: no status page holds ids from {}", n * per),
                    );
                }
            }
        }
        let mut pages = vec![clog.root()];
        pages.extend(clog.directory_pages());
        pages.extend(clog.status_pages().values().copied());
        for &pid in &pages {
            let kind = self.pager.read(pid).map(|page| storage::kind(&page));
            match kind {
                Ok(k) if k == PageKind::CommitLog as u16 => {}
                Ok(_) => self.structure(pid, "commit log: is not a commit log page"),
                Err(e) => self.damaged(e),
            }
        }
        self.own("commit log", pages);
    }

    fn tree(&mut self, name: &str, tree: BTree) {
        if let Err(e) = tree.verify(self.pager) {
            self.tree_error(name, tree.root(), e);
        }
        match tree.pages(self.pager) {
            Ok(pages) => self.own(name, pages),
            Err(e) => self.tree_error(name, tree.root(), e),
        }
    }

    fn heap(&mut self, heap: &Heap) {
        let name = format!("heap {}", heap.id());
        match heap.check(self.pager) {
            Ok(problems) => {
                for (pid, what) in problems {
                    self.structure(pid, format!("{name}: {what}"));
                }
            }
            Err(e) => self.damaged(e),
        }
        match heap.owned_pages(self.pager) {
            Ok(pages) => self.own(&name, pages),
            Err(e) => self.damaged(e),
        }
    }

    /// Check every version's `next` that is not hot-updated: a version with no deleter names no
    /// successor, and one whose deleter is running names a version that deleter wrote.
    fn chains(&mut self, heap: &Heap, clog: &CommitLog) {
        let name = format!("heap {}", heap.id());
        let versions = match heap.headers(self.pager) {
            Ok(v) => v,
            Err(e) => return self.damaged(e),
        };
        for (t, h, flags) in versions {
            let pid = Pid(t.page());
            let Some(next) = h.next else {
                continue;
            };
            if flags.contains(Flags::HOT_UPDATED) {
                continue;
            }
            if h.xmax == 0 {
                self.structure(
                    pid,
                    format!(
                        "{name}: slot {} names a successor and has no deleter",
                        t.slot()
                    ),
                );
                continue;
            }
            if Xid(h.xmax) >= clog.reserved() {
                self.structure(
                    pid,
                    format!(
                        "{name}: slot {} names unreserved deleter {}",
                        t.slot(),
                        h.xmax
                    ),
                );
                continue;
            }
            match clog.status(self.pager, Xid(h.xmax)) {
                Ok(Status::InProgress) => {}
                Ok(_) => continue,
                Err(e) => return self.damaged(e),
            }
            let found = match header_at(self.pager, next) {
                Ok(successor) => successor.is_some_and(|s| s.xmin == h.xmax),
                Err(e) => return self.damaged(e),
            };
            if !found {
                self.structure(
                    pid,
                    format!(
                        "{name}: slot {} is superseded by running {} and names no version of it",
                        t.slot(),
                        h.xmax
                    ),
                );
            }
        }
    }

    fn large(&mut self, large: LargeStore, heaps: &[&Heap]) {
        let name = format!("large-value store {}", large.id());
        let mut versions = Some(BTreeMap::<Tid, u32>::new());
        for heap in heaps {
            match heap.out_of_line(self.pager) {
                Ok(Some(pointers)) => {
                    if let Some(v) = versions.as_mut() {
                        for p in pointers {
                            *v.entry(p.first).or_default() += 1;
                        }
                    }
                }
                Ok(None) => versions = None,
                Err(e) => {
                    self.damaged(e);
                    versions = None;
                }
            }
        }
        let counted = match &versions {
            Some(v) => Some(large.check_chunks(self.pager, v)),
            None => None,
        };
        match counted.transpose() {
            Ok(None) => {}
            Ok(Some(found)) => {
                for (pid, what) in found.problems {
                    self.structure(pid, format!("{name}: {what}"));
                }
                for (t, count, refs) in found.overcounted {
                    self.found(
                        Pid(t.page()),
                        Problem::Overcounted {
                            slot: t.slot(),
                            count,
                            versions: refs,
                        },
                    );
                }
            }
            Err(e) => self.damaged(e),
        }
        match large.check(self.pager) {
            Ok(problems) => {
                for (pid, what) in problems {
                    self.structure(pid, format!("{name}: {what}"));
                }
            }
            Err(e) => self.damaged(e),
        }
        match large.owned_pages(self.pager) {
            Ok(pages) => self.own(&name, pages),
            Err(e) => self.damaged(e),
        }
    }

    fn sum(&mut self, sum: &Sum) {
        let before = self.findings.len();
        for (_, heap) in &sum.heaps {
            self.heap(heap);
        }
        let broken: BTreeSet<Pid> = self.findings[before..].iter().map(|f| f.pid).collect();
        let heaps: BTreeMap<CtorId, &Heap> = sum.heaps.iter().map(|(c, h)| (*c, *h)).collect();
        for index in &sum.indexes {
            let tree = index.tree();
            let name = format!("index {}", tree.owner());
            self.tree(&name, tree);
            match entries(self.pager, &heaps, index, &broken) {
                Ok(problems) => {
                    for (pid, what) in problems {
                        self.structure(pid, format!("{name}: {what}"));
                    }
                }
                Err(Error::Page(e)) => self.damaged(e),
                Err(Error::Index(e)) => self.tree_error(&name, tree.root(), e),
            }
        }
    }

    fn allocation(&mut self) {
        let allocated = match self.pager.allocated() {
            Ok(pages) => pages,
            Err(e) => return self.damaged(e),
        };
        let referenced: Vec<Pid> = self.owners.keys().copied().collect();
        for &pid in &referenced {
            if allocated.binary_search(&pid).is_err() {
                self.found(pid, Problem::Unallocated);
            }
        }
        if !self.complete {
            return;
        }
        for pid in allocated {
            if !self.owners.contains_key(&pid) {
                self.found(pid, Problem::Leaked);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Page(PageError),
    Index(btree::Error),
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Page(e)
    }
}

impl From<btree::Error> for Error {
    fn from(e: btree::Error) -> Error {
        Error::Index(e)
    }
}

/// Returns each entry of `index` whose constructor has no heap in `heaps`, or whose TID names no
/// version, dead slot or redirect of its constructor's heap, by the page the TID names. Entries
/// naming a page in `broken` are not read.
pub(crate) fn entries<R: Runtime>(
    pager: &Pager<R>,
    heaps: &BTreeMap<CtorId, &Heap>,
    index: &Index,
    broken: &BTreeSet<Pid>,
) -> Result<Vec<(Pid, String)>, Error> {
    let mut problems = Vec::new();
    for e in index.range(pager, Bound::Unbounded, Bound::Unbounded)? {
        let pid = Pid(e.tid.page());
        if broken.contains(&pid) {
            continue;
        }
        match heaps.get(&e.ctor) {
            None => problems.push((
                pid,
                format!("entry {:?} names {:?}, of no heap", e.key, e.ctor),
            )),
            Some(h) if !h.holds(pager, e.tid)? => problems.push((
                pid,
                format!(
                    "entry {:?} names slot {}, which holds nothing",
                    e.key,
                    e.tid.slot()
                ),
            )),
            Some(_) => {}
        }
    }
    Ok(problems)
}

/// Check the whole file against `inventory`: every page and the retained log as the pager checks
/// them, every structure's own pages, every index entry against the heap it names, and the
/// allocation bitmap against the pages the structures refer to.
///
/// Precondition: no write is in progress.
pub fn verify<R: Runtime>(pager: &Pager<R>, inventory: &Inventory) -> Report {
    verify_with(pager, inventory, Depth::FULL)
}

/// Check the file against `inventory` as [`verify`] does, to `depth`. A check without the log
/// reports no `Unlogged` page and no log finding, and returns no commits.
///
/// Precondition: no write is in progress.
pub fn verify_with<R: Runtime>(pager: &Pager<R>, inventory: &Inventory, depth: Depth) -> Report {
    let mut walk = Walk {
        pager,
        findings: Vec::new(),
        owners: BTreeMap::new(),
        complete: true,
    };
    let mut log = Vec::new();
    let mut commits = Vec::new();
    if depth.log {
        let v = pager.verify();
        commits = v.commits;
        for f in v.findings {
            match f {
                storage::Finding::Page { pid, damage } => walk.found(pid, Problem::Damaged(damage)),
                storage::Finding::Orphan(pid) => walk.found(pid, Problem::Unlogged),
                storage::Finding::Log(l) => log.push(l),
            }
        }
    }
    if let Some(clog) = inventory.commit_log {
        walk.commit_log(clog);
    }
    let mut stores: Vec<(LargeStore, Vec<&Heap>)> = Vec::new();
    for sum in &inventory.sums {
        walk.sum(sum);
        for (_, heap) in &sum.heaps {
            if let Some(clog) = inventory.commit_log {
                walk.chains(heap, clog);
            }
            match stores.iter_mut().find(|(s, _)| *s == heap.large()) {
                Some((_, hs)) => hs.push(heap),
                None => stores.push((heap.large(), vec![heap])),
            }
        }
    }
    for (store, heaps) in stores {
        walk.large(store, &heaps);
    }
    for (i, &tree) in inventory.trees.iter().enumerate() {
        walk.tree(&format!("tree {i}"), tree);
    }
    walk.own("the caller", inventory.pages.iter().copied());
    walk.allocation();
    let mut findings = walk.findings;
    findings.sort();
    findings.dedup();
    Report {
        findings,
        log,
        commits,
        complete: walk.complete,
    }
}

/// Free every page `report` found leaked, and set every head it found overcounted to the
/// number of versions that point at it. Returns the number of findings mended.
///
/// Preconditions: `report` is complete and was made by [`verify`] against `inventory` with no
/// write since.
///
/// Panics if `report` is not complete.
pub fn repair<R: Runtime>(
    pager: &Pager<R>,
    inventory: &Inventory,
    report: &Report,
) -> Result<usize, PageError> {
    assert!(
        report.complete,
        "a report that did not read every structure whole cannot name leaks"
    );
    let stores: Vec<LargeStore> = inventory
        .sums
        .iter()
        .flat_map(|s| s.heaps.iter().map(|(_, h)| h.large()))
        .collect();
    let mut mended = 0;
    for f in &report.findings {
        match f.problem {
            Problem::Leaked => {
                pager.free(f.pid)?;
            }
            Problem::Overcounted { slot, versions, .. } => {
                let owner = storage::owner(&pager.read(f.pid)?);
                let store = stores
                    .iter()
                    .find(|s| s.id() == owner)
                    .expect("an overcounted head lies in a store of the inventory");
                let head = Tid::new(f.pid.0, slot).expect("48-bit page id");
                store.recount(pager, head, versions)?;
            }
            _ => continue,
        }
        mended += 1;
    }
    Ok(mended)
}
