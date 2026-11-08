//! The dirty page table: per page, the LSN that first dirtied it, the LSN of its latest full image
//! and, while the frame holds changes the file lacks, where replay of the page may start once a
//! force covers its last write.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::MutexGuard;

use super::evidence::Refusal;
use super::pager::{Appending, Forcing, Wrote};
use super::recovery::Replaying;
use super::{Lsn, Pid};

/// The kind of page record a change logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Logged {
    Image,
    Change,
}

#[derive(Debug)]
pub(super) struct DirtyTable {
    atomic: bool,
    pages: BTreeMap<Pid, Dirty>,
    /// Pages written since the last force of the data file began.
    written: BTreeSet<Pid>,
    /// The pages this open wrote to the file, one bit per page number.
    wrote: Vec<u64>,
    /// The freed record logged of each page whose frame was dropped holding changes the file
    /// lacks, by its LSN.
    freeing: BTreeSet<(Lsn, Pid)>,
}

#[derive(Debug)]
struct Dirty {
    rec_lsn: Lsn,
    image: Option<Lsn>,
    frame: Frame,
}

#[derive(Debug)]
enum Frame {
    /// The file holds every change of the page, once a force covers its last write.
    Written,
    /// The frame holds changes the file lacks. `since_write` is the record replay of the page may
    /// start from once a force covers the write these changes follow: their first change under
    /// atomic page writes, else the latest image at that change. `None` when no write precedes
    /// them or a force that covers the write has ended.
    Unwritten { since_write: Option<Lsn> },
    /// The frame was dropped holding changes the file lacks: replay of the page starts at
    /// `rec_lsn` until the freed record logged at `freed` is durable.
    Lost { freed: Option<Lsn> },
}

/// The pages written before a force of the data file began, for the force holding `'f`.
#[must_use]
pub(super) struct Covered<'f> {
    pages: BTreeSet<Pid>,
    _held: &'f Forcing,
}

impl DirtyTable {
    /// Returns an empty table for a file whose page writes are atomic when `atomic`.
    pub(super) fn new(atomic: bool) -> DirtyTable {
        DirtyTable {
            atomic,
            pages: BTreeMap::new(),
            written: BTreeSet::new(),
            wrote: Vec::new(),
            freeing: BTreeSet::new(),
        }
    }

    /// Returns the record a change to `pid` that starts no chain logs, under the checkpoint
    /// `last_checkpoint` the file header names: a full image when the change dirties a clean
    /// frame, or follows a write of the frame whose latest image precedes `last_checkpoint`,
    /// unless page writes are atomic.
    pub(super) fn record_kind(&self, _: &Appending, pid: Pid, last_checkpoint: Lsn) -> Logged {
        let change = self.atomic
            || self.pages.get(&pid).is_some_and(|d| {
                matches!(d.frame, Frame::Unwritten { .. }) || d.image >= Some(last_checkpoint)
            });
        if change {
            Logged::Change
        } else {
            Logged::Image
        }
    }

    /// Enter the page record `kind` of `pid`, logged at `lsn` in the hold of `_`.
    pub(super) fn logged(&mut self, _: &Appending, pid: Pid, lsn: Lsn, kind: Logged) {
        let d = self.pages.entry(pid).or_insert(Dirty {
            rec_lsn: lsn,
            image: None,
            frame: Frame::Unwritten { since_write: None },
        });
        if kind == Logged::Image {
            d.image = Some(lsn);
        }
        if !matches!(d.frame, Frame::Unwritten { .. }) {
            let since_write = if self.atomic { Some(lsn) } else { d.image };
            d.frame = Frame::Unwritten { since_write };
        }
    }

    /// Enter a write of a page to the file.
    pub(super) fn written_out(&mut self, wrote: Wrote) {
        let pid = wrote.pid();
        if let Some(d) = self.pages.get_mut(&pid)
            && matches!(d.frame, Frame::Unwritten { .. })
        {
            d.frame = Frame::Written;
        }
        self.written.insert(pid);
        let (word, bit) = ((pid.0 / 64) as usize, pid.0 % 64);
        if self.wrote.len() <= word {
            self.wrote.resize(word + 1, 0);
        }
        self.wrote[word] |= 1 << bit;
    }

    /// Returns the pages written so far, for the force of the data file that holds `held` and
    /// begins now.
    pub(super) fn forcing<'f>(&mut self, held: &'f MutexGuard<'_, Forcing>) -> Covered<'f> {
        Covered {
            pages: std::mem::take(&mut self.written),
            _held: held,
        }
    }

