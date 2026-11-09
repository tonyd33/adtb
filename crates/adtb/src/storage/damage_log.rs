//! The damage the pager records: a log record at each detection, folded by each checkpoint into
//! one list page the file header names.

use std::collections::{BTreeMap, BTreeSet};

use crate::mvcc::Settling;
use crate::runtime::Runtime;

use super::block::{LogFinding, LogProblem};
use super::error::Damage;
use super::evidence::{Begun, Mapping, Met, meet_recorded};
use super::page::{self, HEADER_LEN, PageKind};
use super::pager::{PageError, PageExclusive, Pager};
use super::policy::{self, Fault, Holder, Phase};
use super::quarantine::Quarantine;
use super::record::{damage_of, damage_words};
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

impl Recorded {
    /// Apply what `met` does to the damage recorded of `pid`. Returns whether that changes it.
    pub(super) fn meet(&mut self, pid: Pid, met: Met) -> bool {
        let mut recorded = self.pages.remove(&pid);
        let changed = meet_recorded(&mut recorded, met);
        if let Some(damage) = recorded {
            self.pages.insert(pid, damage);
        }
        changed
    }
}

fn word(page: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(page[at..at + 8].try_into().unwrap())
}

fn u32_at(page: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(page[at..at + 4].try_into().unwrap())
}

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
        (self.page_size() - ENTRIES) / ENTRY
    }

    /// Returns every damage recorded and not cleared: the list's, the log's since and this
    /// open's.
    pub(crate) fn recorded(&self) -> Recorded {
        self.evidence().all()
    }

    /// Map each damage record not mapped this open to the objects it quarantines during `phase`,
    /// and mark each object through `mark`. `holders` returns what holds a page, `None` when the
    /// mapping is incomplete, which leaves the record unmapped. Every record of a full list maps
    /// with no walk. Returns each object whose mark failed, with why.
    ///
    /// Precondition: the calling thread holds no page latch, and neither the log lock nor the
    /// meta lock.
    ///
    /// Postconditions:
    ///
    /// - A record is mapped only once the marks of its objects are set, so a caller that finds it
    ///   unmapped may find those marks unset.
    /// - A record whose damage or incarnation changes before its walk ends sets no mark and is not
    ///   mapped; the record that replaced it is unmapped, for the next mapping.
    pub fn map_damage(
        &self,
        phase: Phase,
        holders: impl FnMut(Pid) -> Option<BTreeSet<Holder>>,
        mark: impl FnMut(Quarantine) -> Result<(), PageError>,
    ) -> BTreeMap<Quarantine, PageError> {
        if !self.any_unmapped() {
            return BTreeMap::new();
        }
        let begun = self.evidence().begin_mapping(true);
        self.map_records(begun, phase, holders, mark)
    }

    /// Returns whether a damage record of a page is not mapped this open, leaving out a full list.
    pub(crate) fn any_record_unmapped(&self) -> bool {
        self.any_unmapped() && !self.evidence().begin_mapping(true).0.is_empty()
    }

    /// Map as [`Pager::map_damage`] does for a commit, leaving out each record whose last mapping
    /// was incomplete. `holders` adds to its set each page whose damage stopped a walk. Map as
    /// met, with no walk, each such page with no record when the mapping began whose damage this
    /// thread detected.
    /// Returns each object whose mark failed, with why.
    ///
    /// Precondition: as for [`Pager::map_damage`].
    pub fn map_committing(
        &self,
        mut holders: impl FnMut(Pid, &mut BTreeSet<Pid>) -> Option<BTreeSet<Holder>>,
        mut mark: impl FnMut(Quarantine) -> Result<(), PageError>,
    ) -> BTreeMap<Quarantine, PageError> {
        if !self.any_unmapped() {
            return BTreeMap::new();
        }
        let (begun, before) = {
            let e = self.evidence();
            (e.begin_mapping(false), e.all().pages)
        };
        let mut met = BTreeSet::new();
        let running = Phase::Running;
        let mut failed = self.map_records(begun, running, |pid| holders(pid, &mut met), &mut mark);
        let me = self.rt().current();
        let e = self.evidence();
        met.retain(|pid| !before.contains_key(pid) && e.detected_by(*pid) == Some(me));
        drop(e);
        if met.is_empty() {
            return failed;
        }
        let (mut pages, _, epoch) = self.evidence().begin_mapping(true);
        pages.retain(|p| met.contains(&p.pid()));
        let met = |_| Some(BTreeSet::from([Holder::Met]));
        failed.extend(self.map_records((pages, false, epoch), running, met, mark));
        failed
    }

    /// Find the holders of each of `pages` with `holders` and map the page to what they mark
    /// during `phase`; when `full`, map every page as a full list does. Mark through `mark` each
    /// object of a record still current since `begun`. Returns each object whose mark failed,
    /// with why.
    fn map_records(
        &self,
        (pages, full, begun): Begun,
        phase: Phase,
        mut holders: impl FnMut(Pid) -> Option<BTreeSet<Holder>>,
        mut mark: impl FnMut(Quarantine) -> Result<(), PageError>,
    ) -> BTreeMap<Quarantine, PageError> {
        let overflow = policy::marks(Fault::Full, [Holder::DamageList], phase);
        let mut mapped: Vec<Mapping> = Vec::new();
        for p in pages {
            let objects = if full {
                Some(overflow.clone())
            } else {
                holders(p.pid()).map(|held| policy::marks(Fault::Page(p.damage()), held, phase))
            };
            mapped.push((p, objects));
        }
        let mapped = self.evidence().current(begun, mapped);
        let mut objects: BTreeSet<Quarantine> = mapped
            .iter()
            .flat_map(|m| m.1.iter().flatten().copied())
            .collect();
        if full {
            objects.extend(overflow);
        }
        let failed = objects
            .into_iter()
            .filter_map(|o| mark(o).err().map(|e| (o, e)))
            .collect();
        self.evidence().mapped(full, begun, mapped);
        failed
    }

    /// Returns how many damaged pages this open has walked the objects for.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn walked(&self) -> u64 {
        self.evidence().walked()
    }

    /// Returns what the damage of `pid` was mapped to this open, `None` while it is not mapped.
    pub fn mapped_to(&self, pid: Pid) -> Option<BTreeSet<Quarantine>> {
        self.evidence().mapped_to(pid)
    }

    /// Returns the page listing the damage recorded, `Pid(0)` while the file has none.
    pub fn damage_list_page(&self) -> Pid {
        self.evidence().list_page()
    }

    /// Record that the commit log settles the page `refusal` refused from the log. Call before the
    /// page's rewrite.
    ///
    /// Panic if `refusal` is not a lost write.
    pub(super) fn record_settled(&self, _: &Settling, refusal: PageError) {
        let Damage::Stale { expected, found } = refusal.damage else {
            panic!("only a status page that lost a write is settled");
        };
        let damage = Damage::Settled { expected, found };
        self.append().record_damage(refusal.pid, damage);
    }

    /// Allocate and format an empty list page. Returns the page.
    fn format_damage_list(&self) -> Result<Pid, PageError> {
        let mut new = self.reserve();
        let pid = new.page(FORMAT, PageKind::DamageList, 0)?;
        self.update_with(new, &[pid], |p| encode(p[0], &Recorded::default()))?;
        Ok(pid)
    }

    /// Give a new file its empty list page, recording from now on.
    pub(super) fn create_damage_list(&self) -> Result<(), PageError> {
        let pid = self.format_damage_list()?;
        self.evidence().create_list(pid);
        Ok(())
    }

    /// Read the list page into memory, then what replay met past it and the log findings the
    /// header holds. Create the page when the file has none and is not quarantined whole. A
    /// damaged list page quarantines the database, an unreadable one for this open only.
    ///
    /// Fail if the page cannot be created.
    pub(super) fn load_damage_list(&self) -> Result<(), PageError> {
        let (header_log, header_full) =
            self.read_header(|h| (h.log_damage.clone(), h.log_damage_full));
        let mut list = self.damage_list_page();
        let base = if list != Pid(0) {
            let pid = list;
            let read = self.read(pid).map(|p| decode(&p));
            match read {
                Ok(Some(base)) => Some(base),
                Ok(None) => {
                    self.refuse_list(self.damage(pid, Damage::Checksum), Phase::Opening);
                    None
                }
                Err(e) => {
                    self.refuse_list(e, Phase::Opening);
                    None
                }
            }
        } else if self.is_quarantined(Quarantine::Database) {
            None
        } else {
            list = self.format_damage_list()?;
            Some(Recorded::default())
        };
        self.evidence()
            .load_list(base, list, header_log, header_full);
        Ok(())
    }

    /// Stop folding into the list page, which `e` refused during `phase`, and set what its damage
    /// marks.
    fn refuse_list(&self, e: PageError, phase: Phase) {
        self.evidence().drop_list();
        let mark = policy::mark(Fault::Page(e.damage), Holder::DamageList, phase);
        if let Some(object) = mark.durable() {
            self.quarantine(object).ok();
        }
        if let Some(object) = mark.held() {
            self.hold_quarantine(object);
        }
    }

    /// Record every page recovery and the open refused, and log the records from now on, unless
    /// the file is quarantined whole.
    pub(super) fn start_recording(&self) {
        self.append().start_recording();
    }

    /// Returns the list page when a checkpoint is to fold the records into it: once recovery has
    /// ended, while the page serves and the records differ from it.
    pub(super) fn to_fold(&self) -> Option<Pid> {
        self.evidence().to_fold()
    }

    /// Returns the latch on the list page when [`Pager::to_fold`] names it.
    pub(super) fn list_to_fold(&self) -> Option<PageExclusive<'_, R>> {
        let pid = self.to_fold()?;
        match self.exclusive(pid) {
            Ok(g) => Some(g),
            Err(e) => {
                self.refuse_list(e, Phase::Running);
                None
            }
        }
    }

    /// Write `r` to the list page under `g`, logging nothing when it holds `r` already.
    pub(super) fn fold_records(g: &mut PageExclusive<'_, R>, r: &Recorded) {
        g.update(|p| encode(p, r));
    }
}
