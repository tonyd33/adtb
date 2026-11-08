//! What the pager knows of each page beyond its bytes: the LSN a copy read from the file must
//! reach, whether the page is refused and why, the damage recorded of it, in the log or held in
//! memory, and what that damage was mapped to this open.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::runtime::{AlignedBuf, Runtime, ThreadId};

use super::block::LogFinding;
use super::damage_log::Recorded;
use super::error::Damage;
use super::log::Log;
use super::lsnmap::retained;
use super::page::{self, Verdict};
use super::pager::{Appending, Held, Pager, Wrote};
use super::quarantine::Quarantine;
use super::record::Record;
use super::recovery::{Armed, Replaying};
use super::{Lsn, Pid};

/// What a page read in from the file is checked against.
#[derive(Debug)]
enum Gate {
    /// Recovery is replaying the log; no page has an expected LSN until it ends.
    Recovering,
    Armed(Expected),
    /// The open could not read the map or prove the retained log: the database is held
    /// quarantined and no page is checked.
    Off,
}

impl Gate {
    fn expected(&self, pid: Pid) -> Option<Lsn> {
        match self {
            Gate::Armed(e) => e.get(pid),
            Gate::Recovering | Gate::Off => None,
        }
    }
}

/// The LSN each page must have at least when read in.
#[derive(Debug, Default)]
struct Expected {
    /// The page LSN map's entry of each page, by pid, zero for none.
    folded: Vec<Lsn>,
    /// Each page whose last record is past its folded entry, with that record's LSN.
    logged: HashMap<Pid, Lsn>,
}

impl Expected {
    fn get(&self, pid: Pid) -> Option<Lsn> {
        let folded = self.folded.get(pid.0 as usize).copied();
        self.logged.get(&pid).copied().or(folded)
    }

    fn end(&self) -> u64 {
        let folded = self
            .folded
            .iter()
            .rposition(|&l| l > Lsn(0))
            .map_or(0, |i| i + 1);
        let logged = self.logged.keys().map(|p| p.0 + 1).max().unwrap_or(0);
        (folded as u64).max(logged)
    }

    fn absorb<'a>(&mut self, last: impl Iterator<Item = (&'a Pid, &'a Lsn)>) {
        for (&pid, &lsn) in last {
            let at = pid.0 as usize;
            if self.folded.len() <= at {
                self.folded.resize(at + 1, Lsn(0));
            }
            self.folded[at] = self.folded[at].max(lsn);
            if self.logged.get(&pid).is_some_and(|&l| l <= self.folded[at]) {
                self.logged.remove(&pid);
            }
        }
    }

    /// Raise `pid`'s expected LSN to `lsn`, the LSN of its latest record.
    fn raise(&mut self, pid: Pid, lsn: Lsn) {
        if self.folded.get(pid.0 as usize).is_none_or(|&f| f < lsn) {
            self.logged.insert(pid, lsn);
        }
    }
}

/// A record that changes the damage recorded of its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Met {
    Damaged(Damage),
    /// A full-page image, which drops all damage but [`Damage::Settled`].
    Image,
    /// The first record of a page allocated or overwritten unread, which drops all damage.
    Start,
    Freed,
}

/// Apply what `met` does to `recorded`, the damage recorded of a page. Returns whether that
/// changes it.
pub(super) fn meet_recorded(recorded: &mut Option<Damage>, met: Met) -> bool {
    match met {
        Met::Damaged(damage) => recorded.replace(damage) != Some(damage),
        Met::Image if matches!(recorded, Some(Damage::Settled { .. })) => false,
        Met::Image | Met::Start | Met::Freed => recorded.take().is_some(),
    }
}

/// The refusal of a page whose resident copy is dropped.
pub(super) struct Refusal(Pid);

impl Refusal {
    /// Returns the refusal of the page under `latch`.
    pub(super) fn of<R: Runtime>(latch: &Held<'_, R>) -> Refusal {
        Refusal(latch.pid())
    }

