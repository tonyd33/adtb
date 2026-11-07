use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use adtb::runtime::{Entropy, ThreadId, Threads};
use adtb::storage::{
    Damage, Finding, HEADER_LEN, Lsn, Options, PageError, PageKind, Pager, Pid, Quarantine,
};
use adtb_sim::{Config, Sim, SimRuntime};

use adtb_simkit::{harness, seeds};

use harness::{NAME, PAGE, finished};
use seeds::{assert_reached, share, sweep};

type Files = BTreeMap<String, Vec<u8>>;

const SEEDS: Range<u64> = 0..64;
const SEGMENT: u64 = 64 * 1024;
const WRITTEN: u8 = 0xa1;
const CHANGED: u8 = 0xb2;
/// The most steps a late reader waits for its window, and then before its first read.
const LATE_WAIT: u64 = 8192;

fn options() -> Options {
    Options {
        segment_size: SEGMENT,
        threads: 3,
        ..harness::options(16)
    }
}

/// A closed database whose page holds `WRITTEN` in the file, its log padded past two segments.
struct Written {
    files: Files,
    pid: Pid,
    /// The LSN of the page's last record.
    lsn: Lsn,
}

fn written() -> Written {
    let mut sim = Sim::new(1, Config::default());
    finished(&mut sim, |rt| {
        Pager::create(rt, &options()).unwrap();
    });
    let mut page = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        let pid = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        let lsn = pager
            .update(&[pid], |p| p[0][HEADER_LEN] = WRITTEN)
            .unwrap();
        let pad = pager.allocate(1, PageKind::HeapSlotted, 7).unwrap();
        while pager.appended().0 < 2 * SEGMENT + SEGMENT / 2 {
            pager
                .update(&[pad], |p| {
                    p[0][HEADER_LEN..].iter_mut().for_each(|b| *b ^= 1);
                })
                .unwrap();
            pager.flush_log();
        }
        pager.close();
        page = Some((pid, lsn));
    });
    let (pid, lsn) = page.unwrap();
    Written {
        files: sim.files(),
        pid,
        lsn,
    }
}

/// Returns the files of `w` after a process changed its page to `CHANGED` and closed, with the
/// page's bytes in the data file put back as they were: a write the device lost. Returns the LSN
/// of the lost change too.
fn lost(w: &Written) -> (Files, Lsn) {
    let mut sim = Sim::with_files(1, Config::default(), w.files.clone());
    let mut lsn = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        lsn = Some(
            pager
                .update(&[w.pid], |p| p[0][HEADER_LEN] = CHANGED)
                .unwrap(),
        );
        pager.close();
    });
    let mut files = sim.files();
    let at = w.pid.0 as usize * PAGE..(w.pid.0 as usize + 1) * PAGE;
    let file = files.get_mut(NAME).unwrap();
    file[at.clone()].copy_from_slice(&w.files[NAME][at]);
    (files, lsn.unwrap())
}

/// Returns `files` without the log segment starting at `base`.
fn without_segment(mut files: Files, base: u64) -> Files {
    let path = format!("{}/{base:016x}", options().log_dir());
    assert!(files.remove(&path).is_some(), "{path} is a segment");
    files
}

/// What one reader saw of a page while another thread refused it.
#[derive(Debug, Default)]
struct Seen {
    /// The byte each read the reader was served held, up to its first refusal.
    served: Vec<u8>,
    refused: Option<PageError>,
    /// Whether any read began inside the window the reader watches.
    in_window: bool,
}

/// A span another thread marks: begun once `start` is set, over once `end` is.
#[derive(Clone, Default)]
struct Window {
    start: Arc<AtomicBool>,
    end: Arc<AtomicBool>,
}

impl Window {
    fn open(&self) -> bool {
        self.start.load(Ordering::Acquire) && !self.end.load(Ordering::Acquire)
    }
}

