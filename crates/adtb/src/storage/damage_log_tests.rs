//! The damage the pager records: each detection a record that a crash keeps or the next open finds
//! again, folded by checkpoints into the damage list.

use crate::testing::Probe;
use std::collections::{BTreeMap, BTreeSet};

use crate::mvcc::Transactions;
use crate::runtime::{IoError, OpKind, Runtime};
use crate::storage::{
    self, Damage, FIRST_EDITABLE, Finding, HEADER_LEN, LogFinding, LogProblem, Lsn, Options,
    PageKind, Pager, Pid, Quarantine,
};
use crate::test_world as harness;
use adtb_harness::common;
use adtb_harness::damage::{DataFile, Files};
use adtb_harness::search;
use adtb_sim::{Action, Config, Match, Plan, Sim, SimIo, SimRuntime};
use adtb_testkit::profile::assert_reached;

const PAGER: DataFile = DataFile::new(harness::NAME, harness::PAGE);

const SEED: u64 = 11;
const PAGES: usize = 4;

fn options(atomic: bool) -> Options {
    Options {
        atomic_page_writes: atomic,
        ..harness::options(16)
    }
}

/// Write `value` to `pid`.
fn put<R: Runtime>(pager: &Pager<R>, pid: Pid, value: u8) {
    pager.update(&[pid], |p| p[0][HEADER_LEN] = value).unwrap();
}

/// Returns a closed file of [`PAGES`] data pages, each written once, and them.
fn created(atomic: bool) -> (Files, Vec<Pid>) {
    let mut sim = Sim::new(SEED, Config::default());
    let mut pages = Vec::new();
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(atomic)).unwrap();
        pages = (0..PAGES)
            .map(|_| pager.allocate(1, PageKind::HeapSlotted, 7).unwrap())
            .collect();
        for &pid in &pages {
            put(&pager, pid, 1);
        }
        pager.close();
    });
    (sim.files(), pages)
}

/// Returns `files` after a process opens them, runs `body` and closes the pager.
fn closed(files: &Files, atomic: bool, body: impl FnOnce(&Pager<SimRuntime>)) -> Files {
    let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(atomic)).unwrap();
        body(&pager);
        pager.close();
    });
    sim.files()
}

/// Returns `files` after a process opens them, runs `body` and crashes.
fn crashed(files: &Files, atomic: bool, body: impl FnOnce(&Pager<SimRuntime>)) -> Files {
    let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
    common::run_then_crash(&mut sim, |rt, _| {
        let (pager, _) = Pager::open(rt.clone(), &options(atomic)).unwrap();
        body(&pager);
    });
    sim.files()
}

fn with_page(mut files: Files, pid: Pid, page: &[u8]) -> Files {
    PAGER.replace(&mut files, pid, page);
    files
}

/// Returns `files` with page `pid` failing its checksum.
fn rotted(mut files: Files, pid: Pid) -> Files {
    PAGER.flip(&mut files, pid, harness::PAGE / 2 - HEADER_LEN);
    files
}

/// One `damage` call site: the file it starts from, the page it finds damaged, and the call
/// finding it, `None` when the open does.
struct Site {
    name: &'static str,
    files: Files,
    pid: Pid,
    atomic: bool,
    unreadable: bool,
    detect: Option<fn(&Pager<SimRuntime>, Pid) -> Damage>,
}

fn read_refused(pager: &Pager<SimRuntime>, pid: Pid) -> Damage {
    pager.read(pid).unwrap_err().damage
}

fn verify_refused(pager: &Pager<SimRuntime>, pid: Pid) -> Damage {
    let found = pager.verify().findings;
    let damage = Probe::of(pager).refusals()[&pid];
    assert!(found.contains(&Finding::Page { pid, damage }), "{found:?}");
    damage
}

fn refused_dirty(pager: &Pager<SimRuntime>, pid: Pid) -> Damage {
    put(pager, pid, 2);
    pager.flush_log();
    pager.damage(pid, Damage::Checksum).damage
}

