//! A writer of a row ended at each boundary of its update, by commit, a commit refused as a
//! serialization failure, abort, drop, a drop on unwinding and the unwinding of a call, with a
//! second writer updating or inserting the row's key, and at every pair of their boundaries
//! under the deep profile.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use adtb::runtime::Threads;
use adtb::testing::{Change, Gate, Status, Step, StepFn, Xid};
use adtb::{CallOptions, Error, RejectReason, RetryReason, TxnCallOptions, TxnId, TxnOptions};
use adtb_sim::{Config, Sim};
use adtb_simkit::elle::{self, Elems};
use adtb_simkit::racers::Called;
use adtb_simkit::steps::{self, Pair, Reached, Second, Stop, Unwound};
use adtb_testkit::process;
use adtb_testkit::profile::{self, sweep};

const SEEDS: std::ops::Range<u64> = 0..1;

const COUNTER: &str = "insert counters { id: 1, n: 0 }";
const UPDATE: &str = "update c in counters where c.id == 1 set Counter { c with n: 1 }";

/// The boundaries the first writer's update reaches run alone. A commit refused reaches two
/// more, its scans of the two heaps of `cells`.
const UPDATED: [Step; 6] = [
    Step::VersionRead,
    Step::VersionRead,
    Step::VersionRead,
    Step::WriteChanged(Change::Update { heap_only: true }),
    Step::WriteChecked,
    Step::VersionRead,
];

/// How the first writer ends once its update goes on from its stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum End {
    Commit,
    /// A commit refused as a serialization failure: a reader of the row runs beside it, and it
    /// reads an empty relation a third transaction then inserts into and commits.
    CommitRefused,
    Abort,
    /// A drop of the open transaction.
    Drop,
    /// A drop of the open transaction as its thread unwinds from the stop.
    Unwind,
    /// The unwinding from the stop of a call that runs as its own transaction.
    UnwindCall,
}

const ENDS: [End; 6] = [
    End::Commit,
    End::CommitRefused,
    End::Abort,
    End::Drop,
    End::Unwind,
    End::UnwindCall,
];

/// The second writer's write of the row's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Write {
    Update,
    Insert,
}

impl Write {
    fn text(self) -> &'static str {
        match self {
            Write::Update => "update c in counters where c.id == 1 set Counter { c with n: 2 }",
            Write::Insert => "insert counters { id: 1, n: 2 }",
        }
    }
}

/// What the second writer's write met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Met {
    Wrote,
    LaterCommit,
    Duplicate,
}

/// What one case left.
#[derive(Debug)]
struct Ran {
    reached: [Reached; 2],
    /// The first writer's id, once it stopped.
    first: Option<Xid>,
    /// The id of the writer that ran beside the first, if one did.
    beside: Option<Xid>,
    met: Met,
    /// The commit log's status of the first writer.
    status: Option<Status>,
    /// What the transactions held when the first writer stopped, before the second began.
    at_stop: Holding,
    /// What the transactions held after both ended.
    holding: Holding,
    ends_by_drop: usize,
    verified: Vec<String>,
}

/// The transactions running, the serializer's entries, the ids the waits hold and the ids
/// waited on.
type Holding = (BTreeSet<Xid>, usize, BTreeSet<Xid>, BTreeSet<Xid>);

fn holding(db: &adtb::Database<adtb_sim::SimRuntime>) -> Holding {
    let probe = db.testing();
    let gates = BTreeSet::from([Gate::Offline.xid(), Gate::VacuumHolder.xid()]);
    (
        probe.running(),
        probe.tracked(),
        probe.waits_running().difference(&gates).copied().collect(),
        probe.waited_on(),
    )
}

/// Begin a writer that reads the counter, beside the first writer, and returns it with its id.
fn reader(
    db: &adtb::Database<adtb_sim::SimRuntime>,
) -> (adtb::AdHocTransaction<'_, adtb_sim::SimRuntime>, Xid) {
    let mut reader = db.begin(&TxnOptions::default()).expect("the reader begins");
    let x = *db.testing().running().last().expect("the reader runs");
    reader
        .query_text("counters", &[], &TxnCallOptions::default())
        .expect("the reader reads the counter");
    reader
        .query_text(
            "insert elems { k: 1, pos: 0, e: 0 }",
            &[],
            &TxnCallOptions::default(),
        )
        .expect("the reader writes");
    (reader, x)
}

