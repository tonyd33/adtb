use std::ops::Range;
#[expect(dead_code, reason = "this binary never reopens the engine")]
mod engine;
#[path = "support/seeds.rs"]
mod seeds;

use std::sync::{Arc, Mutex};

use adtb::mvcc::{Cause, Limits};
use adtb::runtime::{Entropy, ThreadId, Threads};
use adtb_isolation::{self as iso, Outcome as Ended, SCENARIOS, Step};
use adtb_sim::SimRuntime;
use engine::{Client, Engine, Failure, Kind, Reply, Req, Worker, simulate, simulate_with};

/// Returns the cause of a serialization failure `r` reports.
fn cause<T>(r: &Result<T, Failure>) -> Option<Cause> {
    match r {
        Err(Failure::Serialization(f)) => Some(f.cause),
        _ => None,
    }
}

fn script(engine: &Arc<Engine>, steps: &[Step]) -> Vec<iso::Txn> {
    let mut client = Client {
        engine: engine.clone(),
        threads: Vec::new(),
    };
    let history = iso::run(&mut client, steps);
    for t in client.threads {
        engine.rt.join(t);
    }
    history
}

fn outcome(history: &[iso::Txn], t: u64) -> Ended {
    history.iter().find(|x| x.id.0 == t).unwrap().outcome
}

#[test]
fn every_scenario_runs_without_anomaly() {
    for scenario in SCENARIOS {
        for seed in 0..4 {
            let history = simulate(seed, |e| script(e, scenario.steps));
            assert_eq!(
                Ok(vec![]),
                iso::check(&history),
                "{} seed {seed}",
                scenario.name
            );
        }
    }
}

#[test]
fn write_skew_aborts_one_side() {
    let history = simulate(1, |e| script(e, iso::WRITE_SKEW.steps));
    let sides = [outcome(&history, 1), outcome(&history, 2)];
    assert!(sides.contains(&Ended::Aborted), "{sides:?}");
    assert!(sides.contains(&Ended::Committed), "{sides:?}");
}

#[test]
fn lost_update_by_appends_aborts_one_side() {
    let history = simulate(2, |e| script(e, iso::LOST_UPDATE.steps));
    let sides = [outcome(&history, 1), outcome(&history, 2)];
    assert_eq!([Ended::Committed, Ended::Aborted], sides);
}

use Step::{Append, Begin, Commit, Read};
use seeds::{share, sweep};

/// Fekete, O'Neil and O'Neil's read-only anomaly over keys 1 (X) and 2 (Y): 2 reads both and
/// appends to X; 1 appends to Y and commits; 3 reads both after 1 commits and before 2 does.
const FEKETE: &[Step] = &[
    Begin(2),
    Read(2, 1),
    Read(2, 2),
    Begin(1),
    Read(1, 2),
    Append(1, 2, 10),
    Commit(1),
    Append(2, 1, 20),
    Begin(3),
    Read(3, 1),
    Read(3, 2),
    Commit(3),
    Commit(2),
    Begin(4),
    Read(4, 1),
    Read(4, 2),
    Commit(4),
];

#[test]
fn feketes_read_only_anomaly_aborts_one_transaction() {
    for seed in 0..4 {
        let history = simulate(seed, |e| script(e, FEKETE));
        assert_eq!(Ok(vec![]), iso::check(&history), "seed {seed}");
        let ends = [1, 2, 3].map(|t| outcome(&history, t));
        assert!(ends.contains(&Ended::Aborted), "{ends:?}");
    }
}