fn sites() -> Vec<Site> {
    let (files, pages) = created(false);
    let (atomic, apages) = created(true);
    let p = pages[1];
    let newer = closed(&files, false, |pager| put(pager, p, 2));
    let bitmap = Pid(2);
    let mut map = PAGER.page(&files, bitmap).to_vec();
    map[FIRST_EDITABLE..HEADER_LEN].copy_from_slice(&storage::header_edit(
        1,
        PageKind::HeapSlotted,
        0,
        0,
    ));
    storage::seal(bitmap, &mut map);
    let mut cut = files.clone();
    PAGER.truncate(&mut cut, pages[2].0 as usize);
    let q = apages[1];
    let unreplayed = crashed(&atomic, true, |pager| {
        put(pager, q, 2);
        pager.flush_log();
    });
    let atomic_newer = closed(&atomic, true, |pager| put(pager, q, 2));
    let lost = crashed(&atomic_newer, true, |pager| {
        put(pager, q, 3);
        pager.flush_log();
    });
    let site = |name, files, pid, atomic, detect| Site {
        name,
        files,
        pid,
        atomic,
        unreadable: false,
        detect,
    };
    vec![
        site(
            "read, checksum",
            rotted(files.clone(), p),
            p,
            false,
            Some(read_refused),
        ),
        site(
            "read, lost write",
            with_page(newer, p, PAGER.page(&files, p)),
            p,
            false,
            Some(read_refused),
        ),
        Site {
            unreadable: true,
            ..site(
                "read, unreadable",
                files.clone(),
                p,
                false,
                Some(read_refused),
            )
        },
        site(
            "open, bitmap page",
            with_page(files.clone(), bitmap, &map),
            bitmap,
            false,
            None,
        ),
        site("open, page past the file's end", cut, pages[3], false, None),
        site("replay, checksum", rotted(unreplayed, q), q, true, None),
        site(
            "replay, lost write",
            with_page(lost, q, PAGER.page(&atomic, q)),
            q,
            true,
            None,
        ),
        site(
            "verify, checksum",
            rotted(files.clone(), p),
            p,
            false,
            Some(verify_refused),
        ),
        site(
            "damage of a dirty page",
            files,
            p,
            false,
            Some(refused_dirty),
        ),
    ]
}

/// Returns the damage `site` finds and what the pager records of its page after the call.
fn detected(site: &Site, rt: &SimRuntime) -> (Option<Damage>, Option<Damage>) {
    if site.unreadable {
        let at = PAGER.range(site.pid).start as u64;
        let reads = Match::kind(OpKind::Read).offset(at);
        rt.plan(Plan::new().rule(reads, Action::Fail(IoError::Failed)));
    }
    let (pager, _) = Pager::open(rt.clone(), &options(site.atomic)).unwrap();
    let damage = match site.detect {
        Some(detect) => detect(&pager, site.pid),
        None => Probe::of(&pager).refusals()[&site.pid],
    };
    let recorded = Probe::of(&pager).records().pages().get(&site.pid).copied();
    (Some(damage).filter(|d| *d != Damage::Unreadable), recorded)
}

#[test]
fn each_of_nine_detections_leaves_a_record_a_crash_keeps_but_an_unreadable_page_none() {
    let mut reached = BTreeSet::new();
    for site in sites() {
        let mut sim = Sim::with_files(SEED, Config::default(), site.files.clone());
        let mut found = None;
        common::run_then_crash(&mut sim, |rt, _| found = Some(detected(&site, rt)));
        let (damage, recorded) = found.unwrap();
        assert_eq!(damage, recorded, "{}: the record of the call", site.name);
        let mut after = Sim::with_files(SEED, Config::default(), sim.files());
        harness::finished(&mut after, |rt| {
            let (pager, _) = Pager::open(rt, &options(site.atomic)).unwrap();
            let listed = Probe::of(&pager).records().pages().get(&site.pid).copied();
            let found = pager.verify().findings;
            let of_page = |f: &&Finding| matches!(f, Finding::Page { pid, .. } if *pid == site.pid);
            let found: Vec<&Finding> = found.iter().filter(of_page).collect();
            let Some(damage) = damage else {
                assert_eq!((None, vec![]), (listed, found), "{}", site.name);
                return;
            };
            let listed = listed.unwrap_or_else(|| panic!("{}: no record after a crash", site.name));
            let kind = |d: &Damage| std::mem::discriminant(d);
            if site.detect.is_some() {
                assert_eq!(damage, listed, "{}: the record after a crash", site.name);
                let want = Finding::Page {
                    pid: site.pid,
                    damage,
                };
                assert!(found.contains(&&want), "{}: {found:?}", site.name);
            } else {
                assert_eq!(kind(&damage), kind(&listed), "{}: found again", site.name);
                let again = |f: &&Finding| matches!(f, Finding::Page { damage: d, .. } if kind(d) == kind(&damage));
                assert!(found.iter().any(again), "{}: {found:?}", site.name);
            }
        });
        reached.insert(site.name);
    }
    assert_eq!(9, reached.len(), "{reached:?}");
}