/// Refuse the commit of `tx`, which updated the counter a running [`reader`] read: `tx` reads the
/// empty `cells`, and another transaction, on a thread of its own, inserts a cell and commits.
fn refused(
    rt: &adtb_sim::SimRuntime,
    db: &adtb::Database<adtb_sim::SimRuntime>,
    mut tx: adtb::AdHocTransaction<'_, adtb_sim::SimRuntime>,
) {
    tx.query_text("cells", &[], &TxnCallOptions::default())
        .expect("the first writer reads the cells");
    let other = db.clone();
    let inserted = rt.spawn("cell writer", move || {
        other
            .query_text(
                "insert cells.Even { id: 1, n: 0, m: 0, pad: \"\" }",
                &[],
                &CallOptions::default(),
            )
            .expect("a cell inserts");
    });
    rt.join(inserted);
    let refusal = Error::Retry {
        reason: RetryReason::Serialization {
            txn: tx.id(),
            on: "elle::cells.Even".to_owned(),
        },
    };
    assert_eq!(Err(refusal), tx.commit(None).map(drop));
}

fn of_a_statement(s: Step) -> bool {
    matches!(s.function(), StepFn::Read | StepFn::Write)
}

/// Returns what `got`, the reply to the second writer's write as `id`, met.
fn met(id: TxnId, got: Result<(), Error>) -> Met {
    let later = Error::Retry {
        reason: RetryReason::Serialization {
            txn: id,
            on: "elle::counters.Counter".to_owned(),
        },
    };
    let duplicate = Error::Rejected {
        reason: RejectReason::KeyViolation {
            relation: "elle::counters".to_owned(),
            key: "counters.key(1)".to_owned(),
        },
    };
    match got {
        Ok(()) => Met::Wrote,
        Err(e) if e == later => Met::LaterCommit,
        Err(e) if e == duplicate => Met::Duplicate,
        Err(e) => panic!("the second writer's write fails otherwise: {e:?}"),
    }
}

/// Run the first writer's update to its `i`th boundary and the second's write to its `j`th, then
/// end the first by `end`.
fn run(seed: u64, (end, write): (End, Write), (i, j): (usize, usize)) -> Ran {
    let mut sim = Sim::with_files(seed, Config::default(), elle::created());
    process::run(&mut sim, |rt| {
        let e = Arc::new(Elems::open(&rt, &elle::options()).expect("the workload opens"));
        let db = e.database().clone();
        db.query_text(COUNTER, &[], &CallOptions::default())
            .expect("the counter inserts");
        let beside = (end == End::CommitRefused).then(|| reader(&db));
        let first_db = db.clone();
        let first_rt = rt.clone();
        let ender = move || match end {
            End::UnwindCall => {
                let call = catch_unwind(AssertUnwindSafe(|| {
                    first_db.query_text(UPDATE, &[], &CallOptions::default())
                }));
                unwound(call.map(drop));
            }
            _ => {
                let mut tx = first_db
                    .begin(&TxnOptions::default())
                    .expect("the first writer begins");
                if end == End::Unwind {
                    let updated = catch_unwind(AssertUnwindSafe(move || {
                        tx.query_text(UPDATE, &[], &TxnCallOptions::default())
                            .map(drop)
                    }));
                    unwound(updated.map(drop));
                    return;
                }
                tx.query_text(UPDATE, &[], &TxnCallOptions::default())
                    .expect("the first writer updates");
                match end {
                    End::Commit => drop(tx.commit(None).expect("the first writer commits")),
                    End::CommitRefused => refused(&first_rt, &first_db, tx),
                    End::Abort => tx.rollback().expect("the first writer aborts"),
                    _ => drop(tx),
                }
            }
        };
        let mut first = Called::spawn(&rt, ("first writer", of_a_statement), ender);
        if matches!(end, End::Unwind | End::UnwindCall) {
            first = first.unwinding();
        }
        let second_db = db.clone();
        let mut second = Called::spawn(&rt, ("second writer", of_a_statement), move || {
            let mut tx = second_db
                .begin(&TxnOptions::default())
                .expect("the second writer begins");
            let id = tx.id();
            let stop = TxnCallOptions {
                limit: Some(Duration::from_secs(10)),
                ..TxnCallOptions::default()
            };
            let got = tx.query_text(write.text(), &[], &stop).map(drop);
            if got.is_ok() {
                tx.rollback().expect("the second writer aborts");
            }
            (id, got)
        });
        let mut at_stop = None;
        let reached = steps::interleave(
            &rt,
            &db,
            (&mut first, &mut second),
            (i, j),
            (Box::new(|| at_stop = Some(holding(&db))), Box::new(|_| {})),
        );
        let (id, got) = second.take();
        let beside = beside.map(|(reader, x)| {
            reader.rollback().expect("the reader aborts");
            x
        });
        let probe = db.testing();
        let at_stop = at_stop.unwrap_or_default();
        let stopped = at_stop.0.last().copied();
        Ran {
            reached,
            first: stopped,
            beside,
            met: met(id, got),
            status: stopped.map(|x| probe.status(x).expect("its status")),
            at_stop,
            holding: holding(&db),
            ends_by_drop: probe.take_ends_by_drop(),
            verified: e.verify(),
        }
    })
}

