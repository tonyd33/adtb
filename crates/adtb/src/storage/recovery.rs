use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use adtb_sys::pages;

use crate::runtime::Runtime;

use super::block::{LogFinding, LogProblem, Mode, batch_at, scan};
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
    /// Replay the log and checkpoint. Returns what it did and the pages the file then holds.
    pub(crate) fn recover(&self) -> Result<(Recovery, u64), OpenError> {
        let mut img = {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.load(d))
        };
        let ck = self.checkpoint_lsn();
        let ck_batch = img
            .batch_start(ck)
            .ok_or(damaged_at(ck, LogProblem::Damaged))?;
        let redo = batch_at(&mut img, ck_batch)
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
        let s = scan(&mut img, from, Mode::Recovery, |_, _| {});
        if let Some(f) = s.fatal {
            return Err(OpenError::Log(f));
        }
        if s.first != Some(from) || s.end <= ck {
            return Err(damaged_at(from, LogProblem::Gap));
        }
        self.log()
            .resume(s.end, s.last_seq.expect("a batch was accepted") + 1);
        let mut replay = Replay {
            redo,
            last: BTreeMap::new(),
            unverified: BTreeMap::new(),
            applied: 0,
        };
        let block = img.block;
        let end = s.end;
        scan(&mut img, from, Mode::Recovery, |batch, _| {
            if batch.start >= end {
                return;
            }
            let records = batch
                .records(block)
                .expect("the first scan found every batch whole");
            for (lsn, record) in records {
                self.replay(&mut replay, lsn, record);
            }
        });
        for (pid, damage) in std::mem::take(&mut replay.unverified) {
            self.damage(pid, damage);
        }
        {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.clear_tail(d, &mut img, s.end));
        }
        self.flush_pages(usize::MAX);
        assert!(self.meta().dirty.is_empty(), "recovery left a dirty page");
        let pages = self.count_file_groups();
        self.checkpoint();
        let recovery = Recovery {
            redo,
            end: s.end,
            applied: replay.applied,
            log: s.findings,
            damaged: self.damaged().into_iter().collect(),
        };
        Ok((recovery, pages))
    }

    fn replay(&self, st: &mut Replay, lsn: Lsn, record: Record) {
        let Some((pid, prev)) = record.page() else {
            return;
        };
        let before = st.last.insert(pid, lsn);
        if prev == Lsn::UNCHAINED {
            self.meta().damaged.remove(&pid);
        }
        if let Some(expected) = before.filter(|&l| l != prev && prev != Lsn::UNCHAINED) {
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
        if lsn < st.redo || self.meta().damaged.contains_key(&pid) {
            return;
        }
        let mut g = self.mem.lock_exclusive(pid.0, &self.rt);
        if !g.resident() {
            if let Entry::Vacant(slot) = st.unverified.entry(pid) {
                self.make_room(pid);
                let failed = match self.read_disk(pid) {
                    Ok((buf, Verdict::Valid | Verdict::Blank)) => {
                        g.write(0, &buf);
                        None
                    }
                    Ok((_, Verdict::Corrupt)) => Some(Damage::Checksum),
                    Err(d) => Some(d),
                };
                if let Some(d) = failed {
                    slot.insert(d);
                    self.unreserve(pid);
                }
            }
            if st.unverified.contains_key(&pid) {
                if let Record::Image { page, .. } = record {
                    st.unverified.remove(&pid);
                    self.make_room(pid);
                    self.install_image(st, &mut g, lsn, &page);
                }
                return;
            }
        }
        let have = page::lsn(&g);
        if have >= lsn {
            return;
        }
        match record {
            Record::Image { page, .. } => self.install_image(st, &mut g, lsn, &page),
            Record::Change { edits, .. } if have == prev => {
                self.redo_change(st, &mut g, lsn, &edits);
            }
            _ => {
                drop(g);
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

    fn mark_replayed(&self, st: &mut Replay, pid: Pid, lsn: Lsn) {
        let mut m = self.meta();
        m.changed.insert(pid);
        m.dirty.entry(pid).or_insert(lsn);
        st.applied += 1;
    }

    fn install_image(&self, st: &mut Replay, g: &mut pages::Exclusive<'_>, lsn: Lsn, image: &[u8]) {
        assert_eq!(self.page_size, image.len(), "a page image is one page");
        g.write(0, image);
        g.write(0, &lsn.0.to_le_bytes());
        self.mark_replayed(st, Pid(g.pid()), lsn);
    }

    fn redo_change(&self, st: &mut Replay, g: &mut pages::Exclusive<'_>, lsn: Lsn, edits: &[Edit]) {
        for e in edits {
            g.write(e.offset as usize, &e.bytes);
        }
        g.write(0, &lsn.0.to_le_bytes());
        self.mark_replayed(st, Pid(g.pid()), lsn);
    }
}
