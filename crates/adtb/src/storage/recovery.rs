use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::Runtime;

use super::block::{LogFinding, LogProblem, Mode, batch_at, scan};
use super::damage_log::Met;
use super::header::LOG_DAMAGE_CAPACITY;
use super::page;
use super::pager::{Damage, OpenError, Pager};
use super::record::{MapRoot, Record};
use super::{Lsn, Pid};

/// What recovery found and did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recovery {
    /// Where replay started.
    pub redo: Lsn,
    /// The end of the last accepted batch, where the log continues.
    pub end: Lsn,
    /// The page records replay applied or installed.
    pub applied: usize,
    /// Damage seen in the log that did not stop recovery.
    pub log: Vec<LogFinding>,
    /// Pages replay could not bring up to date.
    pub damaged: Vec<(Pid, Damage)>,
}

struct Replay {
    redo: Lsn,
    /// The checkpoint replay starts from.
    ck: Lsn,
    /// The LSN of the last record seen for each page.
    last: BTreeMap<Pid, Lsn>,
    /// Pages that failed verification, waiting for a full-page image.
    unverified: BTreeMap<Pid, Damage>,
    applied: usize,
}

/// Whether recovery armed the read-in gate: an error holds the map page refused, if that is why
/// it did not.
pub(crate) type Armed = Result<(), Option<super::PageError>>;

fn damaged_at(lsn: Lsn, problem: LogProblem) -> OpenError {
    OpenError::Log(LogFinding { lsn, problem })
}

