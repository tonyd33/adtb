mod common;
mod engine;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use adtb::mvcc::Limits;
use adtb::runtime::{Entropy, Threads};
use adtb_isolation::{self as iso, Op, Outcome as Ended};
use adtb_sim::{Config, Faults, Outcome, Sim, SimRuntime};
use engine::{Engine, Kind, Roots};

/// The seeds one run checks. Milestone 6 exits when every one of them passes.
const SEEDS: u64 = 32;
const CLIENTS: usize = 4;
const TXNS_PER_CLIENT: usize = 6;
/// Processes per seed that may crash before the last, which runs to the end.
const CRASHES: u64 = 3;
const KEYS: i64 = 4;

/// What clients saw, kept across the crashes of the processes they ran in.
#[derive(Debug, Default)]
struct Book {
    txns: BTreeMap<u64, iso::Txn>,
    next_txn: u64,
    next_elem: i64,
}

fn book(b: &Mutex<Book>) -> MutexGuard<'_, Book> {
    b.lock().unwrap_or_else(|e| e.into_inner())
}

impl Book {
    /// Start recording a transaction as aborted, which it is if its process crashes before it
    /// submits its commit.
    fn begin(&mut self) -> u64 {
        self.next_txn += 1;
        let id = self.next_txn;
        self.txns
            .insert(id, iso::Txn::new(id, Ended::Aborted, Vec::new()));
        id
    }

    fn txn(&mut self, id: u64) -> &mut iso::Txn {
        self.txns.get_mut(&id).expect("the transaction began")
    }

    fn elem(&mut self) -> i64 {
        self.next_elem += 1;
        self.next_elem
    }
}

