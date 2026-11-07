//! The damage the pager records: a log record at each detection, folded by each checkpoint into
//! one list page the file header names.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;

use crate::mvcc::Settling;
use crate::runtime::{Runtime, ThreadId};

use super::block::{LogFinding, LogProblem};
use super::log::Log;
use super::page::{self, HEADER_LEN, PageKind};
use super::pager::{Damage, Held, PageError, PageExclusive, Pager};
use super::quarantine::Quarantine;
use super::record::{Record, damage_of, damage_words};
use super::{Lsn, Pid};

const FORMAT: u16 = 1;
const COUNT: usize = HEADER_LEN;
const FLAGS: usize = COUNT + 4;
const ENTRIES: usize = FLAGS + 4;
/// An entry: the kind in the top byte of the page's word, then two LSNs.
const ENTRY: usize = 24;
const KIND_SHIFT: u32 = 56;
/// The kind of an entry naming a stale log block, with no page.
const LOG_STALE: u8 = 16;
const FULL: u32 = 1;

/// The damage recorded and not cleared.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recorded {
    pub pages: BTreeMap<Pid, Damage>,
    /// Stale blocks recovery found past the end of the log.
    pub log: BTreeSet<LogFinding>,
    /// Whether the list ran out of room, so the whole file counts as damaged.
    pub full: bool,
}

/// What the pager records of damage, under the meta lock.
#[derive(Debug, Default)]
pub(crate) struct Records {
    pub(crate) recorded: Recorded,
    /// Whether a detection logs its record. Off until the open's recovery ends.
    pub(crate) logging: bool,
    /// The list page, while a checkpoint may write it.
    pub(crate) list: Option<Pid>,
    /// What replay met past the checkpoint it started from, in log order.
    pub(crate) replayed: Vec<(Pid, Met)>,
    /// Whether the records differ from what the list page holds.
    pub(crate) unfolded: bool,
    /// The damage held in memory only, never logged or folded: found on a file quarantined
    /// whole, on a page freed and not allocated since, or in a file's copy while the pool holds a
    /// sound one.
    pub(crate) held: BTreeMap<Pid, Damage>,
    /// Each page's damage mapped this open, with the objects it quarantines.
    mapped: BTreeMap<Pid, (Damage, BTreeSet<Quarantine>)>,
    /// Whether a full list was mapped this open.
    full_mapped: bool,
    /// Each page's damage whose last mapping was incomplete.
    incomplete: BTreeMap<Pid, Damage>,
    /// The thread whose read last detected each page's damage.
    pub(crate) detected_by: BTreeMap<Pid, ThreadId>,
    /// The pages whose last record of their own this open logged or replayed is a free.
    freed: BTreeSet<Pid>,
    /// The pages of trees a swap replaced, each until its own free.
    pub(crate) retired: BTreeSet<Pid>,
}

/// A record that changes the damage recorded of its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Met {
    Damaged(Damage),
    /// A full-page image, which drops all damage but [`Damage::Settled`].
    Image,
    /// The first record of a page allocated or overwritten unread, which drops all damage.
    Start,
    Freed,
}

impl Recorded {
    /// Apply what `met` does to the damage recorded of `pid`. Returns whether that changes it.
    pub(crate) fn meet(&mut self, pid: Pid, met: Met) -> bool {
        match met {
            Met::Damaged(damage) => self.pages.insert(pid, damage) != Some(damage),
            Met::Image if matches!(self.pages.get(&pid), Some(Damage::Settled { .. })) => false,
            Met::Image | Met::Start | Met::Freed => self.pages.remove(&pid).is_some(),
        }
    }
}