impl<R: Runtime> Pager<R> {
    /// Replay the log, write every page it replayed and arm the read-in gate. Returns what it did,
    /// the pages the file then holds and whether the gate is armed.
    ///
    /// Postconditions: the stale blocks past the log's end are in the file header before the
    /// log's tail is cleared.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    pub(crate) fn recover(&self) -> Result<(Recovery, u64, Armed), OpenError> {
        let mut img = {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.load(d))
        };
        let ck = self.checkpoint_lsn();
        let ck_batch = img
            .batch_start(ck)
            .ok_or(damaged_at(ck, LogProblem::Damaged))?;
        let (redo, map) = batch_at(&mut img, ck_batch)
            .and_then(|b| b.records(img.block).ok())
            .and_then(|rs| {
                rs.into_iter().find_map(|(lsn, r)| match r {
                    Record::Checkpoint { redo, map, .. } if lsn == ck => Some((redo, map)),
                    _ => None,
                })
            })
            .ok_or(damaged_at(ck, LogProblem::Damaged))?;
        self.meta().map = map;
        let from = img
            .batch_start(redo)
            .ok_or(damaged_at(redo, LogProblem::Damaged))?;
        let s = scan(&mut img, from, Mode::Recovery, |_, _| {});
        if let Some(f) = s.fatal {
            return Err(OpenError::Log(f));
        }
        if s.first != Some(from) || s.end <= ck {
            return Err(damaged_at(from, LogProblem::Gap));
        }
        self.log()
            .resume(s.end, s.last_seq.expect("a batch was accepted") + 1);
        self.meta().recovered = s.end;
        let mut replay = Replay {
            redo,
            ck,
            last: BTreeMap::new(),
            unverified: BTreeMap::new(),
            applied: 0,
        };
        let block = img.block;
        let end = s.end;
        let (mut map, mut since) = (map, ck);
        let mut unproven = None;
        scan(&mut img, from, Mode::Recovery, |batch, _| {
            if batch.start >= end {
                return;
            }
            let records = batch
                .records(block)
                .expect("the first scan found every batch whole");
            for (lsn, record) in records {
                if let Record::MapRoot { root, height } = record
                    && lsn > since
                {
                    since = lsn;
                    match replay.last.get(&root).filter(|&&l| l >= batch.start) {
                        Some(&lsn) => map = Some(MapRoot { root, lsn, height }),
                        None => unproven = Some((root, lsn)),
                    }
                }
                self.replay(&mut replay, lsn, record);
            }
        });
        if let Some((pid, at)) = unproven {
            let damage = Damage::LogUnproven { at };
            return Err(OpenError::Page(super::PageError { pid, damage }));
        }
        if let Some(m) = map.as_mut()
            && let Some(&lsn) = replay.last.get(&m.root)
        {
            m.lsn = m.lsn.max(lsn);
        }
        self.meta().map = map;
        for (pid, damage) in std::mem::take(&mut replay.unverified) {
            self.damage(pid, damage);
        }
        self.keep_log_findings(&s.findings);
        {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.clear_tail(d, &mut img, s.end));
        }
        self.flush_pages(usize::MAX);
        assert!(self.meta().dirty.is_empty(), "recovery left a dirty page");
        let pages = self.count_file_groups();
        let armed = self.arm();
        let recovery = Recovery {
            redo,
            end: s.end,
            applied: replay.applied,
            log: s.findings,
            damaged: self.damaged().into_iter().collect(),
        };
        Ok((recovery, pages, armed))
    }

    /// Add the stale blocks of `findings` to those the file header holds, the lowest that fit,
    /// marking the header full when some do not, and force the header when that changes it.
    fn keep_log_findings(&self, findings: &[LogFinding]) {
        let stale = findings.iter().filter(|f| f.problem == LogProblem::Stale);
        let changed = {
            let mut h = self.header.lock(&self.rt);
            let mut kept: BTreeSet<LogFinding> = h.0.log_damage.iter().copied().collect();
            kept.extend(stale);
            let full = h.0.log_damage_full || kept.len() > LOG_DAMAGE_CAPACITY;
            let kept: Vec<LogFinding> = kept.into_iter().take(LOG_DAMAGE_CAPACITY).collect();
            let changed = kept != h.0.log_damage || full != h.0.log_damage_full;
            h.0.log_damage = kept;
            h.0.log_damage_full = full;
            changed
        };
        if changed {
            self.write_header(None);
        }
    }

    /// Refuse `pid` as [`Pager::damage`] does, and drop its dirty entry.
    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn damage_replayed(&self, pid: Pid, damage: Damage) {
        self.damage(pid, damage);
        self.meta().dirty.remove(&pid);
    }

    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn replay(&self, st: &mut Replay, lsn: Lsn, record: Record) {
        let met = match &record {
            Record::Damaged { pid, damage } => Some((*pid, Met::Damaged(*damage))),
            Record::Image { pid, prev, .. } if *prev == Lsn::UNCHAINED => Some((*pid, Met::Start)),
            Record::Image { pid, .. } => Some((*pid, Met::Image)),
            Record::Freed { pid } => Some((*pid, Met::Freed)),
            _ => None,
        };
        if let Some((pid, m)) = met {
            let mut meta = self.meta();
            meta.records.bound(pid, m);
            if lsn > st.ck {
                meta.records.replayed.push((pid, m));
            }
        }
        let Some((pid, prev)) = record.page() else {
            return;
        };
        let before = st.last.insert(pid, lsn);
        if prev == Lsn::UNCHAINED {
            self.meta().damaged.remove(&pid);
        }
        if let Some(expected) = before.filter(|&l| l != prev && prev != Lsn::UNCHAINED) {
            st.unverified.remove(&pid);
            self.damage_replayed(
                pid,
                Damage::Stale {
                    expected,
                    found: prev,
                },
            );
            return;
        }
        if lsn < st.redo || self.meta().damaged.contains_key(&pid) {
            return;
        }
        if !self.hold(pid).resident() {
            if let Entry::Vacant(slot) = st.unverified.entry(pid)
                && let Some(d) = self.replay_page_in(pid)
            {
                slot.insert(d);
            }
            if st.unverified.contains_key(&pid) {
                if let Record::Image { page, .. } = record {
                    st.unverified.remove(&pid);
                    self.install_image(st, pid, lsn, &page);
                }
                return;
            }
        }
        let have = page::lsn(&self.hold(pid));
        if have >= lsn {
            return;
        }
        match record {
            Record::Image { page, .. } => self.install_image(st, pid, lsn, &page),
            Record::Change { edits, .. } if have == prev => {
                self.replay_change(pid, &edits, lsn);
                self.mark_replayed(st, pid, lsn);
            }
            _ => {
                self.damage_replayed(
                    pid,
                    Damage::Stale {
                        expected: prev,
                        found: have,
                    },
                );
            }
        }
    }

    #[expect(clippy::disallowed_methods, reason = "until storage owners land")]
    fn mark_replayed(&self, st: &mut Replay, pid: Pid, lsn: Lsn) {
        let mut m = self.meta();
        m.changed.insert(pid);
        m.dirty.entry(pid).or_insert(lsn);
        st.applied += 1;
    }

    fn install_image(&self, st: &mut Replay, pid: Pid, lsn: Lsn, image: &[u8]) {
        self.replay_image(pid, image, lsn);
        self.mark_replayed(st, pid, lsn);
    }
}
