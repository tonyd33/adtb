//! Recycling the log within one open: once the open's readers of the log have run, each cycle of
//! the checkpointer that finds a whole segment below the floor recycles.

use crate::testing::Probe;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::{Done, Op, OpKind, Runtime};
use crate::storage::{
    self, Damage, Finding, HEADER_LEN, Lsn, Options, PageError, PageKind, Pager, Pid, Quarantine,
};
use crate::test_world as harness;
use adtb_harness::common;
use adtb_harness::damage::{DataFile, Files};
use adtb_harness::search::each_crash_image;
use adtb_sim::{Config, Match, Sim, SimRuntime};

const PAGER: DataFile = DataFile::new(harness::NAME, harness::PAGE);

const SEGMENT: u64 = 64 * 1024;
const EVERY: u64 = 128 * 1024;
/// The most log forced between two checkpoints of these loads: the threshold, and at most half a
/// threshold the load logs while a cycle runs.
const INTERVAL: u64 = EVERY + EVERY / 2;

fn options(atomic: bool) -> Options {
    Options {
        segment_size: SEGMENT,
        checkpoint_every: EVERY,
        atomic_page_writes: atomic,
        ..harness::options(16)
    }
}

/// Returns a closed file of `n` data pages whose log runs past three segments, and the pages.
fn created(n: usize, atomic: bool) -> (Sim, Vec<Pid>) {
    let mut sim = Sim::new(3, Config::default());
    let mut pages = Vec::new();
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(atomic)).unwrap();
        pages = (0..n)
            .map(|_| pager.allocate(1, PageKind::HeapSlotted, 7).unwrap())
            .collect();
        load(&pager, &pages, 3 * SEGMENT);
        pager.close();
    });
    (sim, pages)
}

/// Open the file of `sim`, run `body` on it and close it. Returns what `body` returns.
fn session<T>(
    sim: &mut Sim,
    atomic: bool,
    body: impl FnOnce(&Pager<SimRuntime>, &SimRuntime) -> T,
) -> T {
    let mut out = None;
    harness::finished(sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(atomic)).unwrap();
        out = Some(body(&pager, &rt));
        pager.close();
    });
    out.unwrap()
}

fn put<R: Runtime>(pager: &Pager<R>, pid: Pid, value: u64) {
    pager
        .update(&[pid], |p| {
            p[0][HEADER_LEN..HEADER_LEN + 8].copy_from_slice(&value.to_le_bytes());
        })
        .unwrap();
    pager.flush_log();
}

fn value_of<R: Runtime>(pager: &Pager<R>, pid: Pid) -> u64 {
    let page = pager.read(pid).unwrap();
    u64::from_le_bytes(page[HEADER_LEN..HEADER_LEN + 8].try_into().unwrap())
}

