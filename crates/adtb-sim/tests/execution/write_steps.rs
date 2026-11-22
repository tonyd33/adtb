use std::ops::Range;
use std::sync::Arc;

use adtb::runtime::Threads;
use adtb::testing::{Change, Step};
use adtb::{OpenOptions, TxnCallOptions};
use adtb_sim::SimRuntime;
use adtb_simkit::elle::{self, Elems, Kind, Reply, Req, Worker, simulate_with};
use adtb_simkit::steps::Stop;
use adtb_testkit::isolation::{self as iso, Outcome};
use adtb_testkit::seeds::{assert_reached, sweep};

/// A write of a row of `cells`: of its existing version, or an insert of a row it lacks.
#[derive(Clone, Copy, Debug)]
enum Write {
    Delete,
    /// An update of an unindexed field, which stays on its page.
    HotUpdate,
    /// An update of an indexed field.
    ColdUpdate,
    Retag,
    Insert,
}

const WRITES: [Write; 5] = [
    Write::Delete,
    Write::HotUpdate,
    Write::ColdUpdate,
    Write::Retag,
    Write::Insert,
];

/// A read of one row of `cells`: by its key, by a scan of the whole heap, or by a scan that stops
/// at the row while it has its first value.
#[derive(Clone, Copy, Debug)]
enum Read {
    Key,
    Scan,
    Early,
}

const READS: [Read; 3] = [Read::Key, Read::Scan, Read::Early];

/// The value a read gives a row: 0 before any write, more after any write but a delete.
const VALUE: &str = "c.n + c.m + (match c | Even e -> 0 | Odd o -> 1)";

impl Write {
    /// Returns the step after which the write has changed its page and is to check the SIREAD
    /// locks it meets, and the step it reaches next when it has one.
    fn changed(self) -> (Step, Option<Step>) {
        let change = match self {
            Write::Delete => Change::Delete,
            Write::HotUpdate => Change::Update { heap_only: true },
            Write::ColdUpdate => Change::Update { heap_only: false },
            Write::Retag => Change::Retag,
            Write::Insert => return (Step::WriteInserted, None),
        };
        (Step::WriteChanged(change), Some(Step::WriteChecked))
    }

    /// Returns the values a read gives a row before the write.
    fn first(self) -> &'static [i64] {
        match self {
            Write::Insert => &[],
            _ => &[0],
        }
    }

    /// Returns the reads that meet the write: a read by key or by an early scan of a row it
    /// lacks reaches no step of its own.
    fn reads(self) -> &'static [Read] {
        match self {
            Write::Insert => &[Read::Scan],
            _ => &READS,
        }
    }

    fn of(self, id: i64) -> Req {
        Req::Text(match self {
            Write::Delete => format!("delete c in cells where c.id == {id}"),
            Write::HotUpdate => {
                format!("update c in cells.Even where c.id == {id} set Even {{ c with n: 1 }}")
            }
            Write::ColdUpdate => {
                format!("update c in cells.Even where c.id == {id} set Even {{ c with m: 1 }}")
            }
            Write::Retag => format!("update c in cells.Even where c.id == {id} set Odd {{}}"),
            Write::Insert => format!("insert cells.Even {{ id: {id}, n: 1, m: 0, pad: \"\" }}"),
        })
    }
}

impl Read {
    /// Returns the request reading row `id`.
    fn row(self, id: i64) -> Req {
        Req::Text(match self {
            Read::Key => format!("from c in cells where c.id == {id} select {VALUE}"),
            Read::Scan => format!("from c in cells select c.id * 10 + {VALUE}"),
            Read::Early => {
                let other = A + B - id;
                format!(
                    "if (exists c in cells where c.id != {other} \
                     and (match c | Even e -> e.n == 0 and e.m == 0 | Odd o -> false)) \
                     then 0 else 1"
                )
            }
        })
    }

    /// Returns the values of row `id` in `reply`, the rows the request of [`Read::row`] gave.
    fn values(self, id: i64, reply: Vec<i64>) -> Vec<i64> {
        match self {
            Read::Key | Read::Early => reply,
            Read::Scan => reply
                .into_iter()
                .filter(|v| v / 10 == id)
                .map(|v| v % 10)
                .collect(),
        }
    }
}

