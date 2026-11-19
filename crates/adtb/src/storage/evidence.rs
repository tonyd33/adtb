//! What the pager knows of each page beyond its bytes: the LSN a copy read from the file must
//! reach, whether the page is refused and why, the damage recorded of it, in the log or held in
//! memory, and what that damage was mapped to this open.

use crate::runtime::{AlignedBuf, Runtime};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::marker::PhantomData;

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

/// The right to set, lift or set again the refusal of a page, borrowed from the page's latch, so
/// the latch is held wherever a refusal changes. Replay changes refusals under the `Replaying`
/// witness instead.
pub(super) struct Refusal<'a>(Pid, PhantomData<&'a ()>);

impl<'a> Refusal<'a> {
    /// Returns the refusal of the page under `latch`.
    pub(super) fn of<R: Runtime>(latch: &'a Held<'_, R>) -> Refusal<'a> {
        Refusal(latch.pid(), PhantomData)
    }

    pub(super) fn pid(&self) -> Pid {
        self.0
    }
}

#[cfg(test)]
pub(super) fn refusal_for_test(pid: Pid) -> Refusal<'static> {
    Refusal(pid, PhantomData)
}

/// The generation of an incarnation of a page, telling one from another within the open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Generation(u64);

/// A damage record not mapped this open: its page, its damage and the incarnation it is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pending {
    pid: Pid,
    damage: Damage,
    of: Generation,
}

impl Pending {
    pub(super) fn pid(&self) -> Pid {
        self.pid
    }

    pub(super) fn damage(&self) -> Damage {
        self.damage
    }
}

/// A damage record and what it quarantines, `None` when its mapping is incomplete.
pub(super) type Mapping = (Pending, Option<BTreeSet<Quarantine>>);

/// A count of the changes of any page's damage, incarnation or retirement. A record whose page
/// changed past the epoch its mapping began at is superseded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Epoch(u64);

/// The records a mapping takes, whether a full list is unmapped, and the epoch it begins at.
pub(super) type Begun = (Vec<Pending>, bool, Epoch);

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
    /// The copy's LSN is at or past the log's end.
    Stale(Damage),
    /// The copy's LSN is below what it must reach.
    Behind(Damage),
}

impl Judged {
    pub(super) fn damage(self) -> Option<Damage> {
        match self {
            Judged::Sound => None,
            Judged::Bad(d) | Judged::Stale(d) | Judged::Behind(d) => Some(d),
        }
    }
}

/// What the pager holds of a page with something to say.
#[derive(Debug)]
struct PageEvidence {
    /// Why the page is never handed out.
    refusal: Option<Damage>,
    /// A page of a tree a swap replaced, until its own free.
    retired: bool,
    incarnation: Incarnation,
    /// The epoch of the last change of the damage, the incarnation or the retirement.
    changed: Epoch,
}

/// What the pager holds of the current incarnation of a page. A chain start or a free replaces
/// it whole.
#[derive(Debug)]
struct Incarnation {
    generation: Generation,
    life: Life,
    /// The damage held in memory only, never logged or folded: found on a file quarantined
    /// whole, on a page freed and not allocated since, or in a file's copy while the pool holds
    /// a sound one.
    held: Option<Damage>,
    mapping: MappedTo,
}

#[derive(Debug)]
enum Life {
    /// The damage recorded in the log or the list, to be folded.
    Live { recorded: Option<Damage> },
    /// Ended by a free and not started again: its damage is only ever held.
    Freed,
}

/// What the damage of an incarnation was mapped to this open.
#[derive(Debug, Default)]
struct MappedTo {
    /// The damage whose last complete mapping set its marks, with the objects it marks.
    marked: Option<(Damage, Marked)>,
    /// The damage whose last mapping was incomplete.
    incomplete: Option<Damage>,
}

#[derive(Debug)]
enum Marked {
    Durably(BTreeSet<Quarantine>),
    /// Set only in memory, leaving the record unmapped.
    InMemory(BTreeSet<Quarantine>),
}

impl Marked {
    fn objects(&self) -> &BTreeSet<Quarantine> {
        match self {
            Marked::Durably(objects) | Marked::InMemory(objects) => objects,
        }
    }
}

