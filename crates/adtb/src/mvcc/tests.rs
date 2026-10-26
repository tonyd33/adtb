use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;

use std::ops::Bound;

use super::clog::{Directory, Root, directory_capacity, locate, per_page, set_status, status_at};
use super::txn::{Assign, assign, extend};
use super::{
    Access, Cause, Holds, Limits, Probe, RESERVATION, Safety, Scope, SerialId,
    SerializationFailure, Serializer, Snapshot, Status, Write, Xid, check_write, probe_unique,
    read_before,
};
use crate::key::{CtorId, Tid};
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

fn statuses(table: &[(u64, Status)]) -> impl FnMut(Xid) -> Result<Status, Infallible> + '_ {
    |x| {
        Ok(table
            .iter()
            .find(|(id, _)| *id == x.0)
            .map_or(Status::Committed, |(_, s)| *s))
    }
}

/// Transaction 10 probing at a snapshot that counts 1 to 4 as committed, with 5 to 9 started
/// later or running: 5 and 6 still running, 7 committed after the snapshot, 8 aborted.
fn probe(xmin: u64, xmax: u64) -> Probe {
    let snapshot = Snapshot::new(Xid(5), Xid(7), vec![Xid(5), Xid(6)]);
    let table = [
        (5, Status::InProgress),
        (6, Status::InProgress),
        (7, Status::Committed),
        (8, Status::Aborted),
        (10, Status::InProgress),
    ];
    probe_unique(Xid(10), &snapshot, xmin, xmax, statuses(&table)).unwrap()
}

#[test]
fn a_live_committed_version_in_the_snapshot_conflicts() {
    assert_eq!(Probe::Conflict, probe(3, 0));
}

#[test]
fn a_live_version_committed_after_the_snapshot_fails() {
    assert_eq!(Probe::Fail(later(7)), probe(7, 0));
}

#[test]
fn a_version_inserted_by_a_running_transaction_waits_for_it() {
    assert_eq!(Probe::Wait(Xid(5)), probe(5, 0));
    assert_eq!(Probe::Wait(Xid(5)), probe(5, 5));
}

#[test]
fn a_version_deleted_before_the_snapshot_is_skipped() {
    assert_eq!(Probe::Skip, probe(2, 3));
}

#[test]
fn a_version_deleted_after_the_snapshot_fails() {
    assert_eq!(Probe::Fail(later(7)), probe(2, 7));
}

#[test]
fn a_visible_version_a_running_transaction_deletes_waits_for_the_deleter() {
    assert_eq!(Probe::Wait(Xid(6)), probe(2, 6));
}

#[test]
fn a_version_committed_after_the_snapshot_and_being_deleted_fails() {
    assert_eq!(Probe::Fail(later(7)), probe(7, 6));
}

#[test]
fn dead_versions_and_the_inserters_own_deletes_are_skipped() {
    assert_eq!(Probe::Skip, probe(8, 0));
    assert_eq!(Probe::Skip, probe(2, 10));
    assert_eq!(Probe::Skip, probe(10, 10));
}

#[test]
fn a_version_whose_deleter_aborted_is_live() {
    assert_eq!(Probe::Conflict, probe(2, 8));
    assert_eq!(Probe::Fail(later(7)), probe(7, 8));
}

#[test]
fn the_inserters_own_version_conflicts() {
    assert_eq!(Probe::Conflict, probe(10, 0));
}

#[test]
fn a_write_waits_for_a_running_deleter_and_fails_on_a_committed_one() {
    let table = [
        (5, Status::InProgress),
        (7, Status::Committed),
        (8, Status::Aborted),
    ];
    let check = |xmax| check_write(Xid(10), xmax, statuses(&table)).unwrap();
    assert_eq!(Write::Proceed, check(0));
    assert_eq!(Write::Proceed, check(10));
    assert_eq!(Write::Proceed, check(8));
    assert_eq!(Write::Wait(Xid(5)), check(5));
    assert_eq!(Write::Fail(later(7)), check(7));
}

