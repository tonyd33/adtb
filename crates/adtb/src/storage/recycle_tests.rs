//! Recycling the log: what it removes, what it keeps, and the page LSN map it folds into first.

use crate::testing::Probe;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::runtime::{IoError, OpKind};
use crate::storage::order::ordered_for_test;
use crate::storage::policy::{Holder, Phase};
use crate::storage::{
    self, BackupRun, Damage, Envelope, Finding, HEADER_LEN, Lsn, MarkFailure, Options, PageError,
    PageKind, Pager, Pid, Quarantine,
};
use crate::test_world as harness;
use crate::verify::{self, Inventory, Problem};
use adtb_harness::damage::{DataFile, Files};
use adtb_sim::{Action, Config, Match, Plan, Sim, SimRuntime};
use adtb_testkit::profile::every_case;

const PAGER: DataFile = DataFile::new(harness::NAME, harness::PAGE);

const SEGMENT: u64 = 64 * 1024;

fn options() -> Options {
    Options {
        segment_size: SEGMENT,
        ..harness::options(16)
    }
}

/// Returns the first LSN of every log segment of `files`.
fn segments(files: &Files) -> Vec<u64> {
    let dir = format!("{}/", options().log_dir());
    let names = files.keys().filter_map(|k| k.strip_prefix(&dir));
    names.map(|n| u64::from_str_radix(n, 16).unwrap()).collect()
}

/// Returns a closed file of `n` data pages, and them.
fn created(n: usize) -> (Sim, Vec<Pid>) {
    let mut sim = Sim::new(3, Config::default());
    let mut pages = Vec::new();
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options()).unwrap();
        pages = (0..n)
            .map(|_| pager.allocate(1, PageKind::HeapSlotted, 7).unwrap())
            .collect();
        pager.close();
    });
    (sim, pages)
}

/// Open the file of `sim`, run `body` on it and close it. Returns what `body` returns.
fn session<T>(sim: &mut Sim, body: impl FnOnce(&Pager<SimRuntime>, &SimRuntime) -> T) -> T {
    let mut out = None;
    harness::finished(sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options()).unwrap();
        out = Some(body(&pager, &rt));
        pager.close();
    });
    out.unwrap()
}

/// Write `value` to `pid`, forced.
fn put(pager: &Pager<SimRuntime>, pid: Pid, value: u64) {
    pager
        .update(&[pid], |p| {
            p[0][HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&value.to_le_bytes());
        })
        .unwrap();
    pager.flush_log();
}

/// Write the pages of `pages` in turn until the log has grown by `bytes`. Returns the last value
/// written to each.
fn fill(pager: &Pager<SimRuntime>, pages: &[Pid], bytes: u64) -> BTreeMap<Pid, u64> {
    let start = Probe::of(pager).appended().0;
    let mut written = BTreeMap::new();
    for n in 0.. {
        if Probe::of(pager).appended().0 >= start + bytes {
            break;
        }
        let pid = pages[n % pages.len()];
        put(pager, pid, n as u64 + 1);
        written.insert(pid, n as u64 + 1);
    }
    written
}

