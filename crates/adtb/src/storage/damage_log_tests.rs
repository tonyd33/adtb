//! The damage the pager records: each detection a record that a crash keeps or the next open finds
//! again, folded by checkpoints into the damage list.

use crate::testing::Probe;
use std::collections::{BTreeMap, BTreeSet};

use crate::mvcc::Transactions;
use crate::runtime::{IoError, OpKind, Runtime};
use crate::storage::order::ordered_for_test;
use crate::storage::policy::Holder;
use crate::storage::{
    self, Damage, FIRST_EDITABLE, Finding, HEADER_LEN, LogFinding, LogProblem, Lsn, Options,
    PageKind, Pager, Pid, Quarantine, Shape, Undecoded,
};
use crate::test_world as harness;
use adtb_sim::{Action, Config, Match, Plan, Sim, SimIo, SimRuntime};
use adtb_testkit::common;
use adtb_testkit::damage::{DataFile, Files};
use adtb_testkit::profile::assert_reached;
use adtb_testkit::search;

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

/// A call that finds a page damaged and returns the damage.
type Detect = Box<dyn Fn(&Pager<SimRuntime>, &SimRuntime, Pid) -> Damage>;

/// One `damage` call site: the file it starts from, the page it finds damaged, and the call
/// finding it, `None` when the open does.
struct Site {
    name: String,
    files: Files,
    pid: Pid,
    atomic: bool,
    unreadable: bool,
    detect: Option<Detect>,
}

fn read_refused(pager: &Pager<SimRuntime>, pid: Pid) -> Damage {
    pager.read(pid).unwrap_err().damage
}

fn verify_refused(pager: &Pager<SimRuntime>, _: &SimRuntime, pid: Pid) -> Damage {
    let found = pager.verify().findings;
    let damage = Probe::of(pager).refusals()[&pid];
    assert!(found.contains(&Finding::Page { pid, damage }), "{found:?}");
    damage
}

fn refused_dirty(pager: &Pager<SimRuntime>, _: &SimRuntime, pid: Pid) -> Damage {
    put(pager, pid, 2);
    pager.flush_log();
    pager.damage(pid, Damage::Checksum).damage
}

fn rotted_since_the_checkpoint(pager: &Pager<SimRuntime>, rt: &SimRuntime, pid: Pid) -> Damage {
    rotted_after_its_write(pager, rt, &mut rt.io(), pid).expect("the read refuses the page")
}

fn refused_past_the_end(pager: &Pager<SimRuntime>, _: &SimRuntime, pid: Pid) -> Damage {
    pager.damage(pid, Damage::Checksum).damage
}

const SHAPES: [Shape; 12] = [
    Shape::Blank,
    Shape::Kind,
    Shape::Owner,
    Shape::Frame,
    Shape::Format,
    Shape::Row(Undecoded::Tag),
    Shape::Row(Undecoded::Cut),
    Shape::Row(Undecoded::Trailing),
    Shape::Row(Undecoded::Text),
    Shape::Row(Undecoded::Pointer),
    Shape::Stray,
    Shape::Row(Undecoded::Domain),
];

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
    let site = |name: &str, files, pid, atomic, detect: Option<Detect>| Site {
        name: name.to_owned(),
        files,
        pid,
        atomic,
        unreadable: false,
        detect,
    };
    let mut sites = vec![
        site(
            "read, checksum",
            rotted(files.clone(), p),
            p,
            false,
            Some(Box::new(|pager, _, pid| read_refused(pager, pid))),
        ),
        site(
            "read, lost write",
            with_page(newer, p, PAGER.page(&files, p)),
            p,
            false,
            Some(Box::new(|pager, _, pid| read_refused(pager, pid))),
        ),
        Site {
            unreadable: true,
            ..site(
                "read, unreadable",
                files.clone(),
                p,
                false,
                Some(Box::new(|pager, _, pid| read_refused(pager, pid))),
            )
        },
        site(
            "read, checksum of a page written since the checkpoint",
            files.clone(),
            p,
            false,
            Some(Box::new(rotted_since_the_checkpoint)),
        ),
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
            Some(Box::new(verify_refused)),
        ),
        site(
            "damage of a dirty page",
            files.clone(),
            p,
            false,
            Some(Box::new(refused_dirty)),
        ),
        site(
            "damage of a page past the file and the log",
            files.clone(),
            Pid(1000),
            false,
            Some(Box::new(refused_past_the_end)),
        ),
    ];
    for shape in SHAPES {
        let detect: Detect =
            Box::new(move |pager, _, pid| pager.damage(pid, Damage::Shape(shape)).damage);
        sites.push(site(
            &format!("damage, {shape:?}"),
            files.clone(),
            p,
            false,
            Some(detect),
        ));
    }
    sites
}