#[test]
fn a_deferrable_reader_waits_for_a_safe_snapshot_and_never_fails() {
    for seed in 0..8 {
        let (ends, seen) = simulate(seed, |e| {
            let rt = &e.rt;
            let t2 = Worker::start(e, Kind::ReadWrite);
            t2.call(rt, Req::Read(1)).unwrap();
            t2.call(rt, Req::Read(2)).unwrap();
            let t1 = Worker::start(e, Kind::ReadWrite);
            t1.call(rt, Req::Read(2)).unwrap();
            t1.call(rt, Req::Append(2, 10)).unwrap();
            t1.call(rt, Req::Commit).unwrap();
            t2.call(rt, Req::Append(1, 20)).unwrap();
            let e2 = e.clone();
            let rt2 = rt.clone();
            let seen = Arc::new(Mutex::new(None));
            let out = seen.clone();
            let d = rt.spawn("deferrable", move || {
                let r = Worker::start(&e2, Kind::Deferrable);
                let x = r.call(&rt2, Req::Read(1)).unwrap();
                let y = r.call(&rt2, Req::Read(2)).unwrap();
                r.call(&rt2, Req::Commit).unwrap();
                rt2.join(r.thread);
                *out.lock().unwrap() = Some((x, y));
            });
            for _ in 0..20 {
                rt.yield_now();
            }
            assert!(
                seen.lock().unwrap().is_none(),
                "the reader ran on an unsafe snapshot"
            );
            let end2 = t2.call(rt, Req::Commit);
            rt.join(d);
            for t in [t1.thread, t2.thread] {
                rt.join(t);
            }
            let seen = seen.lock().unwrap().take().unwrap();
            let markers = e.safe_snapshots();
            assert_eq!(
                1,
                markers.len(),
                "only the retaken snapshot is safe: {markers:?}"
            );
            (end2, seen)
        });
        assert_eq!(Ok(vec![]), ends, "seed {seed}");
        assert_eq!((vec![20], vec![10]), seen, "seed {seed}");
    }
}

#[test]
fn a_deferrable_reader_with_no_concurrent_writer_starts_at_once() {
    simulate(0, |e| {
        let rt = &e.rt;
        let w = Worker::start(e, Kind::ReadWrite);
        w.call(rt, Req::Append(1, 1)).unwrap();
        w.call(rt, Req::Commit).unwrap();
        let r = Worker::start(e, Kind::Deferrable);
        assert_eq!(Ok(vec![1]), r.call(rt, Req::Read(1)));
        r.call(rt, Req::Commit).unwrap();
        for t in [w.thread, r.thread] {
            rt.join(t);
        }
    });
}

fn appender(e: &Arc<Engine>, k: i64, elem: i64) -> Worker {
    let w = Worker::start(e, Kind::ReadWrite);
    w.call(&e.rt, Req::Append(k, elem)).unwrap();
    w
}

/// Let the other threads run until `w` answers. Returns its reply, or `None` if it is still
/// waiting after many switches.
fn settle(rt: &SimRuntime, w: &Worker) -> Option<Reply> {
    for _ in 0..200 {
        if let Some(r) = w.try_reply() {
            return Some(r);
        }
        rt.yield_now();
    }
    None
}

#[test]
fn an_insert_of_a_key_a_running_transaction_inserted_waits_and_fails_if_it_commits() {
    for seed in 0..4 {
        let r = simulate(seed, |e| {
            let rt = &e.rt;
            let t1 = appender(e, 1, 1);
            let t2 = Worker::start(e, Kind::ReadWrite);
            t2.send(rt, Req::Append(1, 2));
            assert_eq!(None, settle(rt, &t2), "the second insert did not wait");
            t1.call(rt, Req::Commit).unwrap();
            let r = t2.reply(rt);
            for t in [t1.thread, t2.thread] {
                rt.join(t);
            }
            r
        });
        assert_eq!(Some(Cause::LaterCommit), cause(&r), "seed {seed}");
    }
}

#[test]
fn an_insert_that_waited_proceeds_if_the_other_transaction_aborts() {
    for seed in 0..4 {
        let read = simulate(seed, |e| {
            let rt = &e.rt;
            let t1 = appender(e, 1, 1);
            let t2 = Worker::start(e, Kind::ReadWrite);
            t2.send(rt, Req::Append(1, 2));
            assert_eq!(None, settle(rt, &t2));
            t1.call(rt, Req::Abort).unwrap();
            t2.reply(rt).unwrap();
            t2.call(rt, Req::Commit).unwrap();
            let r = Worker::start(e, Kind::ReadOnly);
            let read = r.call(rt, Req::Read(1));
            r.call(rt, Req::Commit).unwrap();
            for t in [t1.thread, t2.thread, r.thread] {
                rt.join(t);
            }
            read
        });
        assert_eq!(Ok(vec![2]), read, "seed {seed}");
    }
}