/// Spawn a thread that reads `pid` until refused, at most 16 times, noting for each read whether
/// `window` was open when it began. Open `marks` as its first read begins and close it once that
/// read returns. When `late`, wait for
/// the window to open and then a seeded number of steps before the first read.
fn reader(
    rt: &SimRuntime,
    pager: &Pager<SimRuntime>,
    pid: Pid,
    window: &Window,
    marks: &Window,
    late: bool,
) -> (ThreadId, Arc<Mutex<Seen>>) {
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (p, rt2, into) = (pager.clone(), rt.clone(), seen.clone());
    let (window, marks) = (window.clone(), marks.clone());
    let t = rt.spawn("reader", move || {
        if late {
            for _ in 0..LATE_WAIT {
                if window.start.load(Ordering::Acquire) {
                    break;
                }
                rt2.yield_now();
            }
            for _ in 0..rt2.next_u64() % LATE_WAIT {
                rt2.yield_now();
            }
        }
        for _ in 0..16 {
            let inside = window.open();
            marks.start.store(true, Ordering::Release);
            let read = p.read(pid).map(|g| g[HEADER_LEN]);
            marks.end.store(true, Ordering::Release);
            let mut seen = into.lock().unwrap();
            seen.in_window |= inside;
            match read {
                Ok(b) => seen.served.push(b),
                Err(e) => {
                    seen.refused = Some(e);
                    return;
                }
            }
            drop(seen);
            rt2.yield_now();
        }
    });
    (t, seen)
}

fn switching(seed: u64, files: &Files) -> Sim {
    let config = Config {
        switch_rate: 0.5,
        ..Config::default()
    };
    Sim::with_files(seed, config, files.clone())
}

/// Returns the number of `pid`'s damage records in the retained log.
fn records_of(pager: &Pager<SimRuntime>, pid: Pid) -> usize {
    pager
        .logged_damage()
        .iter()
        .filter(|(p, _)| *p == pid)
        .count()
}

/// What one race of a refusal against a reader gave.
#[derive(Debug)]
struct Raced {
    seen: Seen,
    damaged: Option<Damage>,
    records: usize,
}

/// Open `files` under `seed`'s interleaving, run `before` on the open pager, then `refuse` on this
/// thread while a second thread reads `pid`, its window open while `refuse` runs.
fn race(
    seed: u64,
    files: &Files,
    pid: Pid,
    whole: bool,
    late: bool,
    before: impl FnOnce(&Pager<SimRuntime>),
    refuse: impl FnOnce(&Pager<SimRuntime>),
) -> Raced {
    let mut sim = switching(seed, files);
    let mut out = None;
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options()).unwrap();
        assert_eq!(
            whole,
            pager.is_quarantined(Quarantine::Database),
            "seed {seed}"
        );
        before(&pager);
        let window = Window::default();
        let (t, seen) = reader(&rt, &pager, pid, &window, &Window::default(), late);
        for _ in 0..rt.next_u64() % 6 {
            rt.yield_now();
        }
        window.start.store(true, Ordering::Release);
        refuse(&pager);
        window.end.store(true, Ordering::Release);
        rt.join(t);
        out = Some(Raced {
            seen: std::mem::take(&mut *seen.lock().unwrap()),
            damaged: pager.damaged().get(&pid).copied(),
            records: records_of(&pager, pid),
        });
    });
    out.unwrap()
}

/// Change `pid` in memory to `CHANGED`, then refuse it as `Checksum` while a reader reads it, on
/// every seed. Check each reader was served only the changed page, then refused it as `Checksum`,
/// and the pager holds `Checksum`.
fn every_refusal_race_refuses_as_checksum(files: &Files, pid: Pid, whole: bool) {
    let refusal = PageError {
        pid,
        damage: Damage::Checksum,
    };
    let mut raced = 0;
    for seed in sweep(SEEDS) {
        let r = race(
            seed,
            files,
            pid,
            whole,
            false,
            |p| {
                p.update(&[pid], |p| p[0][HEADER_LEN] = CHANGED).unwrap();
            },
            |p| {
                p.damage(pid, Damage::Checksum);
            },
        );
        assert!(
            r.seen.served.iter().all(|&b| b == CHANGED),
            "seed {seed}: {r:?}"
        );
        assert_eq!(Some(refusal), r.seen.refused, "seed {seed}: {r:?}");
        assert_eq!(Some(Damage::Checksum), r.damaged, "seed {seed}: {r:?}");
        raced += usize::from(r.seen.in_window);
    }
    assert_reached(
        "a read that began once the refusal was under way",
        raced,
        share(SEEDS, 8),
    );
}