/// Write `p`, write it to the file, rot it there, evict it and read it. Returns the damage the
/// read refuses it with.
fn rotted_after_its_write(
    pager: &Pager<SimRuntime>,
    rt: &SimRuntime,
    io: &mut SimIo,
    p: Pid,
) -> Option<Damage> {
    put(pager, p, 2);
    pager.flush_log();
    pager.flush_pages(usize::MAX);
    let (file, _) = common::open(rt, io, harness::NAME, false).unwrap();
    let at = PAGER.range(p).start as u64;
    let mut page = common::read(rt, io, file, at, harness::PAGE).unwrap();
    page[harness::PAGE / 2] ^= 0xff;
    common::write(rt, io, file, at, &page).unwrap();
    common::sync(rt, io, file).unwrap();
    for _ in 0..40 {
        let other = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        pager.read(other).unwrap();
    }
    pager.read(p).err().map(|e| e.damage)
}

#[test]
fn rot_under_a_page_the_pool_holds_is_recorded_until_a_change_writes_the_page_out() {
    let (files, pages) = created(true);
    let p = pages[1];
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options(true)).unwrap();
        let write_out = |value| {
            put(&pager, p, value);
            pager.flush_log();
            pager.flush_pages(usize::MAX);
        };
        write_out(2);
        let mut io = rt.io();
        let (file, _) = common::open(&rt, &mut io, harness::NAME, false).unwrap();
        let at = PAGER.range(p).start as u64;
        let mut page = common::read(&rt, &mut io, file, at, harness::PAGE).unwrap();
        page[harness::PAGE / 2] ^= 0xff;
        common::write(&rt, &mut io, file, at, &page).unwrap();
        common::sync(&rt, &mut io, file).unwrap();
        let checked = pager.check_page(p, &pager.read_log(true));
        let rotted = Finding::Page {
            pid: p,
            damage: Damage::Checksum,
        };
        assert_eq!(vec![rotted], checked);
        assert_eq!(
            Some(&Damage::Checksum),
            Probe::of(&pager).records().pages().get(&p)
        );
        assert_eq!(
            2,
            pager.read(p).unwrap()[HEADER_LEN],
            "the pool's copy is served"
        );
        write_out(3);
        assert_eq!(None, Probe::of(&pager).records().pages().get(&p));
        assert_eq!(
            Vec::<Finding>::new(),
            pager.check_page(p, &pager.read_log(true))
        );
        pager.close();
    });
}

#[test]
fn a_checksum_failure_of_a_page_written_since_the_checkpoint_is_listed_after_a_crash() {
    let (files, pages) = created(false);
    let p = pages[1];
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    let mut refused = None;
    common::run_then_crash(&mut sim, |rt, io| {
        let (pager, _) = Pager::open(rt.clone(), &options(false)).unwrap();
        refused = rotted_after_its_write(&pager, rt, io, p);
    });
    assert_eq!(
        Some(Damage::Checksum),
        refused,
        "the read refuses the rotted page"
    );
    let mut sim = Sim::with_files(SEED, Config::default(), sim.files());
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(false)).unwrap();
        assert_eq!(
            Some(&Damage::Checksum),
            Probe::of(&pager).records().pages().get(&p)
        );
        let want = Finding::Page {
            pid: p,
            damage: Damage::Checksum,
        };
        let found = pager.verify().findings;
        assert!(found.contains(&want), "{found:?}");
    });
}

#[test]
fn a_record_forced_before_a_whole_file_mark_is_kept_by_every_later_open_and_after_the_release() {
    let (files, pages) = created(false);
    let p = pages[1];
    let marked = |pager: &Pager<SimRuntime>, rt: &SimRuntime, io: &mut SimIo| {
        let refused = rotted_after_its_write(pager, rt, io, p);
        assert_eq!(Some(Damage::Checksum), refused, "the read refuses the page");
        pager.quarantine(Quarantine::Database).unwrap();
    };
    let mut crash = Sim::with_files(SEED, Config::default(), files.clone());
    common::run_then_crash(&mut crash, |rt, io| {
        let (pager, _) = Pager::open(rt.clone(), &options(false)).unwrap();
        marked(&pager, rt, io);
    });
    let mut close = Sim::with_files(SEED, Config::default(), files);
    common::run(&mut close, |rt, io| {
        let (pager, _) = Pager::open(rt.clone(), &options(false)).unwrap();
        marked(&pager, rt, io);
        pager.close();
    });
    let want = Finding::Page {
        pid: p,
        damage: Damage::Checksum,
    };
    for (what, files) in [("a crash", crash.files()), ("a close", close.files())] {
        let mut sim = Sim::with_files(SEED, Config::default(), files);
        let opened = |sim: &mut Sim, when: &str, release: bool| {
            harness::finished(sim, |rt| {
                let (pager, _) = Pager::open(rt, &options(false)).unwrap();
                let listed = Probe::of(&pager).records().pages().get(&p).copied();
                assert_eq!(Some(Damage::Checksum), listed, "{what}: {when}");
                let found = pager.verify().findings;
                assert!(found.contains(&want), "{what}: {when}: {found:?}");
                if release {
                    pager.release(Quarantine::Database).unwrap();
                }
                pager.close();
            });
        };
        opened(&mut sim, "the first open", false);
        opened(&mut sim, "the second open", true);
        opened(&mut sim, "the open after the release", false);
    }
}

