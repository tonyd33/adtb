//! The checkpointer: within one open, each time the log forced since the last checkpoint reaches
//! the threshold, a background thread writes every changed page and takes a fuzzy checkpoint.

use crate::testing::Probe;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::{Clock, Instant, OpKind, Runtime, Threads};
use crate::storage::{
    self, BackupRun, Crc32c, Damage, Envelope, HEADER_LEN, Lsn, Options, PageKind, Pager, Pid,
};
use crate::test_world as harness;
use adtb_sim::{Action, Config, Match, Outcome, Plan, Sim, SimRuntime, Submitted};
use adtb_sys::runtime::Faults;
use adtb_testkit::common;
use adtb_testkit::damage::{DataFile, Files};
use adtb_testkit::profile::{assert_reached, every_case};
use adtb_testkit::search::each_crash_image;

const PAGER: DataFile = DataFile::new(harness::NAME, harness::PAGE);

const EVERY: u64 = 256 * 1024;
const FRAMES: usize = 16;
const PAGES: usize = 400;
const BLOCK: u64 = 512;
const BLOCK_HEADER: u64 = 56;

fn options(atomic: bool) -> Options {
    Options {
        checkpoint_every: EVERY,
        atomic_page_writes: atomic,
        ..harness::options(FRAMES)
    }
}

/// Returns the files of a closed database of `PAGES` data pages, and them.
fn created(atomic: bool) -> (Files, Vec<Pid>) {
    let mut sim = Sim::new(3, Config::default());
    let mut pages = Vec::new();
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(atomic)).unwrap();
        pages = (0..PAGES)
            .map(|_| pager.allocate(1, PageKind::HeapSlotted, 7).unwrap())
            .collect();
        pager.close();
    });
    (sim.files(), pages)
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

/// A write a load acknowledged.
#[derive(Clone, Copy)]
struct Ack {
    /// The device events recorded when it returned.
    event: usize,
    /// The simulated time it returned at.
    time: Instant,
    pid: Pid,
    value: u64,
    /// The LSN the next record takes once it returned.
    appended: Lsn,
}

#[derive(Default)]
struct Load {
    acked: Vec<Ack>,
    /// After each write to a page other than the hot one, and the hot one's write when it follows,
    /// the next record's LSN, the header's checkpoint LSN and the dirty page table.
    samples: Vec<(Lsn, Lsn, BTreeMap<Pid, Lsn>)>,
    /// The simulated time of each sample.
    times: Vec<Instant>,
}

impl Load {
    /// Returns the times of the samples at which the `rec_lsn` of `pid` had risen since the sample
    /// before while it stayed dirty, to below the LSN the next record took at that sample: a raise,
    /// since a page entering the table again takes an LSN past it.
    fn raised(&self, pid: Pid) -> Vec<Instant> {
        let at = self.samples.windows(2).zip(&self.times[1..]);
        let raised = at.filter(|(w, _)| {
            let rec = |i: usize| w[i].2.get(&pid);
            rec(0).is_some_and(|a| rec(1).is_some_and(|b| b > a && *b < w[0].0))
        });
        raised.map(|(_, t)| *t).collect()
    }
}

/// Write until the log has grown by `bytes`: write `n` puts `base + n` to a page of `pages[1..]` in
/// turn, and every fourth also to the hot page `pages[0]`, each forced.
fn load(
    rt: &SimRuntime,
    pager: &Pager<impl Runtime>,
    pages: &[Pid],
    bytes: u64,
    base: u64,
) -> Load {
    let start = Probe::of(pager).appended().0;
    let mut out = Load::default();
    let mut n = 0;
    let acked = |out: &mut Load, pid, value| {
        put(pager, pid, value);
        out.acked.push(Ack {
            event: rt.recorded(),
            time: rt.now(),
            pid,
            value,
            appended: Probe::of(pager).appended(),
        });
    };
    while Probe::of(pager).appended().0 < start + bytes {
        n += 1;
        acked(
            &mut out,
            pages[1 + n as usize % (pages.len() - 1)],
            base + n,
        );
        if n % 4 == 0 {
            acked(&mut out, pages[0], base + n);
        }
        out.times.push(rt.now());
        out.samples.push((
            Probe::of(pager).appended(),
            Probe::of(pager).checkpoint(),
            Probe::of(pager).dirty(),
        ));
    }
    out
}

