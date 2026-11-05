use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::{AlignedBuf, Runtime};

use super::block::{LogFinding, LogProblem, Mode, commits_from, scan};
use super::log::parse_segment;
use super::page::{self, Verdict};
use super::pager::{Damage, Pager};
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
    pub fn logged_commits(&self) -> BTreeSet<u64> {
        let loaded = {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.try_load(d))
        };
        let mut commits = BTreeSet::new();
        let Ok(mut img) = loaded else {
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

    /// Returns the transaction of every commit record in the retained log from the batch holding
    /// `since`, or from its first whole batch when that batch begins before its first segment.
    ///
    /// Fail with the LSN of the first damage before the end of the log recovery kept: a segment
    /// it cannot list, open or read, a batch whose records do not decode, or a block missing.
    pub fn logged_commits_since(&self, since: Lsn) -> Result<BTreeSet<u64>, Lsn> {
        let loaded = {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.try_load(d))
        };
        let mut img = loaded.map_err(|_| since)?;
        commits_from(&mut img, since, self.meta().recovered)
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
        let loaded = {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.try_load(d))
        };
        let mut img = match loaded {
            Ok(img) => img,
            Err((what, _)) => return RetainedLog::unreadable(end, &what),
        };
        let mut commits = Vec::new();
        let mut safe_snapshots = Vec::new();
        let mut last: BTreeMap<Pid, Lsn> = BTreeMap::new();
        let mut broken: BTreeMap<Pid, Damage> = BTreeMap::new();
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
                        _ => {}
                    }
                    let Some((pid, prev)) = record.page() else {
                        continue;
                    };
                    if prev == Lsn::UNCHAINED {
                        broken.remove(&pid);
                    }
                    let chained = prev != Lsn::UNCHAINED;
                    if let Some(expected) = last.insert(pid, lsn).filter(|&l| chained && l != prev)
                    {
                        broken.entry(pid).or_insert(Damage::Stale {
                            expected,
                            found: prev,
                        });
                    }
                }
            })
        });
        let complete = s
            .as_ref()
            .is_some_and(|s| s.resumed == 0 && first == Some(Lsn(0)));
        let gapless = s.as_ref().is_some_and(|s| s.resumed == 0);
        let recovered = self.meta().recovered;
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
        let refused = self
            .meta()
            .damaged
            .keys()
            .next_back()
            .map_or(0, |p| p.0 + 1);
        self.file_pages().max(logged).max(refused)
    }

    /// Check page `pid` against `log`, refusing it from now on if its contents are lost. Read the
    /// file's copy whether or not the pool holds the page, since only that copy can have rotted;
    /// the pool's copy, when there is one, gives the LSN. A page that looks wrong is read again
    /// under its exclusive latch, which no write-out or eviction holds with.
    ///
    /// Judge a page by what holds it now. A file copy the pool's changed copy will overwrite is not
    /// damage, and a page the allocation bitmap calls free has no finding unless already refused.
    pub fn check_page(&self, pid: Pid, log: &RetainedLog) -> Vec<Finding> {
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
        let known = self.meta().damaged.get(&pid).copied();
        if let Some(damage) = known {
            findings.push(Finding::Page { pid, damage });
            findings.extend(lost_record(pid, damage));
            return findings;
        }
        let judge =
            |disk: &Result<(AlignedBuf, Verdict), Damage>, resident: Option<Lsn>, changed: bool| {
                let bad = match disk {
                    _ if changed => None,
                    Err(d) => Some(*d),
                    Ok((_, Verdict::Corrupt)) => Some(Damage::Checksum),
                    Ok(_) => None,
                };
                let found = match (disk, resident) {
                    (Ok((_, Verdict::Corrupt)) | Err(_), r) => r,
                    (Ok(_), Some(l)) => Some(l),
                    (Ok((buf, _)), None) => Some(page::lsn(buf)),
                };
                let future = match (disk, resident) {
                    (Ok((buf, Verdict::Valid)), None) => {
                        Damage::past_log_end(page::lsn(buf), self.file_end(pid))
                    }
                    _ => None,
                };
                let stale = future.or_else(|| {
                    found.and_then(|found| match (broken.get(&pid), last.get(&pid)) {
                        (Some(&d), _) if found < *end => Some(d),
                        (None, Some(&expected)) if expected != found && found < *end => {
                            Some(Damage::Stale { expected, found })
                        }
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
        let resident = self.mem.lock_shared(pid.0, &self.rt).map(|g| page::lsn(&g));
        let mut seen = judge(&disk, resident, false);
        let wrong = |seen: &Judged| {
            seen.0.is_some() || seen.1.is_none() || seen.2.is_some() || seen.3 || seen.4.is_some()
        };
        // Twice: a page freed and allocated again between the latched read and the look at the
        // bitmap is judged afresh. The bitmap's latch is never taken under this page's.
        for _ in 0..2 {
            if !wrong(&seen) {
                return findings;
            }
            if self.called_free(pid) {
                return findings;
            }
            let g = self.mem.lock_exclusive(pid.0, &self.rt);
            let disk = self.read_disk(pid);
            let resident = g.resident().then(|| page::lsn(&g));
            let changed = self.meta().changed.contains(&pid);
            seen = judge(&disk, resident, changed);
        }
        if !wrong(&seen) || self.called_free(pid) {
            return findings;
        }
        let (bad, found, stale, orphan, unlogged) = seen;
        if let Some(damage) = bad {
            findings.push(Finding::Page { pid, damage });
        }
        let Some(found) = found else {
            // Leave an unreadable page unrecorded for the next read to retry.
            if bad != Some(Damage::Unreadable) {
                self.damage(pid, bad.unwrap_or(Damage::Checksum));
            }
            return findings;
        };
        if let Some(lsn) = unlogged {
            findings.push(unlogged_at(pid, lsn, found));
        }
        if let Some(damage) = stale {
            findings.push(Finding::Page { pid, damage });
            findings.extend(lost_record(pid, damage));
            self.damage(pid, damage);
        } else if orphan && unlogged.is_none() {
            findings.push(Finding::Orphan(pid));
        }
        findings
    }

    /// Returns the number of pages the data file spans now.
    pub(crate) fn file_pages(&self) -> u64 {
        let len = self.io(|d| d.size(self.data));
        len.div_ceil(self.page_size as u64)
    }
}