/// Returns the word counting the log findings the newer header copy of the data file holds.
fn header_log_count(files: &Files) -> u64 {
    let data = &files[harness::NAME];
    let copy = |at: usize| {
        let seq = u64::from_le_bytes(data[at + 16..at + 24].try_into().unwrap());
        let count = u64::from_le_bytes(data[at + 1032..at + 1040].try_into().unwrap());
        (seq, count)
    };
    copy(0).max(copy(harness::PAGE)).1
}

/// Returns the options of a file of `page_size` pages.
fn sized(page_size: u32) -> Options {
    Options {
        page_size,
        ..options(false)
    }
}

/// Returns a file of `page_size` pages whose log holds stale blocks past its end: the blocks of a
/// batch writing `wide` pages that another history from the same file wrote past the end of this
/// one, `blocks` of them when given.
fn stale_past_the_end(page_size: u32, wide: usize, blocks: Option<usize>) -> Files {
    let o = sized(page_size);
    let mut sim = Sim::new(SEED, Config::default());
    let mut pages = Vec::new();
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &o).unwrap();
        pages = (0..wide)
            .map(|_| pager.allocate(1, PageKind::HeapSlotted, 7).unwrap())
            .collect();
        pager.close();
    });
    let files = sim.files();
    let history = |body: &dyn Fn(&Pager<SimRuntime>)| {
        let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
        let mut end = None;
        harness::finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt, &o).unwrap();
            body(&pager);
            end = Some(Probe::of(&pager).durable());
        });
        (sim.files(), end.unwrap())
    };
    let (short, short_end) = history(&|pager| {
        for v in 2..4 {
            put(pager, pages[0], v);
            pager.flush_log();
        }
    });
    let (long, long_end) = history(&|pager| {
        pager
            .update(&pages, |pg| {
                for (i, page) in pg.iter_mut().enumerate() {
                    page[HEADER_LEN..].fill(0x5a ^ i as u8);
                }
            })
            .unwrap();
        pager.flush_log();
    });
    let from = short_end.0 as usize;
    let to = blocks.map_or(long_end.0 as usize, |n| from + n * o.log_block as usize);
    assert!(
        short_end < long_end && to <= long_end.0 as usize,
        "the long batch ends at {long_end:?}"
    );
    let segment = format!("{}/{:016x}", o.log_dir(), 0);
    let mut out = short;
    out.get_mut(&segment).unwrap()[from..to].copy_from_slice(&long[&segment][from..to]);
    out
}

#[test]
fn stale_log_blocks_past_the_end_are_reported_after_a_crash_anywhere_in_the_open() {
    let files = stale_past_the_end(harness::PAGE as u32, 2, None);
    let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
    let mut want = Vec::new();
    harness::finished(&mut sim, |rt| {
        let (pager, recovery) = Pager::open(rt, &options(false)).unwrap();
        want = recovery.log;
        let reported: Vec<LogFinding> = pager
            .verify()
            .findings
            .into_iter()
            .filter_map(|f| match f {
                Finding::Log(l) if l.problem == LogProblem::Stale => Some(l),
                _ => None,
            })
            .collect();
        assert_eq!(want, reported, "the open's own verify");
    });
    assert_reached("stale blocks past the end", want.len(), 1);
    let (mut uncleared, mut in_header, mut listed) = (0, 0, 0);
    search::each_crash_image(
        &files,
        SEED,
        12,
        2,
        |rt| {
            rt.arm(Match::any());
            let (pager, _) = Pager::open(rt.clone(), &options(false)).unwrap();
            pager.close();
            Vec::new()
        },
        |at, image| {
            let count = header_log_count(&image);
            let mut sim = Sim::with_files(SEED, Config::default(), image);
            harness::finished(&mut sim, |rt| {
                let (pager, recovery) = Pager::open(rt, &options(false)).unwrap();
                match (recovery.log.is_empty(), count) {
                    (false, _) => uncleared += 1,
                    (true, 0) => listed += 1,
                    (true, _) => in_header += 1,
                }
                let reported: Vec<LogFinding> = pager
                    .verify()
                    .findings
                    .into_iter()
                    .filter_map(|f| match f {
                        Finding::Log(l) if l.problem == LogProblem::Stale => Some(l),
                        _ => None,
                    })
                    .collect();
                assert_eq!(want, reported, "crash at event {at}");
            });
        },
    );
    assert_reached("a crash before the log's tail is cleared", uncleared, 1);
    assert_reached("a crash with the findings in the file header", in_header, 1);
    assert_reached("a crash with the findings in the damage list", listed, 1);
}