    pub(super) fn pid(&self) -> Pid {
        self.0
    }
}

#[cfg(test)]
pub(super) fn refusal_for_test(pid: Pid) -> Refusal {
    Refusal(pid)
}

/// A damage record and what it quarantines, `None` when its mapping is incomplete.
pub(super) type Mapping = (Pid, Damage, Option<BTreeSet<Quarantine>>);

/// A file's copy of a page as read: its LSN and verdict, or why it cannot be read.
pub(super) type FileCopy = Result<(Lsn, Verdict), Damage>;

/// Returns the file's copy `read` as [`Pager::judge`] takes it.
pub(super) fn file_copy(read: &Result<(AlignedBuf, Verdict), Damage>) -> FileCopy {
    match read {
        Ok((buf, verdict)) => Ok((page::lsn(buf), *verdict)),
        Err(damage) => Err(*damage),
    }
}

/// What a file's copy of a page must reach besides staying behind the log's end.
#[derive(Clone, Copy, Debug)]
pub(super) enum Against {
    /// The page's expected LSN.
    Expected,
    /// The LSN of the pool's copy under the caller's latch.
    Resident(Lsn),
    /// Nothing more.
    LogEnd,
}

/// The judgement of a file's copy of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Judged {
    Sound,
    /// The copy cannot be read or fails its checksum.
    Bad(Damage),
    /// The copy's LSN is at or past the log's end, or below what it must reach.
    Stale(Damage),
}

impl Judged {
    pub(super) fn damage(self) -> Option<Damage> {
        match self {
            Judged::Sound => None,
            Judged::Bad(d) | Judged::Stale(d) => Some(d),
        }
    }
}

/// What the pager holds of one page with something to say.
#[derive(Debug, Default)]
struct PageEvidence {
    /// Why the page is never handed out.
    refusal: Option<Damage>,
    /// The damage recorded in the log or the list, to be folded.
    recorded: Option<Damage>,
    /// The damage held in memory only, never logged or folded: found on a file quarantined
    /// whole, on a page freed and not allocated since, or in a file's copy while the pool holds
    /// a sound one.
    held: Option<Damage>,
    /// The damage mapped this open, with the objects it quarantines.
    mapped: Option<(Damage, BTreeSet<Quarantine>)>,
    /// The damage whose last mapping was incomplete.
    incomplete: Option<Damage>,
    /// The thread whose read last detected the damage.
    detected_by: Option<ThreadId>,
    /// A page of a tree a swap replaced, until its own free.
    retired: bool,
}

impl PageEvidence {
    fn is_empty(&self) -> bool {
        self.refusal.is_none()
            && self.recorded.is_none()
            && self.held.is_none()
            && self.mapped.is_none()
            && self.incomplete.is_none()
            && self.detected_by.is_none()
            && !self.retired
    }

    fn damage(&self) -> Option<Damage> {
        self.held.or(self.recorded)
    }

    /// Returns the damage not mapped this open. A settled page maps to nothing, so it is never
    /// unmapped. `retried` false leaves it out when its last mapping was incomplete.
    fn unmapped(&self, retried: bool) -> Option<Damage> {
        let d = self.damage()?;
        let unmapped = !matches!(d, Damage::Settled { .. })
            && self.mapped.as_ref().is_none_or(|m| m.0 != d)
            && (retried || self.incomplete != Some(d));
        unmapped.then_some(d)
    }
}

/// Whether the evidence holds a record not mapped this open, read without the meta lock.
#[derive(Debug, Clone)]
pub(super) struct Unmapped(Arc<AtomicBool>);