/// The rows the two transactions read and write, and the list keys that stand for them.
const A: i64 = 1;
const B: i64 = 2;

/// Returns the workload's options with a scan passing one row at a time, so that a scan that
/// stops early stops on the page of the row that decides it.
fn options() -> OpenOptions {
    let mut options = elle::options();
    options.testing = std::mem::take(&mut options.testing).with_morsel_rows(Some(1));
    options
}

/// The rows of `cells` that stand between A and B in its heap, so that each is on a page of its
/// own, and that no read of A or B gives.
const FILLER: Range<i64> = 100..110;

/// Returns the list a read of a row of `cells` written at most once by `write`, the append of
/// `elem`, stands for: empty while the read gives the row's first values.
fn list(values: &[i64], write: Write, elem: i64) -> Vec<i64> {
    match values {
        v if v == write.first() => vec![],
        [] | [_] => vec![elem],
        v => panic!("a read of one row gives at most one value: {v:?}"),
    }
}

/// Insert and commit the rows `write` finds: A, then the filler, then B, A and B only when the
/// write is not an insert.
fn fill(rt: &SimRuntime, e: &Arc<Elems>, write: Write) {
    let setup = Worker::start(rt, e, Kind::ReadWrite);
    let row = |id, n, pad: &str| format!("{{ id: {id}, n: {n}, m: 0, pad: \"{pad}\" }}");
    let pad = "x".repeat(1500);
    let filler: Vec<String> = FILLER.map(|id| row(id, 5, &pad)).collect();
    let mut rows = vec![format!("[{}]", filler.join(", "))];
    if write.first() == [0] {
        rows.insert(0, format!("[{}]", row(A, 0, "")));
        rows.push(format!("[{}]", row(B, 0, "")));
    }
    for rows in rows {
        let text = format!("insert cells.Even from ({rows})");
        setup
            .call(rt, Req::Text(text))
            .expect("rows of cells insert");
    }
    setup.call(rt, Req::Commit).expect("the rows commit");
    rt.join(setup.thread());
}

/// One side of the write skew: a transaction that reads one row and writes the other, writing
/// the list element its id names.
struct Side {
    worker: Worker,
    id: u64,
    write: Write,
    ops: Vec<iso::Op>,
    aborted: bool,
}

impl Side {
    fn start(rt: &SimRuntime, e: &Arc<Elems>, id: u64, write: Write) -> Side {
        Side {
            worker: Worker::start(rt, e, Kind::ReadWrite),
            id,
            write,
            ops: Vec::new(),
            aborted: false,
        }
    }

    /// Returns the rows of `r`, `None` when it failed with a serialization failure on one of `on`.
    fn failed(&mut self, r: Reply, on: &'static [&'static str]) -> Option<Vec<i64>> {
        match r {
            Ok(v) => Some(v),
            Err(f) => {
                elle::aborted(self.worker.id(), on)(f);
                self.aborted = true;
                None
            }
        }
    }

    /// Read row `key` by `read`, the other side writing `elem` to it.
    fn read(&mut self, rt: &SimRuntime, read: Read, key: i64, elem: i64) {
        if !self.aborted {
            let reply = self.worker.call(rt, read.row(key));
            self.was_read(reply, read, key, elem);
        }
    }

    /// Record the reply of a read of row `key` by `read`, the other side writing `elem` to it.
    fn was_read(&mut self, reply: Reply, read: Read, key: i64, elem: i64) {
        if let Some(v) = self.failed(reply, elle::READ) {
            let list = list(&read.values(key, v), self.write, elem);
            self.ops.push(iso::Op::Read { key, list });
        }
    }

    /// Record the reply of a write of row `key`.
    fn wrote(&mut self, reply: Reply, key: i64) {
        if self.failed(reply, elle::CELL_WRITE).is_some() {
            let elem = self.id as i64;
            self.ops.push(iso::Op::Append { key, elem });
        }
    }

    fn write(&mut self, rt: &SimRuntime, write: Write, key: i64) {
        if !self.aborted {
            let reply = self.worker.call(rt, write.of(key));
            self.wrote(reply, key);
        }
    }

    fn end(mut self, rt: &SimRuntime) -> iso::Txn {
        if !self.aborted {
            let reply = self.worker.call(rt, Req::Commit);
            self.failed(reply, elle::READ);
        }
        rt.join(self.worker.thread());
        let outcome = if self.aborted {
            Outcome::Aborted
        } else {
            Outcome::Committed
        };
        iso::Txn::new(self.id, outcome, self.ops)
    }
}