/// Write the pages of `pages` in turn, each forced, until the log has grown by `bytes`. Returns
/// the last value written to each.
fn load<R: Runtime>(pager: &Pager<R>, pages: &[Pid], bytes: u64) -> BTreeMap<Pid, u64> {
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

/// Write as [`load`] does, letting the checkpointer run a cycle after each half threshold of log.
fn paced(pager: &Pager<SimRuntime>, rt: &SimRuntime, pages: &[Pid], bytes: u64) {
    let start = Probe::of(pager).appended().0;
    while Probe::of(pager).appended().0 < start + bytes {
        load(pager, pages, EVERY / 2);
        settle(rt);
    }
}

/// Returns the first LSN of each segment the log directory holds now.
fn listed(rt: &SimRuntime, atomic: bool) -> Vec<u64> {
    let mut io = rt.io();
    let dir = options(atomic).log_dir();
    let listed = common::complete(rt, &mut io, Op::List { dir }).result;
    let Ok(Done::Listed(paths)) = listed else {
        panic!("{listed:?}");
    };
    let mut bases: Vec<u64> = paths
        .iter()
        .map(|p| u64::from_str_radix(p.rsplit_once('/').unwrap().1, 16).unwrap())
        .collect();
    bases.sort_unstable();
    bases
}

fn segment_of(lsn: u64) -> u64 {
    lsn - lsn % SEGMENT
}

/// Sleep long enough for the checkpointer to finish a cycle it is due.
fn settle(rt: &SimRuntime) {
    let mut io = rt.io();
    common::sleep(rt, &mut io, Duration::from_secs(1));
}

/// Returns the checkpoint at whose segment, less one segment, the log a sample sees must start,
/// given `named`, the checkpoints the file header named so far in order. A cycle removes what lies
/// before the batch of the redo LSN of the checkpoint before its own, and that redo LSN is at or
/// past the checkpoint before that one. A sample between a cycle's checkpoint and its removal
/// sees the floor of the cycle before.
fn oldest_kept(named: &[u64]) -> Option<u64> {
    named.iter().rev().nth(3).copied()
}

fn long_session_keeps_its_log_bounded(atomic: bool) {
    let (mut sim, pages) = created(64, atomic);
    let (wrote, retained, named, start, last) = session(&mut sim, atomic, |pager, rt| {
        storage::recycle_at_open(pager, None);
        let start = Probe::of(pager).appended().0;
        let mut named: Vec<u64> = vec![Probe::of(pager).checkpoint().0];
        let mut retained = 0;
        let mut wrote = BTreeMap::new();
        let mut n = 0u64;
        while Probe::of(pager).appended().0 < start + 40 * EVERY {
            let pid = pages[n as usize % pages.len()];
            n += 1;
            put(pager, pid, n);
            wrote.insert(pid, n);
            if !n.is_multiple_of(8) {
                continue;
            }
            let checkpoint = Probe::of(pager).checkpoint().0;
            if named.last() != Some(&checkpoint) {
                named.push(checkpoint);
            }
            let first = listed(rt, atomic)[0];
            if let Some(floor) = oldest_kept(&named) {
                assert!(
                    first + SEGMENT >= segment_of(floor),
                    "first segment {first:#x}, checkpoints {named:x?}"
                );
            }
            retained = retained.max(Probe::of(pager).appended().0 - first);
        }
        named.push(Probe::of(pager).appended().0);
        (wrote, retained, named, start, listed(rt, atomic)[0])
    });
    assert!(
        last >= start + 30 * EVERY,
        "the log of this open was not recycled: first segment {last:#x}, open at {start:#x}"
    );
    let longest = named[1..].windows(2).map(|w| w[1] - w[0]).max().unwrap();
    assert!(
        longest <= INTERVAL,
        "checkpoints {longest} bytes apart, over the threshold and a cycle's log"
    );
    let bound = 4 * INTERVAL + 2 * SEGMENT;
    assert!(
        retained <= bound,
        "retained {retained} bytes: a sample sees the log from three checkpoints before the named \
         one, less a straddling batch, so at most four intervals of {INTERVAL} and two segments, {bound}"
    );
    let values = session(&mut sim, atomic, |pager, _| {
        wrote
            .keys()
            .map(|&p| (p, value_of(pager, p)))
            .collect::<BTreeMap<_, _>>()
    });
    assert_eq!(wrote, values);
}

#[test]
fn a_long_session_keeps_its_log_from_the_checkpoint_three_before_the_named_one() {
    long_session_keeps_its_log_bounded(false);
}

#[test]
fn under_atomic_page_writes_a_long_session_keeps_its_log_bounded() {
    long_session_keeps_its_log_bounded(true);
}

#[test]
fn the_checkpointer_recycles_nothing_until_the_opens_readers_of_the_log_have_run() {
    let (mut sim, pages) = created(16, false);
    let (held, after, start) = session(&mut sim, false, |pager, rt| {
        let at_open = listed(rt, false);
        let first = Probe::of(pager).checkpoint();
        paced(pager, rt, &pages, 6 * EVERY);
        assert!(
            Probe::of(pager).checkpoint().0 >= first.0 + 4 * EVERY,
            "the checkpointer ran"
        );
        let held = listed(rt, false)[0] == at_open[0];
        storage::recycle_at_open(pager, None);
        let start = Probe::of(pager).appended().0;
        paced(pager, rt, &pages, 6 * EVERY);
        (held, listed(rt, false)[0], start)
    });
    assert!(
        held,
        "a segment was removed before the open's readers had run"
    );
    assert!(
        after >= start,
        "first segment {after:#x}, released at {start:#x}"
    );
}

#[test]
fn an_archive_behind_keeps_every_segment_it_has_not_archived_within_a_session() {
    let (mut sim, pages) = created(16, false);
    let (stalled, caught_up, through) = session(&mut sim, false, |pager, rt| {
        let through = Arc::new(AtomicU64::new(listed(rt, false)[0] + SEGMENT));
        let hook = Arc::clone(&through);
        pager.set_archive(Arc::new(move || Some(Lsn(hook.load(Ordering::SeqCst)))));
        storage::recycle_at_open(pager, None);
        paced(pager, rt, &pages, 6 * EVERY);
        let stalled = listed(rt, false)[0];
        let end = Probe::of(pager).appended().0;
        through.store(segment_of(end), Ordering::SeqCst);
        paced(pager, rt, &pages, 6 * EVERY);
        (stalled, listed(rt, false)[0], (stalled, segment_of(end)))
    });
    let (archived, later) = through;
    assert_eq!(archived, stalled, "removed past what the archive holds");
    assert!(caught_up > archived && caught_up <= later, "{caught_up:#x}");
}

#[test]
fn a_scrub_snapshot_taken_before_recycles_within_the_session_finds_nothing() {
    let (mut sim, pages) = created(16, false);
    let (found, first, snapshot_first) = session(&mut sim, false, |pager, rt| {
        storage::recycle_at_open(pager, None);
        let snapshot_first = listed(rt, false)[0];
        let log = pager.read_log(true);
        paced(pager, rt, &pages, 8 * EVERY);
        let found: Vec<Finding> = pages
            .iter()
            .flat_map(|&p| pager.check_page(p, &log))
            .collect();
        (found, listed(rt, false)[0], snapshot_first)
    });
    assert!(
        first > snapshot_first,
        "nothing was recycled during the pass"
    );
    assert_eq!(Vec::<Finding>::new(), found);
}

fn lsn_of(files: &Files, pid: Pid) -> Lsn {
    storage::lsn(PAGER.page(files, pid))
}

#[test]
fn a_lost_write_whose_record_was_recycled_within_its_session_is_refused_on_its_first_read() {
    let (mut sim, pages) = created(16, false);
    let before = sim.files();
    let pid = pages[0];
    let first = session(&mut sim, false, |pager, rt| {
        storage::recycle_at_open(pager, None);
        put(pager, pid, 1);
        paced(pager, rt, &pages[1..], 8 * EVERY);
        listed(rt, false)[0]
    });
    let after = sim.files();
    assert!(lsn_of(&after, pid).0 < first, "first segment {first:#x}");
    let mut lost = after.clone();
    PAGER.replace(&mut lost, pid, PAGER.page(&before, pid));
    let read = session(
        &mut Sim::with_files(3, Config::default(), lost),
        false,
        |pager, _| pager.read(pid).map(drop),
    );
    let damage = Damage::Stale {
        expected: lsn_of(&after, pid),
        found: lsn_of(&before, pid),
    };
    assert_eq!(Err(PageError { pid, damage }), read);
}

fn refused_while_changed_frees_the_floor(atomic: bool) {
    let (mut sim, pages) = created(16, atomic);
    let pid = pages[0];
    let (logged, dirty, first) = session(&mut sim, atomic, |pager, rt| {
        storage::recycle_at_open(pager, None);
        put(pager, pid, 1);
        pager.checkpoint();
        put(pager, pid, 2);
        let page = pager.read(pid).unwrap();
        let logged = Lsn(u64::from_le_bytes(page[..8].try_into().unwrap()));
        drop(page);
        pager.damage(pid, Damage::Checksum);
        paced(pager, rt, &pages[1..], 8 * EVERY);
        (
            logged,
            Probe::of(pager).dirty().contains_key(&pid),
            listed(rt, false)[0],
        )
    });
    assert!(!dirty, "the refused page kept its dirty entry");
    assert!(
        first > logged.0,
        "first segment {first:#x}, change at {logged:?}"
    );
    assert_eq!(
        logged,
        lsn_of(&sim.files(), pid),
        "the file holds the change"
    );
    let (value, findings) = session(&mut sim, atomic, |pager, _| {
        let findings = pager.verify().findings;
        (value_of(pager, pid), findings)
    });
    assert_eq!(2, value);
    let damage = Damage::Checksum;
    assert_eq!(vec![Finding::Page { pid, damage }], findings);
}

#[test]
fn a_page_refused_while_changed_is_written_and_holds_no_segment_past_the_next_force() {
    refused_while_changed_frees_the_floor(false);
}

#[test]
fn under_atomic_page_writes_a_page_refused_while_changed_holds_no_segment() {
    refused_while_changed_frees_the_floor(true);
}

#[test]
fn a_database_quarantined_whole_recycles_nothing_within_the_session() {
    let (mut sim, pages) = created(16, false);
    let (before, after, cycled) = session(&mut sim, false, |pager, rt| {
        storage::recycle_at_open(pager, None);
        let before = listed(rt, false);
        pager.quarantine(Quarantine::Database).unwrap();
        let first = Probe::of(pager).checkpoint();
        paced(pager, rt, &pages, 6 * EVERY);
        let cycled = Probe::of(pager).checkpoint().0 >= first.0 + 4 * EVERY;
        (before, listed(rt, false), cycled)
    });
    assert!(cycled, "the checkpointer ran");
    assert_eq!(before[0], after[0]);
}

/// Returns the removals of `removals`, each a device event and a segment's base, in runs of one
/// recycle: removals at most two events apart, a removal and its directory force.
fn recycles(removals: &[(usize, u64)]) -> Vec<Vec<(usize, u64)>> {
    let mut runs: Vec<Vec<(usize, u64)>> = Vec::new();
    for &r in removals {
        match runs.last_mut() {
            Some(run) if r.0 <= run.last().unwrap().0 + 2 => run.push(r),
            _ => runs.push(vec![r]),
        }
    }
    runs
}

fn crash_through_recycles_in_session(atomic: bool) {
    let (sim, pages) = created(48, atomic);
    let files = sim.files();
    let acked: Arc<Mutex<Vec<(usize, Pid, u64)>>> = Arc::default();
    let removals: Arc<Mutex<Vec<(usize, u64)>>> = Arc::default();
    let wanted: BTreeSet<Pid> = pages.iter().copied().collect();
    let dir = format!("{}/", options(atomic).log_dir());
    let mut reached = [0; 2];
    each_crash_image(
        &files,
        5,
        24,
        1,
        |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options(atomic)).unwrap();
            storage::recycle_at_open(&pager, None);
            load(&pager, &pages, 2 * EVERY);
            let removes = rt.logged(Match::kind(OpKind::Remove).prefixed(&dir));
            rt.arm(Match::kinds([]));
            let mut log = Vec::new();
            let start = Probe::of(&pager).appended().0;
            for n in 0.. {
                if Probe::of(&pager).appended().0 >= start + 6 * EVERY {
                    break;
                }
                let pid = pages[n % pages.len()];
                put(&pager, pid, n as u64 + 1_000_000);
                log.push((rt.recorded(), pid, n as u64 + 1_000_000));
            }
            let mut io = rt.io();
            common::sleep(rt, &mut io, Duration::from_secs(1));
            *acked.lock().unwrap() = log;
            *removals.lock().unwrap() = removes
                .entries()
                .iter()
                .map(|e| {
                    let name = &e.path().unwrap()[dir.len()..];
                    (e.recorded(), u64::from_str_radix(name, 16).unwrap())
                })
                .collect();
            let removed = removals.lock().unwrap();
            removed
                .iter()
                .flat_map(|&(e, _)| [e - 1, e, e + 1])
                .collect()
        },
        |at, image| {
            let held: BTreeSet<u64> = image
                .keys()
                .filter_map(|k| k.strip_prefix(&dir))
                .map(|n| u64::from_str_radix(n, 16).unwrap())
                .collect();
            for run in recycles(&removals.lock().unwrap()) {
                let (first, last) = (run[0], run[run.len() - 1]);
                if (first.0 - 1..=first.0).contains(&at) && held.contains(&first.1) {
                    reached[0] += 1;
                }
                if run.len() >= 2
                    && (first.0..last.0).contains(&at)
                    && !held.contains(&first.1)
                    && held.contains(&last.1)
                {
                    reached[1] += 1;
                }
            }
            let acked = acked.lock().unwrap();
            let by: BTreeMap<Pid, u64> = acked
                .iter()
                .filter(|(e, _, _)| *e <= at)
                .map(|&(_, p, v)| (p, v))
                .collect();
            let last: BTreeMap<Pid, u64> = acked.iter().map(|&(_, p, v)| (p, v)).collect();
            let mut sim = Sim::with_files(4, Config::default(), image);
            let (findings, allocated, values) = session(&mut sim, atomic, |pager, _| {
                let values: BTreeMap<Pid, u64> =
                    by.keys().map(|&p| (p, value_of(pager, p))).collect();
                (pager.verify().findings, pager.allocated().unwrap(), values)
            });
            let allocated: BTreeSet<Pid> = allocated.into_iter().collect();
            assert_eq!(
                (Vec::<Finding>::new(), &wanted),
                (findings, &allocated),
                "crash after event {at}"
            );
            for (pid, v) in values {
                assert!(
                    by[&pid] <= v && v <= last[&pid],
                    "crash after event {at}: {pid:?} holds {v}, acknowledged {}",
                    by[&pid]
                );
            }
        },
    );
    assert!(
        reached.iter().all(|&n| n >= 1),
        "crashes before a recycle's first removal and between two of its removals: {reached:?}"
    );
}

#[test]
fn a_crash_between_a_recycles_checkpoint_and_its_removals_or_during_them_loses_nothing() {
    crash_through_recycles_in_session(false);
}

#[test]
fn under_atomic_page_writes_a_crash_through_a_recycle_within_a_session_loses_nothing() {
    crash_through_recycles_in_session(true);
}
