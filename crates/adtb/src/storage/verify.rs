use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::{AlignedBuf, Runtime};

use super::block::{LogFinding, LogProblem, Mode, Resolutions, resolutions_from, scan};
use super::error::Damage;
use super::evidence::{self, Against, file_copy};
use super::log::parse_segment;
use super::page::{self, Verdict};
use super::pager::Pager;
use super::quarantine::Bitmapped;
use super::record::Record;
use super::{Lsn, Pid, Position};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Finding {
    Page {
        pid: Pid,
        damage: Damage,
    },
    /// A valid page that no record of a complete log wrote.
    Orphan(Pid),
    Log(LogFinding),
    /// The damage list ran out of room, so the whole file counts as damaged.
    Unlisted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verification {
    pub findings: Vec<Finding>,
    /// The transaction of every commit record in the retained log, in log order.
    pub commits: Vec<u64>,
    /// Every safe snapshot marker in the retained log, in log order, with its LSN.
    pub safe_snapshots: Vec<(Lsn, Position, Vec<u8>)>,
}

/// A page's damage, the LSN it is judged at, what the log says of that LSN, whether no record wrote
/// it, and the LSN past which the retained log holds every record but none wrote it.
type Judged = (
    Option<Damage>,
    Option<Lsn>,
    Option<Damage>,
    bool,
    Option<Lsn>,
);

/// What the retained log says of every page at one moment.
#[derive(Clone, Debug)]
pub struct RetainedLog {
    /// The log's end when it was read. A page at or past it was written since.
    end: Lsn,
    last: BTreeMap<Pid, Lsn>,
    broken: BTreeMap<Pid, Damage>,
    /// Whether the log runs from its start with no gap, so every page written has a record.
    complete: bool,
    /// Whether the log has no gap.
    gapless: bool,
    /// The end of the log recovery kept, when the log runs from at or before it with no gap.
    recovered: Option<Lsn>,
    /// Whether writers may run during the check.
    live: bool,
    findings: Vec<Finding>,
    commits: Vec<u64>,
    safe_snapshots: Vec<(Lsn, Position, Vec<u8>)>,
}

impl RetainedLog {
    /// Returns a log that says nothing of any page and reports the segment `what`, or the log's
    /// start when `what` names none, as unreadable.
    fn unreadable(end: Lsn, what: &str) -> RetainedLog {
        let lsn = Lsn(parse_segment(what).unwrap_or(0));
        RetainedLog {
            end,
            last: BTreeMap::new(),
            broken: BTreeMap::new(),
            complete: false,
            gapless: false,
            recovered: None,
            live: false,
            findings: vec![Finding::Log(LogFinding {
                lsn,
                problem: LogProblem::Unreadable,
            })],
            commits: Vec::new(),
            safe_snapshots: Vec::new(),
        }
    }
}

fn unlogged_at(pid: Pid, lsn: Lsn, page: Lsn) -> Finding {
    Finding::Log(LogFinding {
        lsn,
        problem: LogProblem::Unlogged { pid, page },
    })
}

/// Returns the log finding a stale page `pid` implies when the LSN it is found at is past the one
/// the log expects.
fn lost_record(pid: Pid, damage: Damage) -> Option<Finding> {
    match damage {
        Damage::Stale { expected, found } if found > expected => {
            Some(unlogged_at(pid, expected, found))
        }
        _ => None,
    }
}

impl<R: Runtime> Pager<R> {
    /// Check every page of the file and the whole retained log, refusing from now on every page
    /// found damaged.
    ///
    /// A page is damaged when the file's copy fails verification and the pool holds no newer one,
    /// or when its LSN differs from that of the last record the log holds for it, or when a record
    /// of it does not continue from the one before.
    pub fn verify(&self) -> Verification {
        self.verify_to(false)
    }

    /// Check as [`Pager::verify`] does while writers run. Reports no log finding at or past the
    /// log's end when the check starts.
    pub fn verify_live(&self) -> Verification {
        self.verify_to(true)
    }

    /// Returns the transaction of every commit record in the retained log, none when its segments
    /// cannot be listed or opened.
    pub(crate) fn logged_commits(&self) -> BTreeSet<u64> {
        let loaded = self.load_log();
        let mut commits = BTreeSet::new();
        let Ok((_pin, mut img)) = loaded else {
            return commits;
        };
        let block = img.block;
        if let Some(first) = img.first_batch() {
            scan(&mut img, first, Mode::Verify, |batch, _| {
                if let Ok(records) = batch.records(block) {
                    for (_, record) in records {
                        if let Record::Commit { xid, .. } = record {
                            commits.insert(xid);
                        }
                    }
                }
            });
        }
        commits
    }

    /// Returns each damage record in the retained log, in log order, none when its segments cannot
    /// be listed or opened.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn logged_damage(&self) -> Vec<(Pid, Damage)> {
        let mut damage = Vec::new();
        let Ok((_pin, mut img)) = self.load_log() else {
            return damage;
        };
        let block = img.block;
        if let Some(first) = img.first_batch() {
            scan(&mut img, first, Mode::Verify, |batch, _| {
                if let Ok(records) = batch.records(block) {
                    for (_, record) in records {
                        if let Record::Damaged { pid, damage: d } = record {
                            damage.push((pid, d));
                        }
                    }
                }
            });
        }
        damage
    }

    /// Returns each safe snapshot record in the retained log, in log order: the commit position
    /// it covers and its snapshot's bytes. None when its segments cannot be listed or opened.
    ///
    /// Panic at a batch whose records do not decode.
    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn logged_safe_snapshots(&self) -> Vec<(Position, Vec<u8>)> {
        let mut markers = Vec::new();
        let Ok((_pin, mut img)) = self.load_log() else {
            return markers;
        };
        let block = img.block;
        if let Some(first) = img.first_batch() {
            scan(&mut img, first, Mode::Verify, |batch, _| {
                let records = batch
                    .records(block)
                    .expect("a batch of the retained log decodes");
                for (_, record) in records {
                    if let Record::SafeSnapshot { at, snapshot } = record {
                        markers.push((at, snapshot));
                    }
                }
            });
        }
        markers
    }

    /// Returns the transaction of every commit record in the retained log from the batch holding
    /// `since`, or from its first whole batch when that batch begins before its first
    /// segment, and what each record there of one of `pages` continues from.
    ///
    /// Fail with the LSN of the first damage before the end of the log recovery kept: a segment
    /// it cannot list, open or read, a batch whose records do not decode, or a block missing.
    pub fn logged_resolutions_since(
        &self,
        since: Lsn,
        pages: &BTreeSet<Pid>,
    ) -> Result<Resolutions, Lsn> {
        let (_pin, mut img) = self.load_log().map_err(|_| since)?;
        let recovered = self.evidence().recovered();
        resolutions_from(&mut img, since, recovered, pages)
    }

    /// Check every page and the retained log. A page whose LSN is at or past the log's end when the
    /// check starts has no orphan finding, and a stale one only when the file's copy is at or past
    /// [`Pager::file_end`]. When `live` it has no unlogged finding either, unless it is stale.
    fn verify_to(&self, live: bool) -> Verification {
        let mut log = self.read_log(live);
        let mut findings = std::mem::take(&mut log.findings);
        for n in Pid::FIRST_DATA.0..self.pages_within(&log) {
            findings.extend(self.check_page(Pid(n), &log));
        }
        let recorded = self.recorded();
        findings.extend(recorded.log.into_iter().map(Finding::Log));
        if recorded.full {
            findings.push(Finding::Unlisted);
        }
        findings.sort();
        findings.dedup();
        Verification {
            findings,
            commits: log.commits,
            safe_snapshots: log.safe_snapshots,
        }
    }

    /// Returns what the retained log says of every page, as of now.
    pub fn read_log(&self, live: bool) -> RetainedLog {
        let end = self.log().next_lsn();
        self.flush_log();
        let (_pin, mut img) = match self.load_log() {
            Ok(loaded) => loaded,
            Err((what, _)) => return RetainedLog::unreadable(end, &what),
        };
        let mut commits = Vec::new();
        let mut safe_snapshots = Vec::new();
        let mut last: BTreeMap<Pid, Lsn> = BTreeMap::new();
        let mut broken: BTreeMap<Pid, Damage> = BTreeMap::new();
        let mut settled: BTreeMap<Pid, Damage> = BTreeMap::new();
        let block = img.block;
        let first = img.first_batch();
        let s = first.map(|first| {
            scan(&mut img, first, Mode::Verify, |batch, after_gap| {
                if after_gap {
                    last.clear();
                }
                let Ok(records) = batch.records(block) else {
                    return;
                };
                for (lsn, record) in records {
                    match &record {
                        Record::Commit { xid, .. } => commits.push(*xid),
                        Record::SafeSnapshot { at, snapshot } => {
                            safe_snapshots.push((lsn, *at, snapshot.clone()));
                        }
                        Record::Damaged {
                            pid,
                            damage: Damage::Settled { expected, found },
                        } => {
                            let (expected, found) = (*expected, *found);
                            settled.insert(*pid, Damage::Stale { expected, found });
                        }
                        _ => {}
                    }
                    let Some((pid, prev)) = record.page() else {
                        continue;
                    };
                    if prev == Lsn::UNCHAINED {
                        broken.remove(&pid);
                        settled.remove(&pid);
                    }
                    let chained = prev != Lsn::UNCHAINED;
                    if let Some(expected) = last.insert(pid, lsn).filter(|&l| chained && l != prev)
                    {
                        let damage = Damage::Stale {
                            expected,
                            found: prev,
                        };
                        if settled.get(&pid) != Some(&damage) {
                            broken.entry(pid).or_insert(damage);
                        }
                    }
                }
            })
        });
        let complete = s
            .as_ref()
            .is_some_and(|s| s.resumed == 0 && first == Some(Lsn(0)));
        let gapless = s.as_ref().is_some_and(|s| s.resumed == 0);
        let recovered = self.evidence().recovered();
        let recovered = (gapless && first.is_some_and(|f| f <= recovered)).then_some(recovered);
        let findings = s.iter().flat_map(|s| &s.findings);
        let findings = findings
            .filter(|f| !live || f.lsn < end)
            .map(|f| Finding::Log(*f))
            .collect();
        RetainedLog {
            end,
            last,
            broken,
            complete,
            gapless,
            recovered,
            live,
            findings,
            commits,
            safe_snapshots,
        }
    }

    /// Returns the end of the page numbers to check against `log`.
    pub fn pages_within(&self, log: &RetainedLog) -> u64 {
        let logged = log.last.keys().next_back().map_or(0, |p| p.0 + 1);
        let evidence = self.evidence().end();
        self.file_pages().max(logged).max(evidence)
    }

    /// Check page `pid` against `log`, refusing it from now on if its contents are lost. Read the
    /// file's copy whether or not the pool holds the page, since only that copy can have rotted;
    /// the pool's copy, when there is one, gives the LSN. A page that looks wrong is read again
    /// under its exclusive latch, which no write-out or eviction holds with.
    ///
    /// Judge a page by what holds it now. A file copy the pool's changed copy will overwrite is not
    /// damage. A page the allocation bitmap calls free has no finding but a chain break of its
    /// last incarnation the log holds and no settle excuses, unless already refused. Report the
    /// damage recorded of it as well.
    pub fn check_page(&self, pid: Pid, log: &RetainedLog) -> Vec<Finding> {
        let mut findings = self.judge_page(pid, log);
        let recorded = self.evidence().damage(pid);
        if let Some(damage) = recorded {
            let found = Finding::Page { pid, damage };
            if !findings.contains(&found) {
                findings.push(found);
            }
        }
        findings
    }

    fn judge_page(&self, pid: Pid, log: &RetainedLog) -> Vec<Finding> {
        let mut findings = Vec::new();
        let RetainedLog {
            end,
            last,
            broken,
            complete,
            gapless,
            recovered,
            live,
            ..
        } = log;
        let known = self.refusal(pid);
        if let Some(damage) = known {
            findings.push(Finding::Page { pid, damage });
            findings.extend(lost_record(pid, damage));
            return findings;
        }
        let mapped = if last.contains_key(&pid) {
            None
        } else {
            self.mapped(pid).unwrap_or_else(|e| {
                findings.push(Finding::Page {
                    pid: e.pid,
                    damage: e.damage,
                });
                None
            })
        };
        let broken = broken.get(&pid).copied();
        let broken = broken.filter(|&d| !self.settled_at(pid, d));
        let judge =
            |disk: &Result<(AlignedBuf, Verdict), Damage>, resident: Option<Lsn>, changed: bool| {
                let judged = self.judge(pid, file_copy(disk), Against::LogEnd);
                let bad = match judged {
                    evidence::Judged::Bad(d) if !changed => Some(d),
                    _ => None,
                };
                let found = match (disk, resident) {
                    (Ok((_, Verdict::Corrupt)) | Err(_), r) => r,
                    (Ok(_), Some(l)) => Some(l),
                    (Ok((buf, _)), None) => Some(page::lsn(buf)),
                };
                let future = match (judged, resident) {
                    (evidence::Judged::Stale(d), None) => Some(d),
                    _ => None,
                };
                let stale = future.or_else(|| {
                    found.and_then(|found| match (broken, last.get(&pid)) {
                        (Some(d), _) if found < *end => Some(d),
                        (None, Some(&expected)) if expected != found && found < *end => {
                            Some(Damage::Stale { expected, found })
                        }
                        (None, None) => mapped
                            .filter(|&expected| found < expected && found < *end)
                            .map(|expected| Damage::Stale { expected, found }),
                        _ => None,
                    })
                });
                let orphan = found.is_some_and(|found| {
                    *complete && !last.contains_key(&pid) && found != Lsn(0) && found < *end
                });
                let unlogged = found
                    .filter(|&found| found != Lsn(0) && (found < *end || !live))
                    .and_then(|found| {
                        let last = last.get(&pid).copied();
                        match *recovered {
                            Some(r) if found >= r && last.is_none_or(|e| found > e) => Some(r),
                            _ => last.filter(|&e| *gapless && found > e),
                        }
                    });
                (bad, found, stale, orphan, unlogged)
            };
        let disk = self.read_disk(pid);
        let resident = self.shared_resident(pid).map(|g| page::lsn(&g));
        let mut seen = judge(&disk, resident, false);
        let wrong = |seen: &Judged| {
            seen.0.is_some() || seen.1.is_none() || seen.2.is_some() || seen.3 || seen.4.is_some()
        };
        // Twice: a page freed and allocated again between the latched read and the look at the
        // bitmap is judged afresh. The bitmap's latch is never taken under this page's.
        let mut latched = None;
        for _ in 0..2 {
            drop(latched.take());
            if !wrong(&seen) {
                return findings;
            }
            if self.bitmapped(pid) == Bitmapped::Free {
                if let Some(damage) = broken {
                    findings.push(Finding::Page { pid, damage });
                    findings.extend(lost_record(pid, damage));
                }
                return findings;
            }
            let g = self.hold(pid);
            let disk = self.read_disk(pid);
            let resident = g.resident().then(|| page::lsn(&g));
            let changed = self.is_unwritten(pid);
            seen = judge(&disk, resident, changed);
            latched = Some(g);
        }
        if !wrong(&seen) {
            return findings;
        }
        let latch = latched.expect("the second judgement holds its latch");
        let (bad, found, stale, orphan, unlogged) = seen;
        if let Some(damage) = bad {
            findings.push(Finding::Page { pid, damage });
            if found.is_some() {
                self.evidence().hold(pid, damage);
            }
        }
        let Some(found) = found else {
            // Leave an unreadable page unrecorded for the next read to retry.
            if bad != Some(Damage::Unreadable) {
                self.refuse_latched(latch, bad.unwrap_or(Damage::Checksum));
            }
            return findings;
        };
        if let Some(lsn) = unlogged {
            findings.push(unlogged_at(pid, lsn, found));
        }
        if let Some(damage) = stale.filter(|&d| !self.settled_at(pid, d)) {
            findings.push(Finding::Page { pid, damage });
            findings.extend(lost_record(pid, damage));
            self.refuse_latched(latch, damage);
        } else if orphan && unlogged.is_none() {
            findings.push(Finding::Orphan(pid));
        }
        findings
    }

    /// Returns whether `pid` is recorded settled at the LSNs `damage` finds it stale at.
    fn settled_at(&self, pid: Pid, damage: Damage) -> bool {
        let Damage::Stale { expected, found } = damage else {
            return false;
        };
        self.evidence().damage(pid) == Some(Damage::Settled { expected, found })
    }

    /// Returns the number of pages the data file spans now.
    pub(crate) fn file_pages(&self) -> u64 {
        let len = self.io(|d| d.size(self.data()));
        len.div_ceil(self.page_size() as u64)
    }
}
