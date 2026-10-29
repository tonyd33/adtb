use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adtb::runtime::{Clock, Entropy, Io, Op, Runtime, Threads};
use adtb::storage::{Lsn, Options, Pager, Position};
use adtb_sim::{
    Config, Faults, Finding, FindingKind, Ledger, Outcome, Recovered, Sim, SimRuntime, check,
};

use adtb_simkit::crash;
use adtb_simkit::harness;

use crash::{crash_after, run_process, silence_fatal_panics};
use harness::finished;

fn options() -> Options {
    Options {
        segment_size: 64 * 1024,
        max_pages: 1024,
        ..harness::options(16)
    }
}

fn create(sim: &mut Sim) {
    finished(sim, |rt| {
        Pager::create(rt, &options()).expect("create");
    });
}

/// What the committers saw, outside the simulated process.
#[derive(Default)]
struct Seen {
    next: u64,
    /// Every event in the order it happened: a commit begun, or acknowledged at a position.
    events: Vec<(u64, Option<Position>)>,
}

/// Commit `per_thread` transactions from each of `threads` threads, each acknowledged once
/// `wait_durable` returns.
fn committers(
    rt: &SimRuntime,
    pager: &Pager<SimRuntime>,
    ledger: &Ledger,
    seen: &Arc<Mutex<Seen>>,
    threads: usize,
    per_thread: usize,
) {
    let workers: Vec<_> = (0..threads)
        .map(|i| {
            let (pager, ledger, seen, rt2) =
                (pager.clone(), ledger.clone(), seen.clone(), rt.clone());
            rt.spawn(&format!("committer {i}"), move || {
                for _ in 0..per_thread {
                    let xid = {
                        let mut s = seen.lock().unwrap();
                        s.next += 1;
                        let xid = s.next;
                        s.events.push((xid, None));
                        xid
                    };
                    ledger.submit(xid);
                    let lsn = pager.commit(xid, &[], |_| {}).unwrap();
                    let at = pager.wait_durable(lsn);
                    assert_eq!(lsn, at.lsn);
                    assert!(pager.durable() > lsn, "acknowledged before it was durable");
                    ledger.acknowledge(xid);
                    seen.lock().unwrap().events.push((xid, Some(at)));
                    if rt2.next_u64().is_multiple_of(3) {
                        rt2.yield_now();
                    }
                }
            })
        })
        .collect();
    for w in workers {
        rt.join(w);
    }
}

#[test]
fn concurrent_commits_share_batches() {
    let mut sim = Sim::new(1, Config::default());
    create(&mut sim);
    let ledger = Ledger::new();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let (batches, commits) = {
        let mut out = (0, 0);
        finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt.clone(), &options()).unwrap();
            let before = pager.batches();
            committers(&rt, &pager, &ledger, &seen, 8, 25);
            out = (pager.batches() - before, 8 * 25);
        });
        out
    };
    assert!(
        batches * 2 <= commits,
        "{batches} batches for {commits} commits: commits are not grouped"
    );
}

#[test]
fn a_commit_acknowledged_before_another_begins_has_the_earlier_position() {
    let mut sim = Sim::new(2, Config::default());
    create(&mut sim);
    let ledger = Ledger::new();
    let seen = Arc::new(Mutex::new(Seen::default()));
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options()).unwrap();
        committers(&rt, &pager, &ledger, &seen, 6, 20);
    });
    let events = &seen.lock().unwrap().events;
    let position = |x: u64| {
        events
            .iter()
            .find_map(|(y, p)| (*y == x).then_some(*p).flatten())
            .unwrap()
    };
    let mut acked: Vec<u64> = Vec::new();
    let mut pairs = 0;
    for (x, p) in events {
        match p {
            None => {
                let later = position(*x);
                for &a in &acked {
                    assert!(position(a) < later, "{x} began after {a} was acknowledged");
                    pairs += 1;
                }
            }
            Some(at) => {
                assert_eq!(1, at.epoch);
                acked.push(*x);
            }
        }
    }
    assert!(pairs > 1000, "{pairs} ordered pairs");
}

#[derive(Debug, Default)]
struct Reopened {
    commits: BTreeSet<u64>,
    findings: Vec<Finding>,
}

impl Recovered for Reopened {
    fn committed(&self) -> BTreeSet<u64> {
        self.commits.clone()
    }

    fn verify(&self) -> Vec<Finding> {
        self.findings.clone()
    }
}