#[test]
fn a_read_depends_on_concurrent_writers_only() {
    let snapshot = Snapshot::new(Xid(5), Xid(7), vec![Xid(5), Xid(6)]);
    let table = [
        (5, Status::InProgress),
        (6, Status::InProgress),
        (7, Status::Committed),
        (8, Status::Aborted),
    ];
    let before =
        |xmin, xmax| read_before(Some(Xid(10)), &snapshot, xmin, xmax, statuses(&table)).unwrap();
    assert_eq!(Some(Xid(5)), before(5, 0));
    assert_eq!(Some(Xid(7)), before(7, 0));
    assert_eq!(Some(Xid(6)), before(2, 6));
    assert_eq!(Some(Xid(7)), before(2, 7));
    assert_eq!(None, before(2, 3));
    assert_eq!(None, before(2, 0));
    assert_eq!(None, before(8, 0));
    assert_eq!(None, before(2, 8));
    assert_eq!(None, before(10, 0));
    assert_eq!(None, before(2, 10));
}

const HEAP: u32 = 10;
const INDEX: Pid = Pid(100);
const G: CtorId = CtorId(1);
const A: CtorId = CtorId(2);
const E: CtorId = CtorId(3);

fn key(k: u8) -> Vec<u8> {
    vec![k]
}

fn read_key(s: &mut Serializer, id: SerialId, k: u8, ctors: &[CtorId]) {
    s.read_range(
        id,
        INDEX,
        Bound::Included(key(k)),
        Bound::Included(key(k)),
        ctors.iter().copied().collect(),
    )
    .unwrap();
}

fn rw(s: &mut Serializer, x: u64) -> SerialId {
    s.begin(Access::ReadWrite(Xid(x)))
}

fn later(x: u64) -> SerializationFailure {
    SerializationFailure::new(Cause::LaterCommit, vec![Xid(x)])
}

fn dangerous(with: &[u64]) -> SerializationFailure {
    SerializationFailure::new(Cause::Dangerous, with.iter().map(|&x| Xid(x)).collect())
}

fn commit(s: &mut Serializer, id: SerialId) -> Result<(), SerializationFailure> {
    s.precommit(id)?;
    s.committed(id);
    Ok(())
}

fn tid(page: u64, slot: u16) -> Tid {
    Tid::new(page, slot).unwrap()
}

#[test]
fn write_skew_aborts_one_side() {
    let mut s = Serializer::default();
    let (t1, t2) = (rw(&mut s, 1), rw(&mut s, 2));
    for t in [t1, t2] {
        read_key(&mut s, t, 1, &[E]);
        read_key(&mut s, t, 2, &[E]);
    }
    s.insert_entry(t1, INDEX, &key(1), E).unwrap();
    s.insert_entry(t2, INDEX, &key(2), E).unwrap();
    assert_eq!(BTreeSet::from([t2]), s.writers_after(t1));
    assert_eq!(BTreeSet::from([t1]), s.writers_after(t2));
    commit(&mut s, t1).unwrap();
    assert_eq!(Err(dangerous(&[1])), commit(&mut s, t2));
}

#[test]
fn write_skew_fails_at_the_write_once_the_other_side_committed() {
    let mut s = Serializer::default();
    let (t1, t2) = (rw(&mut s, 1), rw(&mut s, 2));
    for t in [t1, t2] {
        read_key(&mut s, t, 1, &[E]);
        read_key(&mut s, t, 2, &[E]);
    }
    s.insert_entry(t1, INDEX, &key(1), E).unwrap();
    commit(&mut s, t1).unwrap();
    assert_eq!(Err(dangerous(&[1])), s.insert_entry(t2, INDEX, &key(2), E));
}