impl MappedTo {
    fn is_empty(&self) -> bool {
        self.marked.is_none() && self.incomplete.is_none()
    }

    /// Returns the objects `damage` marks durably.
    fn durably(&self, damage: Damage) -> Option<&BTreeSet<Quarantine>> {
        match &self.marked {
            Some((d, Marked::Durably(objects))) if *d == damage => Some(objects),
            _ => None,
        }
    }

    /// Returns the objects `damage` marks, durably or only in memory.
    fn marks(&self, damage: Damage) -> Option<&BTreeSet<Quarantine>> {
        let (d, marked) = self.marked.as_ref()?;
        (*d == damage).then(|| marked.objects())
    }
}

impl Incarnation {
    fn new(generation: Generation, life: Life) -> Incarnation {
        Incarnation {
            generation,
            life,
            held: None,
            mapping: MappedTo::default(),
        }
    }

    fn recorded(&self) -> Option<Damage> {
        match self.life {
            Life::Live { recorded } => recorded,
            Life::Freed => None,
        }
    }

    fn is_freed(&self) -> bool {
        matches!(self.life, Life::Freed)
    }
}

impl PageEvidence {
    fn new(generation: Generation) -> PageEvidence {
        PageEvidence {
            refusal: None,
            retired: false,
            incarnation: Incarnation::new(generation, Life::Live { recorded: None }),
            changed: Epoch(0),
        }
    }

    fn is_empty(&self) -> bool {
        let i = &self.incarnation;
        matches!(i.life, Life::Live { recorded: None })
            && self.refusal.is_none()
            && i.held.is_none()
            && i.mapping.is_empty()
            && !self.retired
    }

    fn damage(&self) -> Option<Damage> {
        self.incarnation.held.or(self.incarnation.recorded())
    }

    /// Returns the damage not mapped this open. A settled page maps to nothing, so it is never
    /// unmapped. `retried` false leaves it out when its last mapping was incomplete.
    fn unmapped(&self, retried: bool) -> Option<Damage> {
        let d = self.damage()?;
        let mapping = &self.incarnation.mapping;
        let unmapped = !matches!(d, Damage::Settled { .. })
            && mapping.durably(d).is_none()
            && (retried || mapping.incomplete != Some(d));
        unmapped.then_some(d)
    }
}

mod tally {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Whether the evidence holds a record not mapped this open, read without the meta lock.
    #[derive(Debug, Clone)]
    pub(in crate::storage) struct Unmapped(Arc<AtomicBool>);

    impl Unmapped {
        pub(in crate::storage) fn any(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
    }

    /// How many entries hold unmapped damage and whether a full list is unmapped, with the
    /// [`Unmapped`] flag set from both by every change.
    #[derive(Debug)]
    pub(super) struct Tally {
        pages: usize,
        full: bool,
        full_mapped: bool,
        flag: Unmapped,
    }

    impl Tally {
        pub(super) fn new() -> Tally {
            Tally {
                pages: 0,
                full: false,
                full_mapped: false,
                flag: Unmapped(Arc::new(AtomicBool::new(false))),
            }
        }

        pub(super) fn flag(&self) -> Unmapped {
            self.flag.clone()
        }

        /// Count an entry whose damage was unmapped, `before`, and is, `after`, a change.
        pub(super) fn changed(&mut self, before: bool, after: bool) {
            self.pages = self.pages + usize::from(after) - usize::from(before);
            self.publish();
        }

        /// Returns whether the list ran out of room.
        pub(super) fn full(&self) -> bool {
            self.full
        }

        /// Returns whether a full list is not mapped this open.
        pub(super) fn full_unmapped(&self) -> bool {
            self.full && !self.full_mapped
        }

        /// Count the list full as the open found it.
        pub(super) fn found_full(&mut self, full: bool) {
            self.full = full;
            self.publish();
        }

        /// Count the list full from now on when `full`.
        pub(super) fn overflowed(&mut self, full: bool) {
            self.full |= full;
            self.publish();
        }