impl Records {
    /// Record `damage` of `pid` and log it, unless recovery has not ended or `whole` says the file
    /// is quarantined whole, which keeps it in memory. Returns the log's end after the record.
    ///
    /// A settled page found stale at the LSNs it was settled at keeps its record: the settle's
    /// rewrite continues the page's chain from the stale copy. Damage of a page freed and not
    /// allocated since is held in memory, mapped this open and never logged; the page's next
    /// chain start drops it.
    pub(crate) fn detect(
        &mut self,
        log: &mut Log,
        pid: Pid,
        damage: Damage,
        whole: bool,
    ) -> Option<Lsn> {
        if damage == Damage::Unreadable || !self.logging {
            return None;
        }
        if let Damage::Stale { expected, found } = damage
            && self.page(pid) == Some(Damage::Settled { expected, found })
        {
            return None;
        }
        if whole || self.freed.contains(&pid) {
            self.held.insert(pid, damage);
            return None;
        }
        self.held.remove(&pid);
        self.meet(pid, Met::Damaged(damage));
        log.append(&Record::Damaged { pid, damage });
        Some(log.next_lsn())
    }

    /// Drop the damage recorded of `pid`, which a batch frees, and log the free in the batch
    /// unless recovery has not ended or `whole` says the file is quarantined whole. Returns the
    /// LSN of the record logged.
    pub(crate) fn free(&mut self, log: &mut Log, pid: Pid, whole: bool) -> Option<Lsn> {
        self.meet(pid, Met::Freed);
        (self.logging && !whole).then(|| log.append(&Record::Freed { pid }))
    }

    /// Note that `met`, a record of `pid`, ends or starts an incarnation of the page.
    pub(crate) fn bound(&mut self, pid: Pid, met: Met) {
        match met {
            Met::Freed => {
                self.freed.insert(pid);
                self.retired.remove(&pid);
            }
            Met::Start => {
                self.freed.remove(&pid);
            }
            Met::Damaged(_) | Met::Image => {}
        }
    }

    /// Apply what `met` does to the damage recorded of `pid`, to be folded, and to what is held of
    /// it. Returns whether that changes what is to be folded.
    pub(crate) fn meet(&mut self, pid: Pid, met: Met) -> bool {
        self.bound(pid, met);
        let settled = matches!(self.held.get(&pid), Some(Damage::Settled { .. }));
        if met == Met::Freed || met == Met::Start || met == Met::Image && !settled {
            self.held.remove(&pid);
        }
        let changed = self.recorded.meet(pid, met);
        self.unfolded |= changed;
        if self.page(pid).is_none() {
            self.mapped.remove(&pid);
            self.incomplete.remove(&pid);
            self.detected_by.remove(&pid);
        }
        changed
    }

    /// Drop the damage held of `pid`, but a settle, once the page is written out over the file's
    /// copy.
    pub(crate) fn replaced(&mut self, pid: Pid) {
        if !matches!(self.held.get(&pid), Some(Damage::Settled { .. })) {
            self.held.remove(&pid);
        }
    }

    /// Returns each page whose damage is not mapped this open, and whether a full list is not. A
    /// settled page maps to nothing, so it is never unmapped. `retried` false leaves out each
    /// page whose last mapping of the same damage was incomplete.
    fn unmapped(&self, retried: bool) -> (Vec<(Pid, Damage)>, bool) {
        let pages = self.all().pages.into_iter();
        let pages = pages.filter(|(pid, d)| {
            !matches!(d, Damage::Settled { .. })
                && self.mapped.get(pid).is_none_or(|m| m.0 != *d)
                && (retried || self.incomplete.get(pid) != Some(d))
        });
        (pages.collect(), self.recorded.full && !self.full_mapped)
    }

    fn any_unmapped(&self) -> bool {
        let (pages, full) = self.unmapped(true);
        full || !pages.is_empty()
    }

    /// Returns the damage recorded or held of `pid`.
    pub(crate) fn page(&self, pid: Pid) -> Option<Damage> {
        let held = self.held.get(&pid);
        held.or_else(|| self.recorded.pages.get(&pid)).copied()
    }