/// Fekete's read-only anomaly: `t2` reads X and Y and writes X; `t1` writes Y and commits; `t3`
/// reads X and Y after `t1` committed and before `t2` does.
fn fekete(s: &mut Serializer, access: Access) -> (SerialId, SerialId, SerialId) {
    let t2 = rw(s, 2);
    read_key(s, t2, b'x', &[E]);
    read_key(s, t2, b'y', &[E]);
    let t1 = rw(s, 1);
    read_key(s, t1, b'y', &[E]);
    s.insert_entry(t1, INDEX, &key(b'y'), E).unwrap();
    commit(s, t1).unwrap();
    s.insert_entry(t2, INDEX, &key(b'x'), E).unwrap();
    let t3 = s.begin(access);
    (t1, t2, t3)
}

#[test]
fn feketes_read_only_anomaly_aborts_the_writer() {
    let mut s = Serializer::default();
    let (_, t2, t3) = fekete(&mut s, Access::ReadOnly);
    read_key(&mut s, t3, b'x', &[E]);
    s.read_from(t3, Xid(2)).unwrap();
    read_key(&mut s, t3, b'y', &[E]);
    commit(&mut s, t3).unwrap();
    assert_eq!(Err(dangerous(&[1])), commit(&mut s, t2));
}

#[test]
fn a_reader_that_began_before_the_first_commit_completes_no_anomaly() {
    let mut s = Serializer::default();
    let t2 = rw(&mut s, 2);
    read_key(&mut s, t2, b'y', &[E]);
    let t3 = s.begin(Access::ReadOnly);
    let t1 = rw(&mut s, 1);
    s.insert_entry(t1, INDEX, &key(b'y'), E).unwrap();
    commit(&mut s, t1).unwrap();
    s.insert_entry(t2, INDEX, &key(b'x'), E).unwrap();
    s.read_from(t3, Xid(2)).unwrap();
    s.read_from(t3, Xid(1)).unwrap();
    commit(&mut s, t3).unwrap();
    commit(&mut s, t2).unwrap();
}

#[test]
fn a_deferrable_transaction_waits_out_writers_and_takes_no_locks() {
    let mut s = Serializer::default();
    let t1 = rw(&mut s, 1);
    let d = s.begin(Access::Deferrable);
    assert_eq!(Safety::Pending(vec![Xid(1)]), s.safety(d));
    read_key(&mut s, t1, 1, &[E]);
    assert_eq!(vec![d], s.committed(t1));
    assert_eq!(Safety::Safe, s.safety(d));
    read_key(&mut s, d, 1, &[E]);
    assert_eq!(0, s.locks(d));
}

#[test]
fn a_writer_that_aborts_leaves_the_snapshot_safe() {
    let mut s = Serializer::default();
    let t1 = rw(&mut s, 1);
    let d = s.begin(Access::Deferrable);
    assert_eq!(vec![d], s.aborted(t1));
    assert_eq!(Safety::Safe, s.safety(d));
}

#[test]
fn feketes_writer_makes_a_deferrable_snapshot_unsafe() {
    let mut s = Serializer::default();
    let (_, t2, d) = fekete(&mut s, Access::Deferrable);
    assert_eq!(Safety::Pending(vec![Xid(2)]), s.safety(d));
    assert_eq!(vec![d], s.committed(t2));
    assert_eq!(Safety::Unsafe, s.safety(d));
    s.aborted(d);
    let d = s.begin(Access::Deferrable);
    assert_eq!(Safety::Safe, s.safety(d));
}

#[test]
fn a_transaction_with_no_concurrent_writer_starts_safe() {
    let mut s = Serializer::default();
    let r = s.begin(Access::ReadOnly);
    assert_eq!(Safety::Safe, s.safety(r));
    read_key(&mut s, r, 1, &[E]);
    s.read_tuple(r, HEAP, tid(5, 1)).unwrap();
    assert_eq!(0, s.locks(r));
}