/// Returns the last value acknowledged for each page by device event `at`.
fn acked_by(acked: &[Ack], at: usize) -> BTreeMap<Pid, u64> {
    let by = acked.iter().filter(|a| a.event <= at);
    by.map(|a| (a.pid, a.value)).collect()
}

/// Returns the LSN the durable log ends at or before after a crash at device event `at`: the
/// next record's LSN once the first write not acknowledged by then returned, else `last`.
fn log_end_by(acked: &[Ack], at: usize, last: Lsn) -> Lsn {
    acked
        .iter()
        .find(|a| a.event > at)
        .map_or(last, |a| a.appended)
}

/// Returns `n` bytes of the records of `files`'s log from `lsn`, skipping block headers.
fn log_bytes(files: &Files, lsn: Lsn, n: usize) -> Vec<u8> {
    let segment = harness::options(FRAMES).segment_size;
    let mut out = Vec::new();
    let mut at = lsn.0;
    while out.len() < n {
        if at % BLOCK < BLOCK_HEADER {
            at += BLOCK_HEADER - at % BLOCK;
        }
        let name = format!("{}/{:016x}", options(false).log_dir(), at - at % segment);
        out.push(files[&name][(at % segment) as usize]);
        at += 1;
    }
    out
}

/// Returns the LSN and the redo LSN of the checkpoint record the file header of `files` names, as
/// an open chooses its copy: the whole copy with the higher sequence number. A copy is whole when
/// it starts with the magic and its crc32c, seeded by the copy's number and taken with the
/// checksum word at byte 8 zeroed, matches. The header holds its sequence number at byte 16 and
/// the record's LSN at byte 64; the record holds its redo LSN after a tag byte and a length word.
fn header_checkpoint(files: &Files) -> (Lsn, Lsn) {
    let copy = |n: u64| {
        let region = &PAGER.page(files, Pid(n))[..4096];
        let word = |o: usize| u64::from_le_bytes(region[o..o + 8].try_into().unwrap());
        let sum = Crc32c::new()
            .update(&n.to_le_bytes())
            .update(&region[..8])
            .update(&[0; 4])
            .update(&region[12..])
            .finish();
        let whole = region[..8] == *b"adtbfile" && region[8..12] == sum.to_le_bytes();
        whole.then(|| (word(16), word(64)))
    };
    let (_, checkpoint) = copy(0).max(copy(1)).expect("a whole header copy");
    let record = log_bytes(files, Lsn(checkpoint), 13);
    let redo = Lsn(u64::from_le_bytes(record[5..13].try_into().unwrap()));
    (Lsn(checkpoint), redo)
}

/// What an open of a database found: how many records recovery applied, whether it, `verify`
/// and the log found no damage, and the value of each page of `pages`.
struct Reopened {
    applied: usize,
    clean: bool,
    values: BTreeMap<Pid, u64>,
}

fn reopened(rt: SimRuntime, options: &Options, pages: &[Pid]) -> Reopened {
    let (pager, opening) = Pager::open(rt, options).unwrap();
    let recovery = opening.recovery().cloned().expect("the open recovered");
    let findings = pager.verify().findings;
    let values = pages.iter().map(|&p| (p, value_of(&pager, p))).collect();
    pager.close();
    Reopened {
        applied: recovery.applied,
        clean: recovery.damaged == Vec::<(Pid, Damage)>::new()
            && recovery.log.is_empty()
            && findings.is_empty(),
        values,
    }
}

fn reopen(files: Files, atomic: bool, pages: &[Pid]) -> Reopened {
    let mut sim = Sim::with_files(4, Config::default(), files);
    let mut out = None;
    harness::finished(&mut sim, |rt| {
        out = Some(reopened(rt, &options(atomic), pages));
    });
    out.unwrap()
}