#[test]
fn page_damage_recovery_finds_is_reported_after_a_crash_anywhere_in_the_open() {
    let (atomic, pages) = created(true);
    let q = pages[1];
    let unreplayed = crashed(&atomic, true, |pager| {
        put(pager, q, 2);
        pager.flush_log();
    });
    let files = rotted(unreplayed, q);
    let want = Finding::Page {
        pid: q,
        damage: Damage::Checksum,
    };
    let mut images = 0;
    search::each_crash_image(
        &files,
        SEED,
        12,
        2,
        |rt| {
            rt.arm(Match::any());
            let (pager, _) = Pager::open(rt.clone(), &options(true)).unwrap();
            pager.close();
            Vec::new()
        },
        |at, image| {
            images += 1;
            let mut sim = Sim::with_files(SEED, Config::default(), image);
            harness::finished(&mut sim, |rt| {
                let (pager, _) = Pager::open(rt, &options(true)).unwrap();
                let findings = pager.verify().findings;
                assert!(
                    findings.contains(&want),
                    "crash at event {at}: {findings:?}"
                );
                assert_eq!(
                    Some(&Damage::Checksum),
                    Probe::of(&pager).records().pages().get(&q),
                    "crash at event {at}"
                );
            });
        },
    );
    assert_reached("crash images of the open", images, 3);
}

/// Returns a file whose one status page lost the writes after its first, the commit log's root and
/// the status page.
fn lost_status_page() -> (Files, Pid, Pid) {
    let options = harness::options(6);
    let mut sim = Sim::new(SEED, Config::default());
    let mut out = None;
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt.clone(), &options).unwrap();
        let txns = Transactions::create(rt, &pager).unwrap();
        let x = txns.begin(&pager).unwrap();
        txns.commit(&pager, x).unwrap();
        let clog = txns.commit_log();
        out = Some((clog.root(), clog.status_pages()[&0]));
        drop(clog);
        pager.close();
    });
    let (root, status) = out.unwrap();
    let before = sim.files();
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        let txns = Transactions::open(rt, &pager, root).unwrap();
        for _ in 0..3 {
            let x = txns.begin(&pager).unwrap();
            txns.commit(&pager, x).unwrap();
        }
        txns.begin(&pager).unwrap();
        pager.close();
    });
    let files = with_page(sim.files(), status, PAGER.page(&before, status));
    (files, root, status)
}

/// Run an open of `files` that opens the commit log rooted at `root`, then `then`, and a close.
/// Returns what `then` returns, the marks and the files the close leaves.
fn with_commit_log<T>(
    files: Files,
    root: Pid,
    then: impl FnOnce(&Pager<SimRuntime>) -> T,
) -> (T, BTreeSet<Quarantine>, Files) {
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    let mut out = None;
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &harness::options(6)).unwrap();
        Transactions::open(rt, &pager, root).unwrap();
        let got = then(&pager);
        out = Some((got, Probe::of(&pager).marks()));
        pager.close();
    });
    let (got, marks) = out.unwrap();
    (got, marks, sim.files())
}

/// Returns what `verify` finds of page `pid`.
fn findings_of(pager: &Pager<SimRuntime>, pid: Pid) -> Vec<Finding> {
    let found = pager.verify().findings.into_iter();
    found
        .filter(|f| matches!(f, Finding::Page { pid: p, .. } if *p == pid))
        .collect()
}

