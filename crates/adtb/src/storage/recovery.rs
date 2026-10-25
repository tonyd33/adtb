use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::{AlignedBuf, Runtime};

use super::block::{LogFinding, LogProblem, Mode, scan};
use super::page::{self, Verdict};
use super::pager::{Damage, OpenError, Pager};
use super::record::{Edit, Record};
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
    /// The LSN of the last record seen for each page.
    last: BTreeMap<Pid, Lsn>,
    /// Pages that failed verification, waiting for a full-page image.
    unverified: BTreeMap<Pid, Damage>,
    applied: usize,
}

fn damaged_at(lsn: Lsn, problem: LogProblem) -> OpenError {
    OpenError::Log(LogFinding { lsn, problem })
}

impl<R: Runtime> Pager<R> {
    pub(crate) fn recover(&mut self) -> Result<Recovery, OpenError> {
        let img = self.log.load(&mut self.drv);
        let ck = self.header.checkpoint;
        let ck_batch = img
            .batch_start(ck)
            .ok_or(damaged_at(ck, LogProblem::Damaged))?;
        let ck_scan = scan(&img, ck_batch, Mode::Recovery);
        let redo = ck_scan
            .batches
            .first()
            .filter(|b| b.start == ck_batch)
            .and_then(|b| b.records(img.block).ok())
            .and_then(|rs| {
                rs.into_iter().find_map(|(lsn, r)| match r {
                    Record::Checkpoint { redo, .. } if lsn == ck => Some(redo),
                    _ => None,
                })
            })
            .ok_or(damaged_at(ck, LogProblem::Damaged))?;
        let from = img
            .batch_start(redo)
            .ok_or(damaged_at(redo, LogProblem::Damaged))?;
        let s = scan(&img, from, Mode::Recovery);
        if let Some(f) = s.fatal {
            return Err(OpenError::Log(f));
        }
        if s.batches.first().is_none_or(|b| b.start != from) || s.end <= ck {
            return Err(damaged_at(from, LogProblem::Gap));
        }
        self.log
            .resume(s.end, s.last_seq.expect("a batch was accepted") + 1);
        let mut replay = Replay {
            redo,
            last: BTreeMap::new(),
            unverified: BTreeMap::new(),
            applied: 0,
        };
        for batch in &s.batches {
            let records = batch
                .records(img.block)
                .map_err(|_| damaged_at(batch.start, LogProblem::Malformed))?;
            for (lsn, record) in records {
                self.replay(&mut replay, lsn, record);
            }
        }
        for (pid, damage) in std::mem::take(&mut replay.unverified) {
            self.damage(pid, damage);
        }
        self.log.clear_tail(&mut self.drv, &img, s.end);
        self.flush_pages(usize::MAX);
        assert!(self.dirty.is_empty(), "recovery left a dirty page");
        self.checkpoint();
        Ok(Recovery {
            redo,
            end: s.end,
            applied: replay.applied,
            log: s.findings,
            damaged: self.damaged.iter().map(|(p, d)| (*p, *d)).collect(),
        })
    }

    fn replay(&mut self, st: &mut Replay, lsn: Lsn, record: Record) {
        let Some((pid, prev)) = record.page() else {
            return;
        };
        let before = st.last.insert(pid, lsn);
        if let Some(expected) = before.filter(|&l| l != prev) {
            st.unverified.remove(&pid);
            self.damage(
                pid,
                Damage::Stale {
                    expected,
                    found: prev,
                },
            );
            return;
        }
        if lsn < st.redo || self.damaged.contains_key(&pid) {
            return;
        }
        if st.unverified.contains_key(&pid) {
            if let Record::Image { page, .. } = record {
                st.unverified.remove(&pid);
                self.install_image(st, pid, lsn, &page);
            }
            return;
        }
        if !self.frames.contains_key(&pid) {
            self.make_room(&BTreeSet::new());
            let damage = match self.read_disk(pid) {
                Ok((buf, Verdict::Valid | Verdict::Blank)) => {
                    self.install(pid, buf, false);
                    None
                }
                Ok((_, Verdict::Corrupt)) => Some(Damage::Checksum),
                Err(d) => Some(d),
            };
            if let Some(d) = damage {
                match record {
                    Record::Image { page, .. } => self.install_image(st, pid, lsn, &page),
                    _ => {
                        st.unverified.insert(pid, d);
                    }
                }
                return;
            }
        }
        let have = page::lsn(&self.frames[&pid].page);
        if have >= lsn {
            return;
        }
        match record {
            Record::Image { page, .. } => self.install_image(st, pid, lsn, &page),
            Record::Change { edits, .. } if have == prev => self.redo_change(st, pid, lsn, &edits),
            _ => {
                self.damage(
                    pid,
                    Damage::Stale {
                        expected: prev,
                        found: have,
                    },
                );
            }
        }
    }

    fn install_image(&mut self, st: &mut Replay, pid: Pid, lsn: Lsn, image: &[u8]) {
        assert_eq!(self.page_size, image.len(), "a page image is one page");
        if !self.frames.contains_key(&pid) {
            self.make_room(&BTreeSet::new());
        }
        let mut page = AlignedBuf::from_slice(image);
        page::set_lsn(&mut page, lsn);
        self.install(pid, page, true);
        self.dirty.entry(pid).or_insert(lsn);
        st.applied += 1;
    }

    fn redo_change(&mut self, st: &mut Replay, pid: Pid, lsn: Lsn, edits: &[Edit]) {
        let frame = self
            .frames
            .get_mut(&pid)
            .expect("a replayed page is in the pool");
        for e in edits {
            let at = e.offset as usize;
            frame.page[at..at + e.bytes.len()].copy_from_slice(&e.bytes);
        }
        page::set_lsn(&mut frame.page, lsn);
        frame.dirty = true;
        self.dirty.entry(pid).or_insert(lsn);
        st.applied += 1;
    }
}