#[test]
fn an_insert_of_a_key_in_the_snapshot_is_a_key_violation() {
    let r = simulate(3, |e| {
        let rt = &e.rt;
        let t1 = appender(e, 1, 5);
        t1.call(rt, Req::Commit).unwrap();
        let t2 = Worker::start(e, Kind::ReadWrite);
        let r = t2.call(rt, Req::Append(1, 5));
        for t in [t1.thread, t2.thread] {
            rt.join(t);
        }
        r
    });
    assert_eq!(Err(Failure::KeyViolation), r);
}

#[test]
fn an_insert_of_a_key_committed_after_the_snapshot_fails() {
    let r = simulate(4, |e| {
        let rt = &e.rt;
        let t2 = Worker::start(e, Kind::ReadWrite);
        let t1 = appender(e, 1, 5);
        t1.call(rt, Req::Commit).unwrap();
        let r = t2.call(rt, Req::Append(1, 5));
        for t in [t1.thread, t2.thread] {
            rt.join(t);
        }
        r
    });
    assert_eq!(Some(Cause::LaterCommit), cause(&r));
}

#[test]
fn two_inserts_waiting_on_each_other_abort_one() {
    for seed in 0..4 {
        let (r1, r2) = simulate(seed, |e| {
            let rt = &e.rt;
            let t1 = appender(e, 1, 1);
            let t2 = appender(e, 2, 1);
            t1.send(rt, Req::Append(2, 2));
            assert_eq!(None, settle(rt, &t1));
            let r2 = t2.call(rt, Req::Append(1, 2));
            if r2.is_ok() {
                t2.call(rt, Req::Abort).unwrap();
            }
            let r1 = t1.reply(rt);
            if r1.is_ok() {
                t1.call(rt, Req::Abort).unwrap();
            }
            for t in [t1.thread, t2.thread] {
                rt.join(t);
            }
            (r1, r2)
        });
        assert_eq!(Ok(vec![]), r1, "seed {seed}");
        assert_eq!(Some(Cause::Deadlock), cause(&r2), "seed {seed}");
    }
}

fn counter(e: &Arc<Engine>, id: i64) {
    let w = Worker::start(e, Kind::ReadWrite);
    w.call(&e.rt, Req::CreateCounter(id)).unwrap();
    w.call(&e.rt, Req::Commit).unwrap();
    e.rt.join(w.thread);
}

#[test]
fn a_lost_update_fails_the_second_writer() {
    for seed in 0..4 {
        let (r, n) = simulate(seed, |e| {
            let rt = &e.rt;
            counter(e, 7);
            let t1 = Worker::start(e, Kind::ReadWrite);
            let t2 = Worker::start(e, Kind::ReadWrite);
            assert_eq!(Ok(vec![0]), t1.call(rt, Req::GetCounter(7)));
            assert_eq!(Ok(vec![0]), t2.call(rt, Req::GetCounter(7)));
            assert_eq!(Ok(vec![1]), t1.call(rt, Req::Increment(7)));
            t1.call(rt, Req::Commit).unwrap();
            let r = t2.call(rt, Req::Increment(7));
            let check = Worker::start(e, Kind::ReadOnly);
            let n = check.call(rt, Req::GetCounter(7));
            check.call(rt, Req::Commit).unwrap();
            for t in [t1.thread, t2.thread, check.thread] {
                rt.join(t);
            }
            (r, n)
        });
        assert_eq!(Some(Cause::LaterCommit), cause(&r), "seed {seed}");
        assert_eq!(Ok(vec![1]), n, "seed {seed}");
    }
}

