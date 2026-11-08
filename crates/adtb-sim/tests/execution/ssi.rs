use adtb_harness::seeds;
use adtb_simkit::elle::{self, Elems, Kind, Reply, Req, Worker, simulate, simulate_with};
use std::ops::Range;

use std::sync::{Arc, Mutex};

use adtb::mvcc::Limits;
use adtb::mvcc::Snapshot;
use adtb::runtime::{Entropy, ThreadId, Threads};
use adtb::testing::Knobs;
use adtb::{Error, OpenOptions, Position, RejectReason, RetryReason, TxnId, Value};
use adtb_isolation::{self as iso, Outcome as Ended, SCENARIOS, Step};
use adtb_sim::SimRuntime;

/// Returns the transaction a serialization failure `r` reports.
fn failed<T>(r: &Result<T, Error>) -> Option<TxnId> {
    match r {
        Err(Error::Retry {
            reason: RetryReason::Serialization { txn, .. },
        }) => Some(*txn),
        _ => None,
    }
}

fn script(elems: &Elems, steps: &[Step]) -> Vec<iso::Txn> {
    iso::run(&mut elems.client(), steps)
}

fn outcome(history: &[iso::Txn], t: u64) -> Ended {
    history.iter().find(|x| x.id.0 == t).unwrap().outcome
}

