use crate::testing::Probe;
use adtb_testkit::process;
use adtb_testkit::process::finished;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::{Clock, Entropy, Io, IoError, Op, OpKind, Runtime, Threads};
use crate::storage::{Lsn, Options, Pager, Position};
use adtb_sim::{Action, Config, Ledger, Match, Outcome, Plan, Sim, SimRuntime};

use crate::test_world as harness;
use crate::test_world::api;
use crate::test_world::load as open;
use adtb_testkit::profile::assert_reached;
use adtb_testkit::{crash, search};

use crash::silence_fatal_panics;

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
                    assert!(
                        Probe::of(&pager).durable() > lsn,
                        "acknowledged before it was durable"
                    );
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
                    .allocate(1, crate::storage::PageKind::HeapSlotted, 1)
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

const COMMIT_SCHEMA: &str = r"module app
add module app
add type Row = { id: Key rows, name: Text }
add relation rows : Row key id generate sequence
";

const COMMIT_PAIRS: [&str; 3] = [
    r#"insert rows { id: rows.key(10), name: "seed-a" }
       insert rows { id: rows.key(11), name: "seed-b" }"#,
    r#"insert rows { id: rows.key(20), name: "first-a" }
       insert rows { id: rows.key(21), name: "first-b" }"#,
    r#"insert rows { id: rows.key(30), name: "second-a" }
       insert rows { id: rows.key(31), name: "second-b" }"#,
];

fn public_commit_db<R: Runtime>(rt: R) -> crate::Database<R> {
    let mut opts = open::options(1);
    opts.pool_bytes = Some(1024 * 1024);
    crate::Database::open_in(rt, "commit.adt", &crate::Log::new([COMMIT_SCHEMA]), &opts)
        .expect("the commit fixture opens")
}

fn commit_pair<R: Runtime>(
    db: &crate::Database<R>,
    text: &str,
    before_commit: impl FnOnce(),
) -> crate::Position {
    let mut tx = db.begin(&crate::TxnOptions::default()).expect("begin");
    tx.query_text(text, &[], &crate::TxnCallOptions::default())
        .expect("insert both rows");
    before_commit();
    tx.commit(None).expect("acknowledge both rows")
}

#[derive(Clone, Copy, Debug)]
enum CommitStage {
    Statements,
    Commit,
}

#[derive(Default)]
struct PublicCommitSeen {
    attempted: usize,
    acknowledged: Vec<crate::Position>,
    /// The ops numbered when each commit was attempted, and when it returned.
    commits: Vec<(u64, Option<u64>)>,
}

impl PublicCommitSeen {
    /// Returns the stage op `at` was submitted in.
    fn stage(&self, at: u64) -> CommitStage {
        let open = |&(start, end): &(u64, Option<u64>)| start <= at && end.is_none_or(|e| at < e);
        if self.commits.iter().any(open) {
            CommitStage::Commit
        } else {
            CommitStage::Statements
        }
    }
}

fn recovered_commit_prefix(files: search::Files, acknowledged: usize, attempted: usize, at: u64) {
    const PREFIXES: [&[&str]; 3] = [
        &[
            r#"{ id: rows.key(10), name: "seed-a" }"#,
            r#"{ id: rows.key(11), name: "seed-b" }"#,
        ],
        &[
            r#"{ id: rows.key(10), name: "seed-a" }"#,
            r#"{ id: rows.key(11), name: "seed-b" }"#,
            r#"{ id: rows.key(20), name: "first-a" }"#,
            r#"{ id: rows.key(21), name: "first-b" }"#,
        ],
        &[
            r#"{ id: rows.key(10), name: "seed-a" }"#,
            r#"{ id: rows.key(11), name: "seed-b" }"#,
            r#"{ id: rows.key(20), name: "first-a" }"#,
            r#"{ id: rows.key(21), name: "first-b" }"#,
            r#"{ id: rows.key(30), name: "second-a" }"#,
            r#"{ id: rows.key(31), name: "second-b" }"#,
        ],
    ];
    let mut reopened = Sim::with_files(83, Config::default(), files);
    let rows = process::run(&mut reopened, |rt| {
        let db = public_commit_db(rt);
        let mut results = db
            .query_text(
                "from r in rows select { id: r.id, name: r.name }",
                &[],
                &crate::CallOptions::default(),
            )
            .expect("read recovered rows");
        assert_eq!(1, results.len(), "one recovered result at {at}");
        api::sorted(results.remove(0))
    });
    let recovered = PREFIXES
        .iter()
        .position(|prefix| rows.iter().map(String::as_str).eq(prefix.iter().copied()))
        .unwrap_or_else(|| panic!("partial or phantom rows at {at}: {rows:?}"));
    assert!(
        (acknowledged..=attempted).contains(&recovered),
        "failure {at}: {acknowledged} acknowledged transactions, {attempted} attempted transactions, recovered prefix {recovered}"
    );
}