#[test]
fn every_acknowledged_commit_survives_a_crash() {
    silence_fatal_panics();
    for seed in 0..12 {
        let mut sim = Sim::new(
            seed,
            Config {
                faults: Faults {
                    torn_p: 0.5,
                    ..Faults::default()
                },
                ..Config::default()
            },
        );
        create(&mut sim);
        let ledger = Ledger::new();
        let seen = Arc::new(Mutex::new(Seen::default()));
        for round in 0..3u64 {
            sim.crash_at(sim.now() + crash_after(seed, round, 500, 20_000));
            let (l, s) = (ledger.clone(), seen.clone());
            let outcome = run_process(&mut sim, |rt| {
                let (pager, _) = Pager::open(rt.clone(), &options()).unwrap();
                committers(&rt, &pager, &l, &s, 5, 1000);
            });
            assert_eq!(Outcome::Crashed, outcome);
            let mut reopened = Reopened::default();
            finished(&mut sim, |rt| {
                let (pager, _) = Pager::open(rt, &options()).unwrap();
                let v = pager.verify();
                reopened.commits = v.commits.into_iter().collect();
                reopened.findings = v
                    .findings
                    .iter()
                    .map(|f| Finding {
                        kind: FindingKind::Structure,
                        at: format!("{f:?}"),
                    })
                    .collect();
            });
            let violations = check(&ledger, &reopened);
            assert!(
                violations.is_empty(),
                "seed {seed} round {round}: {violations:?}"
            );
            let max = seen.lock().unwrap().next;
            for x in 1..=max {
                if ledger.fate(x) == Some(adtb_sim::Fate::InDoubt) {
                    if reopened.commits.contains(&x) {
                        ledger.acknowledge(x);
                    } else {
                        ledger.abort(x);
                    }
                }
            }
        }
    }
}

#[test]
fn waiting_for_a_record_already_durable_forces_nothing() {
    let mut sim = Sim::new(3, Config::default());
    create(&mut sim);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        let lsn = pager.commit(1, &[], |_| {}).unwrap();
        let first = pager.wait_durable(lsn);
        let batches = pager.batches();
        assert_eq!(first, pager.wait_durable(lsn));
        assert_eq!(batches, pager.batches());
        assert_eq!(Position { epoch: 1, lsn }, first);
        assert!(pager.durable() > lsn);
    });
}

#[test]
#[should_panic(expected = "no record was appended")]
fn waiting_for_a_record_never_appended_panics() {
    let mut sim = Sim::new(4, Config::default());
    create(&mut sim);
    sim.run(|rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        pager.wait_durable(Lsn(u64::MAX / 2));
    });
}

#[test]
fn safe_snapshot_markers_are_logged_in_order_and_survive_a_crash() {
    let mut sim = Sim::new(5, Config::default());
    create(&mut sim);
    let mut logged = Vec::new();
    sim.crash_at(sim.now() + Duration::from_secs(1));
    let outcome = sim.run(|rt| {
        let (pager, _) = Pager::open(rt.clone(), &options()).unwrap();
        for i in 0..5u64 {
            let lsn = pager.commit(10 + i, &[], |_| {}).unwrap();
            let at = pager.wait_durable(lsn);
            let snapshot = vec![i as u8; 3 + i as usize];
            let marker = pager.log_safe_snapshot(at, &snapshot);
            assert!(marker > lsn, "a marker follows the commit it covers");
            logged.push((marker, at, snapshot));
        }
        let last = logged.last().unwrap().0;
        pager.wait_durable(last);
        let mut io = rt.io();
        let at = rt.now() + Duration::from_secs(10);
        io.submit(Op::Timer { at });
        loop {
            rt.park();
        }
    });
    assert_eq!(Outcome::Crashed, outcome);
    let found = {
        let mut out = Vec::new();
        finished(&mut sim, |rt| {
            let (pager, _) = Pager::open(rt, &options()).unwrap();
            out = pager.verify().safe_snapshots;
        });
        out
    };
    assert_eq!(logged, found);
}

#[test]
fn a_marker_names_the_position_it_was_given() {
    let mut sim = Sim::new(6, Config::default());
    create(&mut sim);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &options()).unwrap();
        let at = Position {
            epoch: 1,
            lsn: Lsn(0),
        };
        let marker = pager.log_safe_snapshot(at, b"none running");
        pager.flush_log();
        let v = pager.verify();
        assert_eq!(
            vec![(marker, at, b"none running".to_vec())],
            v.safe_snapshots
        );
        assert!(v.commits.is_empty());
    });
}

