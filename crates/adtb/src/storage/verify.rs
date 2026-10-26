use std::collections::BTreeMap;

use crate::runtime::Runtime;

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
        self.flush_log();
        let img = {
            let mut segments = self.segments.lock(&self.rt);
            self.io(|d| segments.load(d))
        };
        let s = img.first().map(|first| scan(&img, first, Mode::Verify));
        let mut findings: Vec<Finding> = Vec::new();
        let mut commits = Vec::new();
        let mut safe_snapshots = Vec::new();
        let mut last: BTreeMap<Pid, Lsn> = BTreeMap::new();
        let mut broken: BTreeMap<Pid, Damage> = BTreeMap::new();
        let complete = s
            .as_ref()
            .is_some_and(|s| s.resumed.is_empty() && img.first() == Some(Lsn(0)));
        if let Some(s) = &s {
            findings.extend(s.findings.iter().map(|f| Finding::Log(*f)));
            for (i, batch) in s.batches.iter().enumerate() {
                if s.resumed.contains(&i) {
                    last.clear();
                }
                let Ok(records) = batch.records(img.block) else {
                    continue;
                };
                for (lsn, record) in records {
                    match &record {
                        Record::Commit { xid, .. } => commits.push(*xid),
                        Record::SafeSnapshot { at, snapshot } => {
                            safe_snapshots.push((lsn, *at, snapshot.clone()))
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
            }
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
            let disk = self.read_disk(pid);
            let resident = self.mem.lock_shared(pid.0, &self.rt).map(|g| page::lsn(&g));
            let found = match (&disk, resident) {
                (Err(d), _) => {
                    findings.push(Finding::Page { pid, damage: *d });
                    resident
                }
                (Ok((_, Verdict::Corrupt)), _) => {
                    findings.push(Finding::Page {
                        pid,
                        damage: Damage::Checksum,
                    });
                    resident
                }
                (Ok(_), Some(l)) => Some(l),
                (Ok((buf, _)), None) => Some(page::lsn(buf)),
            };
            let Some(found) = found else {
                let damage = if disk.is_err() {
                    Damage::Unreadable
                } else {
                    Damage::Checksum
                };
                self.damage(pid, damage);
                continue;
            };
            let stale = match (broken.get(&pid), last.get(&pid)) {
                (Some(&d), _) => Some(d),
                (None, Some(&expected)) if expected != found => {
                    Some(Damage::Stale { expected, found })
                }
                _ => None,
            };
            if let Some(damage) = stale {
                findings.push(Finding::Page { pid, damage });
                self.damage(pid, damage);
            } else if complete && !last.contains_key(&pid) && found != Lsn(0) {
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
        let len = self
            .io(|d| d.open(&self.name, false))
            .map_or(0, |(_, len)| len);
        len.div_ceil(self.page_size as u64)
    }
}