        /// Count a full list mapped from now on when `mapped`.
        pub(super) fn mapped_full(&mut self, mapped: bool) {
            self.full_mapped |= mapped;
            self.publish();
        }

        fn publish(&self) {
            let any = self.pages > 0 || self.full_unmapped();
            self.flag.0.store(any, Ordering::Release);
        }
    }
}

use self::tally::Tally;
pub(super) use self::tally::Unmapped;

/// The evidence of every page, under the meta lock.
///
/// Invariants:
///
/// - A page has an entry only while it has something to say or its last record this open logged
///   or replayed is a free.
/// - No two incarnations of a page share a generation.
/// - An entry with no damage recorded or held has no mapping and no detecting thread.
/// - `tally` counts the entries whose damage is unmapped. Only `update` changes an entry.
#[derive(Debug)]
pub(super) struct Evidence {
    pages: BTreeMap<Pid, PageEvidence>,
    gate: Gate,
    /// The end of the log recovery kept when the pager opened, or zero for a pager created empty.
    recovered: Lsn,
    /// Whether a detection logs its record. Off until the open's recovery ends.
    logging: bool,
    /// The list page the file header names, `Pid(0)` while the file has none.
    named: Pid,
    /// The list page, while a checkpoint may write it.
    list: Option<Pid>,
    /// What replay met from the redo LSN it started from, in log order, each at its LSN.
    replayed: Vec<(Pid, Met, Lsn)>,
    /// Whether the records differ from what the list page holds.
    unfolded: bool,
    /// The LSN of the first record logged since the last fold that changes the records, `None`
    /// while none has.
    unfolded_from: Option<Lsn>,
    /// The LSN `unfolded_from` held when a fold took the records, until that fold's change of the
    /// list page is logged.
    folding: Option<Lsn>,
    /// Whether the open found no list page to read, or read it with no check of its LSN.
    list_unread: bool,
    /// Stale blocks recovery found past the end of the log.
    log: BTreeSet<LogFinding>,
    /// The entries holding unmapped damage, and whether the list ran out of room, so the whole
    /// file counts as damaged.
    tally: Tally,
    /// The last generation an incarnation took.
    generation: u64,
    /// The last change of any page's damage or incarnation.
    epoch: Epoch,
    /// How many damaged pages this open has walked the objects for.
    walked: u64,
    /// The refused pages a read or latch has met this open.
    #[cfg(any(test, feature = "testing"))]
    met: BTreeSet<Pid>,
}

impl Evidence {
    /// Returns the evidence of a file whose header names the list page `named`, recovering.
    pub(super) fn new(named: Pid) -> Evidence {
        Evidence {
            pages: BTreeMap::new(),
            gate: Gate::Recovering,
            recovered: Lsn(0),
            logging: false,
            named,
            list: None,
            replayed: Vec::new(),
            unfolded: false,
            unfolded_from: None,
            list_unread: false,
            folding: None,
            log: BTreeSet::new(),
            tally: Tally::new(),
            generation: 0,
            epoch: Epoch(0),
            walked: 0,
            #[cfg(any(test, feature = "testing"))]
            met: BTreeSet::new(),
        }
    }

    /// Returns the flag of whether a record is unmapped, read without the meta lock.
    pub(super) fn unmapped_flag(&self) -> Unmapped {
        self.tally.flag()
    }

    /// Apply `f` to the evidence of `pid`, drop its mapping once it has no damage and the entry
    /// once it says nothing, and set the unmapped count and flag. An entry made here takes the next
    /// generation. A change of the damage, the incarnation or the retirement takes the next epoch.
    fn update<T>(&mut self, pid: Pid, f: impl FnOnce(&mut PageEvidence) -> T) -> T {
        let generation = &mut self.generation;
        let p = self.pages.entry(pid).or_insert_with(|| {
            *generation += 1;
            PageEvidence::new(Generation(*generation))
        });
        let before = p.unmapped(true).is_some();
        let identity = |p: &PageEvidence| {
            let i = &p.incarnation;
            (p.damage(), i.generation, i.is_freed(), p.retired)
        };
        let was = identity(p);
        let out = f(p);
        if identity(p) != was {
            self.epoch.0 += 1;
            p.changed = self.epoch;
        }
        if p.damage().is_none() {
            p.incarnation.mapping = MappedTo::default();
        }
        let after = p.unmapped(true).is_some();
        if p.is_empty() {
            self.pages.remove(&pid);
        }
        self.tally.changed(before, after);
        out
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
        if whole {
            self.update(pid, |p| p.incarnation.held = Some(damage));
            return None;
        }
        let changed = self.take_damage(pid, damage)?;
        let at = log.append(&Record::Damaged { pid, damage });
        self.unfold_at(changed, at);
        Some(log.next_lsn())
    }