/// Returns the greatest checkpoint LSN of `samples` below `at`.
fn previous(samples: &[(Lsn, Lsn, BTreeMap<Pid, Lsn>)], at: Lsn) -> Option<Lsn> {
    samples.iter().map(|(_, c, _)| *c).filter(|&c| c < at).max()
}

/// Returns the number of pages `acked` wrote past `from` and by `end`.
fn written_within(acked: &[Ack], from: Lsn, end: Lsn) -> usize {
    let within = acked
        .iter()
        .filter(|a| a.appended > from && a.appended <= end);
    within.map(|a| a.pid).collect::<BTreeSet<Pid>>().len()
}

fn load_keeps_recovery_and_the_dirty_page_table_bounded(atomic: bool) {
    let (files, pages) = created(atomic);
    let ledger = Arc::new(Mutex::new((Load::default(), Lsn(0), 0)));
    let mut points = 0;
    let mut ahead = 0;
    each_crash_image(
        &files,
        7,
        8,
        1,
        |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options(atomic)).unwrap();
            let warm = load(rt, &pager, &pages, 2 * EVERY, 0);
            rt.arm(Match::kinds([]));
            let base = warm.acked.last().unwrap().value;
            let mut run = load(rt, &pager, &pages, 10 * EVERY, base);
            let first_hot = run.acked.iter().find(|a| a.pid == pages[0]).unwrap();
            let before_first_acks = vec![run.acked[0].event - 1, first_hot.event - 1];
            let mut io = rt.io();
            common::sleep(rt, &mut io, Duration::from_secs(1));
            run.acked.splice(0..0, warm.acked);
            run.samples.splice(0..0, warm.samples);
            run.times.splice(0..0, warm.times);
            *ledger.lock().unwrap() = (run, Probe::of(&pager).appended(), base);
            before_first_acks
        },
        |at, image| {
            let (load, last, base) = &*ledger.lock().unwrap();
            // Recovery starts at or past the checkpoint before the one the header names, so it
            // reads the threshold, the span of the named checkpoint's cycle and the log after it.
            let (named, redo) = header_checkpoint(&image);
            let before = previous(&load.samples, named).expect("a checkpoint before the named one");
            let end = log_end_by(&load.acked, at, *last);
            assert!(
                before <= redo && redo <= end,
                "atomic {atomic}: crash after event {at}: redo {redo:?}, previous checkpoint {before:?}"
            );
            assert!(
                named.0 - before.0 >= EVERY,
                "atomic {atomic}: crash after event {at}: checkpoints {before:?} and {named:?} are \
                 under the threshold apart"
            );
            let got = reopen(image, atomic, &pages);
            assert!(got.clean, "atomic {atomic}: crash after event {at}");
            let wrote = acked_by(&load.acked, usize::MAX);
            for (pid, acked) in acked_by(&load.acked, at) {
                let v = got.values[&pid];
                assert!(
                    acked <= v && v <= wrote[&pid],
                    "atomic {atomic}: crash after event {at}: {pid:?} holds {v}, acknowledged {acked}"
                );
                let first_run = load.acked.iter().find(|a| a.pid == pid && a.value > *base);
                if acked <= *base && first_run.is_some_and(|a| a.value == v) {
                    ahead += 1;
                }
            }
            points += 1;
        },
    );
    assert!(points >= 4, "atomic {atomic}: {points} crash points");
    let first =
        "an image holding the run's unacknowledged first write of a page over the warm load's";
    assert_reached(&format!("atomic {atomic}: {first}"), ahead, 1);
    let (load, _, _) = &*ledger.lock().unwrap();
    let mut checkpoints: Vec<Lsn> = load.samples.iter().map(|(_, c, _)| *c).collect();
    checkpoints.dedup();
    assert!(checkpoints.len() >= 8, "atomic {atomic}: {checkpoints:?}");
    for (end, named, dirty) in &load.samples {
        let from = previous(&load.samples, *named).unwrap_or(Lsn(0));
        let bound = written_within(&load.acked, from, *end);
        assert!(
            dirty.len() <= bound,
            "atomic {atomic}: {} dirty pages, bound {bound}",
            dirty.len()
        );
    }
    assert!(
        !load.raised(pages[0]).is_empty(),
        "atomic {atomic}: the hot page's rec_lsn never moved while it stayed dirty"
    );
}

