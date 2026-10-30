use std::collections::{BTreeMap, BTreeSet};

use crate::runtime::{AlignedBuf, Runtime};

use super::block::{LogFinding, Mode, scan};
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

    /// Returns the transaction of every commit record in the retained log.
    pub fn logged_commits(&self) -> BTreeSet<u64> {
        let mut img = {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.load(d))
        };
        let mut commits = BTreeSet::new();
        let block = img.block;
        if let Some(first) = img.first() {
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

    /// Check every page and the retained log. A page whose LSN is at or past `end`, the log's end
    /// when the check starts, was written since and has no finding. A page that looks wrong is
    /// read again under its exclusive latch, which no write-out or eviction holds with.
    fn verify_to(&self, live: bool) -> Verification {
        let end = self.log().next_lsn();
        self.flush_log();
        let mut img = {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.load(d))
        };
        let mut findings: Vec<Finding> = Vec::new();
        let mut commits = Vec::new();
        let mut safe_snapshots = Vec::new();
        let mut last: BTreeMap<Pid, Lsn> = BTreeMap::new();
        let mut broken: BTreeMap<Pid, Damage> = BTreeMap::new();
        let block = img.block;
        let first = img.first();
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
                    if let Some(expected) = last.insert(pid, lsn).filter(|&l| l != prev) {
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
        if let Some(s) = &s {
            findings.extend(
                s.findings
                    .iter()
                    .filter(|f| !live || f.lsn < end)
                    .map(|f| Finding::Log(*f)),
            );
        }
        let file_pages = self.file_pages();
        let logged = last.keys().next_back().map_or(0, |p| p.0 + 1);
        for n in Pid::FIRST_DATA.0..file_pages.max(logged) {
            let pid = Pid(n);
            let known = self.meta().damaged.get(&pid).copied();
            if let Some(damage) = known {
                findings.push(Finding::Page { pid, damage });
                continue;
            }
            let judge = |disk: &Result<(AlignedBuf, Verdict), Damage>, resident: Option<Lsn>| {
                let bad = match disk {
                    Err(d) => Some(*d),
                    Ok((_, Verdict::Corrupt)) => Some(Damage::Checksum),
                    Ok(_) => None,
                };
                let found = match (disk, resident) {
                    (Ok((_, Verdict::Corrupt)) | Err(_), r) => r,
                    (Ok(_), Some(l)) => Some(l),
                    (Ok((buf, _)), None) => Some(page::lsn(buf)),
                };
                let stale = found.and_then(|found| match (broken.get(&pid), last.get(&pid)) {
                    (Some(&d), _) => Some(d),
                    (None, Some(&expected)) if expected != found && found < end => {
                        Some(Damage::Stale { expected, found })
                    }
                    _ => None,
                });
                let orphan = found.is_some_and(|found| {
                    complete && !last.contains_key(&pid) && found != Lsn(0) && found < end
                });
                (bad, found, stale, orphan)
            };
            let disk = self.read_disk(pid);
            let resident = self.mem.lock_shared(pid.0, &self.rt).map(|g| page::lsn(&g));
            let mut seen = judge(&disk, resident);
            if seen.0.is_some() || seen.1.is_none() || seen.2.is_some() || seen.3 {
                let g = self.mem.lock_exclusive(pid.0, &self.rt);
                let disk = self.read_disk(pid);
                let resident = g.resident().then(|| page::lsn(&g));
                seen = judge(&disk, resident);
            }
            let (bad, found, stale, orphan) = seen;
            if let Some(damage) = bad {
                findings.push(Finding::Page { pid, damage });
            }
            if found.is_none() {
                self.damage(pid, bad.unwrap_or(Damage::Checksum));
                continue;
            }
            if let Some(damage) = stale {
                findings.push(Finding::Page { pid, damage });
                self.damage(pid, damage);
            } else if orphan {
                findings.push(Finding::Orphan(pid));
            }
        }
        findings.sort();
        findings.dedup();
        Verification {
            findings,
            commits,
            safe_snapshots,
        }
    }

    /// Returns the number of pages the data file spans now.
    fn file_pages(&self) -> u64 {
        let len = self.io(|d| d.size(self.data));
        len.div_ceil(self.page_size as u64)
    }
}