#[test]
fn a_settled_status_page_is_a_finding_after_reopen_and_after_its_segment_is_recycled() {
    let (files, root, status) = lost_status_page();
    let (recorded, marks, files) = with_commit_log(files, root, |p| {
        Probe::of(p).records().pages().get(&status).copied()
    });
    let Some(damage @ Damage::Settled { .. }) = recorded else {
        panic!("the open settles the status page: {recorded:?}");
    };
    assert_eq!(BTreeSet::new(), marks, "a settle marks nothing");
    let settled = Finding::Page {
        pid: status,
        damage,
    };
    let segment = harness::options(6).segment_size;
    let (again, marks, files) = with_commit_log(files, root, |pager| {
        let found = findings_of(pager, status);
        let scratch = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        while Probe::of(pager).appended().0 <= 2 * segment {
            put(pager, scratch, (Probe::of(pager).appended().0 % 251) as u8);
            pager.flush_log();
        }
        found
    });
    assert!(again.contains(&settled), "after reopen: {again:?}");
    assert_eq!(BTreeSet::new(), marks, "after reopen");
    let (recycled, _, files) = with_commit_log(files, root, Pager::recycle_log);
    assert_reached("segments recycled", recycled, 1);
    let first_segment = format!("{}/{:016x}", harness::options(6).log_dir(), 0);
    assert!(
        !files.contains_key(&first_segment),
        "the settle's segment is recycled"
    );
    let (found, marks, _) = with_commit_log(files, root, |p| findings_of(p, status));
    assert_eq!(
        (vec![settled], BTreeSet::new()),
        (found, marks),
        "after the recycle"
    );
}

#[test]
fn a_settle_in_the_open_that_reads_the_page_stale_logs_no_stale_record() {
    let (files, root, status) = lost_status_page();
    let (recorded, _, files) = with_commit_log(files, root, |p| {
        Probe::of(p).records().pages().get(&status).copied()
    });
    let Some(settled @ Damage::Settled { .. }) = recorded else {
        panic!("the open settles the status page: {recorded:?}");
    };
    let (logged, marks, _) = with_commit_log(files, root, |p| Probe::of(p).damage_in_log());
    assert_eq!(
        (vec![(status, settled)], BTreeSet::new()),
        (logged, marks),
        "the settle's only record, and no mark"
    );
}

#[test]
fn a_status_page_settled_during_its_walk_marks_nothing() {
    let (files, root, status) = lost_status_page();
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &harness::options(6)).unwrap();
        let Damage::Stale { expected, found } = read_refused(&pager, status) else {
            panic!("the status page lost a write");
        };
        let mut walks = 0;
        let failed = pager.map_damage(
            |_| {
                walks += 1;
                Transactions::open(rt.clone(), &pager, root).unwrap();
                Some(BTreeSet::from([Quarantine::Database]))
            },
            |o| pager.quarantine(o),
        );
        assert_eq!(BTreeMap::new(), failed);
        assert_reached("walks of the stale record", walks, 1);
        assert_eq!(
            (BTreeSet::new(), Some(&Damage::Settled { expected, found })),
            (
                Probe::of(&pager).marks(),
                Probe::of(&pager).records().pages().get(&status)
            ),
            "the settle supersedes the record its walk maps"
        );
    });
}

#[test]
fn a_record_superseded_during_its_walk_marks_nothing_and_its_successor_maps() {
    let (files, pages) = created(false);
    let p = pages[0];
    let stale = Damage::Stale {
        expected: Lsn(90),
        found: Lsn(30),
    };
    closed(&files, false, |pager| {
        pager.damage(p, Damage::Checksum);
        let failed = pager.map_damage(
            |pid| {
                pager.damage(pid, stale);
                Some(BTreeSet::from([Quarantine::Heap(1)]))
            },
            |o| pager.quarantine(o),
        );
        assert_eq!(BTreeMap::new(), failed);
        assert_eq!(
            (BTreeSet::new(), None),
            (Probe::of(pager).marks(), pager.mapped_to(p)),
            "the superseded record"
        );
        let failed = pager.map_damage(
            |_| Some(BTreeSet::from([Quarantine::Heap(2)])),
            |o| pager.quarantine(o),
        );
        assert_eq!(BTreeMap::new(), failed);
        let successor = BTreeSet::from([Quarantine::Heap(2)]);
        assert_eq!(
            (successor.clone(), Some(successor)),
            (Probe::of(pager).marks(), pager.mapped_to(p)),
            "the successor maps on its own"
        );
    });
}