impl Unmapped {
    pub(super) fn any(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// The evidence of every page, under the meta lock.
///
/// Invariants:
///
/// - A page has an entry only while it has something to say.
/// - An entry with no damage recorded or held has no mapping and no detecting thread.
/// - `unmapped_pages` counts the entries whose damage is unmapped, and the [`Unmapped`] flag is
///   set exactly while one is or a full list is unmapped. Every method that changes either sets
///   both before it returns.
#[derive(Debug)]
pub(super) struct Evidence {
    pages: BTreeMap<Pid, PageEvidence>,
    gate: Gate,
    /// The end of the log recovery kept when the pager opened, or zero for a pager created empty.
    recovered: Lsn,
    /// Whether a detection logs its record. Off until the open's recovery ends.
    logging: bool,
    /// The list page, while a checkpoint may write it.
    list: Option<Pid>,
    /// What replay met past the checkpoint it started from, in log order.
    replayed: Vec<(Pid, Met)>,
    /// Whether the records differ from what the list page holds.
    unfolded: bool,
    /// Stale blocks recovery found past the end of the log.
    log: BTreeSet<LogFinding>,
    /// Whether the list ran out of room, so the whole file counts as damaged.
    full: bool,
    full_mapped: bool,
    /// The pages whose last record of their own this open logged or replayed is a free.
    freed: BTreeSet<Pid>,
    unmapped_pages: usize,
    unmapped: Unmapped,
}

impl Evidence {
    pub(super) fn new() -> Evidence {
        Evidence {
            pages: BTreeMap::new(),
            gate: Gate::Recovering,
            recovered: Lsn(0),
            logging: false,
            list: None,
            replayed: Vec::new(),
            unfolded: false,
            log: BTreeSet::new(),
            full: false,
            full_mapped: false,
            freed: BTreeSet::new(),
            unmapped_pages: 0,
            unmapped: Unmapped(Arc::new(AtomicBool::new(false))),
        }
    }

    /// Returns the flag of whether a record is unmapped, read without the meta lock.
    pub(super) fn unmapped_flag(&self) -> Unmapped {
        self.unmapped.clone()
    }

    /// Apply `f` to the evidence of `pid`, drop its mapping once it has no damage and the entry
    /// once it says nothing, and set the unmapped count and flag.
    fn update<T>(&mut self, pid: Pid, f: impl FnOnce(&mut PageEvidence) -> T) -> T {
        let p = self.pages.entry(pid).or_default();
        let before = p.unmapped(true).is_some();
        let out = f(p);
        if p.damage().is_none() {
            p.mapped = None;
            p.incomplete = None;
            p.detected_by = None;
        }
        let after = p.unmapped(true).is_some();
        if p.is_empty() {
            self.pages.remove(&pid);
        }
        self.unmapped_pages = self.unmapped_pages + usize::from(after) - usize::from(before);
        self.publish();
        out
    }

    fn publish(&self) {
        let any = self.unmapped_pages > 0 || self.full && !self.full_mapped;
        self.unmapped.0.store(any, Ordering::Release);
    }

    /// Record `damage` of `pid` and log it, unless recovery has not ended or `whole` says the
    /// file is quarantined whole, which keeps it in memory. Returns the log's end after the record.
    ///
    /// A settled page found stale at the LSNs it was settled at keeps its record: the settle's
    /// rewrite continues the page's chain from the stale copy. Damage of a page freed and not
    /// allocated since is held in memory, mapped this open and never logged; the page's next
    /// chain start drops it.
    fn detect(&mut self, log: &mut Log, pid: Pid, damage: Damage, whole: bool) -> Option<Lsn> {
        if damage == Damage::Unreadable || !self.logging {
            return None;
        }
        if let Damage::Stale { expected, found } = damage
            && self.damage(pid) == Some(Damage::Settled { expected, found })
        {
            return None;
        }
        if whole || self.freed.contains(&pid) {
            self.update(pid, |p| p.held = Some(damage));
            return None;
        }
        self.meet(pid, Met::Damaged(damage));
        log.append(&Record::Damaged { pid, damage });
        Some(log.next_lsn())
    }

    /// Note that `met`, a record of `pid`, ends or starts an incarnation of the page.
    fn bound(&mut self, pid: Pid, met: Met) {
        match met {
            Met::Freed => {
                self.freed.insert(pid);
                if self.pages.contains_key(&pid) {
                    self.update(pid, |p| p.retired = false);
                }
            }
            Met::Start => {
                self.freed.remove(&pid);
            }
            Met::Damaged(_) | Met::Image => {}
        }
    }

    /// Apply what `met` does to the damage recorded of `pid`, to be folded, and to what is held of
    /// it. Returns whether that changes what is to be folded.
    fn meet(&mut self, pid: Pid, met: Met) -> bool {
        self.bound(pid, met);
        if !matches!(met, Met::Damaged(_)) && !self.pages.contains_key(&pid) {
            return false;
        }
        let changed = self.update(pid, |p| {
            let settled = matches!(p.held, Some(Damage::Settled { .. }));
            if met != Met::Image || !settled {
                p.held = None;
            }
            meet_recorded(&mut p.recorded, met)
        });
        self.unfolded |= changed;
        changed
    }

    /// Refuse the page `refusal` names as `damage` from now on and record it, unless it is refused
    /// as `damage` already, noting `by` as the thread that detected it. Returns the end of the
    /// damage record logged, `None` when none is.
    pub(super) fn refuse(
        &mut self,
        _: &Appending,
        log: &mut Log,
        refusal: &Refusal,
        damage: Damage,
        whole: bool,
        by: ThreadId,
    ) -> Option<Lsn> {
        let pid = refusal.pid();
        let fresh = self.update(pid, |p| p.refusal.replace(damage) != Some(damage));
        let logged = if fresh {
            self.detect(log, pid, damage, whole)
        } else {
            None
        };
        if self.damage(pid) == Some(damage) {
            self.update(pid, |p| p.detected_by = Some(by));
        }
        logged
    }

    /// Record `damage` of `pid` without refusing it, logging it unless recovery has not ended or
    /// `whole` says the file is quarantined whole. Returns the log's end after the record.
    pub(super) fn record(
        &mut self,
        _: &Appending,
        log: &mut Log,
        pid: Pid,
        damage: Damage,
        whole: bool,
    ) -> Option<Lsn> {
        self.detect(log, pid, damage, whole)
    }

    /// Record every page refused so far, and log the records from now on.
    pub(super) fn start_recording(&mut self, _: &Appending, log: &mut Log, whole: bool) {
        self.logging = true;
        for (pid, damage) in self.refusals() {
            self.detect(log, pid, damage, whole);
        }
    }

    /// Raise the expected LSN of `pid` to `lsn`, the LSN of its record just logged, and apply
    /// `met` when that record is a full image.
    pub(super) fn logged_page(&mut self, _: &Appending, pid: Pid, lsn: Lsn, met: Option<Met>) {
        if let Gate::Armed(e) = &mut self.gate {
            e.raise(pid, lsn);
        }
        if let Some(met) = met {
            self.meet(pid, met);
        }
    }

    /// Drop the damage recorded of `pid`, which a batch frees, and log the free in the batch
    /// unless recovery has not ended or `whole` says the file is quarantined whole. Returns the
    /// LSN of the record logged.
    pub(super) fn free(
        &mut self,
        _: &Appending,
        log: &mut Log,
        pid: Pid,
        whole: bool,
    ) -> Option<Lsn> {
        self.meet(pid, Met::Freed);
        (self.logging && !whole).then(|| log.append(&Record::Freed { pid }))
    }

    /// Drop the damage held of the page `wrote` wrote, but a settle, now the page is written out
    /// over the file's copy.
    pub(super) fn written_out(&mut self, wrote: &Wrote) {
        let pid = wrote.pid();
        if self.pages.get(&pid).is_some_and(|p| p.held.is_some()) {
            self.update(pid, |p| {
                if !matches!(p.held, Some(Damage::Settled { .. })) {
                    p.held = None;
                }
            });
        }
    }

    /// Hold `damage` of `pid` in memory, unlogged, leaving its refusal as it is: for a file's copy
    /// that fails while the pool holds a sound one. The hold ends at the page's next write-out, and
    /// lasts the open when the page is never written out.
    pub(super) fn hold(&mut self, pid: Pid, damage: Damage) {
        if damage != Damage::Unreadable {
            self.update(pid, |p| p.held = Some(damage));
        }
    }

    /// Stop refusing `pid`. Returns its refusal.
    pub(super) fn unrefuse(&mut self, pid: Pid) -> Option<Damage> {
        if !self.pages.contains_key(&pid) {
            return None;
        }
        self.update(pid, |p| p.refusal.take())
    }

    /// Refuse `pid` as `damage` again, undoing a latch that stopped refusing it.
    pub(super) fn restore(&mut self, pid: Pid, damage: Damage) {
        self.update(pid, |p| p.refusal = Some(damage));
    }

    /// Set the end of the log recovery kept.
    pub(super) fn recovered_to(&mut self, _: &Replaying, end: Lsn) {
        self.recovered = end;
    }

    /// Note `met`, a record of `pid` replay met, to be applied to the list once it is read when
    /// `past_checkpoint`.
    pub(super) fn replay_met(&mut self, _: &Replaying, pid: Pid, met: Met, past_checkpoint: bool) {
        self.bound(pid, met);
        if past_checkpoint {
            self.replayed.push((pid, met));
        }
    }

    /// Arm the gate of a file created empty: no page has an expected LSN until a record gives one.
    pub(super) fn arm_created(&mut self) {
        self.gate = Gate::Armed(Expected::default());
    }

    /// Raise the folded expected LSN of each page of `last` below `end` to its LSN, dropping the
    /// later LSN of a page when it is no longer later.
    pub(super) fn absorb_folded(&mut self, last: &BTreeMap<Pid, Lsn>, end: u64) {
        if let Gate::Armed(e) = &mut self.gate {
            e.absorb(last.range(..Pid(end)));
        }
    }

    /// Serve `list` as the list page of a new file, recording from now on.
    pub(super) fn create_list(&mut self, list: Pid) {
        self.list = Some(list);
        self.logging = true;
    }

    /// Stop folding into the list page.
    pub(super) fn drop_list(&mut self) {
        self.list = None;
    }

    /// Take `base`, the list page's records, `None` when the page does not serve, then what replay
    /// met past it and `header_log` and `header_full`, the log findings the file header holds, as
    /// the records. Serve `list` as the list page when `base` is given.
    pub(super) fn load_list(
        &mut self,
        base: Option<Recorded>,
        list: Pid,
        header_log: Vec<LogFinding>,
        header_full: bool,
    ) {
        let sound = base.is_some();
        let mut recorded = base.unwrap_or_default();
        let mut unfolded = !header_log.is_empty() || header_full;
        for (pid, met) in std::mem::take(&mut self.replayed) {
            unfolded |= recorded.meet(pid, met);
        }
        let mut pids: BTreeSet<Pid> = self.pages.keys().copied().collect();
        pids.extend(recorded.pages.keys());
        for pid in pids {
            let damage = recorded.pages.get(&pid).copied();
            self.update(pid, |p| p.recorded = damage);
        }
        self.log = recorded.log;
        self.log.extend(header_log);
        self.full = recorded.full || header_full;
        self.unfolded = unfolded;
        self.list = sound.then_some(list);
        self.publish();
    }

    /// Returns what a fold writes, counting the list full when it holds more than `capacity`.
    pub(super) fn snapshot(&mut self, capacity: usize) -> Recorded {
        self.unfolded = false;
        let pages: BTreeMap<Pid, Damage> = self
            .pages
            .iter()
            .filter_map(|(&pid, p)| Some((pid, p.recorded?)))
            .collect();
        self.full |= pages.len() + self.log.len() > capacity;
        self.publish();
        Recorded {
            pages,
            log: self.log.clone(),
            full: self.full,
        }
    }

    /// Enter each of `mapped` whose damage has not changed since its mapping began, and whether a
    /// full list was mapped.
    pub(super) fn mapped(&mut self, full: bool, mapped: Vec<Mapping>) {
        self.full_mapped |= full;
        for (pid, damage, objects) in mapped {
            if self.damage(pid) != Some(damage) {
                continue;
            }
            self.update(pid, |p| match objects {
                Some(objects) => {
                    p.incomplete = None;
                    p.mapped = Some((damage, objects));
                }
                None => p.incomplete = Some(damage),
            });
        }
        self.publish();
    }

    /// Hold `pids`, pages of a tree a swap replaced, as retired until each one's own free.
    pub(super) fn retire(&mut self, pids: impl IntoIterator<Item = Pid>) {
        for pid in pids {
            self.update(pid, |p| p.retired = true);
        }
    }

    /// Stop holding `pids` as retired.
    pub(super) fn unretire(&mut self, pids: &[Pid]) {
        for &pid in pids {
            if self.pages.contains_key(&pid) {
                self.update(pid, |p| p.retired = false);
            }
        }
    }

    pub(super) fn refusal(&self, pid: Pid) -> Option<Damage> {
        self.pages.get(&pid)?.refusal
    }

    /// Returns every page refused, with why.
    pub(super) fn refusals(&self) -> BTreeMap<Pid, Damage> {
        let pages = self.pages.iter();
        pages
            .filter_map(|(&pid, p)| Some((pid, p.refusal?)))
            .collect()
    }

    /// Returns the damage recorded or held of `pid`.
    pub(super) fn damage(&self, pid: Pid) -> Option<Damage> {
        self.pages.get(&pid)?.damage()
    }

    /// Returns the damage recorded and held.
    pub(super) fn all(&self) -> Recorded {
        let pages = self.pages.iter();
        Recorded {
            pages: pages
                .filter_map(|(&pid, p)| Some((pid, p.damage()?)))
                .collect(),
            log: self.log.clone(),
            full: self.full,
        }
    }

    /// Returns one past the last page refused or with damage recorded or held, 0 when none is.
    pub(super) fn end(&self) -> u64 {
        let mut pages = self.pages.iter().rev();
        let last = pages.find(|(_, p)| p.refusal.is_some() || p.damage().is_some());
        last.map_or(0, |(pid, _)| pid.0 + 1)
    }

    /// Returns each page whose damage is not mapped this open, and whether a full list is not.
    /// `retried` false leaves out each page whose last mapping of the same damage was incomplete.
    pub(super) fn unmapped(&self, retried: bool) -> (Vec<(Pid, Damage)>, bool) {
        let pages = self.pages.iter();
        let pages = pages.filter_map(|(&pid, p)| Some((pid, p.unmapped(retried)?)));
        (pages.collect(), self.full && !self.full_mapped)
    }

    pub(super) fn detected_by(&self, pid: Pid) -> Option<ThreadId> {
        self.pages.get(&pid)?.detected_by
    }

    /// Returns what the damage of `pid` was mapped to this open, `None` while it is not mapped.
    pub(super) fn mapped_to(&self, pid: Pid) -> Option<BTreeSet<Quarantine>> {
        let p = self.pages.get(&pid)?;
        let (damage, objects) = p.mapped.as_ref()?;
        (p.damage() == Some(*damage)).then(|| objects.clone())
    }

    /// Returns the pages held as retired, none of them freed since.
    pub(super) fn retired(&self) -> Vec<Pid> {
        let pages = self.pages.iter();
        pages
            .filter(|(_, p)| p.retired)
            .map(|(&pid, _)| pid)
            .collect()
    }

    /// Returns the list page while the records are to be folded into it.
    pub(super) fn to_fold(&self) -> Option<Pid> {
        if !self.logging || !self.unfolded {
            return None;
        }
        self.list
    }

    pub(super) fn expected(&self, pid: Pid) -> Option<Lsn> {
        self.gate.expected(pid)
    }

    /// Returns one past the last page with an expected LSN, 0 when none has one.
    pub(super) fn expected_end(&self) -> u64 {
        match &self.gate {
            Gate::Armed(e) => e.end(),
            Gate::Recovering | Gate::Off => 0,
        }
    }

    /// Returns the number of pages whose expected LSN is past their folded entry.
    #[cfg(test)]
    pub(super) fn later(&self) -> usize {
        match &self.gate {
            Gate::Armed(e) => e.logged.len(),
            Gate::Recovering | Gate::Off => 0,
        }
    }

    pub(super) fn recovering(&self) -> bool {
        matches!(self.gate, Gate::Recovering)
    }

    pub(super) fn armed(&self) -> bool {
        matches!(self.gate, Gate::Armed(_))
    }

    pub(super) fn recovered(&self) -> Lsn {
        self.recovered
    }

    /// Returns the judgement of `file`, the file's copy of `pid`, below `file_end` and reaching
    /// what `against` names.
    pub(super) fn judge(
        &self,
        pid: Pid,
        file: FileCopy,
        against: Against,
        file_end: Lsn,
    ) -> Judged {
        let found = match file {
            Err(damage) => return Judged::Bad(damage),
            Ok((_, Verdict::Corrupt)) => return Judged::Bad(Damage::Checksum),
            Ok((found, Verdict::Valid | Verdict::Blank)) => found,
        };
        let expected = match against {
            Against::Expected => self.gate.expected(pid),
            Against::Resident(lsn) => Some(lsn),
            Against::LogEnd => None,
        };
        let behind = expected
            .filter(|&expected| found < expected)
            .map(|expected| Damage::Stale { expected, found });
        Damage::past_log_end(found, file_end)
            .or(behind)
            .map_or(Judged::Sound, Judged::Stale)
    }
}

impl<R: Runtime> Pager<R> {
    /// Returns the pages refused so far, each with what is wrong with it.
    pub(crate) fn damaged(&self) -> BTreeMap<Pid, Damage> {
        self.evidence().refusals()
    }

    pub(super) fn refusal(&self, pid: Pid) -> Option<Damage> {
        self.evidence().refusal(pid)
    }

    /// Returns the judgement of `file`, the file's copy of `pid`, against [`Pager::file_end`] and
    /// what `against` names.
    ///
    /// Preconditions: as for [`Pager::file_end`].
    pub(super) fn judge(&self, pid: Pid, file: FileCopy, against: Against) -> Judged {
        let end = self.file_end(pid);
        self.evidence().judge(pid, file, against, end)
    }

    /// End recovery: set each page's expected LSN from the page LSN map, then the retained log.
    /// Turn the gate off, setting none, when the log cannot prove a record before the end
    /// recovery kept, or fail with the map page refused.
    pub(super) fn arm(&self) -> Armed {
        let armed = self.map_entries().map_err(Some).and_then(|folded| {
            let end = self.evidence().recovered();
            let (_pin, mut img) = self.load_log().map_err(|_| None)?;
            let last = retained(&mut img, end).map_err(|_| None)?;
            let mut expected = Expected {
                folded,
                logged: HashMap::new(),
            };
            for (pid, lsn) in last {
                expected.raise(pid, lsn);
            }
            Ok(expected)
        });
        let mut e = self.evidence();
        match armed {
            Ok(expected) => {
                e.gate = Gate::Armed(expected);
                Ok(())
            }
            Err(err) => {
                e.gate = Gate::Off;
                Err(err)
            }
        }
    }
}