/// Check that `ended` unwound from a stop, if it did not run whole.
fn unwound(ended: std::thread::Result<()>) {
    if let Err(payload) = ended {
        assert!(
            payload.downcast_ref::<Unwound>().is_some(),
            "the first writer unwinds from its stop, not a panic of its own"
        );
    }
}

/// Returns what the second writer's `write` meets once the first ended by `end`, `before`
/// saying whether the second ended, or changed the row, before the first went on from its stop.
fn expected(write: Write, end: End, before: bool) -> Met {
    match (end, write) {
        (End::Commit, _) if !before => Met::LaterCommit,
        (_, Write::Update) => Met::Wrote,
        (_, Write::Insert) => Met::Duplicate,
    }
}

/// Returns each way `r`, a run at `pair`, breaks the end of the first writer: the second writer
/// proceeds or fails as the first's end decides, within its stop; the first's end is recorded
/// unless it unwound from a call, and the call's alone counts as an end by drop; at its stop the
/// first writer, with the writer beside it, is running, in the serializer and in the waits;
/// nothing is left running, in the serializer or waited on once both end; and `verify` is clean.
fn judged((end, write): (End, Write), pair: (usize, usize), r: &Ran) -> Vec<String> {
    let mut broken = Vec::new();
    let mut expect = |what: &str, ok: bool, got: &dyn std::fmt::Debug| {
        if !ok {
            broken.push(format!("{end:?} {write:?} at {pair:?}: {what}: {got:?}"));
        }
    };
    let [a, b] = &r.reached;
    if !a.stopped() {
        return broken;
    }
    let j = if profile::every_case() {
        pair.1
    } else {
        usize::MAX
    };
    let changed = (b.steps().iter()).any(|s| matches!(s, Step::WriteChanged(_)));
    let before = b.second(j) == Second::Whole || changed;
    let met = expected(write, end, before);
    expect("the second writer's write", r.met == met, &r.met);
    let status = match end {
        End::Commit => Status::Committed,
        End::CommitRefused | End::Abort | End::Drop | End::Unwind => Status::Aborted,
        End::UnwindCall => Status::InProgress,
    };
    expect(
        "the first writer's status",
        r.status == Some(status),
        &r.status,
    );
    let drops = usize::from(end == End::UnwindCall);
    expect("the ends by drop", r.ends_by_drop == drops, &r.ends_by_drop);
    let none = (BTreeSet::new(), 0, BTreeSet::new(), BTreeSet::new());
    let ids = r.first.into_iter().chain(r.beside).collect::<BTreeSet<_>>();
    let one = (ids.clone(), ids.len(), ids, BTreeSet::new());
    expect("what is held at the stop", r.at_stop == one, &r.at_stop);
    expect("what is held", r.holding == none, &r.holding);
    expect("verify", r.verified.is_empty(), &r.verified);
    expect("the first writer's id", r.first.is_some(), &r.first);
    broken
}