    /// Note that `met`, a record of `pid`, ends or starts an incarnation of the page: replace the
    /// one it ends whole. Returns whether that drops damage recorded.
    fn bound(&mut self, pid: Pid, met: Met) -> bool {
        let life = match met {
            Met::Freed => Life::Freed,
            Met::Start if self.pages.contains_key(&pid) => Life::Live { recorded: None },
            Met::Start | Met::Damaged(_) | Met::Image => return false,
        };
        self.generation += 1;
        let next = Incarnation::new(Generation(self.generation), life);
        self.update(pid, |p| {
            if next.is_freed() {
                p.retired = false;
            }
            std::mem::replace(&mut p.incarnation, next)
                .recorded()
                .is_some()
        })
    }

    /// Take `damage` of `pid` into its incarnation: recorded, to be folded and logged, when it is
    /// live, held when it is freed. Returns whether it is recorded, with whether that changes what
    /// is to be folded.
    fn take_damage(&mut self, pid: Pid, damage: Damage) -> Option<bool> {
        let taken = self.update(pid, |p| {
            let i = &mut p.incarnation;
            i.held = None;
            match &mut i.life {
                Life::Live { recorded } => Some(meet_recorded(recorded, Met::Damaged(damage))),
                Life::Freed => {
                    i.held = Some(damage);
                    None
                }
            }
        });
        self.unfolded |= taken == Some(true);
        taken
    }

    fn is_freed(&self, pid: Pid) -> bool {
        let p = self.pages.get(&pid);
        p.is_some_and(|p| p.incarnation.is_freed())
    }

    /// Apply what `met` does to the damage recorded of `pid`, to be folded, and to what is held of
    /// it. Returns whether that changes what is to be folded.
    fn meet(&mut self, pid: Pid, met: Met) -> bool {
        let changed = match met {
            Met::Damaged(damage) => return self.take_damage(pid, damage).unwrap_or(false),
            Met::Freed | Met::Start => self.bound(pid, met),
            Met::Image if !self.pages.contains_key(&pid) => false,
            Met::Image => self.update(pid, |p| {
                let i = &mut p.incarnation;
                if !matches!(i.held, Some(Damage::Settled { .. })) {
                    i.held = None;
                }
                match &mut i.life {
                    Life::Live { recorded } => meet_recorded(recorded, met),
                    Life::Freed => false,
                }
            }),
        };
        self.unfolded |= changed;
        changed
    }

    /// Refuse the page `refusal` names as `damage` from now on and record it, unless it is refused
    /// as `damage` already. Returns the end of the damage record logged, `None` when none is.
    pub(super) fn refuse(
        &mut self,
        _: &Appending,
        log: &mut Log,
        refusal: &Refusal<'_>,
        damage: Damage,
        whole: bool,
    ) -> Option<Lsn> {
        let pid = refusal.pid();
        let fresh = self.update(pid, |p| p.refusal.replace(damage) != Some(damage));
        if fresh {
            self.detect(log, pid, damage, whole)
        } else {
            None
        }
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
            let changed = self.meet(pid, met);
            self.unfold_at(changed, lsn);
        }
    }