/// Returns the damage `site` finds and what the pager records of its page after the call,
/// closing the pager after when `close`.
fn detected(site: &Site, rt: &SimRuntime, close: bool) -> (Option<Damage>, Option<Damage>) {
    if site.unreadable {
        let at = PAGER.range(site.pid).start as u64;
        let reads = Match::kind(OpKind::Read).offset(at);
        rt.plan(Plan::new().rule(reads, Action::Fail(IoError::Failed)));
    }
    let (pager, _) = Pager::open(rt.clone(), &options(site.atomic)).unwrap();
    let damage = match &site.detect {
        Some(detect) => detect(&pager, rt, site.pid),
        None => Probe::of(&pager).refusals()[&site.pid],
    };
    let recorded = Probe::of(&pager).records().pages().get(&site.pid).copied();
    if close {
        pager.close();
    }
    (Some(damage).filter(|d| *d != Damage::Unreadable), recorded)
}

/// Space: every call site that refuses a page as damaged, each shape among them, each ended by a
/// crash, which the log keeps the record through, and by a close, which the list keeps it
/// through.
#[test]
fn each_detection_leaves_a_record_a_crash_and_a_close_keep_but_an_unreadable_page_none() {
    let mut reached = BTreeSet::new();
    for site in sites() {
        for close in [false, true] {
            let name = format!(
                "{} then {}",
                site.name,
                if close { "a close" } else { "a crash" }
            );
            let mut sim = Sim::with_files(SEED, Config::default(), site.files.clone());
            let mut found = None;
            if close {
                harness::finished(&mut sim, |rt| found = Some(detected(&site, &rt, true)));
            } else {
                common::run_then_crash(&mut sim, |rt, _| found = Some(detected(&site, rt, false)));
            }
            let (damage, recorded) = found.unwrap();
            assert_eq!(damage, recorded, "{name}: the record of the call");
            let mut after = Sim::with_files(SEED, Config::default(), sim.files());
            harness::finished(&mut after, |rt| {
                let (pager, _) = Pager::open(rt, &options(site.atomic)).unwrap();
                let listed = Probe::of(&pager).records().pages().get(&site.pid).copied();
                let found = pager.verify().findings;
                let of_page =
                    |f: &&Finding| matches!(f, Finding::Page { pid, .. } if *pid == site.pid);
                let found: Vec<&Finding> = found.iter().filter(of_page).collect();
                let Some(damage) = damage else {
                    assert_eq!((None, vec![]), (listed, found), "{name}");
                    return;
                };
                let listed = listed.unwrap_or_else(|| panic!("{name}: no record after"));
                let kind = |d: &Damage| std::mem::discriminant(d);
                if site.detect.is_some() {
                    assert_eq!(damage, listed, "{name}: the record after");
                    let want = Finding::Page {
                        pid: site.pid,
                        damage,
                    };
                    assert!(found.contains(&&want), "{name}: {found:?}");
                } else {
                    assert_eq!(kind(&damage), kind(&listed), "{name}: found again");
                    let again = |f: &&Finding| matches!(f, Finding::Page { damage: d, .. } if kind(d) == kind(&damage));
                    assert!(found.iter().any(again), "{name}: {found:?}");
                }
            });
            reached.insert(name);
        }
    }
    assert_eq!(2 * 23, reached.len(), "{reached:?}");
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
fn a_record_forced_before_a_whole_file_mark_is_kept_by_every_later_open_and_after_the_release() {
    let (files, pages) = created(false);
    let p = pages[1];
    let marked = |pager: &Pager<SimRuntime>, rt: &SimRuntime, io: &mut SimIo| {
        let refused = rotted_after_its_write(pager, rt, io, p);
        assert_eq!(Some(Damage::Checksum), refused, "the read refuses the page");
        pager
            .quarantine(Quarantine::Database, &ordered_for_test())
            .unwrap();
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

/// What the open of a file reports: the stale log blocks and whether the file is unlisted, by
/// `verify`, and the page findings and damage record of page `pid`.
type Reported = (Vec<LogFinding>, bool, Vec<Finding>, Option<Damage>);

fn reported(pager: &Pager<SimRuntime>, pid: Pid) -> Reported {
    let found = pager.verify().findings;
    let stale = found.iter().filter_map(|f| match f {
        Finding::Log(l) if l.problem == LogProblem::Stale => Some(*l),
        _ => None,
    });
    let of_page = found
        .iter()
        .filter(|f| matches!(f, Finding::Page { pid: p, .. } if *p == pid));
    (
        stale.collect(),
        found.contains(&Finding::Unlisted),
        of_page.copied().collect(),
        Probe::of(pager).records().pages().get(&pid).copied(),
    )
}

/// Returns what an open reports given the stale log blocks its recovery found.
type Wanted = Box<dyn Fn(&[LogFinding]) -> Reported>;

/// Space: a crash at every point of an open that finds stale log blocks past the end of the log,
/// more of them than the file header holds, or page damage replay finds.
#[test]
fn what_an_open_finds_is_reported_after_a_crash_anywhere_in_the_open() {
    let capacity = 62;
    let (atomic, pages) = created(true);
    let q = pages[1];
    let unreplayed = crashed(&atomic, true, |pager| {
        put(pager, q, 2);
        pager.flush_log();
    });
    let rotted_page = Finding::Page {
        pid: q,
        damage: Damage::Checksum,
    };
    let rows: [(&str, Files, Options, Wanted); 3] = [
        (
            "stale blocks",
            stale_past_the_end(harness::PAGE as u32, 2, None),
            options(false),
            Box::new(|log| (log.to_vec(), false, Vec::new(), None)),
        ),
        (
            "more stale blocks than the header holds",
            stale_past_the_end(16384, 5, Some(capacity + 1)),
            sized(16384),
            Box::new(move |log| {
                assert_eq!(capacity + 1, log.len(), "{log:?}");
                (log[..capacity].to_vec(), true, Vec::new(), None)
            }),
        ),
        (
            "page damage replay finds",
            rotted(unreplayed, q),
            options(true),
            Box::new(move |_| (Vec::new(), false, vec![rotted_page], Some(Damage::Checksum))),
        ),
    ];
    for (name, files, o, wanted) in rows {
        let mut sim = Sim::with_files(SEED, Config::default(), files.clone());
        let mut want = None;
        harness::finished(&mut sim, |rt| {
            let (pager, opening) = Pager::open(rt, &o).unwrap();
            let recovery = opening.recovery().cloned().expect("the open recovered");
            let w = wanted(&recovery.log);
            assert_eq!(w, reported(&pager, q), "{name}: the open");
            want = Some(w);
        });
        let want = want.unwrap();
        let stale = !want.0.is_empty();
        let (mut uncleared, mut in_header, mut listed) = (0, 0, 0);
        search::each_crash_image(
            &files,
            SEED,
            12,
            2,
            |rt| {
                rt.arm(Match::any());
                let (pager, _) = Pager::open(rt.clone(), &o).unwrap();
                pager.close();
                Vec::new()
            },
            |at, image| {
                let count = header_log_count(&image);
                let mut sim = Sim::with_files(SEED, Config::default(), image);
                harness::finished(&mut sim, |rt| {
                    let (pager, opening) = Pager::open(rt, &o).unwrap();
                    let recovery = opening.recovery().cloned().expect("the open recovered");
                    match (recovery.log.is_empty(), count) {
                        (false, _) => uncleared += 1,
                        (true, 0) => listed += 1,
                        (true, _) => in_header += 1,
                    }
                    assert_eq!(want, reported(&pager, q), "{name}: crash at event {at}");
                });
            },
        );
        let images = uncleared + in_header + listed;
        assert_reached(&format!("{name}: crash images of the open"), images, 3);
        if stale {
            assert_reached(
                &format!("{name}: before the log's tail is cleared"),
                uncleared,
                1,
            );
            assert_reached(
                &format!("{name}: with the findings in the header"),
                in_header,
                1,
            );
            assert_reached(&format!("{name}: with the findings in the list"), listed, 1);
        }
    }
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
        let begun = txns.begin(&pager).unwrap();
        txns.commit(&pager, begun).unwrap();
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
            let begun = txns.begin(&pager).unwrap();
            txns.commit(&pager, begun).unwrap();
        }
        std::mem::forget(txns.begin(&pager).unwrap());
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

/// What changes a damage record while the mapper walks its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Supersede {
    /// The commit log settles the stale status page the record names.
    Settle,
    /// The page is found damaged again, otherwise.
    Damage,
    Free,
}

/// Space: a record superseded during its walk by a settle of its status page, a new damage of its
/// page or a free of it; then a second mapping of what replaced it.
#[test]
fn a_record_superseded_during_its_walk_marks_nothing_and_what_replaced_it_maps_on_its_own() {
    let mut got = Vec::new();
    let mut want = Vec::new();
    for supersede in [Supersede::Settle, Supersede::Damage, Supersede::Free] {
        let (files, root, pid, o) = match supersede {
            Supersede::Settle => {
                let (files, root, status) = lost_status_page();
                (files, root, status, harness::options(6))
            }
            Supersede::Damage | Supersede::Free => {
                let (files, pages) = created(false);
                (files, Pid(0), pages[0], options(false))
            }
        };
        let mut sim = Sim::with_files(SEED, Config::default(), files);
        harness::finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt.clone(), &o).unwrap();
            let first = match supersede {
                Supersede::Settle => read_refused(&pager, pid),
                Supersede::Damage | Supersede::Free => pager.damage(pid, Damage::Checksum).damage,
            };
            let stale = Damage::Stale {
                expected: Lsn(90),
                found: Lsn(30),
            };
            let mut walks = 0;
            let failed = pager.map_damage(
                |walked| {
                    walks += 1;
                    match supersede {
                        Supersede::Settle => {
                            drop(Transactions::open(rt.clone(), &pager, root).unwrap());
                        }
                        Supersede::Damage => drop(pager.damage(walked, stale)),
                        Supersede::Free => drop(pager.free(walked).unwrap()),
                    }
                    let holder = match supersede {
                        Supersede::Settle => Holder::CommitLog,
                        Supersede::Damage | Supersede::Free => heap(1),
                    };
                    Some(BTreeSet::from([holder]))
                },
                &ordered_for_test(),
            );
            let superseded = (
                failed,
                walks,
                Probe::of(&pager).marks(),
                pager.mapped_to(pid),
                Probe::of(&pager).records().pages().get(&pid).copied(),
            );
            let failed = pager.map_damage(|_| Some(BTreeSet::from([heap(2)])), &ordered_for_test());
            let replaced = (failed, Probe::of(&pager).marks(), pager.mapped_to(pid));
            got.push((supersede, superseded, replaced));
            let none = BTreeMap::new();
            let empty = BTreeSet::new();
            let heap2 = BTreeSet::from([Quarantine::Heap(2)]);
            want.push(match (supersede, first) {
                (Supersede::Settle, Damage::Stale { expected, found }) => (
                    supersede,
                    (
                        none.clone(),
                        1,
                        empty.clone(),
                        None,
                        Some(Damage::Settled { expected, found }),
                    ),
                    (none, empty, None),
                ),
                (Supersede::Damage, _) => (
                    supersede,
                    (none.clone(), 1, empty, None, Some(stale)),
                    (none, heap2.clone(), Some(heap2)),
                ),
                (Supersede::Free, _) => (
                    supersede,
                    (
                        none.clone(),
                        1,
                        empty.clone(),
                        Some(empty.clone()),
                        Some(Damage::Checksum),
                    ),
                    (none, empty.clone(), Some(empty)),
                ),
                (_, first) => panic!("the status page lost a write; read {first:?}"),
            });
        });
    }
    assert_eq!(3, got.len());
    assert_eq!(want, got);
}