#[test]
fn a_write_of_a_row_another_transaction_is_writing_waits_for_it() {
    for seed in 0..4 {
        let (committed, aborted) = simulate(seed, |e| {
            let rt = &e.rt;
            counter(e, 1);
            counter(e, 2);
            let mut out = Vec::new();
            for (id, commit) in [(1, true), (2, false)] {
                let t1 = Worker::start(e, Kind::ReadWrite);
                t1.call(rt, Req::Increment(id)).unwrap();
                let t2 = Worker::start(e, Kind::ReadWrite);
                t2.send(rt, Req::Increment(id));
                assert_eq!(None, settle(rt, &t2), "the second write did not wait");
                t1.call(rt, if commit { Req::Commit } else { Req::Abort })
                    .unwrap();
                let r = t2.reply(rt);
                if r.is_ok() {
                    t2.call(rt, Req::Commit).unwrap();
                }
                for t in [t1.thread, t2.thread] {
                    rt.join(t);
                }
                out.push(r);
            }
            (out[0].clone(), out[1].clone())
        });
        assert_eq!(Some(Cause::LaterCommit), cause(&committed), "seed {seed}");
        assert_eq!(Ok(vec![1]), aborted, "seed {seed}");
    }
}

/// Clients running random transactions of reads and appends over a few keys.
fn random_history(
    seed: u64,
    limits: Limits,
    clients: usize,
    per_client: usize,
) -> (Vec<iso::Txn>, usize) {
    simulate_with(seed, limits, move |e| {
        let rt = &e.rt;
        let history = Arc::new(Mutex::new(Vec::new()));
        let next = Arc::new(Mutex::new(0_u64));
        let threads: Vec<ThreadId> = (0..clients)
            .map(|_| {
                let (e, history, next) = (e.clone(), history.clone(), next.clone());
                let rt2 = rt.clone();
                rt.spawn("client", move || {
                    for _ in 0..per_client {
                        let id = {
                            let mut n = next.lock().unwrap();
                            *n += 1;
                            *n
                        };
                        let txn = random_txn(&e, &rt2, id);
                        history.lock().unwrap().push(txn);
                    }
                })
            })
            .collect();
        for t in threads {
            rt.join(t);
        }
        let reader = e.begin(Kind::ReadOnly);
        let mut ops = Vec::new();
        for key in 0..KEYS {
            let list = e.read(&reader, key).unwrap();
            ops.push(iso::Op::Read { key, list });
        }
        e.commit(reader).unwrap();
        let mut history = std::mem::take(&mut *history.lock().unwrap());
        history.push(iso::Txn::new(0, Ended::Committed, ops));
        (history, e.tracked())
    })
}

const KEYS: i64 = 3;

fn random_txn(e: &Engine, rt: &SimRuntime, id: u64) -> iso::Txn {
    let kind = match rt.next_u64() % 10 {
        0 => Kind::Deferrable,
        1 => Kind::ReadOnly,
        _ => Kind::ReadWrite,
    };
    let read_only = kind != Kind::ReadWrite;
    let tx = e.begin(kind);
    let mut ops = Vec::new();
    let steps = 1 + rt.next_u64() % 4;
    for step in 0..steps {
        rt.yield_now();
        let key = (rt.next_u64() % KEYS as u64) as i64;
        let r = if read_only || rt.next_u64().is_multiple_of(2) {
            e.read(&tx, key).map(|list| iso::Op::Read { key, list })
        } else {
            let elem = (id * 10 + step) as i64;
            e.append(&tx, key, elem)
                .map(|()| iso::Op::Append { key, elem })
        };
        match r {
            Ok(op) => ops.push(op),
            Err(f) => {
                assert!(f.is_serialization(), "{f:?}");
                assert_ne!(Kind::Deferrable, kind, "a deferrable transaction failed");
                e.abort(tx);
                return iso::Txn::new(id, Ended::Aborted, ops);
            }
        }
    }
    rt.yield_now();
    let outcome = match e.commit(tx) {
        Ok(()) => Ended::Committed,
        Err(_) if kind == Kind::Deferrable => panic!("a deferrable transaction failed"),
        Err(_) => Ended::Aborted,
    };
    iso::Txn::new(id, outcome, ops)
}

