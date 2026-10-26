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
    let threads: Vec<_> = (0..CLIENTS)
        .map(|_| {
            let (e, b, rt2) = (e.clone(), b.clone(), rt.clone());
            rt.spawn("client", move || client(&e, &rt2, &b))
        })
        .collect();
    for t in threads {
        rt.join(t);
    }
}

fn faults() -> Faults {
    Faults {
        lose_unsynced: true,
        delay_completions: true,
        ..Faults::default()
    }
}

fn open(rt: SimRuntime, roots: Roots) -> Arc<Engine> {
    Arc::new(Engine::open(rt, roots, Limits::default()).expect("the database reopens"))
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
    let mut crashed = 0;
    for round in 0..CRASHES {
        let after = 500 + (seed * 7919 + round * 104_729) % 20_000;
        sim.crash_at(sim.now() + Duration::from_micros(after));
        let outcome = sim.run(|rt| clients(&open(rt, roots), &b));
        if outcome == Outcome::Crashed {
            crashed += 1;
        }
        let mut problems = Vec::new();
        let checked = sim.run(|rt| problems = open(rt, roots).verify());
        assert_eq!(Outcome::Finished, checked);
        assert!(
            problems.is_empty(),
            "seed {seed} round {round}: {problems:#?}"
        );
    }
    let mut last = Vec::new();
    let finished = sim.run(|rt| {
        let e = open(rt, roots);
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