/// Where the read of side 1 and the write of side 2 meet.
#[derive(Clone, Copy, Debug)]
enum At {
    /// The read runs whole before the write begins.
    Before,
    /// The read runs whole with the writer stopped at the `n`th step it reaches, from 1.
    Writer(usize),
    /// The write runs whole and side 2 commits with the reader stopped at the `n`th step it
    /// reaches, from 1.
    Reader(usize),
}

/// What one history showed of the steps of the side stopped.
struct Steps {
    /// The step the side stopped at, `None` when it ended its statement first.
    stopped: Option<Step>,
    /// Every step the side reached in its statement.
    reached: Vec<Step>,
}

fn of_a_write(step: Step) -> bool {
    matches!(
        step,
        Step::WriteChanged(_) | Step::WriteChecked | Step::WriteInserted
    )
}

/// Returns side 2's write of A with `at`'s stop set on it, and what it showed.
fn stopped_write(
    rt: &SimRuntime,
    e: &Arc<Elems>,
    sides: (&mut Side, &mut Side),
    w: Write,
    read: Read,
    n: usize,
) -> Steps {
    let (one, two) = sides;
    let stop = Stop::set(rt, e.database(), (two.worker.thread(), of_a_write), n);
    let mut stopped = None;
    if !two.aborted {
        two.worker.send(rt, w.of(A));
        match stop.stopped(rt, &mut || two.worker.try_reply()) {
            Ok(step) => {
                stopped = Some(step);
                one.read(rt, read, A, 2);
                stop.release(rt);
                let reply = two.worker.reply(rt);
                two.wrote(reply, A);
            }
            Err(reply) => {
                two.wrote(reply, A);
                one.read(rt, read, A, 2);
            }
        }
    }
    Steps {
        stopped,
        reached: stop.reached(),
    }
}

/// Run the write skew of `write` and `read` over rows A and B of `cells`: side 2 reads B and
/// writes A; side 1 reads A, meeting side 2's write as `at` says, then writes B. Side 2 ends
/// before side 1 writes. Returns the history with a final read of both rows, and the steps of the
/// side `at` stops.
fn skew(write: Write, read: Read, at: At, seed: u64) -> (Vec<iso::Txn>, Steps) {
    simulate_with(
        seed,
        &options(),
        Box::new(|rt, e| {
            fill(rt, e, write);
            let (mut one, mut two) = (Side::start(rt, e, 1, write), Side::start(rt, e, 2, write));
            two.read(rt, read, B, 1);
            let none = || Steps {
                stopped: None,
                reached: vec![],
            };
            let (two, steps) = match at {
                At::Before => {
                    one.read(rt, read, A, 2);
                    two.write(rt, write, A);
                    (two.end(rt), none())
                }
                At::Writer(n) => {
                    let steps = stopped_write(rt, e, (&mut one, &mut two), write, read, n);
                    (two.end(rt), steps)
                }
                At::Reader(n) => {
                    let stop = Stop::set(rt, e.database(), (one.worker.thread(), |_| true), n);
                    one.worker.send(rt, read.row(A));
                    let (reply, stopped, two) =
                        match stop.stopped(rt, &mut || one.worker.try_reply()) {
                            Ok(step) => {
                                two.write(rt, write, A);
                                let two = two.end(rt);
                                stop.release(rt);
                                (one.worker.reply(rt), Some(step), two)
                            }
                            Err(reply) => {
                                two.write(rt, write, A);
                                (reply, None, two.end(rt))
                            }
                        };
                    one.was_read(reply, read, A, 2);
                    let reached = stop.reached();
                    (two, Steps { stopped, reached })
                }
            };
            one.write(rt, write, B);
            let one = one.end(rt);
            let last = Worker::start(rt, e, Kind::ReadOnly);
            let ops = [(A, 2), (B, 1)]
                .map(|(key, elem)| {
                    let v = last.call(rt, Read::Key.row(key)).expect("a read-only read");
                    iso::Op::Read {
                        key,
                        list: list(&v, write, elem),
                    }
                })
                .into();
            last.call(rt, Req::Commit).expect("a reader commits");
            rt.join(last.thread());
            let history = vec![iso::Txn::new(0, Outcome::Committed, ops), one, two];
            (history, steps)
        }),
    )
}