#[test]
fn a_range_lock_ignores_inserts_of_other_constructors() {
    let mut s = Serializer::default();
    let (r, w) = (rw(&mut s, 1), rw(&mut s, 2));
    read_key(&mut s, r, 7, &[G, A]);
    s.insert_entry(w, INDEX, &key(7), E).unwrap();
    s.insert_entry(w, INDEX, &key(8), G).unwrap();
    assert!(s.writers_after(r).is_empty());
    s.insert_entry(w, INDEX, &key(7), G).unwrap();
    assert_eq!(BTreeSet::from([w]), s.writers_after(r));
}

#[test]
fn a_range_covers_the_keys_between_its_bounds() {
    let mut s = Serializer::default();
    let (r, w) = (rw(&mut s, 1), rw(&mut s, 2));
    s.read_range(
        r,
        INDEX,
        Bound::Excluded(key(3)),
        Bound::Unbounded,
        BTreeSet::from([E]),
    )
    .unwrap();
    s.insert_entry(w, INDEX, &key(3), E).unwrap();
    assert!(s.writers_after(r).is_empty());
    s.insert_entry(w, INDEX, &[3, 0], E).unwrap();
    assert_eq!(BTreeSet::from([w]), s.writers_after(r));
}

#[test]
fn a_tuple_lock_conflicts_with_a_write_of_that_version_only() {
    let mut s = Serializer::default();
    let (r, w) = (rw(&mut s, 1), rw(&mut s, 2));
    s.read_tuple(r, HEAP, tid(5, 1)).unwrap();
    s.write_tuple(w, HEAP, tid(5, 2)).unwrap();
    s.insert_heap(w, HEAP).unwrap();
    assert!(s.writers_after(r).is_empty());
    s.write_tuple(w, HEAP, tid(5, 1)).unwrap();
    assert_eq!(BTreeSet::from([w]), s.writers_after(r));
}

#[test]
fn a_heap_lock_conflicts_with_any_insert_into_the_heap() {
    let mut s = Serializer::default();
    let (r, w) = (rw(&mut s, 1), rw(&mut s, 2));
    s.read_heap(r, HEAP).unwrap();
    s.insert_heap(w, HEAP + 1).unwrap();
    assert!(s.writers_after(r).is_empty());
    s.insert_heap(w, HEAP).unwrap();
    assert_eq!(BTreeSet::from([w]), s.writers_after(r));
}

#[test]
fn tuple_locks_promote_to_a_page_lock_then_a_heap_lock() {
    let limits = Limits {
        tuples_per_page: 2,
        pages_per_heap: 2,
        ranges_per_index: 2,
        ..Limits::default()
    };
    let mut s = Serializer::new(limits);
    let (r, w) = (rw(&mut s, 1), rw(&mut s, 2));
    for slot in 0..3 {
        s.read_tuple(r, HEAP, tid(5, slot)).unwrap();
    }
    assert_eq!(1, s.locks(r));
    s.write_tuple(w, HEAP, tid(5, 9)).unwrap();
    assert_eq!(BTreeSet::from([w]), s.writers_after(r));

    let w2 = rw(&mut s, 3);
    s.insert_heap(w2, HEAP).unwrap();
    assert!(s.readers_before(w2).is_empty());
    for page in 6..8 {
        for slot in 0..3 {
            s.read_tuple(r, HEAP, tid(page, slot)).unwrap();
        }
    }
    assert_eq!(1, s.locks(r));
    s.insert_heap(w2, HEAP).unwrap();
    assert_eq!(BTreeSet::from([r]), s.readers_before(w2));
}

#[test]
fn range_locks_promote_to_an_index_lock_keeping_their_constructors() {
    let limits = Limits {
        tuples_per_page: 2,
        pages_per_heap: 2,
        ranges_per_index: 2,
        ..Limits::default()
    };
    let mut s = Serializer::new(limits);
    let (r, w) = (rw(&mut s, 1), rw(&mut s, 2));
    read_key(&mut s, r, 1, &[G]);
    read_key(&mut s, r, 2, &[A]);
    read_key(&mut s, r, 3, &[G]);
    assert_eq!(1, s.locks(r));
    s.insert_entry(w, INDEX, &key(200), E).unwrap();
    assert!(s.writers_after(r).is_empty());
    s.insert_entry(w, INDEX, &key(200), A).unwrap();
    assert_eq!(BTreeSet::from([w]), s.writers_after(r));
}

