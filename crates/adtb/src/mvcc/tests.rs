use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use super::clog::{Directory, Root, directory_capacity, locate, per_page, set_status, status_at};
use super::txn::extend;
use super::{Holds, RESERVATION, Scope, Snapshot, Status, Xid};
use crate::storage::{HEADER_LEN, Pid};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn status_bits_round_trip_without_disturbing_neighbours() {
    let mut page = vec![0; 4096];
    let n = per_page(page.len()) as usize;
    assert_eq!((4096 - HEADER_LEN) * 4, n);
    for i in (0..n).step_by(3) {
        set_status(&mut page, i, Status::Committed);
    }
    for i in (1..n).step_by(3) {
        set_status(&mut page, i, Status::Aborted);
    }
    for i in 0..n {
        let want = match i % 3 {
            0 => Status::Committed,
            1 => Status::Aborted,
            _ => Status::InProgress,
        };
        assert_eq!(want, status_at(&page, i), "index {i}");
    }
}

#[test]
#[should_panic(expected = "resolved twice")]
fn a_status_resolves_once() {
    let mut page = vec![0; 4096];
    set_status(&mut page, 9, Status::Committed);
    set_status(&mut page, 9, Status::Aborted);
}

#[test]
fn root_and_directory_round_trip() {
    let mut page = vec![0; 4096];
    let root = Root {
        reserved: Xid(5000),
        resolved: Xid(4000),
        bound: Xid(17),
        first_directory: Pid(9),
    };
    root.write(&mut page);
    assert_eq!(root, Root::read(&page));
    let cap = directory_capacity(page.len());
    assert_eq!((4096 - 40) / 16, cap);
    let dir = Directory {
        pid: Pid(9),
        next: Some(Pid(31)),
        entries: (0..cap as u64).map(|n| (n * 3, Pid(100 + n))).collect(),
    };
    dir.write(&mut page);
    assert_eq!(dir, Directory::read(Pid(9), &page));
    let empty = Directory {
        pid: Pid(9),
        next: None,
        entries: Vec::new(),
    };
    empty.write(&mut page);
    assert_eq!(empty, Directory::read(Pid(9), &page));
}

#[test]
fn ids_never_wrap() {
    let per = per_page(65536);
    let top = Xid(u64::MAX - 1);
    let (n, i) = locate(top, per);
    assert_eq!(top.0, n * per + i as u64);
    assert_eq!(Some(Xid(1 + RESERVATION)), extend(Xid(1)));
    assert_eq!(Some(Xid(u64::MAX)), extend(Xid(u64::MAX - 3)));
    assert_eq!(None, extend(Xid(u64::MAX)));

    let old = Xid(1);
    let s = Snapshot::new(top, Xid(u64::MAX), vec![top]);
    let committed = |_| Ok::<_, Infallible>(Status::Committed);
    assert_eq!(Ok(true), s.counts_committed(old, committed));
    assert_eq!(Ok(false), s.counts_committed(top, committed));
    assert_eq!(Ok(true), s.sees(None, old.0, 0, committed));
    assert_eq!(Ok(false), s.sees(None, old.0, u64::MAX - 2, committed));
}

#[test]
fn the_in_progress_test_precedes_the_lookup() {
    let s = Snapshot::new(
        Xid(10),
        Xid(20),
        vec![10, 14].into_iter().map(Xid).collect(),
    );
    let unreachable = |x: Xid| -> Result<Status, Infallible> { panic!("looked up {x:?}") };
    for x in [10, 14, 20, 21, 1000] {
        assert_eq!(Ok(false), s.counts_committed(Xid(x), unreachable));
    }
    assert_eq!(Ok(true), s.sees(Some(Xid(14)), 14, 0, unreachable));
    assert_eq!(Ok(false), s.sees(Some(Xid(14)), 14, 14, unreachable));
}

#[test]
#[should_panic(expected = "not sorted")]
fn a_snapshot_rejects_an_unsorted_in_progress_set() {
    Snapshot::new(Xid(1), Xid(9), vec![Xid(5), Xid(3)]);
}