/// Returns the files after damaging `n` distinct pages of a new file, a checkpoint and a close.
fn damaged_pages(n: usize) -> (Files, usize) {
    let mut sim = Sim::new(SEED, Config::default());
    let mut capacity = 0;
    harness::finished(&mut sim, |rt| {
        let pager = Pager::create(rt, &options(false)).unwrap();
        capacity = pager.damage_list_capacity();
        let first = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        for i in 0..n {
            pager.damage(Pid(first.0 + i as u64), Damage::Checksum);
        }
        pager.close();
    });
    (sim.files(), capacity)
}

#[test]
fn one_damaged_page_past_the_lists_capacity_records_the_whole_file() {
    let (_, capacity) = damaged_pages(0);
    assert_reached("list capacity", capacity, 2);
    for (n, full) in [(capacity, false), (capacity + 1, true)] {
        let (files, _) = damaged_pages(n);
        let mut sim = Sim::with_files(SEED, Config::default(), files);
        harness::finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt, &options(false)).unwrap();
            let records = Probe::of(&pager).records();
            assert_eq!(
                (full, n.min(capacity)),
                (records.full(), records.pages().len()),
                "{n} pages"
            );
            let unlisted = pager.verify().findings.contains(&Finding::Unlisted);
            assert_eq!(full, unlisted, "{n} pages");
            pager.map_damage(|_| Some(BTreeSet::new()), |o| pager.quarantine(o));
            let database = BTreeSet::from([Quarantine::Database]);
            let marks = if full { database } else { BTreeSet::new() };
            assert_eq!(marks, Probe::of(&pager).marks(), "{n} pages");
        });
    }
}

/// Returns `pager`'s marks after its records map through a walk that finds every page in the
/// database, and how many pages it walked for.
fn mapped_whole(pager: &Pager<SimRuntime>) -> (BTreeSet<Quarantine>, usize) {
    let mut walked = 0;
    let failed = pager.map_damage(
        |_| {
            walked += 1;
            Some(BTreeSet::from([Quarantine::Database]))
        },
        |o| pager.quarantine(o),
    );
    assert_eq!(BTreeMap::new(), failed);
    (Probe::of(pager).marks(), walked)
}

#[test]
fn a_settled_status_page_and_stale_log_blocks_past_the_end_map_to_no_mark() {
    let (files, root, status) = lost_status_page();
    let ((recorded, mapped), _, _) = with_commit_log(files, root, |p| {
        (
            Probe::of(p).records().pages().get(&status).copied(),
            mapped_whole(p),
        )
    });
    assert!(
        matches!(recorded, Some(Damage::Settled { .. })),
        "{recorded:?}"
    );
    assert_eq!((BTreeSet::new(), 0), mapped, "a settled page");
    let files = stale_past_the_end(harness::PAGE as u32, 2, None);
    let mut sim = Sim::with_files(SEED, Config::default(), files);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(false)).unwrap();
        assert_reached(
            "stale blocks past the end",
            Probe::of(&pager).records().log().len(),
            1,
        );
        assert_eq!((BTreeSet::new(), 0), mapped_whole(&pager), "stale blocks");
    });
}

#[test]
fn a_file_marked_whole_writes_no_record_and_keeps_its_records_in_memory() {
    let (files, pages) = created(false);
    let p = pages[0];
    let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
    let mut list = Pid(0);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(false)).unwrap();
        list = pager.damage_list_page();
        pager.quarantine(Quarantine::Database).unwrap();
        let end = Probe::of(&pager).appended();
        pager.damage(p, Damage::Checksum);
        assert_eq!(
            end,
            Probe::of(&pager).appended(),
            "the record is not logged"
        );
        assert_eq!(
            Some(&Damage::Checksum),
            Probe::of(&pager).records().pages().get(&p),
            "the record is in memory"
        );
        pager.close();
    });
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(false)).unwrap();
        assert_eq!(
            None,
            Probe::of(&pager).records().pages().get(&p),
            "the next open"
        );
        pager.close();
    });
    assert_eq!(
        PAGER.page(&files, list),
        PAGER.page(&sim.files(), list),
        "the list page is unchanged"
    );
}

/// Returns the stale log findings and whether the list is full, as `verify` of `pager` reports.
fn stale_reported(pager: &Pager<SimRuntime>) -> (Vec<LogFinding>, bool) {
    let found = pager.verify().findings;
    let stale = found.iter().filter_map(|f| match f {
        Finding::Log(l) if l.problem == LogProblem::Stale => Some(*l),
        _ => None,
    });
    (stale.collect(), found.contains(&Finding::Unlisted))
}