fn sized(threads: usize, pool_frames: usize) -> Options {
    Options {
        threads,
        pool_frames,
        ..options()
    }
}

#[test]
#[should_panic(expected = "a pool of 19 frames serves at most 3 threads, not 4")]
fn opening_with_too_small_a_pool_for_its_threads_panics() {
    let mut sim = Sim::new(7, Config::default());
    create(&mut sim);
    sim.run(|rt| {
        Pager::open(rt, &sized(4, 19)).unwrap();
    });
}

#[test]
fn a_pool_serves_five_frames_per_thread() {
    assert_eq!(5, adtb::storage::LATCHES_PER_THREAD);
    let mut sim = Sim::new(8, Config::default());
    create(&mut sim);
    finished(&mut sim, |rt| {
        let (pager, _) = Pager::open(rt, &sized(4, 20)).unwrap();
        assert_eq!(4, pager.max_threads());
    });
}

#[test]
#[should_panic(expected = "the pool is stalled")]
fn a_pool_every_frame_of_which_stays_latched_reports_a_stall() {
    let mut sim = Sim::new(9, Config::default());
    create(&mut sim);
    let options = Options {
        stall_after: Duration::from_millis(5),
        ..sized(1, 5)
    };
    sim.run(move |rt| {
        let (pager, _) = Pager::open(rt.clone(), &options).unwrap();
        let pages: Vec<_> = (0..6)
            .map(|_| {
                pager
                    .allocate(1, adtb::storage::PageKind::HeapSlotted, 1)
                    .unwrap()
            })
            .collect();
        let latched = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (p, first, rt2, flag) = (
            pager.clone(),
            pages[..3].to_vec(),
            rt.clone(),
            latched.clone(),
        );
        let holder = rt.spawn("holder", move || {
            let _held: Vec<_> = first.iter().map(|&pid| p.exclusive(pid).unwrap()).collect();
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            for _ in 0..100_000 {
                rt2.yield_now();
            }
        });
        while !latched.load(std::sync::atomic::Ordering::SeqCst) {
            rt.yield_now();
        }
        let _held: Vec<_> = pages[3..5]
            .iter()
            .map(|&pid| pager.exclusive(pid).unwrap())
            .collect();
        pager.read(pages[5]).unwrap();
        rt.join(holder);
    });
}

/// Returns the largest buffer any `Read` op of `trace` carries.
fn largest_read(trace: &str) -> usize {
    trace
        .lines()
        .filter(|l| l.contains(" Read {"))
        .filter_map(|l| {
            let at = l.find("len: ")? + 5;
            l[at..]
                .split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn recovery_and_verify_read_the_log_a_chunk_at_a_time() {
    const SEGMENT: u64 = 4 << 20;
    let big = Options {
        segment_size: SEGMENT,
        ..options()
    };
    let mut sim = Sim::new(10, Config::default());
    let b = big.clone();
    finished(&mut sim, move |rt| {
        let pager = Pager::create(rt, &b).unwrap();
        let pid = pager
            .allocate(1, adtb::storage::PageKind::HeapSlotted, 1)
            .unwrap();
        for i in 0..3000u64 {
            pager
                .update(&[pid], |p| p[0][100..108].copy_from_slice(&i.to_le_bytes()))
                .unwrap();
            pager.commit(i + 1, &[], |_| {}).unwrap();
        }
        pager.flush_log();
        assert!(
            pager.durable().0 > 2 * 64 * 1024,
            "the log spans several chunks"
        );
    });
    let before = sim.trace().len();
    let b = big;
    finished(&mut sim, move |rt| {
        let (pager, recovery) = Pager::open(rt, &b).unwrap();
        assert!(
            recovery.applied > 2000,
            "recovery replayed {}",
            recovery.applied
        );
        let v = pager.verify();
        assert_eq!(3000, v.commits.len());
        assert!(v.findings.is_empty(), "{:?}", v.findings);
    });
    let trace = sim.trace();
    let largest = largest_read(&trace[before..]);
    assert!(largest > 0, "no read was traced");
    assert!(
        largest <= 64 * 1024,
        "a read of {largest} bytes, against log segments of {SEGMENT}"
    );
}