    /// Enter that a force of the data file covers `covered`: drop each page written since with no
    /// change since, and raise the `rec_lsn` of each changed since to where its replay may start.
    /// Leave a page written again since the force began, since the force may have missed that
    /// write, and a page whose frame was dropped with changes the file lacks.
    pub(super) fn forced(&mut self, covered: Covered<'_>) {
        for pid in covered.pages {
            if self.written.contains(&pid) {
                continue;
            }
            let Some(d) = self.pages.get_mut(&pid) else {
                continue;
            };
            match &mut d.frame {
                Frame::Written => {
                    self.pages.remove(&pid);
                }
                Frame::Unwritten { since_write } => {
                    if let Some(start) = since_write.take() {
                        d.rec_lsn = start.max(d.rec_lsn);
                    }
                }
                Frame::Lost { .. } => {}
            }
        }
    }

    /// Enter that the resident copy of the page `refusal` names is dropped. A copy dropped holding
    /// changes the file lacks keeps the page's entry, and so its `rec_lsn`, until a free of the
    /// page is durable or a record starting its next chain is logged.
    pub(super) fn refused(&mut self, refusal: &Refusal<'_>) {
        if let Some(d) = self.pages.get_mut(&refusal.pid())
            && let Frame::Unwritten { .. } = d.frame
        {
            d.frame = Frame::Lost { freed: None };
        }
    }

    /// Enter the freed record of `pid` logged at `lsn` in the hold of `_`.
    pub(super) fn freeing(&mut self, _: &Appending, pid: Pid, lsn: Lsn) {
        if let Some(d) = self.pages.get_mut(&pid)
            && let Frame::Lost { freed } = &mut d.frame
        {
            *freed = Some(lsn);
            self.freeing.insert((lsn, pid));
        }
    }

    /// Enter that the log is durable below `end`: drop the entry of each page whose frame was
    /// dropped with changes the file lacks and whose freed record since is durable.
    pub(super) fn freed_durable(&mut self, end: Lsn) {
        while let Some(&(lsn, pid)) = self.freeing.first()
            && lsn < end
        {
            self.freeing.pop_first();
            if self
                .pages
                .get(&pid)
                .is_some_and(|d| matches!(d.frame, Frame::Lost { freed: Some(f) } if f == lsn))
            {
                self.pages.remove(&pid);
            }
        }
    }

    /// Enter a record of `pid` at `lsn` that recovery replayed.
    pub(super) fn replayed(&mut self, _: &Replaying, pid: Pid, lsn: Lsn) {
        let d = self.pages.entry(pid).or_insert(Dirty {
            rec_lsn: lsn,
            image: None,
            frame: Frame::Unwritten { since_write: None },
        });
        if !matches!(d.frame, Frame::Unwritten { .. }) {
            d.frame = Frame::Unwritten { since_write: None };
        }
    }

    /// Drop the entry of `pid`, a page recovery refused.
    pub(super) fn replay_refused(&mut self, _: &Replaying, pid: Pid) {
        self.pages.remove(&pid);
    }

    /// Returns each page's `rec_lsn`, by page.
    pub(super) fn snapshot(&self) -> Vec<(Pid, Lsn)> {
        self.pages.iter().map(|(p, d)| (*p, d.rec_lsn)).collect()
    }

    /// Returns up to `n` pages whose frames hold changes the file lacks, least `rec_lsn` first.
    pub(super) fn oldest(&self, n: usize) -> Vec<Pid> {
        let mut by_lsn: Vec<(Lsn, Pid)> = self
            .pages
            .iter()
            .filter(|(_, d)| matches!(d.frame, Frame::Unwritten { .. }))
            .map(|(p, d)| (d.rec_lsn, *p))
            .collect();
        by_lsn.sort_unstable();
        by_lsn.into_iter().take(n).map(|(_, p)| p).collect()
    }

    /// Returns whether the frame of `pid` holds changes the file lacks.
    pub(super) fn is_unwritten(&self, pid: Pid) -> bool {
        self.pages
            .get(&pid)
            .is_some_and(|d| matches!(d.frame, Frame::Unwritten { .. }))
    }