#[test]
fn a_migration_conflicts_with_statements_at_older_versions() {
    let mut s = Serializer::default();
    let (old, new, m) = (rw(&mut s, 1), rw(&mut s, 2), rw(&mut s, 3));
    s.read_schema(old, 4).unwrap();
    s.read_schema(new, 5).unwrap();
    s.migrate(m, 5).unwrap();
    assert_eq!(BTreeSet::from([old]), s.readers_before(m));
}

#[test]
fn a_bulk_load_conflicts_with_heap_locks_and_ranges_over_its_constructors() {
    let mut s = Serializer::default();
    let (scan, other, range, load) = (rw(&mut s, 1), rw(&mut s, 2), rw(&mut s, 3), rw(&mut s, 4));
    s.read_heap(scan, HEAP).unwrap();
    read_key(&mut s, other, 1, &[A]);
    read_key(&mut s, range, 1, &[G, A]);
    s.bulk_insert(load, &[HEAP], &[(INDEX, BTreeSet::from([G]))])
        .unwrap();
    assert_eq!(BTreeSet::from([scan, range]), s.readers_before(load));
}

#[test]
fn a_split_turns_locks_on_the_rewritten_heaps_into_heap_locks_on_the_new_ones() {
    let mut s = Serializer::default();
    let (r, w) = (rw(&mut s, 1), rw(&mut s, 2));
    s.read_tuple(r, HEAP, tid(5, 1)).unwrap();
    read_key(&mut s, r, 1, &[G]);
    s.rewrite(&[HEAP], &[INDEX], &[20, 21]);
    assert_eq!(2, s.locks(r));
    s.insert_entry(w, INDEX, &key(1), G).unwrap();
    assert!(s.writers_after(r).is_empty());
    s.insert_heap(w, 21).unwrap();
    assert_eq!(BTreeSet::from([w]), s.writers_after(r));
}

#[test]
fn a_writer_that_began_after_the_reader_committed_does_not_conflict() {
    let mut s = Serializer::default();
    let (r, keep) = (rw(&mut s, 1), rw(&mut s, 2));
    read_key(&mut s, r, 1, &[E]);
    commit(&mut s, r).unwrap();
    let w = rw(&mut s, 3);
    s.insert_entry(w, INDEX, &key(1), E).unwrap();
    assert!(s.readers_before(w).is_empty());
    s.insert_entry(keep, INDEX, &key(1), E).unwrap();
    assert_eq!(BTreeSet::from([r]), s.readers_before(keep));
}

#[test]
fn committed_transactions_are_forgotten_once_no_concurrent_one_runs() {
    let mut s = Serializer::default();
    let (t1, t2) = (rw(&mut s, 1), rw(&mut s, 2));
    read_key(&mut s, t1, 1, &[E]);
    commit(&mut s, t1).unwrap();
    assert_eq!(2, s.tracked());
    commit(&mut s, t2).unwrap();
    assert_eq!(0, s.tracked());
}

#[test]
fn a_doomed_transaction_fails_its_next_operation() {
    let mut s = Serializer::default();
    let (_, t2, t3) = fekete(&mut s, Access::ReadOnly);
    s.read_from(t3, Xid(2)).unwrap();
    assert_eq!(Err(dangerous(&[1])), s.check(t2));
    assert_eq!(Err(dangerous(&[1])), s.insert_entry(t2, INDEX, &key(9), E));
}