/// End the first writer by each end at each boundary of its update, with the second writer run
/// whole or blocked, or under the deep profile at every pair. Check that the first stopped at
/// each boundary of its update for every end, and that the second waited on it exactly from its
/// write on.
#[test]
fn a_writer_waiting_on_one_ended_at_each_boundary_proceeds_or_fails_as_its_end_decides() {
    let mut broken = Vec::new();
    let mut reached: BTreeMap<(End, Write), Vec<Step>> = BTreeMap::new();
    for seed in sweep(SEEDS) {
        for end in ENDS {
            for write in [Write::Update, Write::Insert] {
                let mut one = |pair| {
                    let r = run(seed, (end, write), pair);
                    broken.extend(judged((end, write), pair, &r));
                    r.reached
                };
                let (stops, pairs) = if profile::every_case() {
                    steps::every_pair(&mut one)
                } else {
                    steps::every_boundary(&mut one)
                };
                let mut expected = UPDATED.to_vec();
                if end == End::CommitRefused {
                    expected.extend([Step::ScanRead; 2]);
                }
                assert_eq!(expected, stops, "{end:?} {write:?}: the first's stops");
                if !profile::every_case() {
                    let each: Vec<Pair> = expected
                        .iter()
                        .enumerate()
                        .map(|(n, &s)| {
                            let written = n >= 3;
                            (
                                s,
                                if written {
                                    Second::Blocked
                                } else {
                                    Second::Whole
                                },
                            )
                        })
                        .collect();
                    assert_eq!(each, pairs, "{end:?} {write:?}: the pairs run");
                }
                reached.insert((end, write), stops);
            }
        }
    }
    assert_eq!(Vec::<String>::new(), broken);
    assert_eq!(ENDS.len() * 2, reached.len(), "every end with every write");
}

/// A call that updates a row and unwinds after its write leaves its id in progress in the commit
/// log; a transaction begun after it counts the call aborted and updates the row, or is refused
/// the row's key as a duplicate of the version the call left, and `verify` is clean.
#[test]
fn a_write_after_a_call_unwound_past_its_write_counts_the_call_aborted() {
    for write in [Write::Update, Write::Insert] {
        let mut sim = Sim::with_files(0, Config::default(), elle::created());
        process::run(&mut sim, |rt| {
            let e = Arc::new(Elems::open(&rt, &elle::options()).expect("the workload opens"));
            let db = e.database().clone();
            db.query_text(COUNTER, &[], &CallOptions::default())
                .expect("the counter inserts");
            let written = |s: Step| s == Step::WriteChecked;
            let stop = Stop::set(&rt, &db, (None, written), 1);
            let caller = db.clone();
            let call = rt.spawn("caller", move || {
                let call = catch_unwind(AssertUnwindSafe(|| {
                    caller.query_text(UPDATE, &[], &CallOptions::default())
                }));
                unwound(call.map(drop));
            });
            assert_eq!(
                Ok(Step::WriteChecked),
                stop.stopped(&rt, &mut || None::<()>)
            );
            let x = *db.testing().running().first().expect("the call runs");
            stop.unwind(&rt);
            rt.join(call);
            assert_eq!(Ok(Status::InProgress), db.testing().status(x));
            assert_eq!(1, db.testing().take_ends_by_drop());
            let mut tx = db.begin(&TxnOptions::default()).expect("a writer begins");
            let id = tx.id();
            let got = tx.query_text(write.text(), &[], &TxnCallOptions::default());
            let expected = match write {
                Write::Update => Met::Wrote,
                Write::Insert => Met::Duplicate,
            };
            assert_eq!(expected, met(id, got.map(drop)), "{write:?}");
            drop(tx);
            assert_eq!(Vec::<String>::new(), e.verify(), "{write:?}");
        });
    }
}

#[test]
fn a_vacuum_after_a_call_unwound_past_its_write_frees_its_version_and_passes_its_id() {
    let mut sim = Sim::with_files(0, Config::default(), elle::created());
    process::run(&mut sim, |rt| {
        let e = Arc::new(Elems::open(&rt, &elle::options()).expect("the workload opens"));
        let db = e.database().clone();
        db.query_text(COUNTER, &[], &CallOptions::default())
            .expect("the counter inserts");
        let written = |s: Step| s == Step::WriteChecked;
        let stop = Stop::set(&rt, &db, (None, written), 1);
        let caller = db.clone();
        let call = rt.spawn("caller", move || {
            let call = catch_unwind(AssertUnwindSafe(|| {
                caller.query_text(UPDATE, &[], &CallOptions::default())
            }));
            unwound(call.map(drop));
        });
        assert_eq!(
            Ok(Step::WriteChecked),
            stop.stopped(&rt, &mut || None::<()>)
        );
        let x = *db.testing().running().first().expect("the call runs");
        stop.unwind(&rt);
        rt.join(call);
        assert_eq!(Ok(Status::InProgress), db.testing().status(x));
        assert_eq!(1, db.testing().take_ends_by_drop());
        let report = db
            .testing()
            .vacuum("elle::counters")
            .expect("the vacuum runs");
        assert_eq!(
            (1, 1, db.testing().next_xid()),
            (report.freed, report.cleared, report.clean_below)
        );
        assert_eq!(Vec::<String>::new(), e.verify());
    });
}