    /// Returns the LSN the file's copy of `pid` is below, given the end `recovered` of the log
    /// recovery kept: `recovered` until this open writes the page, then `None`, for the log's next
    /// LSN.
    pub(super) fn file_end(&self, pid: Pid, recovered: Lsn) -> Option<Lsn> {
        let (word, bit) = ((pid.0 / 64) as usize, pid.0 % 64);
        let wrote = self.wrote.get(word).is_some_and(|w| w >> bit & 1 == 1);
        (!wrote).then_some(recovered)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use adtb_testkit::Rng;
    use adtb_testkit::profile::{assert_reached, sweep};

    use super::{DirtyTable, Logged};
    use crate::storage::evidence::refusal_for_test;
    use crate::storage::pager::{appending_for_test, forcing_for_test, wrote_for_test};
    use crate::storage::recovery::replaying_for_test;
    use crate::storage::{Lsn, Pid};

    const PAGES: u64 = 6;
    const STEPS: usize = 300;
    const RECOVERED: Lsn = Lsn(7);

    /// The table's facts as plain collections, with the pages refused holding changes the file
    /// lacks and the freed record logged of each.
    #[derive(Default)]
    struct Model {
        atomic: bool,
        rec_lsn: BTreeMap<Pid, Lsn>,
        image: BTreeMap<Pid, Lsn>,
        unwritten: BTreeSet<Pid>,
        replay_start: BTreeMap<Pid, Lsn>,
        written: BTreeSet<Pid>,
        wrote: BTreeSet<Pid>,
        lost: BTreeMap<Pid, Option<Lsn>>,
    }

    impl Model {
        fn record_kind(&self, pid: Pid, last_checkpoint: Lsn) -> Logged {
            let dirty = self.rec_lsn.contains_key(&pid);
            let image_old = self.image.get(&pid).is_none_or(|i| *i < last_checkpoint);
            if self.atomic || (dirty && (self.unwritten.contains(&pid) || !image_old)) {
                Logged::Change
            } else {
                Logged::Image
            }
        }

        fn logged(&mut self, pid: Pid, lsn: Lsn, kind: Logged) {
            let written_since_change =
                self.rec_lsn.contains_key(&pid) && !self.unwritten.contains(&pid);
            self.rec_lsn.entry(pid).or_insert(lsn);
            if kind == Logged::Image {
                self.image.insert(pid, lsn);
            }
            if written_since_change {
                let start = if self.atomic {
                    Some(lsn)
                } else {
                    self.image.get(&pid).copied()
                };
                if let Some(start) = start {
                    self.replay_start.insert(pid, start);
                }
            }
            self.unwritten.insert(pid);
            self.lost.remove(&pid);
        }

        fn written_out(&mut self, pid: Pid) {
            self.unwritten.remove(&pid);
            self.replay_start.remove(&pid);
            self.written.insert(pid);
            self.wrote.insert(pid);
        }

        fn forced(&mut self, covered: BTreeSet<Pid>) -> bool {
            let mut raised = false;
            for pid in covered.difference(&self.written) {
                if self.lost.contains_key(pid) {
                    continue;
                }
                let start = self.replay_start.remove(pid);
                if !self.unwritten.contains(pid) {
                    self.rec_lsn.remove(pid);
                    self.image.remove(pid);
                } else if let (Some(start), Some(rec_lsn)) = (start, self.rec_lsn.get_mut(pid)) {
                    raised |= start > *rec_lsn;
                    *rec_lsn = start.max(*rec_lsn);
                }
            }
            raised
        }

        fn refused(&mut self, pid: Pid) {
            if self.unwritten.remove(&pid) {
                self.lost.insert(pid, None);
            }
            self.replay_start.remove(&pid);
        }

        fn freeing(&mut self, pid: Pid, lsn: Lsn) {
            if let Some(freed) = self.lost.get_mut(&pid) {
                *freed = Some(lsn);
            }
        }

        fn freed_durable(&mut self, end: Lsn) -> usize {
            let durable: Vec<Pid> = self
                .lost
                .iter()
                .filter(|(_, f)| f.is_some_and(|f| f < end))
                .map(|(p, _)| *p)
                .collect();
            for pid in &durable {
                self.lost.remove(pid);
                self.rec_lsn.remove(pid);
                self.image.remove(pid);
            }
            durable.len()
        }

        fn replayed(&mut self, pid: Pid, lsn: Lsn) {
            self.rec_lsn.entry(pid).or_insert(lsn);
            self.unwritten.insert(pid);
            self.lost.remove(&pid);
        }

        fn replay_refused(&mut self, pid: Pid) {
            self.lost.remove(&pid);
            self.rec_lsn.remove(&pid);
            self.image.remove(&pid);
            self.unwritten.remove(&pid);
            self.replay_start.remove(&pid);
        }

        fn oldest(&self, n: usize) -> Vec<Pid> {
            let mut by_lsn: Vec<(Lsn, Pid)> = self
                .unwritten
                .iter()
                .map(|p| (self.rec_lsn[p], *p))
                .collect();
            by_lsn.sort();
            by_lsn.into_iter().take(n).map(|(_, p)| p).collect()
        }
    }

    #[derive(Default)]
    struct Reached {
        raised: usize,
        covered_written: usize,
        covered_logged: usize,
        covered_refused: usize,
        refused_dirty: usize,
        replay_refused_dirty: usize,
        change_after_write: usize,
        lost_covered: usize,
        lost_freed: usize,
        lost_logged: usize,
    }

    impl Reached {
        fn assert_all(&self) {
            assert_reached("a force raising a rec_lsn", self.raised, 1);
            assert_reached(
                "a write of a page a running force covers",
                self.covered_written,
                1,
            );
            assert_reached(
                "a change to a page a running force covers",
                self.covered_logged,
                1,
            );
            assert_reached(
                "a refusal of a page a running force covers",
                self.covered_refused,
                1,
            );
            assert_reached("a refusal of a dirty page", self.refused_dirty, 1);
            assert_reached(
                "a recovery refusal of a dirty page",
                self.replay_refused_dirty,
                1,
            );
            assert_reached(
                "a change after a write logged without an image",
                self.change_after_write,
                1,
            );
            assert_reached(
                "a force covering a page refused with changes the file lacks",
                self.lost_covered,
                1,
            );
            assert_reached(
                "a durable free dropping a page refused with changes the file lacks",
                self.lost_freed,
                1,
            );
            assert_reached(
                "a record logged of a page refused with changes the file lacks",
                self.lost_logged,
                1,
            );
        }
    }

    fn agree(seed: u64, step: usize, table: &DirtyTable, model: &Model) {
        let at = format!("seed {seed} step {step}");
        let expected: Vec<(Pid, Lsn)> = model.rec_lsn.iter().map(|(p, l)| (*p, *l)).collect();
        assert_eq!(expected, table.snapshot(), "{at}: snapshot");
        for n in [1, 3, PAGES as usize] {
            assert_eq!(model.oldest(n), table.oldest(n), "{at}: oldest({n})");
        }
        for p in 0..PAGES {
            let pid = Pid(p);
            assert_eq!(
                model.unwritten.contains(&pid),
                table.is_unwritten(pid),
                "{at}: is_unwritten({p})"
            );
            let end = (!model.wrote.contains(&pid)).then_some(RECOVERED);
            assert_eq!(end, table.file_end(pid, RECOVERED), "{at}: file_end({p})");
        }
    }

    /// Check that generated sequences of the table's operations answer as its model does.
    #[test]
    fn the_table_answers_as_its_model_over_plain_collections() {
        let mut reached = Reached::default();
        let replaying = replaying_for_test();
        let appending = appending_for_test();
        for seed in sweep(0..64) {
            let mut rng = Rng::new(seed);
            let atomic = seed % 4 == 3;
            let lock = std::sync::Mutex::new(forcing_for_test());
            let held = lock
                .lock()
                .expect("the test's forcing lock is never poisoned");
            let mut table = DirtyTable::new(atomic);
            let mut model = Model {
                atomic,
                ..Model::default()
            };
            let mut next = Lsn(10);
            let mut last_checkpoint = Lsn(0);
            let mut forcing: Option<(super::Covered<'_>, BTreeSet<Pid>)> = None;
            for step in 0..STEPS {
                let pid = Pid(rng.below(PAGES));
                let in_covered = forcing.as_ref().is_some_and(|(_, c)| c.contains(&pid));
                match rng.below(18) {
                    0..=5 => {
                        let kind = model.record_kind(pid, last_checkpoint);
                        assert_eq!(
                            kind,
                            table.record_kind(&appending, pid, last_checkpoint),
                            "seed {seed} step {step}: record_kind({})",
                            pid.0
                        );
                        if kind == Logged::Change
                            && !atomic
                            && model.rec_lsn.contains_key(&pid)
                            && !model.unwritten.contains(&pid)
                        {
                            reached.change_after_write += 1;
                        }
                        let kind = if rng.below(8) == 0 {
                            Logged::Image
                        } else {
                            kind
                        };
                        if in_covered {
                            reached.covered_logged += 1;
                        }
                        if model.lost.contains_key(&pid) {
                            reached.lost_logged += 1;
                        }
                        model.logged(pid, next, kind);
                        table.logged(&appending, pid, next, kind);
                        next = Lsn(next.0 + 1 + rng.below(3));
                    }
                    6..=8 => {
                        if in_covered {
                            reached.covered_written += 1;
                        }
                        model.written_out(pid);
                        table.written_out(wrote_for_test(pid));
                    }
                    9 | 10 => match forcing.take() {
                        None => {
                            let covered = std::mem::take(&mut model.written);
                            forcing = Some((table.forcing(&held), covered));
                        }
                        Some((covered, modelled)) => {
                            let lost = modelled.difference(&model.written);
                            reached.lost_covered +=
                                lost.filter(|p| model.lost.contains_key(p)).count();
                            if model.forced(modelled) {
                                reached.raised += 1;
                            }
                            table.forced(covered);
                        }
                    },
                    11 => {
                        if model.rec_lsn.contains_key(&pid) {
                            reached.refused_dirty += 1;
                        }
                        if in_covered {
                            reached.covered_refused += 1;
                        }
                        model.refused(pid);
                        table.refused(&refusal_for_test(pid));
                    }
                    12 => {
                        model.replayed(pid, next);
                        table.replayed(&replaying, pid, next);
                        next = Lsn(next.0 + 1);
                    }
                    13 => {
                        if model.rec_lsn.contains_key(&pid) {
                            reached.replay_refused_dirty += 1;
                        }
                        model.replay_refused(pid);
                        table.replay_refused(&replaying, pid);
                    }
                    14 => {
                        model.freeing(pid, next);
                        table.freeing(&appending, pid, next);
                        next = Lsn(next.0 + 1);
                    }
                    15 => {
                        let end = Lsn(next.0 - rng.below(4));
                        reached.lost_freed += model.freed_durable(end);
                        table.freed_durable(end);
                    }
                    _ => last_checkpoint = Lsn(last_checkpoint.0.max(next.0 - rng.below(8))),
                }
                agree(seed, step, &table, &model);
            }
        }
        reached.assert_all();
    }

    /// Change page 1 at 10, write it, change it at 12 and refuse it.
    fn refused_after_a_write(table: &mut DirtyTable) {
        let appending = appending_for_test();
        table.logged(&appending, Pid(1), Lsn(10), Logged::Image);
        table.written_out(wrote_for_test(Pid(1)));
        table.logged(&appending, Pid(1), Lsn(12), Logged::Change);
        table.refused(&refusal_for_test(Pid(1)));
    }

    fn force(table: &mut DirtyTable) {
        let lock = std::sync::Mutex::new(forcing_for_test());
        let held = lock
            .lock()
            .expect("the test's forcing lock is never poisoned");
        let covered = table.forcing(&held);
        table.forced(covered);
    }

    #[test]
    fn a_page_refused_with_changes_the_file_lacks_keeps_its_rec_lsn_through_a_covering_force() {
        let mut table = DirtyTable::new(false);
        refused_after_a_write(&mut table);
        force(&mut table);
        assert_eq!(vec![(Pid(1), Lsn(10))], table.snapshot());
        assert_eq!(
            (false, Vec::<Pid>::new()),
            (table.is_unwritten(Pid(1)), table.oldest(1))
        );
    }

    #[test]
    fn a_page_refused_after_its_last_write_leaves_at_the_next_covering_force() {
        let mut table = DirtyTable::new(false);
        let appending = appending_for_test();
        table.logged(&appending, Pid(1), Lsn(10), Logged::Image);
        table.written_out(wrote_for_test(Pid(1)));
        table.refused(&refusal_for_test(Pid(1)));
        force(&mut table);
        assert_eq!(Vec::<(Pid, Lsn)>::new(), table.snapshot());
    }

    #[test]
    fn a_page_refused_with_changes_the_file_lacks_leaves_once_its_free_is_durable() {
        let mut table = DirtyTable::new(false);
        refused_after_a_write(&mut table);
        table.freeing(&appending_for_test(), Pid(1), Lsn(14));
        table.freed_durable(Lsn(14));
        force(&mut table);
        assert_eq!(vec![(Pid(1), Lsn(10))], table.snapshot());
        table.freed_durable(Lsn(15));
        assert_eq!(Vec::<(Pid, Lsn)>::new(), table.snapshot());
    }

    #[test]
    fn a_durable_free_leaves_the_entry_of_a_later_incarnation() {
        let mut table = DirtyTable::new(false);
        let appending = appending_for_test();
        refused_after_a_write(&mut table);
        table.freeing(&appending, Pid(1), Lsn(14));
        table.logged(&appending, Pid(1), Lsn(16), Logged::Image);
        table.freed_durable(Lsn(17));
        assert_eq!(vec![(Pid(1), Lsn(10))], table.snapshot());
        assert!(table.is_unwritten(Pid(1)));
        table.refused(&refusal_for_test(Pid(1)));
        assert_eq!(vec![(Pid(1), Lsn(10))], table.snapshot());
    }
}