#[test]
fn the_total_budget_promotes_the_largest_holders_locks() {
    let limits = Limits {
        total: 4,
        ..Limits::default()
    };
    let mut s = Serializer::new(limits);
    let (big, small, w) = (rw(&mut s, 1), rw(&mut s, 2), rw(&mut s, 3));
    s.read_tuple(small, HEAP, tid(9, 1)).unwrap();
    for slot in 0..3 {
        s.read_tuple(big, HEAP, tid(5, slot)).unwrap();
    }
    assert_eq!(4, s.held());
    s.read_tuple(big, HEAP, tid(6, 0)).unwrap();
    assert_eq!((2, 1), (s.locks(big), s.locks(small)));
    s.write_tuple(w, HEAP, tid(5, 7)).unwrap();
    assert_eq!(BTreeSet::from([big]), s.readers_before(w));
    for k in 0..4 {
        read_key(&mut s, big, k, &[G]);
    }
    assert!(s.held() <= 4, "{} locks held", s.held());
    s.insert_entry(w, INDEX, &key(200), G).unwrap();
    s.write_tuple(w, HEAP, tid(6, 9)).unwrap();
    assert_eq!(BTreeSet::from([big]), s.readers_before(w));
}

#[test]
fn a_failure_names_the_other_transactions_of_its_conflict() {
    let mut s = Serializer::default();
    let (t1, t2, t3) = (rw(&mut s, 1), rw(&mut s, 2), rw(&mut s, 3));
    read_key(&mut s, t1, 1, &[E]);
    s.insert_entry(t2, INDEX, &key(1), E).unwrap();
    read_key(&mut s, t2, 2, &[E]);
    s.insert_entry(t3, INDEX, &key(2), E).unwrap();
    commit(&mut s, t3).unwrap();
    assert_eq!(Err(dangerous(&[1, 3])), commit(&mut s, t2));
}

#[test]
fn begin_reserves_at_the_end_of_a_reservation_and_refuses_once_ids_are_spent() {
    assert_eq!(Assign::Next, assign(Xid(5), Xid(1025)));
    assert_eq!(
        Assign::Reserve(Xid(1025 + RESERVATION)),
        assign(Xid(1025), Xid(1025))
    );
    assert_eq!(
        Assign::Reserve(Xid(u64::MAX)),
        assign(Xid(u64::MAX - 3), Xid(u64::MAX - 3))
    );
    assert_eq!(Assign::Next, assign(Xid(u64::MAX - 1), Xid(u64::MAX)));
    assert_eq!(Assign::Exhausted, assign(Xid(u64::MAX), Xid(u64::MAX)));
}

#[test]
fn a_snapshot_round_trips_through_its_marker_encoding() {
    let s = Snapshot::new(Xid(5), Xid(9), vec![Xid(5), Xid(7)]);
    assert_eq!(32, s.encode().len());
    assert_eq!(Some(s.clone()), Snapshot::decode(&s.encode()));
    assert_eq!(None, Snapshot::decode(&s.encode()[..20]));
    let unsorted = Snapshot::new(Xid(5), Xid(9), vec![Xid(5), Xid(7)]).encode();
    let mut swapped = unsorted.clone();
    swapped[16..24].copy_from_slice(&unsorted[24..32]);
    swapped[24..32].copy_from_slice(&unsorted[16..24]);
    assert_eq!(None, Snapshot::decode(&swapped));
}

#[test]
fn the_serializer_reports_each_snapshot_it_finds_safe_once() {
    let mut s = Serializer::default();
    let at_once = s.begin(Access::ReadOnly);
    assert_eq!(vec![at_once], s.take_safe());
    let (t1, t2) = (rw(&mut s, 1), rw(&mut s, 2));
    let pending = s.begin(Access::Deferrable);
    assert!(s.take_safe().is_empty());
    commit(&mut s, t1).unwrap();
    assert!(s.take_safe().is_empty());
    commit(&mut s, t2).unwrap();
    assert_eq!(vec![pending], s.take_safe());
    assert!(s.take_safe().is_empty());
}