#[test]
fn a_write_load_keeps_what_recovery_reads_and_the_dirty_page_table_bounded() {
    for atomic in [false, true] {
        load_keeps_recovery_and_the_dirty_page_table_bounded(atomic);
    }
}

fn crash_in_a_cycle_loses_nothing(atomic: bool) {
    let (files, pages) = created(atomic);
    let wrote: Arc<Mutex<BTreeMap<Pid, u64>>> = Arc::default();
    let start: Arc<Mutex<Option<Files>>> = Arc::default();
    let mut phases = [0; 3];
    each_crash_image(
        &files,
        9,
        24,
        1,
        |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options(atomic)).unwrap();
            let before = Probe::of(&pager).checkpoint();
            let load = load(rt, &pager, &pages, EVERY, 0);
            *wrote.lock().unwrap() = acked_by(&load.acked, usize::MAX);
            rt.arm(Match::kinds([]));
            let mut io = rt.io();
            common::sleep(rt, &mut io, Duration::from_secs(1));
            assert!(
                Probe::of(&pager).checkpoint() > before,
                "atomic {atomic}: the checkpointer ran"
            );
            Vec::new()
        },
        |at, image| {
            let first = start
                .lock()
                .unwrap()
                .get_or_insert_with(|| image.clone())
                .clone();
            let header = 0..PAGER.range(Pid::FIRST_DATA).start;
            let log = |f: &Files| -> Vec<Vec<u8>> {
                let dir = format!("{}/", options(atomic).log_dir());
                let held = f.iter().filter(|(k, _)| k.starts_with(&dir));
                held.map(|(_, v)| v.clone()).collect()
            };
            let phase = if image[harness::NAME][header.clone()] != first[harness::NAME][header] {
                Some(2)
            } else if log(&image) != log(&first) {
                Some(1)
            } else {
                (image[harness::NAME] != first[harness::NAME]).then_some(0)
            };
            if let Some(k) = phase {
                phases[k] += 1;
            }
            let got = reopen(image, atomic, &pages);
            assert!(got.clean, "atomic {atomic}: crash after event {at}");
            let wrote = wrote.lock().unwrap();
            let held: BTreeMap<Pid, u64> = got
                .values
                .into_iter()
                .filter(|(p, _)| wrote.contains_key(p))
                .collect();
            assert_eq!(*wrote, held, "atomic {atomic}: crash after event {at}");
        },
    );
    assert!(phases.iter().all(|&n| n >= 1), "atomic {atomic} {phases:?}");
}

#[test]
fn a_crash_at_any_step_of_a_checkpoint_cycle_loses_no_acknowledged_write() {
    for atomic in [false, true] {
        crash_in_a_cycle_loses_nothing(atomic);
    }
}

#[test]
fn a_backup_taken_while_the_checkpointer_runs_restores_and_verifies_clean() {
    let (files, pages) = created(false);
    let mut sim = Sim::with_files(5, Config::default(), files);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(false)).unwrap();
        let mut during = None;
        let mut visit = |pid: Pid, _: &mut [u8]| {
            if pid == pages[0] && during.is_none() {
                let before = Probe::of(&pager).checkpoint();
                let load = load(&rt, &pager, &pages[1..], 3 * EVERY, 0);
                during = Some((before, Probe::of(&pager).checkpoint(), load));
            }
            true
        };
        let run = &mut BackupRun {
            target: "backup",
            envelope: Envelope {
                release: "test".into(),
                edition: "2026-10".into(),
                schema_version: 1,
                schema_hash: "00".into(),
                catalog_version: 1,
            },
            visit: &mut visit,
            name: &|q| format!("{q:?}"),
        };
        pager.backup(run).unwrap();
        let (before, after, load) = during.unwrap();
        assert!(after > before, "the checkpointer ran during the backup");
        pager.close();
        storage::restore_file(&rt, "backup", "copy.adt", "copy.adt-log").unwrap();
        let wrote = acked_by(&load.acked, usize::MAX);
        let copy = Options {
            name: "copy.adt".into(),
            ..options(false)
        };
        let got = reopened(rt, &copy, &pages[1..]);
        assert!(got.clean);
        let held: BTreeMap<Pid, u64> = got
            .values
            .into_iter()
            .filter(|(p, _)| wrote.contains_key(p))
            .collect();
        assert_eq!(wrote, held);
    });
}