    /// Returns the damage recorded and held.
    pub(crate) fn all(&self) -> Recorded {
        let mut all = self.recorded.clone();
        all.pages.extend(&self.held);
        all
    }

    /// Returns what a fold writes, counting the list full when it holds more than `capacity`.
    fn snapshot(&mut self, capacity: usize) -> Recorded {
        self.unfolded = false;
        let r = &mut self.recorded;
        r.full |= r.pages.len() + r.log.len() > capacity;
        r.clone()
    }
}

fn word(page: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(page[at..at + 8].try_into().unwrap())
}

fn u32_at(page: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(page[at..at + 4].try_into().unwrap())
}

/// A damage record and what it quarantines, `None` when its mapping is incomplete.
type Mapping = (Pid, Damage, Option<BTreeSet<Quarantine>>);

/// Returns the list page `page` holds, `None` when it is no list page.
fn decode(page: &[u8]) -> Option<Recorded> {
    if page::kind(page) != PageKind::DamageList as u16 {
        return None;
    }
    let n = u32_at(page, COUNT) as usize;
    if n > (page.len() - ENTRIES) / ENTRY {
        return None;
    }
    let mut out = Recorded {
        full: u32_at(page, FLAGS) & FULL != 0,
        ..Recorded::default()
    };
    for i in 0..n {
        let at = ENTRIES + i * ENTRY;
        let head = word(page, at);
        let kind = (head >> KIND_SHIFT) as u8;
        let (a, b) = (Lsn(word(page, at + 8)), Lsn(word(page, at + 16)));
        if kind == LOG_STALE {
            out.log.insert(LogFinding {
                lsn: a,
                problem: LogProblem::Stale,
            });
        } else {
            let pid = Pid(head & ((1 << KIND_SHIFT) - 1));
            out.pages.insert(pid, damage_of(kind, a, b)?);
        }
    }
    Some(out)
}

/// Write `r` to list page `page`, its pages first, as many entries as fit.
fn encode(page: &mut [u8], r: &Recorded) {
    let pages = r.pages.iter().map(|(pid, &d)| {
        let (kind, a, b) = damage_words(d);
        assert!(
            pid.0 >> KIND_SHIFT == 0,
            "a page id leaves the top byte of its word for the kind"
        );
        (u64::from(kind) << KIND_SHIFT | pid.0, a, b)
    });
    let log = r.log.iter().map(|f| {
        assert_eq!(
            f.problem,
            LogProblem::Stale,
            "the list holds only stale log blocks"
        );
        (u64::from(LOG_STALE) << KIND_SHIFT, f.lsn, Lsn(0))
    });
    let capacity = (page.len() - ENTRIES) / ENTRY;
    page[HEADER_LEN..].fill(0);
    let mut n = 0;
    for (head, a, b) in pages.chain(log).take(capacity) {
        let at = ENTRIES + n * ENTRY;
        page[at..at + 8].copy_from_slice(&head.to_le_bytes());
        page[at + 8..at + 16].copy_from_slice(&a.0.to_le_bytes());
        page[at + 16..at + 24].copy_from_slice(&b.0.to_le_bytes());
        n += 1;
    }
    page[COUNT..COUNT + 4].copy_from_slice(&(n as u32).to_le_bytes());
    let flags = if r.full { FULL } else { 0 };
    page[FLAGS..FLAGS + 4].copy_from_slice(&flags.to_le_bytes());
}

impl<R: Runtime> Pager<R> {
    /// Returns the entries one list page holds.
    pub fn damage_list_capacity(&self) -> usize {
        (self.page_size - ENTRIES) / ENTRY
    }

