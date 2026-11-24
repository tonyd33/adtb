//! Recycling the log: what it removes, what it keeps, and the page LSN map it folds into first.

use crate::testing::Probe;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::storage::policy::{Holder, Phase};
use crate::storage::{
    self, BackupRun, Damage, Envelope, Finding, HEADER_LEN, Lsn, MarkFailure, Options, PageError,
    PageKind, Pager, Pid, Quarantine,
};
use crate::storage::{Judged, MapPoint, ordered_for_test};
use crate::test_world as harness;
use crate::verify::{self, Inventory, Problem};
use adtb_sim::{Config, Sim, SimRuntime};
use adtb_testkit::damage::{DataFile, Files};
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
fn a_recycle_keeps_the_log_from_the_checkpoint_the_last_session_closed_with() {
    let (mut sim, pages) = created(8);
    let mut got = Vec::new();
    let mut want = Vec::new();
    for session_no in 0..8 {
        let (start, end) = session(&mut sim, |pager, _| {
            let start = Probe::of(pager).appended().0;
            pager.recycle_log();
            fill(pager, &pages, 3 * SEGMENT);
            (start, Probe::of(pager).appended().0)
        });
        let held: Vec<u64> = (start / SEGMENT..=end / SEGMENT)
            .map(|s| s * SEGMENT)
            .collect();
        want.push((session_no, held));
        got.push((session_no, segments(&sim.files())));
    }
    assert_eq!(want[1..], got[1..]);
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
    let kept: Vec<u64> = segments(&sim.files())
        .into_iter()
        .filter(|s| before.contains(s))
        .collect();
    assert_eq!((1, before[1..].to_vec()), (removed, kept));
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

/// Space: recycles while a backup copies the file's first page and while it copies a data page,
/// each after the log has grown by four segments and a checkpoint has passed them.
#[test]
fn recycles_while_a_backup_runs_remove_nothing_and_it_restores_every_value() {
    let (mut sim, pages) = created(8);
    session(&mut sim, |pager, _| fill(pager, &pages, 3 * SEGMENT));
    let (during, backup, after) = session(&mut sim, |pager, _| {
        storage::recycle_at_open(pager, None);
        let mut during = Vec::new();
        let mut visit = |pid: Pid, _: &mut [u8]| {
            let at = match pid {
                Pid(0) => "first page",
                p if p == pages[0] => "data page",
                _ => return true,
            };
            if !during.iter().any(|&(was, _)| was == at) {
                for _ in 0..2 {
                    fill(pager, &pages[1..], 2 * SEGMENT);
                    pager.flush_pages(usize::MAX);
                    pager.checkpoint();
                    during.push((at, pager.recycle_log()));
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
    let removed =
        |at: &str| -> Vec<usize> { during.iter().filter(|r| r.0 == at).map(|r| r.1).collect() };
    assert_eq!(vec![0, 0], removed("first page"));
    assert_eq!(vec![0, 0], removed("data page"));
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

/// Map every record of `pager` as running, by its storage's own holders and no walk. Returns the
/// marks that failed, and the marks then.
fn mapped_once(
    pager: &Pager<SimRuntime>,
) -> (BTreeMap<Quarantine, MarkFailure>, BTreeSet<Quarantine>) {
    let holders = |pid| pager.holders_of(pid, Phase::Running, |_| {});
    let failed = pager.map_at(&MapPoint::pass(), holders, &ordered_for_test());
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
        pager.damage_tested(leaf, damage);
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

/// Returns what `held` finds in a session of `files` without the second of its log segments, the
/// files after it, and what `whole` finds in the next session, with the segment back. Page
/// `revert.1` of `files` is first reverted to its copy in `revert.0`.
fn across_a_gap<T, U>(
    files: &Files,
    revert: Option<(&Files, Pid)>,
    held: impl FnOnce(&Pager<SimRuntime>) -> T,
    whole: impl FnOnce(&Pager<SimRuntime>) -> U,
) -> (T, Files, U) {
    let segments = segments(files);
    assert!(segments.len() >= 3, "{segments:?}");
    let gap = format!("{}/{:016x}", options().log_dir(), segments[1]);
    let mut gapped = match revert {
        Some((before, pid)) => reverted(files.clone(), before, pid),
        None => files.clone(),
    };
    gapped.remove(&gap).unwrap();
    let mut sim = Sim::with_files(3, Config::default(), gapped);
    let found = session(&mut sim, |pager, _| held(pager));
    let after = sim.files();
    let mut back = after.clone();
    back.insert(gap.clone(), files[&gap].clone());
    let again = session(
        &mut Sim::with_files(3, Config::default(), back),
        |pager, _| whole(pager),
    );
    (found, after, again)
}

/// Space: an open held by a gap in its retained log that touches nothing, sets a mark on a marks
/// page it read unchecked, folds a damage list it read unchecked, or releases a mark by a swap;
/// then the open after the gap is filled.
#[test]
fn an_open_held_by_a_log_gap_quarantines_the_database_and_writes_no_mark_or_list_it_read_unchecked()
{
    let database = BTreeSet::from([Quarantine::Database]);
    let (mut sim, pages) = created(8);
    session(&mut sim, |pager, _| fill(pager, &pages, 3 * SEGMENT));
    let marks = |pager: &Pager<SimRuntime>| Probe::of(pager).marks();
    let (held, _, after) = across_a_gap(&sim.files(), None, marks, marks);
    assert_eq!(
        (database.clone(), BTreeSet::new()),
        (held, after),
        "a gap alone"
    );

    let (mut sim, _) = created(8);
    let before = sim.files();
    let marks_page = session(&mut sim, |pager, _| {
        pager
            .quarantine(Quarantine::Index(3), &ordered_for_test())
            .unwrap();
        fill(pager, &pages[1..], 3 * SEGMENT);
        pager.quarantine_page()
    });
    let heap = Holder::Relation {
        object: Quarantine::Heap(7),
        engine_module: false,
    };
    let mark = |pager: &Pager<SimRuntime>| {
        let loaded = Probe::of(pager).marks();
        pager.damage_tested(pages[0], Damage::Checksum);
        let failed = pager.map_at(
            &MapPoint::pass(),
            |_| Some(BTreeSet::from([heap])),
            &ordered_for_test(),
        );
        let mapped = (pager.mapped_to(pages[0]), pager.marked_to(pages[0]));
        (loaded, failed, mapped, Probe::of(pager).marks())
    };
    let (held, after, again) = across_a_gap(&sim.files(), Some((&before, marks_page)), mark, marks);
    let failed = BTreeMap::from([(
        Quarantine::Heap(7),
        MarkFailure::Unverified { page: marks_page },
    )]);
    let heap7 = BTreeSet::from([Quarantine::Heap(7)]);
    let both = BTreeSet::from([Quarantine::Database, Quarantine::Heap(7)]);
    assert_eq!(
        (
            (database.clone(), failed, (None, Some(heap7)), both),
            lsn_of(&before, marks_page),
            database.clone()
        ),
        (held, lsn_of(&after, marks_page), again),
        "a mark on an unchecked marks page"
    );

    let (mut sim, _) = created(8);
    let before = sim.files();
    let mut list = Pid(0);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        let pager = &pager;
        pager.damage_tested(pages[0], Damage::Checksum);
        pager.checkpoint();
        pager.flush_pages(usize::MAX);
        fill(pager, &pages[2..], 3 * SEGMENT);
        pager.flush_pages(usize::MAX);
        pager.checkpoint();
        pager.damage_tested(pages[1], Damage::Checksum);
        pager.flush_log();
        list = pager.damage_list_page();
    });
    let files = sim.files();
    assert_ne!(
        lsn_of(&before, list),
        lsn_of(&files, list),
        "the fold wrote the list"
    );
    let fold = |pager: &Pager<SimRuntime>| {
        pager.checkpoint();
        Probe::of(pager).marks()
    };
    let recorded = |pager: &Pager<SimRuntime>| {
        let recorded = Probe::of(pager).records().pages().get(&pages[0]).copied();
        (Probe::of(pager).marks(), recorded)
    };
    let (held, after, again) = across_a_gap(&files, Some((&before, list)), fold, recorded);
    assert_eq!(
        (database.clone(), lsn_of(&before, list), (database, None)),
        (held, lsn_of(&after, list), again),
        "a fold of an unchecked damage list"
    );

    let (mut sim, _) = created(8);
    let [to, from] = [pages[0], pages[1]];
    session(&mut sim, |pager, _| {
        pager
            .quarantine(Quarantine::Index(3), &ordered_for_test())
            .unwrap();
        put(pager, from, 9);
        fill(pager, &pages[2..], 3 * SEGMENT);
    });
    let swap = |pager: &Pager<SimRuntime>| {
        pager
            .replace_contents(to, from, Some(Quarantine::Index(3)))
            .unwrap();
        (value_of(pager, to), Probe::of(pager).marks())
    };
    let read = |pager: &Pager<SimRuntime>| (value_of(pager, to), Probe::of(pager).marks());
    let (held, _, again) = across_a_gap(&sim.files(), None, swap, read);
    let all = BTreeSet::from([Quarantine::Database, Quarantine::Index(3)]);
    assert_eq!(
        ((9, all), (9, BTreeSet::from([Quarantine::Index(3)]))),
        (held, again),
        "a swap releasing a mark"
    );
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
    let (judged, file) = pager.read_judged_tested(pid);
    assert_eq!(Judged::Sound, judged, "the copy written at {first:?}");
    let file = file.unwrap();
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
        pager.damage_tested(pid, Damage::Checksum);
        pager.flush_pages(usize::MAX);
        let dirty = Probe::of(pager).dirty();
        assert_eq!(Some(&first), dirty.get(&pid), "{dirty:?}");
        let (ck, _, _) = pager.log_checkpoint_tested();
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

/// Space: a page changed after its write and refused with the change unwritten, under a whole
/// quarantine or none, then a checkpoint, the session ended by a close or a crash.
#[test]
fn a_change_refused_unwritten_is_replayed_by_the_next_open() {
    let mut got = Vec::new();
    for whole in [false, true] {
        for crash in [false, true] {
            let (mut sim, pages) = created(40);
            let pid = pages[0];
            let change = |pager: &Pager<SimRuntime>| {
                if whole {
                    pager
                        .quarantine(Quarantine::Database, &ordered_for_test())
                        .unwrap();
                }
                changed_after_its_write(pager, pid, &pages[1..]);
                pager.damage_tested(pid, Damage::Checksum);
                pager.checkpoint();
            };
            if crash {
                adtb_testkit::common::run_then_crash(
                    &mut sim,
                    Box::new(|rt, _| {
                        change(&Pager::open(rt.clone(), &options()).unwrap().0);
                    }),
                );
            } else {
                session(&mut sim, |pager, _| change(pager));
            }
            let read: Result<u64, PageError> = session(&mut sim, |pager, _| {
                let page = pager.read(pid)?;
                Ok(u64::from_le_bytes(
                    page[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap(),
                ))
            });
            got.push(((whole, crash), read));
        }
    }
    let want: Vec<_> = got.iter().map(|&(row, _)| (row, Ok(2))).collect();
    assert_eq!(4, got.len());
    assert_eq!(want, got);
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