    /// End the incarnation of `pid`, which a batch frees, with its damage recorded, and log the
    /// free in the batch unless recovery has not ended or `whole` says the file is quarantined
    /// whole. End a refusal as stale, the ended incarnation's lost write, and hold any other but
    /// an unreadable one as the damage of the free copy, mapped to nothing. Returns the LSN of the
    /// record logged.
    pub(super) fn free(
        &mut self,
        _: &Appending,
        log: &mut Log,
        pid: Pid,
        whole: bool,
    ) -> Option<Lsn> {
        let changed = self.meet(pid, Met::Freed);
        self.update(pid, |p| match p.refusal {
            Some(Damage::Stale { .. }) => p.refusal = None,
            Some(Damage::Unreadable) | None => {}
            Some(damage) => {
                let i = &mut p.incarnation;
                i.held = Some(damage);
                i.mapping.marked = Some((damage, Marked::Durably(BTreeSet::new())));
            }
        });
        let at = (self.logging && !whole).then(|| log.append(&Record::Freed { pid }));
        if let Some(at) = at {
            self.unfold_at(changed, at);
        }
        at
    }

    /// Note the record logged at `at` as unfolded when it `changed` the records, or may have
    /// changed what the unread list page holds.
    fn unfold_at(&mut self, changed: bool, at: Lsn) {
        if (changed || self.list_unread) && self.unfolded_from.is_none() {
            self.unfolded_from = Some(at);
        }
    }

    /// Returns the least LSN a checkpoint's redo LSN may take: that of the first record logged
    /// since the last logged fold that changes the records.
    pub(super) fn redo_bound(&self) -> Option<Lsn> {
        self.unfolded_from.into_iter().chain(self.folding).min()
    }

    /// Note that the change of the list page the last fold took is logged.
    pub(super) fn fold_logged(&mut self) {
        self.folding = None;
    }

    /// Drop the damage held of the page `wrote` wrote, but a settle, now the page is written out
    /// over the file's copy.
    pub(super) fn written_out(&mut self, wrote: &Wrote) {
        let pid = wrote.pid();
        let held = |p: &PageEvidence| p.incarnation.held;
        if self.pages.get(&pid).and_then(held).is_some() {
            self.update(pid, |p| {
                if !matches!(held(p), Some(Damage::Settled { .. })) {
                    p.incarnation.held = None;
                }
            });
        }
    }

    /// Hold `damage` of `pid` in memory, unlogged, leaving its refusal as it is: for a file's copy
    /// that fails while the pool holds a sound one. The hold ends at the page's next write-out, and
    /// lasts the open when the page is never written out.
    pub(super) fn hold(&mut self, pid: Pid, damage: Damage) {
        if damage != Damage::Unreadable {
            self.update(pid, |p| p.incarnation.held = Some(damage));
        }
    }

    /// Stop refusing the page `refusal` names. Returns its refusal.
    pub(super) fn unrefuse(&mut self, refusal: &Refusal<'_>) -> Option<Damage> {
        self.lift(refusal.pid())
    }

    /// Stop refusing `pid`, whose replayed record starts a chain.
    pub(super) fn replay_start(&mut self, _: &Replaying, pid: Pid) {
        self.lift(pid);
    }

    fn lift(&mut self, pid: Pid) -> Option<Damage> {
        if !self.pages.contains_key(&pid) {
            return None;
        }
        self.update(pid, |p| p.refusal.take())
    }

    /// Refuse the page `refusal` names as `damage` again, undoing a latch that stopped refusing it.
    pub(super) fn restore(&mut self, refusal: &Refusal<'_>, damage: Damage) {
        self.update(refusal.pid(), |p| p.refusal = Some(damage));
    }

    /// Set the end of the log recovery kept.
    pub(super) fn recovered_to(&mut self, _: &Replaying, end: Lsn) {
        self.recovered = end;
    }

    /// Note `met`, a record of `pid` replay met at `at`, to be applied to the list once it is read.
    ///
    /// Precondition: replay starts at or below the first record logged since the list's last
    /// logged fold that changed the records.
    pub(super) fn replay_met(&mut self, _: &Replaying, pid: Pid, met: Met, at: Lsn) {
        self.bound(pid, met);
        self.replayed.push((pid, met, at));
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
        self.named = list;
        self.list = Some(list);
        self.logging = true;
    }

    /// Stop folding into the list page.
    pub(super) fn drop_list(&mut self) {
        self.list = None;
    }