fn value_of(pager: &Pager<SimRuntime>, pid: Pid) -> u64 {
    let page = pager.read(pid).unwrap();
    u64::from_le_bytes(page[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap())
}

/// Returns the value of each of `pages` in the file restored from the backup `sim` holds.
fn restored_values(sim: &Sim, pages: &[Pid]) -> BTreeMap<Pid, u64> {
    let mut copy = Sim::with_files(4, Config::default(), sim.files());
    let mut out = BTreeMap::new();
    harness::finished(&mut copy, |rt| {
        storage::restore_file(&rt, "backup", "copy.adt", "copy.adt-log").unwrap();
        let options = Options {
            name: "copy.adt".into(),
            ..options()
        };
        let (pager, _) = Pager::open(rt, &options).unwrap();
        out = pages.iter().map(|&p| (p, value_of(&pager, p))).collect();
        pager.close();
    });
    out
}

fn reverted(mut files: Files, from: &Files, pid: Pid) -> Files {
    PAGER.replace(&mut files, pid, PAGER.page(from, pid));
    files
}

fn lsn_of(files: &Files, pid: Pid) -> Lsn {
    storage::lsn(PAGER.page(files, pid))
}

#[test]
fn the_log_holds_only_the_last_session_and_the_current_one() {
    let (mut sim, pages) = created(8);
    let mut held = Vec::new();
    for _ in 0..8 {
        let removed = session(&mut sim, |pager, _| {
            let removed = pager.recycle_log();
            fill(pager, &pages, 3 * SEGMENT);
            removed
        });
        held.push((removed, segments(&sim.files()).len()));
    }
    let written = session(&mut sim, |pager, _| Probe::of(pager).appended().0);
    assert!(written > 24 * SEGMENT, "{written}");
    assert!(held[1..].iter().all(|&(r, h)| r >= 2 && h <= 5), "{held:?}");
}

#[test]
fn an_archive_holding_nothing_keeps_every_segment_and_one_bounds_the_recycle() {
    let (mut sim, pages) = created(8);
    session(&mut sim, |pager, _| fill(pager, &pages, 3 * SEGMENT));
    let before = segments(&sim.files());
    let kept = session(&mut sim, |pager, _| {
        pager.set_archive(Arc::new(|| None));
        pager.recycle_log()
    });
    assert_eq!((0, before.clone()), (kept, segments(&sim.files())));
    let removed = session(&mut sim, |pager, _| {
        pager.set_archive(Arc::new(|| Some(Lsn(SEGMENT))));
        pager.recycle_log()
    });
    assert_eq!((1, before[1..].to_vec()), (removed, segments(&sim.files())));
}

fn envelope() -> Envelope {
    Envelope {
        release: "test".into(),
        edition: "2026-10".into(),
        schema_version: 1,
        schema_hash: "00".into(),
        catalog_version: 1,
    }
}

#[test]
fn a_recycle_while_a_backup_copies_the_log_removes_nothing() {
    let (mut sim, pages) = created(8);
    session(&mut sim, |pager, _| fill(pager, &pages, 3 * SEGMENT));
    let (during, after) = session(&mut sim, |pager, _| {
        let mut during = Vec::new();
        let mut visit = |pid: Pid, _: &mut [u8]| {
            if pid == Pid(0) {
                during.push(pager.recycle_log());
            }
            true
        };
        let run = &mut BackupRun {
            target: "backup",
            envelope: envelope(),
            visit: &mut visit,
            name: &|q| format!("{q:?}"),
        };
        pager.backup(run).unwrap();
        (during, pager.recycle_log())
    });
    assert!(
        !during.is_empty() && during.iter().all(|&n| n == 0),
        "{during:?}"
    );
    assert!(after >= 2, "{after}");
}

#[test]
fn recycles_while_a_backup_copies_its_pages_keep_the_log_it_copies() {
    let (mut sim, pages) = created(8);
    session(&mut sim, |pager, _| fill(pager, &pages, 3 * SEGMENT));
    let (during, backup, after) = session(&mut sim, |pager, _| {
        storage::recycle_at_open(pager, None);
        let mut during = Vec::new();
        let mut visit = |pid: Pid, _: &mut [u8]| {
            if pid == pages[0] && during.is_empty() {
                for _ in 0..2 {
                    fill(pager, &pages[1..], 2 * SEGMENT);
                    pager.flush_pages(usize::MAX);
                    pager.checkpoint();
                    during.push(pager.recycle_log());
                }
            }
            true
        };
        let run = &mut BackupRun {
            target: "backup",
            envelope: envelope(),
            visit: &mut visit,
            name: &|q| format!("{q:?}"),
        };
        let backup = pager.backup(run).map(|m| (m.start, m.end));
        (during, backup, pager.recycle_log())
    });
    assert_eq!(vec![0, 0], during);
    let (start, end) = backup.unwrap();
    assert!(end.lsn.0 - start.lsn.0 > 4 * SEGMENT, "{start:?} {end:?}");
    assert!(
        after >= 2,
        "the backup's pin was not what held the log: {after}"
    );
    let restored = restored_values(&sim, &pages);
    let values = session(&mut sim, |pager, _| {
        pages
            .iter()
            .map(|&p| (p, value_of(pager, p)))
            .collect::<BTreeMap<_, _>>()
    });
    assert_eq!(values, restored);
}

/// Returns the files of a closed session whose page `pages[0]` was last written in a batch
/// spanning a segment boundary, before a second session that recycled it, and those files.
fn recycled_across_a_boundary(pages: &[Pid], sim: &mut Sim) -> (Files, Files) {
    session(sim, |pager, _| {
        for n in 0.. {
            if SEGMENT - Probe::of(pager).appended().0 % SEGMENT <= 1024 {
                break;
            }
            put(pager, pages[1 + n % 8], n as u64 + 1);
        }
        let at = Probe::of(pager).appended().0;
        pager
            .update(&[pages[1]], |p| p[0][HEADER_LEN..].fill(9))
            .unwrap();
        pager.update(&[pages[0]], |p| p[0][HEADER_LEN] = 8).unwrap();
        pager.flush_log();
        let end = Probe::of(pager).appended().0 - 1;
        assert_ne!(at / SEGMENT, end / SEGMENT, "the batch spans a boundary");
    });
    let written = sim.files();
    let removed = session(sim, |pager, _| pager.recycle_log());
    assert!(removed >= 1, "{removed}");
    (written, sim.files())
}

#[test]
fn a_lost_write_whose_record_was_recycled_is_found_by_verify_and_backup() {
    let (mut sim, pages) = created(600);
    let before = sim.files();
    let (written, recycled) = recycled_across_a_boundary(&pages, &mut sim);
    let page = pages[0];
    let lost = reverted(recycled, &before, page);
    let stale = Damage::Stale {
        expected: lsn_of(&written, page),
        found: lsn_of(&before, page),
    };
    let mut sim = Sim::with_files(3, Config::default(), lost);
    let (findings, damaged) = session(&mut sim, |pager, _| {
        let run = &mut BackupRun {
            target: "backup",
            envelope: envelope(),
            visit: &mut |_, _| true,
            name: &|q| format!("{q:?}"),
        };
        let damaged = pager.backup(run).unwrap().damaged;
        (pager.verify().findings, damaged)
    });
    assert_eq!(vec![page], damaged);
    assert!(
        findings.contains(&Finding::Page {
            pid: page,
            damage: stale
        }),
        "{findings:?}"
    );
}

#[test]
fn a_lost_write_of_a_map_page_quarantines_the_database() {
    let (mut sim, pages) = created(600);
    let (_, first) = recycled_across_a_boundary(&pages, &mut sim);
    session(&mut sim, |pager, _| {
        fill(pager, &pages[550..], 2 * SEGMENT);
    });
    session(&mut sim, |pager, _| pager.recycle_log());
    let last = sim.files();
    let map = session(&mut sim, |pager, _| pager.map_pages().unwrap());
    assert_eq!(3, map.len(), "a root of height 2 and two leaves: {map:?}");
    let mut reached = 0;
    for &page in &map {
        if lsn_of(&first, page) == lsn_of(&last, page) {
            continue;
        }
        let lost = reverted(last.clone(), &first, page);
        let mut sim = Sim::with_files(3, Config::default(), lost);
        let (marked, expected) = session(&mut sim, |pager, _| {
            pager.verify();
            (mapped_once(pager), held_unverified(pager))
        });
        assert_eq!(expected, marked, "{page:?}");
        reached += 1;
    }
    assert!(reached >= 2, "{reached} of {map:?}");
}

/// Returns a closed file of 600 data pages whose map has a root and two leaves, its pages, and
/// the root and a leaf.
fn mapped() -> (Sim, Vec<Pid>, Pid, Pid) {
    let (mut sim, pages) = created(600);
    recycled_across_a_boundary(&pages, &mut sim);
    session(&mut sim, |pager, _| {
        fill(pager, &pages[550..], 2 * SEGMENT);
    });
    session(&mut sim, |pager, _| pager.recycle_log());
    let map = session(&mut sim, |pager, _| pager.map_pages().unwrap());
    let [root, leaf, _] = map[..] else {
        panic!("a root of height 2 and two leaves: {map:?}");
    };
    (sim, pages, root, leaf)
}

#[test]
fn an_unreadable_map_page_is_refused_unmarked_and_read_by_the_next_open() {
    let (mut sim, _, _, leaf) = mapped();
    let mut out = None;
    harness::finished(&mut sim, |rt| {
        let reads = Match::kind(OpKind::Read).offset(PAGER.range(leaf).start as u64);
        rt.plan(Plan::new().rule(reads, Action::Fail(IoError::Failed)));
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        out = Some((pager.map_pages(), Probe::of(&pager).marks()));
        pager.close();
    });
    let (refused, marks) = out.unwrap();
    let damage = Damage::Unreadable;
    assert_eq!(
        (
            Err(PageError { pid: leaf, damage }),
            BTreeSet::from([Quarantine::Database])
        ),
        (refused, marks)
    );
    let after = session(&mut sim, |pager, _| {
        (pager.map_pages().map(|m| m.len()), Probe::of(pager).marks())
    });
    assert_eq!((Ok(3), BTreeSet::new()), after);
}

/// Map every record of `pager` as running, by its storage's own holders and no walk. Returns the
/// marks that failed, and the marks then.
fn mapped_once(
    pager: &Pager<SimRuntime>,
) -> (BTreeMap<Quarantine, MarkFailure>, BTreeSet<Quarantine>) {
    let holders = |pid| pager.holders_of(pid, Phase::Running, |_| {});
    let failed = pager.map_damage(holders, &ordered_for_test());
    (failed, Probe::of(pager).marks())
}

/// Returns the failure of a database mark set in an open that read the marks page of `pager`
/// unchecked, and the marks it leaves.
fn held_unverified(
    pager: &Pager<SimRuntime>,
) -> (BTreeMap<Quarantine, MarkFailure>, BTreeSet<Quarantine>) {
    let page = pager.quarantine_page();
    (
        BTreeMap::from([(Quarantine::Database, MarkFailure::Unverified { page })]),
        BTreeSet::from([Quarantine::Database]),
    )
}

#[test]
fn a_damaged_map_page_met_in_session_is_recorded_unmarked_and_maps_to_the_database() {
    let (mut sim, _, _, leaf) = mapped();
    session(&mut sim, |pager, _| {
        let damage = Damage::Checksum;
        pager.damage(leaf, damage);
        assert_eq!(Err(PageError { pid: leaf, damage }), pager.map_pages());
        assert_eq!(BTreeSet::new(), Probe::of(pager).marks(), "the walk");
        assert_eq!(Some(&damage), Probe::of(pager).records().pages().get(&leaf));
        for phase in [Phase::Opening, Phase::Running] {
            let held = pager.holders_of(leaf, phase, |_| {});
            assert_eq!(Some(BTreeSet::from([Holder::LsnMap])), held, "{phase:?}");
        }
        let database = BTreeSet::from([Quarantine::Database]);
        assert_eq!((BTreeMap::new(), database.clone()), mapped_once(pager));
        assert_eq!(Some(database), pager.mapped_to(leaf));
    });
}

#[test]
fn a_checksum_bad_map_page_quarantines_the_database_and_verify_reports_the_rest() {
    let (sim, pages, root, leaf) = mapped();
    let data = pages[3];
    for bad in [root, leaf] {
        let mut files = sim.files();
        for pid in [bad, data] {
            PAGER.flip(&mut files, pid, 100 - HEADER_LEN);
        }
        let mut sim = Sim::with_files(3, Config::default(), files);
        let (findings, (marked, expected), allocated, leaked) = session(&mut sim, |pager, _| {
            let findings = pager.verify().findings;
            let inventory = Inventory {
                commit_log: None,
                sums: Vec::new(),
                trees: Vec::new(),
                pages: pages.clone(),
            };
            let report = verify::verify(pager, &inventory);
            let leaked = report
                .findings
                .iter()
                .filter(|f| f.problem == Problem::Leaked);
            let leaked: Vec<Pid> = leaked.map(|f| f.pid).collect();
            (
                findings,
                (mapped_once(pager), held_unverified(pager)),
                pager.allocated(),
                leaked,
            )
        });
        assert_eq!(expected, marked, "{bad:?}");
        let refused = PageError {
            pid: bad,
            damage: Damage::Checksum,
        };
        assert_eq!((Err(refused), Vec::new()), (allocated, leaked), "{bad:?}");
        for pid in [bad, data] {
            let finding = Finding::Page {
                pid,
                damage: Damage::Checksum,
            };
            assert!(findings.contains(&finding), "{pid:?}: {findings:?}");
        }
    }
}

/// Crash a reopen of `files` at evenly spaced points of its recycle. Returns how many crashes
/// fell in the fold, the checkpoint and the removal, told apart by what the crash left on disk:
/// segments removed, a header written, or only log written.
fn crash_through_a_recycle(files: &Files, pages: &[Pid], written: &BTreeMap<Pid, u64>) -> [u32; 3] {
    let mut opened = Sim::with_files(4, Config::default(), files.clone());
    opened.run(|rt| {
        Pager::open(rt, &options()).unwrap();
    });
    let opened = opened.files();
    let mut probe = Sim::with_files(4, Config::default(), files.clone());
    probe.record();
    session(&mut probe, |pager, _| assert!(pager.recycle_log() >= 2));
    let recording = probe.recording();
    let headers = 0..PAGER.range(Pid::FIRST_DATA).start;
    let log = |f: &Files| -> Vec<Vec<u8>> {
        let dir = format!("{}/", options().log_dir());
        let held = f.iter().filter(|(k, _)| k.starts_with(&dir));
        held.map(|(_, v)| v.clone()).collect()
    };
    let phase = |left: &Files| {
        if segments(left).len() < segments(files).len() {
            Some(2)
        } else if left[harness::NAME][headers.clone()] != opened[harness::NAME][headers.clone()] {
            Some(1)
        } else {
            (log(left) != log(&opened)).then_some(0)
        }
    };
    let mut phases: [Vec<usize>; 3] = Default::default();
    for at in 0..=recording.len() {
        if let Some(k) = phase(&recording.images(at, 1, 0)[0]) {
            phases[k].push(at);
        }
    }
    let per_phase = if every_case() { usize::MAX } else { 4 };
    let wanted: BTreeSet<Pid> = pages.iter().copied().collect();
    let mut kinds = [0; 3];
    for (k, points) in phases.iter().enumerate() {
        let step = points.len().div_ceil(per_phase).max(1);
        for &at in points.iter().step_by(step) {
            let mut sim =
                Sim::with_files(4, Config::default(), recording.images(at, 1, 0).remove(0));
            let (findings, allocated, values) = session(&mut sim, |pager, _| {
                pager.recycle_log();
                let values: BTreeMap<Pid, u64> = written
                    .keys()
                    .map(|&pid| {
                        let page = pager.read(pid).unwrap();
                        let value = u64::from_le_bytes(
                            page[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap(),
                        );
                        (pid, value)
                    })
                    .collect();
                (pager.verify().findings, pager.allocated().unwrap(), values)
            });
            let allocated: BTreeSet<Pid> = allocated.into_iter().collect();
            assert_eq!(
                (Vec::new(), &wanted, written),
                (findings, &allocated, &values),
                "phase {k}, crash after event {at}"
            );
            kinds[k] += 1;
        }
    }
    kinds
}

#[test]
fn a_crash_at_any_point_of_a_recycle_reopens_with_every_write_and_no_leaked_page() {
    let (mut sim, pages) = created(600);
    let written = session(&mut sim, |pager, _| fill(pager, &pages, 2 * SEGMENT));
    let kinds = crash_through_a_recycle(&sim.files(), &pages, &written);
    assert!(kinds.iter().all(|&n| n >= 1), "{kinds:?}");
}

#[test]
fn a_crash_at_any_point_of_a_recycle_growing_the_map_reopens_with_every_write() {
    let (mut sim, mut pages) = created(50);
    let mut written = session(&mut sim, |pager, _| fill(pager, &pages, 2 * SEGMENT));
    session(&mut sim, |pager, _| pager.recycle_log());
    assert_eq!(
        1,
        session(&mut sim, |pager, _| pager.map_pages().unwrap().len())
    );
    session(&mut sim, |pager, _| {
        pages.extend((0..550).map(|_| pager.allocate(1, PageKind::HeapSlotted, 7).unwrap()));
        written.extend(fill(pager, &pages, 2 * SEGMENT));
    });
    let files = sim.files();
    let kinds = crash_through_a_recycle(&files, &pages, &written);
    assert!(kinds.iter().all(|&n| n >= 1), "{kinds:?}");
    let mut grown = Sim::with_files(4, Config::default(), files);
    session(&mut grown, |pager, _| pager.recycle_log());
    assert_eq!(
        3,
        session(&mut grown, |pager, _| pager.map_pages().unwrap().len())
    );
}

#[test]
fn a_change_to_a_page_damaged_before_its_write_is_replayed_by_the_next_open() {
    let (mut sim, pages) = created(1);
    let pid = pages[0];
    session(&mut sim, |pager, _| {
        put(pager, pid, 1);
        pager.checkpoint();
        put(pager, pid, 2);
        pager.damage(pid, Damage::Checksum);
        pager.checkpoint();
    });
    let value = session(&mut sim, |pager, _| {
        let page = pager.read(pid).unwrap();
        u64::from_le_bytes(page[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap())
    });
    assert_eq!(2, value);
}

#[test]
fn a_gap_in_the_retained_log_quarantines_the_database_for_the_open_and_marks_nothing() {
    let (mut sim, pages) = created(8);
    session(&mut sim, |pager, _| fill(pager, &pages, 3 * SEGMENT));
    let files = sim.files();
    let held = segments(&files);
    assert!(held.len() >= 3, "{held:?}");
    let gap = format!("{}/{:016x}", options().log_dir(), held[1]);
    let mut gapped = files.clone();
    gapped.remove(&gap).unwrap();
    let mut sim = Sim::with_files(3, Config::default(), gapped);
    let marks = session(&mut sim, |pager, _| Probe::of(pager).marks());
    assert_eq!(BTreeSet::from([Quarantine::Database]), marks);
    let mut back = sim.files();
    back.insert(
        gap,
        files[&format!("{}/{:016x}", options().log_dir(), held[1])].clone(),
    );
    let marks = session(
        &mut Sim::with_files(3, Config::default(), back),
        |pager, _| Probe::of(pager).marks(),
    );
    assert_eq!(BTreeSet::new(), marks);
}

#[test]
fn a_marks_page_loaded_unchecked_is_never_written_and_the_next_checked_open_refuses_it() {
    let (mut sim, pages) = created(8);
    let before = sim.files();
    let marks = session(&mut sim, |pager, _| {
        pager
            .quarantine(Quarantine::Index(3), &ordered_for_test())
            .unwrap();
        fill(pager, &pages[1..], 3 * SEGMENT);
        pager.quarantine_page()
    });
    let files = sim.files();
    let held = segments(&files);
    assert!(held.len() >= 3, "{held:?}");
    let gap = format!("{}/{:016x}", options().log_dir(), held[1]);
    let mut gapped = reverted(files.clone(), &before, marks);
    gapped.remove(&gap).unwrap();
    let mut sim = Sim::with_files(3, Config::default(), gapped);
    let heap = Holder::Relation {
        object: Quarantine::Heap(7),
        engine_module: false,
    };
    let (loaded, failed, after) = session(&mut sim, |pager, _| {
        let loaded = Probe::of(pager).marks();
        pager.damage(pages[0], Damage::Checksum);
        let failed = pager.map_damage(|_| Some(BTreeSet::from([heap])), &ordered_for_test());
        assert_eq!(None, pager.mapped_to(pages[0]), "the record stays unmapped");
        let heap = Some(BTreeSet::from([Quarantine::Heap(7)]));
        assert_eq!(heap, pager.marked_to(pages[0]));
        (loaded, failed, Probe::of(pager).marks())
    });
    assert_eq!(BTreeSet::from([Quarantine::Database]), loaded);
    assert_eq!(
        BTreeMap::from([(Quarantine::Heap(7), MarkFailure::Unverified { page: marks })]),
        failed
    );
    assert_eq!(
        BTreeSet::from([Quarantine::Database, Quarantine::Heap(7)]),
        after
    );
    assert_eq!(lsn_of(&before, marks), lsn_of(&sim.files(), marks));
    let mut back = sim.files();
    back.insert(gap.clone(), files[&gap].clone());
    let marks = session(
        &mut Sim::with_files(3, Config::default(), back),
        |pager, _| Probe::of(pager).marks(),
    );
    assert_eq!(BTreeSet::from([Quarantine::Database]), marks);
}

#[test]
fn a_damage_list_loaded_unchecked_is_never_folded_and_the_next_checked_open_refuses_it() {
    let (mut sim, pages) = created(8);
    let before = sim.files();
    let mut list = Pid(0);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        let pager = &pager;
        pager.damage(pages[0], Damage::Checksum);
        pager.checkpoint();
        pager.flush_pages(usize::MAX);
        fill(pager, &pages[2..], 3 * SEGMENT);
        pager.flush_pages(usize::MAX);
        pager.checkpoint();
        pager.damage(pages[1], Damage::Checksum);
        pager.flush_log();
        list = pager.damage_list_page();
    });
    let files = sim.files();
    assert_ne!(
        lsn_of(&before, list),
        lsn_of(&files, list),
        "the fold wrote the list"
    );
    let held = segments(&files);
    assert!(held.len() >= 3, "{held:?}");
    let gap = format!("{}/{:016x}", options().log_dir(), held[1]);
    let mut gapped = reverted(files.clone(), &before, list);
    gapped.remove(&gap).unwrap();
    let mut sim = Sim::with_files(3, Config::default(), gapped);
    let marks = session(&mut sim, |pager, _| {
        pager.checkpoint();
        Probe::of(pager).marks()
    });
    assert_eq!(BTreeSet::from([Quarantine::Database]), marks);
    assert_eq!(lsn_of(&before, list), lsn_of(&sim.files(), list));
    let mut back = sim.files();
    back.insert(gap.clone(), files[&gap].clone());
    let (marks, recorded) = session(
        &mut Sim::with_files(3, Config::default(), back),
        |pager, _| {
            let recorded = Probe::of(pager).records().pages().get(&pages[0]).copied();
            (Probe::of(pager).marks(), recorded)
        },
    );
    assert_eq!(
        (BTreeSet::from([Quarantine::Database]), None),
        (marks, recorded)
    );
}

#[test]
fn a_swap_releasing_a_mark_on_an_unchecked_marks_page_keeps_the_mark() {
    let (mut sim, pages) = created(8);
    let [to, from] = [pages[0], pages[1]];
    session(&mut sim, |pager, _| {
        pager
            .quarantine(Quarantine::Index(3), &ordered_for_test())
            .unwrap();
        put(pager, from, 9);
        fill(pager, &pages[2..], 3 * SEGMENT);
    });
    let files = sim.files();
    let held = segments(&files);
    assert!(held.len() >= 3, "{held:?}");
    let gap = format!("{}/{:016x}", options().log_dir(), held[1]);
    let mut gapped = files.clone();
    gapped.remove(&gap).unwrap();
    let mut sim = Sim::with_files(3, Config::default(), gapped);
    let (value, marks) = session(&mut sim, |pager, _| {
        pager
            .replace_contents(to, from, Some(Quarantine::Index(3)))
            .unwrap();
        (value_of(pager, to), Probe::of(pager).marks())
    });
    let all = BTreeSet::from([Quarantine::Database, Quarantine::Index(3)]);
    assert_eq!((9, all), (value, marks));
    let mut back = sim.files();
    back.insert(gap.clone(), files[&gap].clone());
    let (value, marks) = session(
        &mut Sim::with_files(3, Config::default(), back),
        |pager, _| (value_of(pager, to), Probe::of(pager).marks()),
    );
    assert_eq!((9, BTreeSet::from([Quarantine::Index(3)])), (value, marks));
}

#[test]
fn a_lost_write_whose_record_is_recycled_is_refused_on_its_first_read() {
    let (mut sim, pages) = created(8);
    let before = sim.files();
    let pid = pages[0];
    session(&mut sim, |pager, _| {
        put(pager, pid, 1);
        fill(pager, &pages[1..], 3 * SEGMENT);
    });
    assert!(session(&mut sim, |pager, _| pager.recycle_log()) >= 1);
    let after = sim.files();
    let first = segments(&after)[0];
    let lost = reverted(after.clone(), &before, pid);
    let read = session(
        &mut Sim::with_files(3, Config::default(), lost),
        |pager, _| pager.read(pid).map(drop),
    );
    let damage = Damage::Stale {
        expected: lsn_of(&after, pid),
        found: lsn_of(&before, pid),
    };
    assert!(lsn_of(&after, pid).0 < first, "{first}");
    assert_eq!(Err(PageError { pid, damage }), read);
}

#[test]
fn a_lost_write_past_the_maps_entry_of_its_page_is_refused_on_its_first_read() {
    let (mut sim, pages) = created(8);
    let pid = pages[0];
    session(&mut sim, |pager, _| {
        put(pager, pid, 1);
        fill(pager, &pages[1..], 3 * SEGMENT);
    });
    assert!(session(&mut sim, |pager, _| pager.recycle_log()) >= 1);
    let before = sim.files();
    let first = segments(&before)[0];
    assert!(lsn_of(&before, pid).0 < first, "{first}");
    session(&mut sim, |pager, _| put(pager, pid, 2));
    let after = sim.files();
    let lost = reverted(after.clone(), &before, pid);
    let read = session(
        &mut Sim::with_files(3, Config::default(), lost),
        |pager, _| pager.read(pid).map(drop),
    );
    let damage = Damage::Stale {
        expected: lsn_of(&after, pid),
        found: lsn_of(&before, pid),
    };
    assert_eq!(Err(PageError { pid, damage }), read);
}

/// Set `pid` to 1, write it out by reading `pages` through the pool, then set it to 2, each change
/// forced. Returns the LSN of each change.
fn changed_after_its_write(pager: &Pager<SimRuntime>, pid: Pid, pages: &[Pid]) -> [Lsn; 2] {
    let set = |value: u64| {
        let lsn = pager
            .update(&[pid], |p| {
                p[0][HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&value.to_le_bytes());
            })
            .unwrap();
        pager.flush_log();
        lsn
    };
    let first = set(1);
    for &other in pages {
        drop(pager.read(other).unwrap());
    }
    let (file, _) = pager.read_disk(pid).unwrap();
    assert_eq!(1u64.to_le_bytes(), file[HEADER_LEN..HEADER_LEN + 8]);
    [first, set(2)]
}

/// In one session, change `pid` after its write and refuse it under a whole quarantine with that
/// change unwritten; force the data file and take a checkpoint; lift the quarantine, free `pid`
/// and recycle past the change. Assert the dirty entry of `pid` after the force and after the
/// free. Returns the LSN of each change, of the checkpoint and of the free, and the session's disk
/// events.
fn refused_then_freed(pid: Pid, pages: &[Pid], sim: &mut Sim) -> ([Lsn; 4], adtb_sim::Recording) {
    sim.record();
    let lsns = session(sim, |pager, _| {
        pager.release_session_log();
        let [first, lost] = changed_after_its_write(pager, pid, pages);
        pager
            .quarantine(Quarantine::Database, &ordered_for_test())
            .unwrap();
        pager.damage(pid, Damage::Checksum);
        pager.flush_pages(usize::MAX);
        let dirty = Probe::of(pager).dirty();
        assert_eq!(Some(&first), dirty.get(&pid), "{dirty:?}");
        let (ck, _, _) = pager.log_checkpoint();
        pager.release(Quarantine::Database).unwrap();
        let freed = pager.free(pid).unwrap();
        pager.flush_log();
        let dirty = Probe::of(pager).dirty();
        assert!(!dirty.contains_key(&pid), "{dirty:?}");
        for _ in 0..2 {
            fill(pager, pages, SEGMENT);
            pager.flush_pages(usize::MAX);
            pager.checkpoint();
        }
        assert!(pager.recycle_log() >= 1);
        [first, lost, ck, freed]
    });
    let kept = segments(&sim.files())[0];
    assert!(lsns[1].0 < kept, "{lsns:?} {kept}");
    (lsns, sim.recording())
}

#[test]
fn a_change_refused_unwritten_under_a_whole_quarantine_reads_back_after_the_close() {
    let (mut sim, pages) = created(40);
    let pid = pages[0];
    session(&mut sim, |pager, _| {
        pager
            .quarantine(Quarantine::Database, &ordered_for_test())
            .unwrap();
        changed_after_its_write(pager, pid, &pages[1..]);
        pager.damage(pid, Damage::Checksum);
    });
    let read = session(&mut sim, |pager, _| {
        pager
            .read(pid)
            .map(|p| u64::from_le_bytes(p[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap()))
    });
    assert_eq!(Ok(2), read);
}

#[test]
fn a_change_dropped_unwritten_with_its_page_is_replayed_until_the_page_is_durably_freed() {
    let (mut sim, pages) = created(40);
    let pid = pages[0];
    session(&mut sim, |pager, _| fill(pager, &pages[1..], 3 * SEGMENT));
    session(&mut sim, |pager, _| assert!(pager.recycle_log() >= 1));
    let ([first, lost, ck, freed], recording) = refused_then_freed(pid, &pages[1..], &mut sim);
    let mut past_checkpoint = 0;
    let mut freed_seen = 0;
    for at in 0..=recording.len() {
        let mut crashed =
            Sim::with_files(4, Config::default(), recording.images(at, 1, 0).remove(0));
        let mut seen = None;
        harness::finished(&mut crashed, |rt| {
            let (pager, opening) = Pager::open(rt, &options()).unwrap();
            let recovery = opening.recovery().cloned().expect("the open recovered");
            let read = (!pager.is_free(pid).unwrap()).then(|| {
                pager
                    .read(pid)
                    .map(|p| u64::from_le_bytes(p[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap()))
            });
            seen = Some((recovery.end, read));
            pager.close();
        });
        let (end, read) = seen.unwrap();
        let expected = match () {
            () if end <= first => Some(Ok(0)),
            () if end <= lost => Some(Ok(1)),
            () if end <= freed => Some(Ok(2)),
            () => None,
        };
        assert_eq!(expected, read, "crash after event {at}, log end {end:?}");
        past_checkpoint += usize::from(ck < end && end <= freed);
        freed_seen += usize::from(end > freed);
    }
    assert!(
        past_checkpoint >= 1 && freed_seen >= 1,
        "{past_checkpoint} {freed_seen}"
    );
}
