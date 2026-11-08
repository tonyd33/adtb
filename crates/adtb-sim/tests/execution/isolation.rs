use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use adtb::runtime::{Entropy, OpKind, Threads};
use adtb_isolation::{self as iso, Op, Outcome as Ended};
use adtb_sim::{Action, Config, Ledger, Match, Plan, Sim, SimRuntime};
use adtb_simkit::api;
use adtb_simkit::crash::{Crashes, Reopen, Rounds, crash_loop};
use adtb_simkit::elle::{self, Elems, Kind};
use adtb_simkit::open::FILE;
use adtb_simkit::seeds::{assert_reached, share, sweep};
use adtb_sys::runtime::Faults;

/// The seeds one run checks. Milestone 6 exits when every one of them passes.
const SEEDS: Range<u64> = 0..8;
const CLIENTS: usize = 4;
const TXNS_PER_CLIENT: usize = 6;
/// Processes per seed that may crash before the last, which runs to the end.
const CRASHES: usize = 3;
const KEYS: i64 = 4;

/// What clients saw, kept across the crashes of the processes they ran in.
#[derive(Debug, Default)]
struct Book {
    txns: BTreeMap<u64, iso::Txn>,
    next_txn: u64,
    next_elem: i64,
    /// The transaction that appended each element.
    writers: BTreeMap<i64, u64>,
    /// The process each transaction ran in, by the crash loop's round.
    processes: BTreeMap<u64, usize>,
    process: usize,
    /// The elements of transactions left in doubt that a reopen did not hold.
    lost: BTreeSet<i64>,
}

fn book(b: &Mutex<Book>) -> MutexGuard<'_, Book> {
    b.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Book {
    /// Start recording a transaction as aborted, which it is if its process crashes before it
    /// submits its commit.
    fn begin(&mut self) -> u64 {
        self.next_txn += 1;
        let id = self.next_txn;
        self.txns
            .insert(id, iso::Txn::new(id, Ended::Aborted, Vec::new()));
        self.processes.insert(id, self.process);
        id
    }

    fn txn(&mut self, id: u64) -> &mut iso::Txn {
        self.txns.get_mut(&id).expect("the transaction began")
    }

    /// Returns a new element for transaction `id` to append.
    fn elem(&mut self, id: u64) -> i64 {
        self.next_elem += 1;
        self.writers.insert(self.next_elem, id);
        self.next_elem
    }

    /// Returns the transactions that appended an element of `lists`.
    fn writers_of(&self, lists: &[Vec<i64>]) -> BTreeSet<u64> {
        lists.iter().flatten().map(|e| self.writers[e]).collect()
    }

    /// Record as lost each element of a transaction in doubt that `lists` do not hold.
    fn lose_absent(&mut self, lists: &[Vec<i64>]) {
        let held: BTreeSet<i64> = lists.iter().flatten().copied().collect();
        let lost: Vec<i64> = self
            .writers
            .iter()
            .filter(|(e, w)| self.txns[w].outcome == Ended::InDoubt && !held.contains(e))
            .map(|(e, _)| *e)
            .collect();
        self.lost.extend(lost);
    }

    /// Returns how many transactions read an element another transaction of their process
    /// appended and the crash lost.
    fn reads_lost(&self) -> usize {
        let lost = |reader: u64, e: &i64| {
            let writer = self.writers[e];
            writer != reader
                && self.lost.contains(e)
                && self.processes[&writer] == self.processes[&reader]
        };
        self.txns
            .values()
            .filter(|t| {
                t.ops.iter().any(|op| match op {
                    Op::Read { list, .. } => list.iter().any(|e| lost(t.id.0, e)),
                    Op::Append { .. } => false,
                })
            })
            .count()
    }
}

/// Commit `tx` for book transaction `id`, recording its fate in the book and, when it appended,
/// in `ledger`.
fn commit(tx: elle::Txn<'_>, id: u64, kind: Kind, b: &Mutex<Book>, ledger: &Ledger) {
    let wrote = {
        let mut b = book(b);
        let t = b.txn(id);
        t.outcome = Ended::InDoubt;
        t.ops.iter().any(|op| matches!(op, Op::Append { .. }))
    };
    if wrote {
        ledger.submit(id);
    }
    let outcome = match tx.commit(None) {
        Ok(_) => Ended::Committed,
        Err(f) => {
            assert_ne!(Kind::Deferrable, kind, "a deferrable commit failed: {f:?}");
            elle::aborted(f);
            Ended::Aborted
        }
    };
    if wrote {
        match outcome {
            Ended::Committed => ledger.acknowledge(id),
            _ => ledger.abort(id),
        }
    }
    book(b).txn(id).outcome = outcome;
}