fn heap(id: u32) -> Holder {
    Holder::Relation {
        object: Quarantine::Heap(id),
        engine_module: false,
    }
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
            pager.map_damage(|_| Some(BTreeSet::new()), &ordered_for_test());
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
            Some(BTreeSet::from([Holder::CommitLog]))
        },
        &ordered_for_test(),
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
        pager
            .quarantine(Quarantine::Database, &ordered_for_test())
            .unwrap();
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

/// What ends a damage record of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Removal {
    /// A change of the page, logged as a full-page image.
    Image,
    Free,
    /// A free and an allocation of the page again, which changes it.
    Reallocate,
}

/// Space: a page's damage record listed by a close, held in the log by a crash, or made in the
/// same session, then removed by a free or a free and an allocation, or by a full-page image of
/// the listed page, which no read refuses; the session ended by a crash or a close.
#[test]
fn an_image_or_a_free_of_a_damaged_page_removes_its_record_unless_the_free_copy_holds_it() {
    let (files, pages) = created(false);
    let p = pages[1];
    let listed = closed(&files, false, |pager| {
        pager.damage(p, Damage::Checksum);
    });
    let in_log = crashed(&files, false, |pager| {
        pager.damage(p, Damage::Checksum);
    });
    let mut got = Vec::new();
    let mut want = Vec::new();
    for (origin, files) in [
        ("listed", &listed),
        ("in the log", &in_log),
        ("this session", &files),
    ] {
        let image = (origin == "listed").then_some(Removal::Image);
        for removal in image
            .into_iter()
            .chain([Removal::Free, Removal::Reallocate])
        {
            for close in [false, true] {
                let mut in_session = None;
                let remove = |pager: &Pager<SimRuntime>| {
                    if origin == "this session" {
                        pager.damage(p, Damage::Checksum);
                    }
                    let before = Probe::of(pager).records().pages().get(&p).copied();
                    match removal {
                        Removal::Image => put(pager, p, 9),
                        Removal::Free => {
                            pager.free(p).unwrap();
                        }
                        Removal::Reallocate => {
                            pager.free(p).unwrap();
                            assert_eq!(Ok(p), pager.allocate(1, PageKind::HeapSlotted, 7));
                            put(pager, p, 9);
                        }
                    }
                    pager.flush_log();
                    let after = Probe::of(pager).records().pages().get(&p).copied();
                    let mapped = pager.mapped_to(p);
                    in_session = Some((before, after, mapped, mapped_whole(pager)));
                };
                let after = if close {
                    closed(files, false, remove)
                } else {
                    crashed(files, false, remove)
                };
                let mut sim = Sim::with_files(SEED, Config::default(), after);
                let mut reopened = None;
                harness::finished(&mut sim, |rt| {
                    let (pager, _) = Pager::open(rt, &options(false)).unwrap();
                    let listed = Probe::of(&pager).records().pages().get(&p).copied();
                    let found = pager.verify().findings;
                    let of_p = |f: &&Finding| matches!(f, Finding::Page { pid, .. } if *pid == p);
                    let found: Vec<Finding> = found.iter().filter(of_p).copied().collect();
                    reopened = Some((listed, found));
                });
                got.push((origin, removal, close, in_session, reopened));
                let held = origin == "this session" && matches!(removal, Removal::Free);
                let kept = held.then_some(Damage::Checksum);
                let mapped = held.then(BTreeSet::<Quarantine>::new);
                let session = (Some(Damage::Checksum), kept, mapped, (BTreeSet::new(), 0));
                want.push((
                    origin,
                    removal,
                    close,
                    Some(session),
                    Some((None, Vec::new())),
                ));
            }
        }
    }
    assert_eq!(14, got.len());
    assert_eq!(want, got);
}