/// Returns the page locks an early read of A and one of B leave, each in a transaction of its own,
/// over the rows a write of an existing version finds.
fn early_pages(seed: u64) -> [usize; 2] {
    simulate_with(
        seed,
        &options(),
        Box::new(|rt, e| {
            fill(rt, e, Write::Delete);
            [A, B].map(|id| {
                let Req::Text(text) = Read::Early.row(id) else {
                    unreachable!("a read is text")
                };
                let mut tx = e.begin(Kind::ReadWrite).expect("a transaction");
                tx.query_text(&text, &[], &TxnCallOptions::default())
                    .expect("an early read");
                let locks = e.database().testing().locks(&tx);
                tx.rollback().expect("a rollback");
                locks.iter().filter(|l| l.starts_with("page ")).count()
            })
        }),
    )
}

/// The anomalies of the histories judged, and how many were judged.
#[derive(Default)]
struct Judged {
    count: usize,
    anomalies: Vec<String>,
}

impl Judged {
    fn judge(&mut self, history: &[iso::Txn], case: String) {
        self.count += 1;
        match iso::check(history) {
            Ok(found) if found.is_empty() => {}
            found => self.anomalies.push(format!("{case}: {found:?}")),
        }
    }
}

/// Run the skew with side `side` stopped at each step it reaches in turn, judging each history.
/// Returns the steps it stopped at, each with the step it reached next.
fn every_step(
    write: Write,
    read: Read,
    seed: u64,
    side: fn(usize) -> At,
    judged: &mut Judged,
) -> Vec<(Step, Option<Step>)> {
    let mut stops = Vec::new();
    for n in 1.. {
        let (history, steps) = skew(write, read, side(n), seed);
        let case = format!(
            "{write:?} {read:?} seed {seed} {:?} of {:?}",
            side(n),
            steps.reached
        );
        judged.judge(&history, case);
        let Some(step) = steps.stopped else {
            assert_eq!(n - 1, steps.reached.len(), "{write:?} {read:?} seed {seed}");
            return stops;
        };
        stops.push((step, steps.reached.get(n).copied()));
    }
    unreachable!("a statement reaches finitely many steps")
}

#[test]
fn a_read_and_a_write_of_a_version_stopped_at_each_others_steps_leave_serializable_histories() {
    const SEEDS: Range<u64> = 0..1;
    let [a, b] = early_pages(0);
    assert!(
        a == 1 && b > 1,
        "an early read of A stops on its page, before B's: {a} and {b} page locks"
    );
    let mut judged = Judged::default();
    let (mut between, mut read_stops) = (0, 0);
    let mut cases = 0;
    for write in WRITES {
        for &read in write.reads() {
            for seed in sweep(SEEDS) {
                cases += 1;
                let (history, _) = skew(write, read, At::Before, seed);
                let case = format!("{write:?} {read:?} seed {seed} before the write");
                judged.judge(&history, case);
                let stops = every_step(write, read, seed, At::Writer, &mut judged);
                if stops.contains(&write.changed()) {
                    between += 1;
                }
                let stops = every_step(write, read, seed, At::Reader, &mut judged);
                let lock = match read {
                    Read::Key => Step::VersionRead,
                    Read::Scan => Step::ScanRead,
                    Read::Early => Step::ScanStopped,
                };
                if stops.iter().any(|(s, _)| *s == lock) {
                    read_stops += 1;
                }
            }
        }
    }
    assert_eq!(
        Vec::<String>::new(),
        judged.anomalies,
        "of {} histories",
        judged.count
    );
    assert_reached(
        "a read between a write's change, of the write's kind, and its check",
        between,
        cases,
    );
    assert_reached(
        "a write between a read and the end of its locking step",
        read_stops,
        cases,
    );
}