#[test]
fn a_close_while_the_checkpointer_runs_waits_for_it_and_the_next_open_replays_nothing() {
    let (files, pages) = created(false);
    let mut sim = Sim::with_files(6, Config::default(), files);
    let mut wrote = BTreeMap::new();
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(false)).unwrap();
        let load = load(&rt, &pager, &pages, EVERY, 0);
        wrote = acked_by(&load.acked, usize::MAX);
        let closer = pager;
        let spawner = rt.clone();
        rt.plan(Plan::new().at(
            0,
            Action::Run(Box::new(move || {
                spawner.spawn("close", move || closer.close());
            })),
        ));
        rt.arm(Match::kind(OpKind::Write).path(harness::NAME));
        let mut io = rt.io();
        common::sleep(&rt, &mut io, Duration::from_secs(1));
    });
    assert!(
        sim.numbered().struck().is_some(),
        "the close began during the checkpointer's write"
    );
    let got = reopen(sim.files(), false, &pages);
    assert_eq!((0, true), (got.applied, got.clean));
    for (pid, v) in wrote {
        assert_eq!(v, got.values[&pid], "{pid:?}");
    }
}

/// Faults that tear every write a crash leaves unforced, by sector.
fn torn() -> Config {
    Config {
        faults: Faults {
            torn_p: 1.0,
            ..Faults::none(512)
        },
        ..Config::default()
    }
}

/// Run a load on the file of `files` under `torn` faults, crashing at `crash` when given. Returns
/// the simulation, what the load did when it finished, and the times at which a write of the hot
/// page began.
fn torn_run(files: &Files, pages: &[Pid], crash: Option<Instant>) -> (Sim, Load, Vec<Instant>) {
    let mut sim = Sim::with_files(11, torn(), files.clone());
    if let Some(at) = crash {
        sim.crash_at(at);
    }
    let hot = PAGER.range(pages[0]).start as u64;
    let writes = sim.logged(Match::kind(OpKind::Write).path(harness::NAME).offset(hot));
    let mut done = Load::default();
    let outcome = sim.run(|rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(false)).unwrap();
        done = load(&rt, &pager, pages, 12 * EVERY, 0);
        let mut io = rt.io();
        common::sleep(&rt, &mut io, Duration::from_secs(1));
    });
    let expected = if crash.is_some() {
        Outcome::Crashed
    } else {
        Outcome::Finished
    };
    assert_eq!(expected, outcome);
    let times = writes.entries().iter().map(Submitted::at).collect();
    (sim, done, times)
}