    /// Take `base`, the list page's records, `None` when the page does not serve, then what replay
    /// met past it and `header_log` and `header_full`, the log findings the file header holds, as
    /// the records. Name `list` as the list page, and serve it when `base` is given and the
    /// read-in gate is armed.
    pub(super) fn load_list(
        &mut self,
        base: Option<Recorded>,
        list: Pid,
        header_log: Vec<LogFinding>,
        header_full: bool,
    ) {
        let sound = base.is_some() && matches!(self.gate, Gate::Armed(_));
        self.list_unread = !sound;
        let mut recorded = base.unwrap_or_default();
        let mut unfolded = !header_log.is_empty() || header_full;
        for (pid, met, at) in std::mem::take(&mut self.replayed) {
            let changed = recorded.meet(pid, met);
            self.unfold_at(changed, at);
            unfolded |= changed;
        }
        let mut pids: BTreeSet<Pid> = self.pages.keys().copied().collect();
        pids.extend(recorded.pages.keys());
        for pid in pids {
            let damage = recorded.pages.get(&pid).copied();
            self.update(pid, |p| match &mut p.incarnation.life {
                Life::Live { recorded } => *recorded = damage,
                Life::Freed => p.incarnation.held = damage.or(p.incarnation.held),
            });
        }
        self.log = recorded.log;
        self.log.extend(header_log);
        self.tally.found_full(recorded.full || header_full);
        self.unfolded = unfolded;
        self.named = list;
        self.list = sound.then_some(list);
    }

    /// Returns what a fold writes, counting the list full when it holds more than `capacity`.
    pub(super) fn snapshot(&mut self, capacity: usize) -> Recorded {
        self.unfolded = false;
        self.folding = self.unfolded_from.take();
        let pages: BTreeMap<Pid, Damage> = self
            .pages
            .iter()
            .filter_map(|(&pid, p)| Some((pid, p.incarnation.recorded()?)))
            .collect();
        self.tally
            .overflowed(pages.len() + self.log.len() > capacity);
        Recorded {
            pages,
            log: self.log.clone(),
            full: self.tally.full(),
        }
    }

    /// Returns whether the record `pending` names is still current: its damage, incarnation and
    /// retirement unchanged since `begun`, the epoch its mapping began at.
    fn is_current(&self, begun: Epoch, pending: &Pending) -> bool {
        let p = self.pages.get(&pending.pid);
        p.is_some_and(|p| p.changed <= begun && p.incarnation.generation == pending.of)
    }

    /// Returns each of `mapped` still current since `begun`, the epoch its mapping began at.
    pub(super) fn current(&self, begun: Epoch, mut mapped: Vec<Mapping>) -> Vec<Mapping> {
        mapped.retain(|(p, _)| self.is_current(begun, p));
        mapped
    }

    /// Enter each of `mapped` still current since `begun`, the epoch its mapping began at, and
    /// whether a full list was mapped.
    pub(super) fn mapped(&mut self, full: bool, begun: Epoch, mapped: Vec<Mapping>) {
        self.tally.mapped_full(full);
        for (pending, objects) in mapped {
            if !self.is_current(begun, &pending) {
                continue;
            }
            let Pending { pid, damage, .. } = pending;
            self.update(pid, |p| {
                let mapping = &mut p.incarnation.mapping;
                match objects {
                    Some(objects) => {
                        *mapping = MappedTo {
                            marked: Some((damage, Marked::Durably(objects))),
                            incomplete: None,
                        };
                    }
                    None => mapping.incomplete = Some(damage),
                }
            });
        }
    }