#[test]
fn every_scenario_runs_without_anomaly() {
    for scenario in SCENARIOS {
        for seed in 0..4 {
            let history = simulate(seed, |_, e| script(e, scenario.steps));
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
    let history = simulate(1, |_, e| script(e, iso::WRITE_SKEW.steps));
    let sides = [outcome(&history, 1), outcome(&history, 2)];
    assert!(sides.contains(&Ended::Aborted), "{sides:?}");
    assert!(sides.contains(&Ended::Committed), "{sides:?}");
}

#[test]
fn lost_update_by_appends_aborts_one_side() {
    let history = simulate(2, |_, e| script(e, iso::LOST_UPDATE.steps));
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
        let history = simulate(seed, |_, e| script(e, FEKETE));
        assert_eq!(Ok(vec![]), iso::check(&history), "seed {seed}");
        let ends = [1, 2, 3].map(|t| outcome(&history, t));
        assert!(ends.contains(&Ended::Aborted), "seed {seed}: {ends:?}");
    }
}

/// Returns each safe snapshot marker in `elems`'s log: the commit position it covers and its
/// snapshot.
fn markers(elems: &Elems) -> Vec<(Position, Snapshot)> {
    let probe = elems.database().testing().probe();
    probe
        .safe_snapshots()
        .into_iter()
        .map(|(at, snapshot)| (Position::from(at), snapshot))
        .collect()
}

#[test]
fn a_deferrable_reader_waits_for_a_safe_snapshot_and_never_fails() {
    for seed in 0..8 {
        let (ends, seen) = simulate(seed, |rt, e| {
            let logged = markers(e).len();
            let t2 = Worker::start(rt, e, Kind::ReadWrite);
            t2.call(rt, Req::Read(1)).unwrap();
            t2.call(rt, Req::Read(2)).unwrap();
            let t1 = Worker::start(rt, e, Kind::ReadWrite);
            t1.call(rt, Req::Read(2)).unwrap();
            t1.call(rt, Req::Append(2, 10)).unwrap();
            t1.call(rt, Req::Commit).unwrap();
            t2.call(rt, Req::Append(1, 20)).unwrap();
            let e2 = e.clone();
            let seen = Arc::new(Mutex::new(None));
            let out = seen.clone();
            let d = rt.spawn("deferrable", move || {
                let mut r = e2.begin(Kind::Deferrable).unwrap();
                let snapshot = e2.database().testing().snapshot(&r).unwrap();
                let x = e2.read(&mut r, 1).unwrap();
                let y = e2.read(&mut r, 2).unwrap();
                r.commit(None).unwrap();
                *out.lock().unwrap() = Some((x, y, snapshot));
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
            for t in [t1.thread(), t2.thread()] {
                rt.join(t);
            }
            let (x, y, snapshot) = seen.lock().unwrap().take().unwrap();
            let new: Vec<Snapshot> = markers(e)
                .split_off(logged)
                .into_iter()
                .map(|(_, s)| s)
                .collect();
            assert_eq!(vec![snapshot], new, "only the retaken snapshot is safe");
            (end2.map(drop), (x, y))
        });
        assert_eq!(Ok(()), ends, "seed {seed}");
        assert_eq!((vec![20], vec![10]), seen, "seed {seed}");
    }
}

#[test]
fn a_deferrable_reader_with_no_concurrent_writer_starts_at_once() {
    simulate(0, |rt, e| {
        let w = Worker::start(rt, e, Kind::ReadWrite);
        w.call(rt, Req::Append(1, 1)).unwrap();
        w.call(rt, Req::Commit).unwrap();
        let r = Worker::start(rt, e, Kind::Deferrable);
        assert_eq!(Ok(vec![1]), r.call(rt, Req::Read(1)));
        r.call(rt, Req::Commit).unwrap();
        for t in [w.thread(), r.thread()] {
            rt.join(t);
        }
    });
}

fn appender(rt: &SimRuntime, e: &Arc<Elems>, k: i64, elem: i64) -> Worker {
    let w = Worker::start(rt, e, Kind::ReadWrite);
    w.call(rt, Req::Append(k, elem)).unwrap();
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
        let (r, t2) = simulate(seed, |rt, e| {
            let t1 = appender(rt, e, 1, 1);
            let t2 = Worker::start(rt, e, Kind::ReadWrite);
            t2.send(rt, Req::Append(1, 2));
            assert_eq!(
                None,
                settle(rt, &t2),
                "seed {seed}: the second insert did not wait"
            );
            t1.call(rt, Req::Commit).unwrap();
            let r = t2.reply(rt);
            for t in [t1.thread(), t2.thread()] {
                rt.join(t);
            }
            (r, t2.id())
        });
        assert_eq!(Some(t2), failed(&r), "seed {seed}");
    }
}

#[test]
fn an_insert_that_waited_proceeds_if_the_other_transaction_aborts() {
    for seed in 0..4 {
        let read = simulate(seed, |rt, e| {
            let t1 = appender(rt, e, 1, 1);
            let t2 = Worker::start(rt, e, Kind::ReadWrite);
            t2.send(rt, Req::Append(1, 2));
            assert_eq!(None, settle(rt, &t2), "seed {seed}");
            t1.call(rt, Req::Abort).unwrap();
            t2.reply(rt).unwrap();
            t2.call(rt, Req::Commit).unwrap();
            for t in [t1.thread(), t2.thread()] {
                rt.join(t);
            }
            e.lists(1..2)
        });
        assert_eq!(vec![vec![2]], read, "seed {seed}");
    }
}

#[test]
fn an_insert_of_a_key_in_the_snapshot_violates_its_unique_constraint() {
    let r = simulate(3, |rt, e| {
        let t1 = appender(rt, e, 1, 5);
        t1.call(rt, Req::Commit).unwrap();
        let t2 = Worker::start(rt, e, Kind::ReadWrite);
        let r = t2.call(rt, Req::Append(1, 5));
        for t in [t1.thread(), t2.thread()] {
            rt.join(t);
        }
        r
    });
    let elem = |id, pos| Value::Ctor(0, [id, 1, pos, 5].map(Value::Int).into());
    let pair = |a, b| Value::Record([a, b].into());
    assert_eq!(
        Err(Error::Rejected {
            reason: RejectReason::Violated {
                constraint: "elle::elems.unique(k, e)".to_owned(),
                report: vec![pair(elem(1, 0), elem(2, 1)), pair(elem(2, 1), elem(1, 0)),],
                redacted: 0,
            }
        }),
        r
    );
}

#[test]
fn an_insert_of_a_key_committed_after_the_snapshot_fails() {
    let (r, t2) = simulate(4, |rt, e| {
        let t2 = Worker::start(rt, e, Kind::ReadWrite);
        let t1 = appender(rt, e, 1, 5);
        t1.call(rt, Req::Commit).unwrap();
        let r = t2.call(rt, Req::Append(1, 5));
        for t in [t1.thread(), t2.thread()] {
            rt.join(t);
        }
        (r, t2.id())
    });
    assert_eq!(Some(t2), failed(&r));
}

#[test]
fn two_inserts_waiting_on_each_other_abort_one() {
    for seed in 0..4 {
        let (r1, r2, t2) = simulate(seed, |rt, e| {
            let t1 = appender(rt, e, 1, 1);
            let t2 = appender(rt, e, 2, 1);
            t1.send(rt, Req::Append(2, 2));
            assert_eq!(None, settle(rt, &t1), "seed {seed}");
            let r2 = t2.call(rt, Req::Append(1, 2));
            if r2.is_ok() {
                t2.call(rt, Req::Abort).unwrap();
            }
            let r1 = t1.reply(rt);
            if r1.is_ok() {
                t1.call(rt, Req::Abort).unwrap();
            }
            for t in [t1.thread(), t2.thread()] {
                rt.join(t);
            }
            (r1, r2, t2.id())
        });
        assert_eq!(Ok(vec![]), r1, "seed {seed}");
        assert_eq!(Some(t2), failed(&r2), "seed {seed}");
    }
}

fn counter(rt: &SimRuntime, e: &Arc<Elems>, id: i64) {
    let w = Worker::start(rt, e, Kind::ReadWrite);
    w.call(
        rt,
        Req::Text(format!("insert counters {{ id: {id}, n: 0 }}")),
    )
    .unwrap();
    w.call(rt, Req::Commit).unwrap();
    rt.join(w.thread());
}

fn get(id: i64) -> Req {
    Req::Text(format!("from c in counters where c.id == {id} select c.n"))
}

fn increment(id: i64) -> Req {
    Req::Text(format!(
        "update c in counters where c.id == {id} set {{ c with n: c.n + 1 }}"
    ))
}

#[test]
fn a_lost_update_fails_the_second_writer() {
    for seed in 0..4 {
        let (r, n, t2) = simulate(seed, |rt, e| {
            counter(rt, e, 7);
            let t1 = Worker::start(rt, e, Kind::ReadWrite);
            let t2 = Worker::start(rt, e, Kind::ReadWrite);
            assert_eq!(Ok(vec![0]), t1.call(rt, get(7)), "seed {seed}");
            assert_eq!(Ok(vec![0]), t2.call(rt, get(7)), "seed {seed}");
            t1.call(rt, increment(7)).unwrap();
            t1.call(rt, Req::Commit).unwrap();
            let r = t2.call(rt, increment(7));
            let check = Worker::start(rt, e, Kind::ReadOnly);
            let n = check.call(rt, get(7));
            check.call(rt, Req::Commit).unwrap();
            for t in [t1.thread(), t2.thread(), check.thread()] {
                rt.join(t);
            }
            (r, n, t2.id())
        });
        assert_eq!(Some(t2), failed(&r), "seed {seed}");
        assert_eq!(Ok(vec![1]), n, "seed {seed}");
    }
}

#[test]
fn a_write_of_a_row_another_transaction_is_writing_waits_for_it() {
    for seed in 0..4 {
        let (committed, aborted, n) = simulate(seed, |rt, e| {
            counter(rt, e, 1);
            counter(rt, e, 2);
            let mut out = Vec::new();
            for (id, commit) in [(1, true), (2, false)] {
                let t1 = Worker::start(rt, e, Kind::ReadWrite);
                t1.call(rt, increment(id)).unwrap();
                let t2 = Worker::start(rt, e, Kind::ReadWrite);
                t2.send(rt, increment(id));
                assert_eq!(
                    None,
                    settle(rt, &t2),
                    "seed {seed}: the second write did not wait"
                );
                t1.call(rt, if commit { Req::Commit } else { Req::Abort })
                    .unwrap();
                let r = t2.reply(rt);
                if r.is_ok() {
                    t2.call(rt, Req::Commit).unwrap();
                }
                for t in [t1.thread(), t2.thread()] {
                    rt.join(t);
                }
                out.push((r, t2.id()));
            }
            let check = Worker::start(rt, e, Kind::ReadOnly);
            let n = [1, 2].map(|id| check.call(rt, get(id)));
            check.call(rt, Req::Commit).unwrap();
            rt.join(check.thread());
            (out[0].clone(), out[1].clone(), n)
        });
        assert_eq!(Some(committed.1), failed(&committed.0), "seed {seed}");
        assert_eq!(Ok(vec![]), aborted.0, "seed {seed}");
        assert_eq!([Ok(vec![1]), Ok(vec![1])], n, "seed {seed}");
    }
}

const KEYS: i64 = 3;

/// Clients running random transactions of reads and appends over a few keys. Returns the
/// history and how many transactions the serializer still tracks after they end.
fn random_history(
    seed: u64,
    options: &OpenOptions,
    clients: usize,
    per_client: usize,
) -> (Vec<iso::Txn>, usize) {
    simulate_with(seed, options, move |rt, e| {
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
        let ops = (0..KEYS)
            .zip(e.lists(0..KEYS))
            .map(|(key, list)| iso::Op::Read { key, list })
            .collect();
        let mut history = std::mem::take(&mut *history.lock().unwrap());
        history.push(iso::Txn::new(0, Ended::Committed, ops));
        (history, e.database().testing().tracked())
    })
}

fn random_txn(e: &Elems, rt: &SimRuntime, id: u64) -> iso::Txn {
    let kind = match rt.next_u64() % 10 {
        0 => Kind::Deferrable,
        1 => Kind::ReadOnly,
        _ => Kind::ReadWrite,
    };
    let mut tx = e.begin(kind).unwrap();
    let mut ops = Vec::new();
    let steps = 1 + rt.next_u64() % 4;
    for step in 0..steps {
        rt.yield_now();
        let key = (rt.next_u64() % KEYS as u64) as i64;
        let r = if kind != Kind::ReadWrite || rt.next_u64().is_multiple_of(2) {
            e.read(&mut tx, key).map(|list| iso::Op::Read { key, list })
        } else {
            let elem = (id * 10 + step) as i64;
            e.append(&mut tx, key, elem)
                .map(|()| iso::Op::Append { key, elem })
        };
        match r {
            Ok(op) => ops.push(op),
            Err(f) => {
                assert_ne!(Kind::Deferrable, kind, "a deferrable transaction failed");
                elle::aborted(f);
                return iso::Txn::new(id, Ended::Aborted, ops);
            }
        }
    }
    rt.yield_now();
    let outcome = match tx.commit(None) {
        Ok(_) => Ended::Committed,
        Err(f) => {
            assert_ne!(Kind::Deferrable, kind, "a deferrable commit failed: {f:?}");
            elle::aborted(f);
            Ended::Aborted
        }
    };
    iso::Txn::new(id, outcome, ops)
}

#[test]
fn every_committed_history_of_random_clients_is_serializable() {
    const SEEDS: Range<u64> = 0..24;
    let (mut committed, mut aborted) = (0, 0);
    for seed in sweep(SEEDS) {
        let (history, tracked) = random_history(seed, &elle::options(), 4, 6);
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
    let run = || random_history(5, &elle::options(), 4, 4).0;
    assert_eq!(run(), run());
}

#[test]
fn histories_stay_serializable_when_every_lock_promotes() {
    let options = OpenOptions {
        testing: Knobs::default().with_limits(Limits {
            tuples_per_page: 1,
            pages_per_heap: 1,
            ranges_per_index: 1,
            total: 8,
        }),
        ..elle::options()
    };
    for seed in 0..12 {
        let (history, _) = random_history(seed, &options, 4, 6);
        assert_eq!(Ok(vec![]), iso::check(&history), "seed {seed}");
    }
}

#[test]
fn locks_of_finished_transactions_are_released() {
    simulate(6, |rt, e| {
        let w = appender(rt, e, 1, 1);
        let r = Worker::start(rt, e, Kind::ReadWrite);
        r.call(rt, Req::Read(1)).unwrap();
        w.call(rt, Req::Commit).unwrap();
        r.call(rt, Req::Commit).unwrap();
        for t in [w.thread(), r.thread()] {
            rt.join(t);
        }
        assert_eq!(0, e.database().testing().tracked());
    });
}

#[test]
fn every_snapshot_found_safe_logs_a_marker_at_the_last_commit_before_it() {
    simulate(10, |_, e| {
        let testing = e.database().testing();
        let mut first = e.begin(Kind::ReadWrite).unwrap();
        e.append(&mut first, 2, 1).unwrap();
        let before = first.commit(None).unwrap();
        let logged = markers(e).len();
        let mut w = e.begin(Kind::ReadWrite).unwrap();
        e.append(&mut w, 1, 1).unwrap();
        let pending = e.begin(Kind::ReadOnly).unwrap();
        assert_eq!(
            logged,
            markers(e).len(),
            "a pending snapshot logged a marker"
        );
        let written = w.commit(None).unwrap();
        let after = e.begin(Kind::ReadOnly).unwrap();
        let snapshots = [&pending, &after].map(|t| testing.snapshot(t).unwrap());
        let [p, a] = snapshots;
        assert_eq!(
            vec![(before, p), (written, a)],
            markers(e).split_off(logged)
        );
        pending.commit(None).unwrap();
        after.commit(None).unwrap();
    });
}