#[test]
fn visibility_matches_a_model_of_random_histories() {
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for round in 0..200 {
        let mut ends: BTreeMap<Xid, Option<(usize, Status)>> = BTreeMap::new();
        let mut running: BTreeSet<Xid> = BTreeSet::new();
        let mut next = Xid::FIRST;
        let mut snapshots: Vec<(usize, Option<Xid>, Snapshot)> = Vec::new();
        let mut versions: Vec<(u64, u64)> = Vec::new();
        for event in 0..300 {
            match rng.below(10) {
                0..=2 => {
                    running.insert(next);
                    ends.insert(next, None);
                    next = Xid(next.0 + 1);
                }
                3..=5 if !running.is_empty() => {
                    let pick = rng.below(running.len() as u64) as usize;
                    let x = *running.iter().nth(pick).unwrap();
                    running.remove(&x);
                    let status = if rng.below(3) == 0 {
                        Status::Aborted
                    } else {
                        Status::Committed
                    };
                    ends.insert(x, Some((event, status)));
                }
                6 | 7 if !running.is_empty() => {
                    let pick = rng.below(running.len() as u64) as usize;
                    let writer = *running.iter().nth(pick).unwrap();
                    if versions.is_empty() || rng.below(2) == 0 {
                        versions.push((writer.0, 0));
                    } else {
                        let v = rng.below(versions.len() as u64) as usize;
                        if versions[v].1 == 0 {
                            versions[v].1 = writer.0;
                        }
                    }
                }
                _ => {
                    let in_progress: Vec<Xid> = running.iter().copied().collect();
                    let xmin = in_progress.first().copied().unwrap_or(next);
                    let reader = (!in_progress.is_empty() && rng.below(2) == 0)
                        .then(|| in_progress[rng.below(in_progress.len() as u64) as usize]);
                    snapshots.push((event, reader, Snapshot::new(xmin, next, in_progress)));
                }
            }
        }
        let status = |x: Xid| -> Result<Status, Infallible> {
            Ok(ends[&x].map_or(Status::InProgress, |(_, s)| s))
        };
        for (taken, reader, s) in &snapshots {
            let committed_then = |x: u64| {
                Some(Xid(x)) == *reader
                    || ends
                        .get(&Xid(x))
                        .copied()
                        .flatten()
                        .is_some_and(|(at, st)| at < *taken && st == Status::Committed)
            };
            for &(xmin, xmax) in &versions {
                let want = committed_then(xmin) && (xmax == 0 || !committed_then(xmax));
                assert_eq!(
                    Ok(want),
                    s.sees(*reader, xmin, xmax, status),
                    "round {round}: version ({xmin}, {xmax}) at {s:?} for {reader:?}"
                );
            }
        }
    }
}

#[test]
fn the_watermark_never_passes_a_live_snapshot() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut holds = Holds::default();
    let mut live: Vec<(super::SnapshotId, Xid, Scope)> = Vec::new();
    let mut replicas: BTreeMap<u64, BTreeMap<u32, Xid>> = BTreeMap::new();
    let mut next = 1;
    for _ in 0..5000 {
        next += rng.below(3);
        match rng.below(8) {
            0..=2 => {
                let xmin = Xid(next - rng.below(next.min(20)));
                let scope = if rng.below(4) == 0 {
                    Scope::Every
                } else {
                    Scope::Heaps((0..rng.below(3)).map(|_| rng.below(6) as u32).collect())
                };
                live.push((holds.hold(xmin, scope.clone()), xmin, scope));
            }
            3 | 4 if !live.is_empty() => {
                let (id, _, _) = live.swap_remove(rng.below(live.len() as u64) as usize);
                holds.release(id);
            }
            5 => {
                let replica = rng.below(3);
                let report: BTreeMap<u32, Xid> = (0..rng.below(3))
                    .map(|_| (rng.below(6) as u32, Xid(next - rng.below(next.min(30)))))
                    .collect();
                holds.report_replica(replica, report.clone());
                replicas.insert(replica, report);
            }
            6 => {
                let replica = rng.below(3);
                holds.forget_replica(replica);
                replicas.remove(&replica);
            }
            _ => {}
        }
        for heap in 0..6 {
            let w = holds.watermark(heap, Xid(next));
            let covering = live
                .iter()
                .filter(|(_, _, scope)| match scope {
                    Scope::Every => true,
                    Scope::Heaps(hs) => hs.contains(&heap),
                })
                .map(|(_, xmin, _)| *xmin);
            let reported = replicas.values().filter_map(|r| r.get(&heap).copied());
            let oldest = covering.chain(reported).fold(Xid(next), Xid::min);
            assert_eq!(oldest, w, "heap {heap}");
        }
    }
}

#[test]
fn a_scoped_reader_holds_back_only_its_heaps() {
    let mut holds = Holds::default();
    let report = holds.hold(Xid(5), Scope::Heaps(BTreeSet::from([1])));
    assert_eq!(Xid(5), holds.watermark(1, Xid(50)));
    assert_eq!(Xid(50), holds.watermark(2, Xid(50)));
    let interactive = holds.hold(Xid(9), Scope::Every);
    assert_eq!(Xid(9), holds.watermark(2, Xid(50)));
    holds.release(report);
    assert_eq!(Xid(9), holds.watermark(1, Xid(50)));
    holds.release(interactive);
    assert_eq!(Xid(50), holds.watermark(1, Xid(50)));
}