    /// Enter each of `unwritten` still current since `begun` as marked only in memory, leaving it
    /// unmapped.
    pub(super) fn unwritten(&mut self, begun: Epoch, unwritten: Vec<Mapping>) {
        for (pending, objects) in unwritten {
            if !self.is_current(begun, &pending) {
                continue;
            }
            let Pending { pid, damage, .. } = pending;
            self.update(pid, |p| {
                let marked = objects.map(|o| (damage, Marked::InMemory(o)));
                p.incarnation.mapping.marked = marked;
            });
        }
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
            full: self.tally.full(),
        }
    }

    /// Returns one past the last page refused or with damage recorded or held, 0 when none is.
    pub(super) fn end(&self) -> u64 {
        let mut pages = self.pages.iter().rev();
        let last = pages.find(|(_, p)| p.refusal.is_some() || p.damage().is_some());
        last.map_or(0, |(pid, _)| pid.0 + 1)
    }

    /// Returns each page whose damage is not mapped this open, whether a full list is not, and
    /// the epoch the mapping begins at. `retried` false leaves out each page whose last mapping of
    /// the same damage was incomplete.
    pub(super) fn begin_mapping(&self, retried: bool) -> Begun {
        let pages = self.pages.iter();
        let pages = pages.filter_map(|(&pid, p)| {
            let damage = p.unmapped(retried)?;
            let of = p.incarnation.generation;
            Some(Pending { pid, damage, of })
        });
        (pages.collect(), self.tally.full_unmapped(), self.epoch)
    }

    /// Returns what the damage of `pid` was mapped to this open, `None` while it is not mapped.
    #[cfg(any(test, feature = "testing"))]
    pub(super) fn mapped_to(&self, pid: Pid) -> Option<BTreeSet<Quarantine>> {
        let p = self.pages.get(&pid)?;
        p.incarnation.mapping.durably(p.damage()?).cloned()
    }

    /// Returns what the damage of `pid` marks this open, durably or only in memory, `None` while
    /// no mapping has set its marks.
    pub(super) fn marked_to(&self, pid: Pid) -> Option<BTreeSet<Quarantine>> {
        let p = self.pages.get(&pid)?;
        p.incarnation.mapping.marks(p.damage()?).cloned()
    }

    /// Returns the pages held as retired, none of them freed since.
    pub(super) fn retired(&self) -> Vec<Pid> {
        let pages = self.pages.iter();
        pages
            .filter(|(_, p)| p.retired)
            .map(|(&pid, _)| pid)
            .collect()
    }

    /// Returns the list page the file header names, `Pid(0)` while the file has none.
    pub(super) fn list_page(&self) -> Pid {
        self.named
    }

    /// Count one damaged page whose objects a mapping walks.
    pub(super) fn walked_one(&mut self) {
        self.walked += 1;
    }

    /// Returns how many damaged pages this open has walked the objects for.
    #[cfg(any(test, feature = "testing"))]
    pub(super) fn walked(&self) -> u64 {
        self.walked
    }

    /// Returns the list page while the records are to be folded into it, or a fold that took
    /// them has not logged its change of the page.
    pub(super) fn to_fold(&self) -> Option<Pid> {
        if !self.logging || (!self.unfolded && self.folding.is_none()) {
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

    pub(super) fn armed(&self) -> bool {
        matches!(self.gate, Gate::Armed(_))
    }

    pub(super) fn recovered(&self) -> Lsn {
        self.recovered
    }

    /// Returns the judgement of `file`, the file's copy of `pid`, below `file_end` and reaching
    /// what `against` names. A page freed this open has no expected LSN.
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
            Against::Expected if self.is_freed(pid) => None,
            Against::Expected => self.gate.expected(pid),
            Against::Resident(lsn) => Some(lsn),
            Against::LogEnd => None,
        };
        if let Some(damage) = Damage::past_log_end(found, file_end) {
            return Judged::Stale(damage);
        }
        expected
            .filter(|&expected| found < expected)
            .map_or(Judged::Sound, |expected| {
                Judged::Behind(Damage::Stale { expected, found })
            })
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

    /// Returns the refusal of `pid` that a read or latch of it meets.
    pub(super) fn met_refusal(&self, pid: Pid) -> Option<Damage> {
        let refusal = self.refusal(pid);
        #[cfg(any(test, feature = "testing"))]
        if refusal.is_some() {
            self.evidence().met.insert(pid);
        }
        refusal
    }

    /// Returns whether a read or latch of `pid` has met its refusal this open.
    #[cfg(any(test, feature = "testing"))]
    pub fn refusal_met(&self, pid: Pid) -> bool {
        self.evidence().met.contains(&pid)
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