#[test]
fn one_stale_log_block_past_what_the_header_holds_records_the_whole_file() {
    let capacity = 62;
    let files = stale_past_the_end(16384, 5, Some(capacity + 1));
    let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
    let mut want = Vec::new();
    harness::finished(&mut sim, |rt| {
        let (pager, recovery) = Pager::open(rt, &sized(16384)).unwrap();
        assert_eq!(capacity + 1, recovery.log.len(), "{:?}", recovery.log);
        want = recovery.log[..capacity].to_vec();
        assert_eq!((want.clone(), true), stale_reported(&pager), "the open");
    });
    let mut images = 0;
    search::each_crash_image(
        &files,
        SEED,
        12,
        2,
        |rt| {
            rt.arm(Match::any());
            let (pager, _) = Pager::open(rt.clone(), &sized(16384)).unwrap();
            pager.close();
            Vec::new()
        },
        |at, image| {
            images += 1;
            let mut sim = Sim::with_files(SEED, Config::default(), image);
            harness::finished(&mut sim, |rt| {
                let (pager, _) = Pager::open(rt, &sized(16384)).unwrap();
                let got = stale_reported(&pager);
                assert_eq!((want.clone(), true), got, "crash at event {at}");
            });
        },
    );
    assert_reached("crash images of the open", images, 3);
}

#[test]
fn a_damaged_page_freed_or_freed_and_allocated_again_is_reported_by_nothing() {
    let (files, pages) = created(false);
    let p = pages[1];
    let listed = closed(&files, false, |pager| {
        pager.damage(p, Damage::Checksum);
    });
    let unlisted = crashed(&files, false, |pager| {
        pager.damage(p, Damage::Checksum);
    });
    for (what, files) in [("listed", listed), ("in the log", unlisted)] {
        let check = |pager: &Pager<SimRuntime>, when: &str| {
            assert_eq!(
                None,
                Probe::of(pager).records().pages().get(&p),
                "{what}: {when}"
            );
            let found = pager.verify().findings;
            let of_p = |f: &&Finding| matches!(f, Finding::Page { pid, .. } if *pid == p);
            let found: Vec<&Finding> = found.iter().filter(of_p).collect();
            assert_eq!(Vec::<&Finding>::new(), found, "{what}: {when}");
        };
        let freed = |pager: &Pager<SimRuntime>| {
            assert_eq!(
                Some(&Damage::Checksum),
                Probe::of(pager).records().pages().get(&p),
                "{what}: before the free"
            );
            pager.free(p).unwrap();
            pager.flush_log();
            check(pager, "after the free");
        };
        let only_freed = crashed(&files, false, freed);
        let allocated = crashed(&files, false, |pager| {
            freed(pager);
            assert_eq!(Ok(p), pager.allocate(1, PageKind::HeapSlotted, 7), "{what}");
            put(pager, p, 9);
            pager.flush_log();
            check(pager, "after the allocation");
        });
        for (when, after) in [("the free", only_freed), ("the allocation", allocated)] {
            let mut sim = Sim::with_files(SEED, Config::default(), after);
            harness::finished(&mut sim, |rt| {
                let (pager, _) = Pager::open(rt, &options(false)).unwrap();
                check(&pager, &format!("a crash after {when}"));
            });
        }
    }
}

#[test]
fn a_full_page_image_logged_after_the_record_drops_it() {
    let (files, pages) = created(false);
    let p = pages[1];
    let planted = closed(&files, false, |pager| {
        pager.damage(p, Damage::Checksum);
    });
    let after = crashed(&planted, false, |pager| {
        assert_eq!(
            Some(&Damage::Checksum),
            Probe::of(pager).records().pages().get(&p)
        );
        put(pager, p, 9);
        pager.flush_log();
        assert_eq!(
            None,
            Probe::of(pager).records().pages().get(&p),
            "after the image"
        );
    });
    let mut sim = Sim::with_files(SEED, Config::default(), after);
    harness::finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options(false)).unwrap();
        assert_eq!(
            None,
            Probe::of(&pager).records().pages().get(&p),
            "after a crash"
        );
    });
}

#[test]
fn the_damage_of_a_page_freed_since_maps_to_nothing_with_no_walk() {
    let (files, pages) = created(false);
    let p = pages[0];
    closed(&files, false, |pager| {
        pager.damage(p, Damage::Checksum);
        pager.free(p).unwrap();
        assert_eq!(
            Some(&Damage::Checksum),
            Probe::of(pager).records().pages().get(&p),
            "held as the free copy's damage"
        );
        assert_eq!((BTreeSet::new(), 0), mapped_whole(pager));
        assert_eq!(Some(BTreeSet::new()), pager.mapped_to(p));
    });
}