#[test]
fn a_crash_tearing_a_write_of_a_page_whose_rec_lsn_was_raised_loses_no_acknowledged_write() {
    let (files, pages) = created(false);
    let (_, done, writes) = torn_run(&files, &pages, None);
    let after = done
        .raised(pages[0])
        .into_iter()
        .filter_map(|r| writes.iter().copied().find(|&w| w > r));
    let mut crashes: Vec<Instant> = after.collect();
    crashes.dedup();
    assert!(
        !crashes.is_empty(),
        "no write of the hot page began after its rec_lsn was raised"
    );
    let wrote = acked_by(&done.acked, usize::MAX);
    let hot = format!("@{} units", PAGER.range(pages[0]).start);
    let most = if every_case() { crashes.len() } else { 4 };
    let mut tore = 0;
    for &t in crashes.iter().take(most) {
        let crash = t + Duration::from_nanos(1);
        let (sim, _, _) = torn_run(&files, &pages, Some(crash));
        let trace = sim.trace();
        let mut torn = trace
            .lines()
            .filter(|l| l.contains("crash torn") && l.contains(&hot));
        tore += usize::from(torn.any(|l| l.contains("true") && l.contains("false")));
        let got = reopen(sim.files(), false, &pages);
        assert!(got.clean, "crash at {crash:?}");
        let by = done.acked.iter().filter(|a| a.time < crash);
        for (pid, v) in by.map(|a| (a.pid, a.value)).collect::<BTreeMap<_, _>>() {
            let held = got.values[&pid];
            assert!(
                v <= held && held <= wrote[&pid],
                "crash at {crash:?}: {pid:?} holds {held}, acknowledged {v}"
            );
        }
    }
    assert!(tore >= 1, "no crash tore the hot page's write");
}

const USERS: &str = r"module app
add module app
add type User = { id: Key users, name: Text }
add relation users : User key id generate sequence
";