#[test]
fn every_committed_history_of_random_clients_is_serializable() {
    const SEEDS: Range<u64> = 0..24;
    let (mut committed, mut aborted) = (0, 0);
    for seed in sweep(SEEDS) {
        let (history, tracked) = random_history(seed, Limits::default(), 4, 6);
        assert_eq!(Ok(vec![]), iso::check(&history), "seed {seed}");
        assert_eq!(0, tracked, "seed {seed} left transactions tracked");
        for t in &history {
            match t.outcome {
                Ended::Committed => committed += 1,
                _ => aborted += 1,
            }
        }
    }
    assert!(
        committed > share(SEEDS, 200),
        "only {committed} transactions committed"
    );
    assert!(
        aborted >= share(SEEDS, 11),
        "only {aborted} transactions aborted"
    );
}

#[test]
fn a_seed_replays_the_same_history() {
    let run = || random_history(5, Limits::default(), 4, 4).0;
    assert_eq!(run(), run());
}

#[test]
fn histories_stay_serializable_when_every_lock_promotes() {
    let limits = Limits {
        tuples_per_page: 1,
        pages_per_heap: 1,
        ranges_per_index: 1,
        total: 8,
    };
    for seed in 0..12 {
        let (history, _) = random_history(seed, limits, 4, 6);
        assert_eq!(Ok(vec![]), iso::check(&history), "seed {seed}");
    }
}

#[test]
fn locks_of_finished_transactions_are_released() {
    simulate(6, |e| {
        let rt = &e.rt;
        let w = appender(e, 1, 1);
        let r = Worker::start(e, Kind::ReadWrite);
        r.call(rt, Req::Read(1)).unwrap();
        w.call(rt, Req::Commit).unwrap();
        r.call(rt, Req::Commit).unwrap();
        for t in [w.thread, r.thread] {
            rt.join(t);
        }
        assert_eq!(0, e.tracked());
    });
}

#[test]
fn a_snapshot_reads_up_to_the_last_commit_before_it() {
    let mut sim = adtb_sim::Sim::new(9, adtb_sim::Config::default());
    sim.run(|rt| {
        let pager = adtb::storage::Pager::create(rt.clone(), &engine::options()).unwrap();
        let txns = adtb::mvcc::Transactions::create(rt, &pager).unwrap();
        let (before, _) = txns.take(adtb::mvcc::Scope::Every).unwrap();
        let x = txns.begin(&pager).unwrap();
        let (during, _) = txns.take(adtb::mvcc::Scope::Every).unwrap();
        let lsn = txns.commit(&pager, x).unwrap();
        let (after, _) = txns.take(adtb::mvcc::Scope::Every).unwrap();
        assert_eq!(None, txns.read_upto(before));
        assert_eq!(None, txns.read_upto(during));
        assert_eq!(Some(lsn), txns.read_upto(after));
        assert!(pager.durable() <= lsn, "the commit was forced");
    });
}

#[test]
fn every_snapshot_found_safe_logs_a_marker() {
    simulate(10, |e| {
        let w = e.begin(Kind::ReadWrite);
        e.append(&w, 1, 1).unwrap();
        let pending = e.begin(Kind::ReadOnly);
        assert!(
            e.safe_snapshots().is_empty(),
            "a pending snapshot logged a marker"
        );
        e.commit(w).unwrap();
        let after = e.begin(Kind::ReadOnly);
        let markers = e.safe_snapshots();
        assert_eq!(2, markers.len(), "{markers:?}");
        assert_eq!(Engine::snapshot(&pending), &markers[0].1);
        assert_eq!(Engine::snapshot(&after), &markers[1].1);
        assert_eq!(e.read_position(&after), Some(markers[1].0));
        assert!(markers[0].0 < markers[1].0, "{markers:?}");
        e.commit(pending).unwrap();
        e.commit(after).unwrap();
    });
}