fn client(e: &Elems, rt: &SimRuntime, b: &Mutex<Book>, ledger: &Ledger) {
    for _ in 0..TXNS_PER_CLIENT {
        let kind = match rt.next_u64() % 10 {
            0 => Kind::Deferrable,
            1 => Kind::ReadOnly,
            _ => Kind::ReadWrite,
        };
        let id = book(b).begin();
        let mut tx = e.begin(kind).expect("a transaction begins");
        let mut failed = false;
        for _ in 0..=(rt.next_u64() % 4) {
            rt.yield_now();
            let key = (rt.next_u64() % KEYS as u64) as i64;
            let op = if kind != Kind::ReadWrite || rt.next_u64().is_multiple_of(2) {
                e.read(&mut tx, key).map(|list| Op::Read { key, list })
            } else {
                let elem = book(b).elem(id);
                e.append(&mut tx, key, elem)
                    .map(|()| Op::Append { key, elem })
            };
            match op {
                Ok(op) => book(b).txn(id).ops.push(op),
                Err(f) => {
                    assert_ne!(Kind::Deferrable, kind, "a deferrable transaction failed");
                    elle::aborted(f);
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            continue;
        }
        rt.yield_now();
        commit(tx, id, kind, b, ledger);
    }
}

fn clients(rt: &SimRuntime, e: &Arc<Elems>, b: &Arc<Mutex<Book>>, ledger: &Ledger) {
    let threads: Vec<_> = (0..CLIENTS)
        .map(|_| {
            let (e, b, l, rt2) = (e.clone(), b.clone(), ledger.clone(), rt.clone());
            rt.spawn("client", move || client(&e, &rt2, &b, &l))
        })
        .collect();
    for t in threads {
        rt.join(t);
    }
}

fn faults() -> Faults {
    Faults {
        lose_unsynced: true,
        ..Faults::none(512)
    }
}

fn opened(rt: &SimRuntime) -> Arc<Elems> {
    Arc::new(Elems::open(rt, &elle::options()).expect("the database opens"))
}

/// Returns the reopened database after a crash: the transactions whose appends it holds and
/// what `verify` finds.
fn recovered(rt: &SimRuntime, b: &Mutex<Book>) -> Reopen {
    let e = opened(rt);
    let lists = e.lists(0..KEYS);
    let mut b = book(b);
    b.lose_absent(&lists);
    Reopen::new(b.writers_of(&lists), e.verify())
}

/// Returns a new simulation of `seed` holding a new workload database, its ops completing in 0.1
/// to 2 ms.
fn created(seed: u64) -> Sim {
    let config = Config {
        faults: faults(),
        latency: (Duration::from_micros(100), Duration::from_millis(2)),
        ..Config::default()
    };
    Sim::with_files(seed, config, elle::created())
}

/// What one seed's history search saw.
#[derive(Debug, PartialEq, Eq)]
struct Searched {
    txns: Vec<iso::Txn>,
    crashed: usize,
    reads_lost: usize,
}

/// Run the workload for one seed, crashing up to `CRASHES` processes, and return its history
/// with a last transaction that reads every key.
fn history(seed: u64) -> Searched {
    let mut sim = created(seed);
    let b = Arc::new(Mutex::new(Book::default()));
    let out = crash_loop(
        &mut sim,
        seed,
        faults(),
        Rounds::new(CRASHES, 2_000, 40_000),
        |r| {
            book(&b).process = r.index();
            let (l, b) = (r.ledger().clone(), b.clone());
            r.crash_after_setup(opened, |rt, e| clients(&rt, &e, &b, &l));
        },
        |rt, _| recovered(&rt, &b),
    );
    book(&b).process = CRASHES;
    let last = api::run(&mut sim, |rt| {
        let e = opened(&rt);
        clients(&rt, &e, &b, out.ledger());
        let lists = e.lists(0..KEYS);
        assert_eq!(0, e.database().testing().tracked(), "seed {seed}");
        (0..KEYS)
            .zip(lists)
            .map(|(key, list)| Op::Read { key, list })
            .collect()
    });
    let b = book(&b);
    let mut txns: Vec<iso::Txn> = b.txns.values().cloned().collect();
    txns.push(iso::Txn::new(0, Ended::Committed, last));
    Searched {
        txns,
        crashed: out.crashed(),
        reads_lost: b.reads_lost(),
    }
}

#[test]
fn no_seed_of_random_clients_with_crashes_shows_an_anomaly() {
    let (mut crashed, mut committed, mut in_doubt, mut reads) = (0, 0, 0, 0);
    for seed in sweep(SEEDS) {
        let s = history(seed);
        assert_eq!(Ok(vec![]), iso::check(&s.txns), "seed {seed}");
        crashed += s.crashed;
        committed += s
            .txns
            .iter()
            .filter(|t| t.outcome == Ended::Committed)
            .count();
        in_doubt += s
            .txns
            .iter()
            .filter(|t| t.outcome == Ended::InDoubt)
            .count();
        reads += s.reads_lost;
    }
    eprintln!("{crashed} crashes, {committed} commits, {in_doubt} in doubt, {reads} reads");
    assert_reached("a crashed process", crashed, share(SEEDS, 12));
    assert_reached("a commit", committed, share(SEEDS, 300));
    assert_reached("a crash interrupting a commit", in_doubt, share(SEEDS, 20));
    assert_reached("a read of a commit the crash lost", reads, share(SEEDS, 3));
}

#[test]
fn a_seed_replays_the_same_history() {
    assert_eq!(history(7), history(7));
}

/// Write out `e`'s log, then hold every later force of it. No record appended before the call
/// waits on a held force.
fn hold_log_forces(rt: &SimRuntime, e: &Elems) {
    e.database().testing().flush_log();
    rt.plan(Plan::new().rule(
        Match::kind(OpKind::Sync).prefixed(&format!("{FILE}-log")),
        Action::Hold,
    ));
}

/// Seeds of the targeted durability test.
const DURABILITY_SEEDS: Range<u64> = 0..4;

/// A writer appends to key 1 and writes the log. Every later force of the log is held, and once
/// a reader has committed an empty read the writer commits, so its commit is visible and never
/// durable; readers read it meanwhile, and the process crashes. A recovered process then appends
/// to key 1 again and reads it. Returns the history, the crash loop's result and how many
/// readers committed an empty read under the hold.
fn unflushed_reads(seed: u64) -> (Vec<iso::Txn>, Crashes<Reopen>, usize) {
    let mut sim = created(seed);
    let b = Arc::new(Mutex::new(Book::default()));
    let empty = Arc::new(AtomicUsize::new(0));
    let out = crash_loop(
        &mut sim,
        seed,
        faults(),
        Rounds::new(2, 50_000, 1),
        |r| {
            let (l, b) = (r.ledger().clone(), b.clone());
            if r.index() == 1 {
                return api::run(r.sim(), |rt| {
                    let e = opened(&rt);
                    let id = book(&b).begin();
                    let mut tx = e.begin(Kind::ReadWrite).unwrap();
                    let elem = book(&b).elem(id);
                    e.append(&mut tx, 1, elem).unwrap();
                    let list = e.read(&mut tx, 1).unwrap();
                    book(&b).txn(id).ops =
                        vec![Op::Append { key: 1, elem }, Op::Read { key: 1, list }];
                    commit(tx, id, Kind::ReadWrite, &b, &l);
                });
            }
            r.crash_after_setup(opened, |rt, e| {
                let id = book(&b).begin();
                let mut tx = e.begin(Kind::ReadWrite).unwrap();
                let elem = book(&b).elem(id);
                e.append(&mut tx, 1, elem).unwrap();
                book(&b).txn(id).ops.push(Op::Append { key: 1, elem });
                hold_log_forces(&rt, &e);
                for _ in 0..2 {
                    let (e, b, l, rt2) = (e.clone(), b.clone(), l.clone(), rt.clone());
                    let empty = empty.clone();
                    rt.spawn("reader", move || {
                        loop {
                            let mut tx = e.begin(Kind::ReadOnly).unwrap();
                            let list = e.read(&mut tx, 1).unwrap();
                            if list.is_empty() {
                                tx.commit(None).unwrap();
                                empty.fetch_add(1, Ordering::Relaxed);
                                rt2.yield_now();
                                continue;
                            }
                            let id = book(&b).begin();
                            book(&b).txn(id).ops.push(Op::Read { key: 1, list });
                            return commit(tx, id, Kind::ReadOnly, &b, &l);
                        }
                    });
                }
                while empty.load(Ordering::Relaxed) == 0 {
                    rt.yield_now();
                }
                commit(tx, id, Kind::ReadWrite, &b, &l);
            });
        },
        |rt, _| recovered(&rt, &b),
    );
    let txns = book(&b).txns.values().cloned().collect();
    (txns, out, empty.load(Ordering::Relaxed))
}

#[test]
fn a_reader_of_an_unflushed_commit_is_acknowledged_only_once_it_is_durable() {
    for seed in sweep(DURABILITY_SEEDS) {
        let (txns, out, empty) = unflushed_reads(seed);
        assert_reached(
            &format!("seed {seed}: an acknowledged empty read under the hold"),
            empty,
            1,
        );
        assert!(
            out.rounds()[0].crashed(),
            "seed {seed}: the writer's commit was forced"
        );
        let readers: Vec<&iso::Txn> = txns
            .iter()
            .filter(|t| {
                !t.ops.is_empty()
                    && t.ops
                        .iter()
                        .all(|op| matches!(op, Op::Read { list, .. } if !list.is_empty()))
            })
            .collect();
        assert!(
            !readers.is_empty(),
            "seed {seed}: no reader saw the unflushed commit"
        );
        for r in readers {
            assert_eq!(
                Ended::InDoubt,
                r.outcome,
                "seed {seed}: a reader was acknowledged"
            );
        }
        assert_eq!(Ok(vec![]), iso::check(&txns), "seed {seed}");
    }
}