#[test]
fn a_reader_racing_the_refusal_of_a_changed_page_is_refused_as_the_refusal_says() {
    let w = written();
    every_refusal_race_refuses_as_checksum(&w.files, w.pid, false);
}

#[test]
fn a_reader_racing_the_refusal_of_a_changed_page_of_a_file_quarantined_whole_is_never_served_the_file_copy()
 {
    let w = written();
    let gap = without_segment(w.files.clone(), SEGMENT);
    every_refusal_race_refuses_as_checksum(&gap, w.pid, true);
}

#[test]
fn verify_racing_a_reader_of_a_page_that_lost_its_write_records_its_refusal_once() {
    let w = written();
    let (files, expected) = lost(&w);
    let stale = Damage::Stale {
        expected,
        found: w.lsn,
    };
    let refusal = PageError {
        pid: w.pid,
        damage: stale,
    };
    let mut raced = 0;
    for seed in sweep(SEEDS) {
        let r = race(
            seed,
            &files,
            w.pid,
            false,
            true,
            |_| (),
            |p| drop(p.verify()),
        );
        assert_eq!(Vec::<u8>::new(), r.seen.served, "seed {seed}: {r:?}");
        assert_eq!(Some(refusal), r.seen.refused, "seed {seed}: {r:?}");
        assert_eq!(Some(stale), r.damaged, "seed {seed}: {r:?}");
        assert_eq!(1, r.records, "seed {seed}: {r:?}");
        raced += usize::from(r.seen.in_window);
    }
    assert_reached(
        "a read that began once verify was under way",
        raced,
        share(SEEDS, 8),
    );
}

#[test]
fn verify_of_a_file_quarantined_whole_refuses_the_page_that_lost_its_write() {
    let w = written();
    let (files, expected) = lost(&w);
    let gap = without_segment(files, SEGMENT);
    let refusal = PageError {
        pid: w.pid,
        damage: Damage::Stale {
            expected,
            found: w.lsn,
        },
    };
    let mut sim = Sim::with_files(1, Config::default(), gap);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        assert!(pager.is_quarantined(Quarantine::Database));
        let found = pager.verify().findings;
        let page = Finding::Page {
            pid: w.pid,
            damage: refusal.damage,
        };
        assert!(found.contains(&page), "{found:?}");
        assert_eq!(Err(refusal), pager.read(w.pid).map(|g| g[HEADER_LEN]));
    });
}

#[test]
fn two_readers_racing_the_read_in_of_a_page_that_lost_its_write_record_its_refusal_once() {
    let w = written();
    let (files, expected) = lost(&w);
    let refusal = PageError {
        pid: w.pid,
        damage: Damage::Stale {
            expected,
            found: w.lsn,
        },
    };
    let mut raced = 0;
    for seed in sweep(SEEDS) {
        let mut sim = switching(seed, &files);
        finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options()).unwrap();
            let first_read = Window::default();
            let (a, first) = reader(&rt, &pager, w.pid, &Window::default(), &first_read, false);
            let (b, second) = reader(&rt, &pager, w.pid, &first_read, &Window::default(), false);
            rt.join(a);
            rt.join(b);
            for seen in [&first, &second] {
                let seen = seen.lock().unwrap();
                assert_eq!(Vec::<u8>::new(), seen.served, "seed {seed}: {seen:?}");
                assert_eq!(Some(refusal), seen.refused, "seed {seed}: {seen:?}");
            }
            assert_eq!(1, records_of(&pager, w.pid), "seed {seed}");
            raced += usize::from(second.lock().unwrap().in_window);
        });
    }
    assert_reached(
        "a second read that began while the first ran",
        raced,
        share(SEEDS, 8),
    );
}