#[test]
fn public_commits_preserve_acknowledged_pairs_under_write_and_sync_failure() {
    silence_fatal_panics();
    let seed = 83;
    let seen = Arc::new(Mutex::new(PublicCommitSeen::default()));
    let mut initial = Sim::new(seed, Config::default());
    let seed_ack = process::run(&mut initial, |rt| {
        let db = public_commit_db(rt);
        commit_pair(&db, COMMIT_PAIRS[0], || {})
    });
    assert!(seed_ack.lsn > 0, "the seed is publicly acknowledged");
    let files = initial.files();
    let workload = |rt: &SimRuntime| {
        *seen.lock().unwrap() = PublicCommitSeen::default();
        let db = public_commit_db(rt.clone());
        rt.arm(Match::kinds([OpKind::Write, OpKind::Sync]));
        let numbered = || rt.numbered().kinds().len() as u64;
        for text in &COMMIT_PAIRS[1..] {
            let position = commit_pair(&db, text, || {
                let mut seen = seen.lock().unwrap();
                seen.attempted += 1;
                seen.commits.push((numbered(), None));
            });
            let mut seen = seen.lock().unwrap();
            seen.acknowledged.push(position);
            seen.commits.last_mut().unwrap().1 = Some(numbered());
        }
        rt.disarm();
    };
    let mut baseline = Sim::with_files(seed, Config::default(), files.clone());
    process::run(&mut baseline, |rt| workload(&rt));
    assert_eq!(
        2,
        seen.lock().unwrap().attempted,
        "both commits are attempted"
    );
    assert_eq!(
        2,
        seen.lock().unwrap().acknowledged.len(),
        "both commits return"
    );
    recovered_commit_prefix(baseline.files(), 2, 2, u64::MAX);
    let (mut writes, mut syncs, mut before_ack, mut after_ack) = (0, 0, 0, 0);
    let mut positions = BTreeSet::new();
    search::each_fatal_failure(&files, seed, 4, &[], workload, |failed| {
        assert_eq!(None, failed.out, "failure {}: {}", failed.at, failed.struck);
        assert!(positions.insert(failed.at), "a distinct failure position");
        match failed.struck.kind() {
            OpKind::Write => writes += 1,
            OpKind::Sync => syncs += 1,
            other => panic!("the search only fails writes and file forces, not {other:?}"),
        }
        let (acked, attempted, stage) = {
            let seen = seen.lock().unwrap();
            (
                seen.acknowledged.clone(),
                seen.attempted,
                seen.stage(failed.at),
            )
        };
        eprintln!(
            "failure {} {:?}: struck {stage:?}; acknowledged {}, attempted {attempted}",
            failed.at,
            failed.struck.kind(),
            acked.len()
        );
        assert!(
            acked.len() <= attempted && attempted <= acked.len() + 1 && attempted <= 2,
            "the external commit ledger"
        );
        match acked.len() {
            0 => before_ack += 1,
            1 => after_ack += 1,
            n => panic!("failure {} returned {n} new acknowledgements", failed.at),
        }
        for position in &acked {
            assert!(
                position.lsn > seed_ack.lsn,
                "failure {}: {position:?}",
                failed.at
            );
        }
        recovered_commit_prefix(failed.files, acked.len(), attempted, failed.at);
    });
    assert_reached("distinct failure positions", positions.len(), 3);
    assert_reached("failed writes", writes, 1);
    assert_reached("failed file forces", syncs, 1);
    assert_reached("fatal before the first new acknowledgement", before_ack, 1);
    assert_reached("fatal after an earlier acknowledgement", after_ack, 1);
}

#[test]
fn public_commits_preserve_acknowledged_pairs_under_crashes_at_sampled_events() {
    let seed = 83;
    let mut initial = Sim::new(seed, Config::default());
    process::run(&mut initial, |rt| {
        let db = public_commit_db(rt);
        commit_pair(&db, COMMIT_PAIRS[0], || {});
    });
    let files = initial.files();
    let attempts = Mutex::new(Vec::new());
    let acks = Mutex::new(Vec::new());
    let (mut before_ack, mut after_ack, mut in_commit, mut images) = (0, 0, 0, 0);
    let mut events = BTreeSet::new();
    search::each_crash_image(
        &files,
        seed,
        3,
        2,
        |rt| {
            let db = public_commit_db(rt.clone());
            rt.arm(Match::any());
            for text in &COMMIT_PAIRS[1..] {
                commit_pair(&db, text, || {
                    attempts.lock().unwrap().push(rt.recorded());
                });
                acks.lock().unwrap().push(rt.recorded());
            }
            rt.disarm();
            acks.lock().unwrap().clone()
        },
        |at, image| {
            let acknowledged = acks.lock().unwrap().iter().filter(|&&e| e <= at).count();
            let attempted = attempts
                .lock()
                .unwrap()
                .iter()
                .filter(|&&e| e <= at)
                .count();
            events.insert(at);
            images += 1;
            match acknowledged {
                0 => before_ack += 1,
                _ => after_ack += 1,
            }
            if attempted > acknowledged {
                in_commit += 1;
            }
            recovered_commit_prefix(image, acknowledged, attempted, at as u64);
        },
    );
    assert_eq!(2, acks.lock().unwrap().len(), "both commits return");
    assert_reached("crash events", events.len(), 3);
    assert_reached("crash images", images, 3);
    assert_reached("crash before the first new acknowledgement", before_ack, 1);
    assert_reached("crash after an earlier acknowledgement", after_ack, 1);
    assert_reached("crash inside a commit", in_commit, 1);
}

#[test]
fn a_failed_create_of_the_log_ends_the_process_with_its_cause() {
    let mut sim = Sim::new(32, Config::default());
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sim.run(|rt| {
            rt.arm(Match::kind(OpKind::CreateNew).containing("-log/"));
            rt.plan(Plan::new().at(0, Action::Fail(IoError::Failed)));
            drop(Pager::create(rt, &options()));
        })
    }));
    let struck = sim.numbered().struck().map(adtb_sim::Struck::kind);
    assert_eq!(Some(OpKind::CreateNew), struck, "the failure fired");
    let payload = caught.expect_err("the failed create ends the process");
    let cause = payload
        .downcast_ref::<crate::runtime::FatalIo>()
        .map(crate::runtime::FatalIo::cause);
    assert_eq!(Some(IoError::Failed), cause);
}