fn client(e: &Engine, rt: &SimRuntime, b: &Mutex<Book>) {
    for _ in 0..TXNS_PER_CLIENT {
        let kind = match rt.next_u64() % 10 {
            0 => Kind::Deferrable,
            1 => Kind::ReadOnly,
            _ => Kind::ReadWrite,
        };
        let id = book(b).begin();
        let tx = e.begin(kind);
        let mut failed = false;
        for _ in 0..1 + rt.next_u64() % 4 {
            rt.yield_now();
            let key = (rt.next_u64() % KEYS as u64) as i64;
            let op = if kind != Kind::ReadWrite || rt.next_u64().is_multiple_of(2) {
                e.read(&tx, key).map(|list| Op::Read { key, list })
            } else {
                let elem = book(b).elem();
                e.append(&tx, key, elem).map(|()| Op::Append { key, elem })
            };
            match op {
                Ok(op) => book(b).txn(id).ops.push(op),
                Err(f) => {
                    assert!(f.is_serialization(), "txn {id}: {f:?}");
                    assert_ne!(Kind::Deferrable, kind, "a deferrable transaction failed");
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            e.abort(tx);
            continue;
        }
        rt.yield_now();
        book(b).txn(id).outcome = Ended::InDoubt;
        let outcome = match e.commit(tx) {
            Ok(()) => Ended::Committed,
            Err(f) => {
                assert_ne!(Kind::Deferrable, kind, "a deferrable commit failed: {f:?}");
                Ended::Aborted
            }
        };
        book(b).txn(id).outcome = outcome;
    }
}

fn clients(e: &Arc<Engine>, b: &Arc<Mutex<Book>>) {
    let rt = &e.rt;
    let flusher = e.flusher();
    let threads: Vec<_> = (0..CLIENTS)
        .map(|_| {
            let (e, b, rt2) = (e.clone(), b.clone(), rt.clone());
            rt.spawn("client", move || client(&e, &rt2, &b))
        })
        .collect();
    for t in threads {
        rt.join(t);
    }
    e.stop(flusher);
}

fn faults() -> Faults {
    Faults {
        lose_unsynced: true,
        delay_completions: true,
        ..Faults::default()
    }
}

fn open(rt: SimRuntime, roots: Roots, group_commit: Option<Duration>) -> Arc<Engine> {
    let e = Engine::open(rt, roots, Limits::default()).expect("the database reopens");
    Arc::new(match group_commit {
        Some(interval) => e.with_group_commit(interval),
        None => e,
    })
}

/// Returns the group commit interval of `seed`: none for even seeds, whose commits force the
/// log themselves, and up to two milliseconds for odd ones, so transactions read commits a
/// crash may still lose.
fn group_commit(seed: u64) -> Option<Duration> {
    (!seed.is_multiple_of(2)).then(|| Duration::from_micros(100 + (seed * 2_654_435_761) % 1900))
}

/// Run the workload for one seed, crashing up to `CRASHES` processes, and return its history
/// with a last transaction that reads every key, and the number of processes that crashed.
fn history(seed: u64) -> (Vec<iso::Txn>, u64) {
    let mut sim = Sim::new(
        seed,
        Config {
            faults: faults(),
            ..Config::default()
        },
    );
    let mut roots = None;
    let made = sim.run(|rt| roots = Some(Engine::create(rt, Limits::default()).roots()));
    assert_eq!(Outcome::Finished, made);
    let roots = roots.unwrap();
    let b = Arc::new(Mutex::new(Book::default()));
    let delay = group_commit(seed);
    let mut crashed = 0;
    for round in 0..CRASHES {
        let after = 500 + (seed * 7919 + round * 104_729) % 20_000;
        sim.crash_at(sim.now() + Duration::from_micros(after));
        let outcome = sim.run(|rt| clients(&open(rt, roots, delay), &b));
        if outcome == Outcome::Crashed {
            crashed += 1;
        }
        let mut problems = Vec::new();
        let checked = sim.run(|rt| problems = open(rt, roots, None).verify());
        assert_eq!(Outcome::Finished, checked);
        assert!(
            problems.is_empty(),
            "seed {seed} round {round}: {problems:#?}"
        );
    }
    let mut last = Vec::new();
    let finished = sim.run(|rt| {
        let e = open(rt, roots, delay);
        clients(&e, &b);
        let reader = e.begin(Kind::ReadOnly);
        for key in 0..KEYS {
            let list = e.read(&reader, key).unwrap();
            last.push(Op::Read { key, list });
        }
        e.commit(reader).unwrap();
        assert_eq!(0, e.tracked(), "transactions left tracked");
    });
    assert_eq!(Outcome::Finished, finished);
    let mut txns: Vec<iso::Txn> = std::mem::take(&mut book(&b).txns).into_values().collect();
    txns.push(iso::Txn::new(0, Ended::Committed, last));
    (txns, crashed)
}

#[test]
fn no_seed_of_random_clients_with_crashes_shows_an_anomaly() {
    let (mut crashed, mut committed, mut in_doubt) = (0, 0, 0);
    for seed in 0..SEEDS {
        let (txns, c) = history(seed);
        assert_eq!(Ok(vec![]), iso::check(&txns), "seed {seed}");
        crashed += c;
        committed += txns
            .iter()
            .filter(|t| t.outcome == Ended::Committed)
            .count();
        in_doubt += txns.iter().filter(|t| t.outcome == Ended::InDoubt).count();
    }
    assert!(crashed >= SEEDS, "only {crashed} processes crashed");
    assert!(committed > 10 * SEEDS as usize, "only {committed} commits");
    assert!(in_doubt > 0, "no crash interrupted a commit");
}

#[test]
fn a_seed_replays_the_same_history() {
    assert_eq!(history(7), history(7));
}

/// Seeds of the targeted durability test.
const DURABILITY_SEEDS: u64 = 16;

/// Under a long group commit interval, a writer appends to key 1 and waits for its commit to
/// become durable, readers read the commit while it is visible and not yet durable, and the
/// process crashes before the flusher forces the log. A recovered process then appends to key 1
/// again and reads it.
fn unflushed_reads(seed: u64) -> Vec<iso::Txn> {
    let mut sim = Sim::new(
        seed,
        Config {
            faults: faults(),
            ..Config::default()
        },
    );
    let mut roots = None;
    let made = sim.run(|rt| roots = Some(Engine::create(rt, Limits::default()).roots()));
    assert_eq!(Outcome::Finished, made);
    let roots = roots.unwrap();
    let b = Arc::new(Mutex::new(Book::default()));
    sim.crash_at(sim.now() + Duration::from_millis(100));
    let crashed = sim.run(|rt| {
        let e = open(rt, roots, Some(Duration::from_secs(1)));
        let flusher = e.flusher();
        let rt = &e.rt;
        let (e2, b2) = (e.clone(), b.clone());
        let writer = rt.spawn("writer", move || {
            let id = book(&b2).begin();
            let tx = e2.begin(Kind::ReadWrite);
            let elem = book(&b2).elem();
            e2.append(&tx, 1, elem).unwrap();
            book(&b2).txn(id).ops.push(Op::Append { key: 1, elem });
            book(&b2).txn(id).outcome = Ended::InDoubt;
            e2.commit(tx).unwrap();
            book(&b2).txn(id).outcome = Ended::Committed;
        });
        let readers: Vec<_> = (0..2)
            .map(|_| {
                let (e, b) = (e.clone(), b.clone());
                rt.spawn("reader", move || {
                    loop {
                        let tx = e.begin(Kind::ReadOnly);
                        let list = e.read(&tx, 1).unwrap();
                        if list.is_empty() {
                            e.commit(tx).unwrap();
                            e.rt.yield_now();
                            continue;
                        }
                        let id = book(&b).begin();
                        book(&b).txn(id).ops.push(Op::Read { key: 1, list });
                        book(&b).txn(id).outcome = Ended::InDoubt;
                        e.commit(tx).unwrap();
                        book(&b).txn(id).outcome = Ended::Committed;
                        return;
                    }
                })
            })
            .collect();
        for t in readers.into_iter().chain([writer]) {
            rt.join(t);
        }
        e.stop(flusher);
    });
    assert_eq!(
        Outcome::Crashed,
        crashed,
        "seed {seed}: the flusher forced the log"
    );
    let finished = sim.run(|rt| {
        let e = open(rt, roots, None);
        let id = book(&b).begin();
        let tx = e.begin(Kind::ReadWrite);
        let elem = book(&b).elem();
        e.append(&tx, 1, elem).unwrap();
        let list = e.read(&tx, 1).unwrap();
        e.commit(tx).unwrap();
        let mut b = book(&b);
        let t = b.txn(id);
        t.ops = vec![Op::Append { key: 1, elem }, Op::Read { key: 1, list }];
        t.outcome = Ended::Committed;
    });
    assert_eq!(Outcome::Finished, finished);
    std::mem::take(&mut book(&b).txns).into_values().collect()
}

#[test]
fn a_reader_of_an_unflushed_commit_is_acknowledged_only_once_it_is_durable() {
    for seed in 0..DURABILITY_SEEDS {
        let txns = unflushed_reads(seed);
        let read = txns.iter().any(|t| {
            !t.ops.is_empty()
                && t.ops
                    .iter()
                    .all(|op| matches!(op, Op::Read { list, .. } if !list.is_empty()))
        });
        assert!(read, "seed {seed}: no reader saw the unflushed commit");
        assert_eq!(Ok(vec![]), iso::check(&txns), "seed {seed}");
    }
}