    /// Returns every damage recorded and not cleared: the list's, the log's since and this
    /// open's.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(crate) fn recorded(&self) -> Recorded {
        self.meta().records.all()
    }

    /// Map each damage record not mapped this open to the objects it quarantines, and mark each
    /// object through `mark`. `holders` returns what the damage of a page quarantines, `None`
    /// when the mapping is incomplete, which leaves the record unmapped. Every record of a full
    /// list maps to the database with no walk. Returns each object whose mark failed, with why.
    ///
    /// Precondition: the calling thread holds no page latch, and neither the log lock nor the
    /// meta lock.
    ///
    /// Postcondition: a record is mapped only once the marks of its objects are set, so a caller
    /// that finds it unmapped may find those marks unset.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub fn map_damage(
        &self,
        holders: impl FnMut(Pid) -> Option<BTreeSet<Quarantine>>,
        mark: impl FnMut(Quarantine) -> Result<(), PageError>,
    ) -> BTreeMap<Quarantine, PageError> {
        if !self.any_unmapped.load(Ordering::Acquire) {
            return BTreeMap::new();
        }
        let (pages, full) = self.meta().records.unmapped(true);
        self.map_records(pages, full, holders, mark)
    }

    /// Map as [`Pager::map_damage`] does for a commit, leaving out each record whose last mapping
    /// was incomplete. `holders` adds to its set each page whose damage stopped a walk. Map to
    /// the database, with no walk, each such page with no record when the mapping began whose
    /// damage this thread detected.
    /// Returns each object whose mark failed, with why.
    ///
    /// Precondition: as for [`Pager::map_damage`].
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub fn map_committing(
        &self,
        mut holders: impl FnMut(Pid, &mut BTreeSet<Pid>) -> Option<BTreeSet<Quarantine>>,
        mut mark: impl FnMut(Quarantine) -> Result<(), PageError>,
    ) -> BTreeMap<Quarantine, PageError> {
        if !self.any_unmapped.load(Ordering::Acquire) {
            return BTreeMap::new();
        }
        let (pages, full, before) = {
            let m = self.meta();
            let (pages, full) = m.records.unmapped(false);
            (pages, full, m.records.all().pages)
        };
        let mut met = BTreeSet::new();
        let mut failed = self.map_records(pages, full, |pid| holders(pid, &mut met), &mut mark);
        let me = self.rt.current();
        let m = self.meta();
        met.retain(|pid| !before.contains_key(pid) && m.records.detected_by.get(pid) == Some(&me));
        drop(m);
        if met.is_empty() {
            return failed;
        }
        let (pages, _) = self.meta().records.unmapped(true);
        let pages = pages.into_iter().filter(|(pid, _)| met.contains(pid));
        let database = || Some(BTreeSet::from([Quarantine::Database]));
        failed.extend(self.map_records(pages.collect(), false, |_| database(), mark));
        failed
    }

    /// Map each of `pages` to what `holders` says it quarantines, or every page to the database
    /// when `full`, and mark each object through `mark`. Returns each object whose mark failed,
    /// with why.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn map_records(
        &self,
        pages: Vec<(Pid, Damage)>,
        full: bool,
        mut holders: impl FnMut(Pid) -> Option<BTreeSet<Quarantine>>,
        mut mark: impl FnMut(Quarantine) -> Result<(), PageError>,
    ) -> BTreeMap<Quarantine, PageError> {
        let database = BTreeSet::from([Quarantine::Database]);
        let mut mapped: Vec<Mapping> = Vec::new();
        for (pid, damage) in pages {
            let objects = if full {
                Some(database.clone())
            } else {
                holders(pid)
            };
            mapped.push((pid, damage, objects));
        }
        let mut objects: BTreeSet<Quarantine> = mapped
            .iter()
            .flat_map(|m| m.2.iter().flatten().copied())
            .collect();
        if full {
            objects.insert(Quarantine::Database);
        }
        let failed = objects
            .into_iter()
            .filter_map(|o| mark(o).err().map(|e| (o, e)))
            .collect();
        let mut m = self.meta();
        let records = &mut m.records;
        records.full_mapped |= full;
        for (pid, damage, objects) in mapped {
            if records.page(pid) != Some(damage) {
                continue;
            }
            match objects {
                Some(objects) => {
                    records.incomplete.remove(&pid);
                    records.mapped.insert(pid, (damage, objects));
                }
                None => {
                    records.incomplete.insert(pid, damage);
                }
            }
        }
        self.note_unmapped(records);
        failed
    }

    /// Returns how many damaged pages this open has walked the objects for.
    #[cfg(feature = "testing")]
    pub(crate) fn walked(&self) -> u64 {
        self.walked.load(Ordering::Relaxed)
    }

    /// Returns what the damage of `pid` was mapped to this open, `None` while it is not mapped.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub fn mapped_to(&self, pid: Pid) -> Option<BTreeSet<Quarantine>> {
        let m = self.meta();
        let (damage, objects) = m.records.mapped.get(&pid)?;
        (m.records.page(pid) == Some(*damage)).then(|| objects.clone())
    }

    /// Refuse the page under `latch` as `damage` from now on and record it, unless it is refused
    /// as `damage` already. Returns the end of the damage record logged, `None` when none is.
    ///
    /// Postcondition: a reader that takes the latch next finds the page refused.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(super) fn publish_refusal(&self, latch: &Held<'_, R>, damage: Damage) -> Option<Lsn> {
        let pid = latch.pid();
        let mut log = self.log();
        let whole = self.is_quarantined(Quarantine::Database);
        let mut m = self.meta();
        let fresh = m.damaged.insert(pid, damage) != Some(damage);
        m.changed.remove(&pid);
        let logged = fresh
            .then(|| m.records.detect(&mut log, pid, damage, whole))
            .flatten();
        if m.records.page(pid) == Some(damage) {
            m.records.detected_by.insert(pid, self.rt.current());
        }
        self.note_unmapped(&m.records);
        logged
    }

    /// Set whether `records` holds a record not mapped this open.
    pub(super) fn note_unmapped(&self, records: &Records) {
        self.any_unmapped
            .store(records.any_unmapped(), Ordering::Release);
    }

    /// Returns the page listing the damage recorded, `Pid(0)` while the file has none.
    pub fn damage_list_page(&self) -> Pid {
        self.header.lock(&self.rt).0.damage_list
    }

    /// Record that the commit log settles the page `refusal` refused from the log. Call before the
    /// page's rewrite.
    ///
    /// Panic if `refusal` is not a lost write.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(crate) fn record_settled(&self, _: &Settling, refusal: PageError) {
        let Damage::Stale { expected, found } = refusal.damage else {
            panic!("only a status page that lost a write is settled");
        };
        let damage = Damage::Settled { expected, found };
        let mut log = self.log();
        let whole = self.is_quarantined(Quarantine::Database);
        self.meta()
            .records
            .detect(&mut log, refusal.pid, damage, whole);
    }

    /// Hold `damage` of `pid` in memory, unlogged, leaving the page's memory as it is: for a
    /// file's copy that fails while the pool holds a sound one. The hold ends at the page's next
    /// write-out, and lasts the open when the page is never written out.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(super) fn hold_damage(&self, pid: Pid, damage: Damage) {
        if damage == Damage::Unreadable {
            return;
        }
        let mut m = self.meta();
        m.records.held.insert(pid, damage);
        self.note_unmapped(&m.records);
    }

    /// Allocate and format an empty list page and name it in the header.
    fn format_damage_list(&self) -> Result<Pid, PageError> {
        let mut new = self.reserve();
        let pid = new.page(FORMAT, PageKind::DamageList, 0)?;
        self.update_with(new, &[pid], |p| encode(p[0], &Recorded::default()))?;
        self.header.lock(&self.rt).0.damage_list = pid;
        Ok(pid)
    }

    /// Give a new file its empty list page, recording from now on.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(super) fn create_damage_list(&self) -> Result<(), PageError> {
        let pid = self.format_damage_list()?;
        let mut m = self.meta();
        m.records.list = Some(pid);
        m.records.logging = true;
        Ok(())
    }

    /// Read the list page into memory, then what replay met past it and the log findings the
    /// header holds. Create the page when the file has none and is not quarantined whole. A
    /// damaged list page quarantines the database, an unreadable one for this open only.
    ///
    /// Fail if the page cannot be created.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(super) fn load_damage_list(&self) -> Result<(), PageError> {
        let (pid, header_log, header_full) = {
            let h = self.header.lock(&self.rt);
            (h.0.damage_list, h.0.log_damage.clone(), h.0.log_damage_full)
        };
        let base = if pid != Pid(0) {
            let read = self.read(pid).map(|p| decode(&p));
            match read {
                Ok(Some(base)) => Some(base),
                Ok(None) => {
                    self.refuse_list(self.damage(pid, Damage::Checksum));
                    None
                }
                Err(e) => {
                    self.refuse_list(e);
                    None
                }
            }
        } else if self.is_quarantined(Quarantine::Database) {
            None
        } else {
            self.format_damage_list()?;
            Some(Recorded::default())
        };
        let mut m = self.meta();
        let records = &mut m.records;
        let sound = base.is_some();
        let mut recorded = base.unwrap_or_default();
        let mut unfolded = !header_log.is_empty() || header_full;
        for (pid, met) in std::mem::take(&mut records.replayed) {
            unfolded |= recorded.meet(pid, met);
        }
        recorded.log.extend(header_log);
        recorded.full |= header_full;
        records.recorded = recorded;
        records.unfolded = unfolded;
        self.note_unmapped(records);
        let list = self.header.lock(&self.rt).0.damage_list;
        records.list = sound.then_some(list);
        Ok(())
    }

    /// Stop folding into the list page, which `e` refused, and quarantine the database: for this
    /// open only when the page cannot be read.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn refuse_list(&self, e: PageError) {
        self.meta().records.list = None;
        if e.damage != Damage::Unreadable {
            self.quarantine(Quarantine::Database).ok();
        }
        self.hold_quarantine(Quarantine::Database);
    }

    /// Record every page recovery and the open refused, and log the records from now on, unless
    /// the file is quarantined whole.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(super) fn start_recording(&self) {
        let mut log = self.log();
        let whole = self.is_quarantined(Quarantine::Database);
        let mut m = self.meta();
        m.records.logging = true;
        let refused: Vec<(Pid, Damage)> = m.damaged.iter().map(|(&p, &d)| (p, d)).collect();
        for (pid, damage) in refused {
            m.records.detect(&mut log, pid, damage, whole);
        }
        self.note_unmapped(&m.records);
    }

    /// Returns the list page when a checkpoint is to fold the records into it: once recovery has
    /// ended, while the page serves and the records differ from it.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(super) fn to_fold(&self) -> Option<Pid> {
        let m = self.meta();
        let r = &m.records;
        if !r.logging || !r.unfolded {
            return None;
        }
        r.list
    }

    /// Returns the latch on the list page when [`Pager::to_fold`] names it.
    pub(super) fn list_to_fold(&self) -> Option<PageExclusive<'_, R>> {
        let pid = self.to_fold()?;
        match self.exclusive(pid) {
            Ok(g) => Some(g),
            Err(e) => {
                self.refuse_list(e);
                None
            }
        }
    }

    /// Returns what a fold writes, for a checkpoint holding the log lock.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(super) fn records_to_fold(&self) -> Recorded {
        let capacity = self.damage_list_capacity();
        self.meta().records.snapshot(capacity)
    }

    /// Write `r` to the list page under `g`, logging nothing when it holds `r` already.
    pub(super) fn fold_records(g: &mut PageExclusive<'_, R>, r: &Recorded) {
        g.update(|p| encode(p, r));
    }
}