/// Commit the insert of user `id` to `db`.
fn commit_user<R: Runtime>(db: &crate::Database<R>, id: i64) {
    let text = format!(r#"insert users {{ id: users.key({id}), name: "user {id}" }}"#);
    let mut t = db.begin(&crate::TxnOptions::default()).unwrap();
    t.query_text(&text, &[], &crate::TxnCallOptions::default())
        .unwrap();
    t.commit(None).unwrap();
}

/// Keep every commit acknowledged under checkpoints and page flushes of other threads across a
/// crash.
#[test]
fn first_checkpoint_under_live_writers_keeps_every_acked_commit() {
    use crate::test_world::load as open;
    let log = || crate::Log::new([USERS]);
    let file = "checkpointed.adt";
    for seed in adtb_testkit::profile::sweep(40..42) {
        let acked: Arc<Mutex<Vec<i64>>> = Arc::default();
        let torn = Faults {
            torn_p: 0.5,
            ..Faults::none(512)
        };
        let mut sim = Sim::new(
            seed,
            Config {
                faults: torn,
                ..Config::default()
            },
        );
        adtb_testkit::process::run(&mut sim, |rt| {
            open::opened(&rt, file, &log(), &open::options(2))
                .close()
                .unwrap();
        });
        sim.crash_at(sim.now() + Duration::from_millis(300));
        let shared = acked.clone();
        let outcome = sim.run(move |rt| {
            let db = open::opened(&rt, file, &log(), &open::options(2));
            for w in 0..2 {
                let (db, acked) = (db.clone(), shared.clone());
                rt.spawn("writer", move || {
                    for id in (w..).step_by(2) {
                        commit_user(&db, id);
                        acked.lock().unwrap().push(id);
                    }
                });
            }
            let other = db.clone();
            rt.spawn("flusher", move || {
                loop {
                    other.testing().flush_pages(usize::MAX);
                }
            });
            loop {
                db.testing().flush_pages(usize::MAX);
                db.testing().checkpoint();
                rt.yield_now();
            }
        });
        assert_eq!(Outcome::Crashed, outcome, "seed {seed}");
        let acked: BTreeSet<i64> = acked.lock().unwrap().iter().copied().collect();
        assert!(!acked.is_empty(), "seed {seed}: no commit before the crash");
        adtb_testkit::process::run(&mut sim, |rt| {
            let none = crate::OpenOptions {
                create: None,
                ..open::options(2)
            };
            let db = open::open_in(&rt, file, &log(), &none).unwrap();
            let rows = crate::test_world::api::query(&db, "from u in users select u.id");
            let rows: BTreeSet<i64> = rows
                .iter()
                .map(|r| {
                    r.trim_start_matches("users.key(")
                        .trim_end_matches(')')
                        .parse()
                        .unwrap()
                })
                .collect();
            assert!(
                acked.is_subset(&rows),
                "seed {seed}: {:?} lost",
                acked.difference(&rows)
            );
            let holding = db.testing().verify_holding().unwrap().count();
            assert_eq!(0, holding, "seed {seed}");
            db.close().unwrap();
        });
    }
}

/// Keep a page changed and written while another thread's force of the data file runs, through a
/// checkpoint whose header reaches the device ahead of that page.
#[test]
fn page_written_during_a_force_stays_dirty_through_a_checkpoint() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let options = Options {
        page_size: 4096,
        segment_size: 1 << 20,
        pool_frames: 64,
        max_pages: 4096,
        ..Options::new("forced.adt")
    };
    let mut sim = Sim::new(31, Config::default());
    sim.record();
    sim.crash_at(sim.now() + Duration::from_secs(1));
    let mut changed = None;
    let updated = Arc::new(AtomicBool::new(false));
    let published = sim.logged(Match::kind(OpKind::Rename).path("forced.adt"));
    let mut logs = None;
    let outcome = sim.run(|rt| {
        let pager = Arc::new(Pager::create(rt.clone(), &options).unwrap());
        let data = published.entries()[0].from().unwrap().to_owned();
        let pid = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        changed = Some(pid);
        pager.update(&[pid], |v| v[0][100] = 1).unwrap();
        pager.flush_log();
        let second = pager.clone();
        let (spawner, first, marked, seen) =
            (rt.clone(), rt.clone(), updated.clone(), updated.clone());
        let during = Action::Run(Box::new(move || {
            let (waker, waiter) = (first.clone(), first.current());
            spawner.spawn("second force", move || {
                second.update(&[pid], |v| v[0][100] = 2).unwrap();
                marked.store(true, Ordering::SeqCst);
                waker.unpark(waiter);
                second.flush_log();
                second.flush_pages(usize::MAX);
            });
            while !seen.load(Ordering::SeqCst) {
                first.park();
            }
        }));
        let data_forces = Match::kind(OpKind::Sync).path(&data);
        let page = pid.0 * u64::from(options.page_size);
        logs = Some((
            rt.logged(data_forces.clone()),
            rt.logged(Match::kind(OpKind::Write).path(&data).covers(page)),
            page,
        ));
        rt.plan(
            Plan::new()
                .first(data_forces.clone(), during)
                .rule(data_forces, Action::Hold),
        );
        rt.spawn("first force", move || {
            pager.flush_pages(usize::MAX);
            pager.checkpoint();
        });
        let mut io = rt.io();
        common::sleep(&rt, &mut io, Duration::from_secs(10));
        unreachable!("the crash comes first");
    });
    assert_eq!(Outcome::Crashed, outcome);
    assert!(
        updated.load(Ordering::SeqCst),
        "the page changed during the first force"
    );
    let (forces, writes, page) = logs.expect("the run planned");
    let forces = forces.entries();
    assert!(
        forces.len() >= 2,
        "a later force of the data file was held: {forces:?}"
    );
    let first = &forces[0];
    let changed_write = writes
        .entries()
        .into_iter()
        .find(|w| {
            let at = (page - w.offset().unwrap()) as usize;
            w.bytes().unwrap()[at + 100] == 2
        })
        .expect("the changed page was written");
    let completed = first.completed().expect("the first force completed");
    assert!(
        first.ticket() < changed_write.ticket() && changed_write.at() < completed,
        "the page's write came during the first force: {first:?}, {changed_write:?}"
    );
    let pid = changed.unwrap();
    let recording = sim.recording();
    let images = recording.images(recording.len(), 64, 31);
    for (i, image) in images.into_iter().enumerate() {
        let mut after = Sim::with_files(32, Config::default(), image);
        adtb_testkit::process::run(&mut after, |rt| {
            let (copy, _) = Pager::open(rt, &options).unwrap();
            assert_eq!(2, copy.read(pid).unwrap()[100], "image {i}");
            copy.close();
        });
    }
}
