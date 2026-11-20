use std::ops::Range;
use std::sync::Arc;

use adtb::runtime::Threads;
use adtb::testing::{Change, Step};
use adtb_sim::SimRuntime;
use adtb_simkit::elle::{self, Elems, Kind, Reply, Req, Worker, simulate};
use adtb_simkit::steps::Stop;
use adtb_testkit::isolation::{self as iso, Outcome};
use adtb_testkit::seeds::{assert_reached, sweep};

/// A write of an existing version of `cells`.
#[derive(Clone, Copy, Debug)]
enum Write {
    Delete,
    /// An update of an unindexed field, which stays on its page.
    HotUpdate,
    /// An update of an indexed field.
    ColdUpdate,
    Retag,
}

const WRITES: [Write; 4] = [
    Write::Delete,
    Write::HotUpdate,
    Write::ColdUpdate,
    Write::Retag,
];

/// A read of one row of `cells`: by its key, or by a scan of the whole heap.
#[derive(Clone, Copy, Debug)]
enum Read {
    Key,
    Scan,
}

const READS: [Read; 2] = [Read::Key, Read::Scan];

/// The value a read gives a row: 0 before any write, more after any write but a delete.
const VALUE: &str = "c.n + c.m + (match c | Even e -> 0 | Odd o -> 1)";

impl Write {
    /// Returns what the write does to the version it supersedes.
    fn change(self) -> Change {
        match self {
            Write::Delete => Change::Delete,
            Write::HotUpdate => Change::Update { heap_only: true },
            Write::ColdUpdate => Change::Update { heap_only: false },
            Write::Retag => Change::Retag,
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
        })
    }
}

impl Read {
    /// Returns the request reading row `id`.
    fn row(self, id: i64) -> Req {
        Req::Text(match self {
            Read::Key => format!("from c in cells where c.id == {id} select {VALUE}"),
            Read::Scan => format!("from c in cells select c.id * 10 + {VALUE}"),
        })
    }

    /// Returns the values of row `id` in `reply`, the rows the request of [`Read::row`] gave.
    fn values(self, id: i64, reply: Vec<i64>) -> Vec<i64> {
        match self {
            Read::Key => reply,
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

/// Returns the list a read of a row of `cells` written at most once, by the append of `elem`,
/// stands for: empty while the read gives the row's first value.
fn list(values: &[i64], elem: i64) -> Vec<i64> {
    match values {
        [0] => vec![],
        [] | [_] => vec![elem],
        v => panic!("a read of one row gives at most one value: {v:?}"),
    }
}

/// One side of the write skew: a transaction that reads one row and writes the other, writing
/// the list element its id names.
struct Side {
    worker: Worker,
    id: u64,
    ops: Vec<iso::Op>,
    aborted: bool,
}

impl Side {
    fn start(rt: &SimRuntime, e: &Arc<Elems>, id: u64) -> Side {
        Side {
            worker: Worker::start(rt, e, Kind::ReadWrite),
            id,
            ops: Vec::new(),
            aborted: false,
        }
    }

    fn failed(&mut self, r: Reply) -> Option<Vec<i64>> {
        match r {
            Ok(v) => Some(v),
            Err(f) => {
                elle::aborted(f);
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
        if let Some(v) = self.failed(reply) {
            let list = list(&read.values(key, v), elem);
            self.ops.push(iso::Op::Read { key, list });
        }
    }

    /// Record the reply of a write of row `key`.
    fn wrote(&mut self, reply: Reply, key: i64) {
        if self.failed(reply).is_some() {
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
            self.failed(reply);
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
    matches!(step, Step::WriteChanged(_) | Step::WriteChecked)
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
        match stop.stopped(rt, || two.worker.try_reply()) {
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
    simulate(seed, |rt, e| {
        let setup = Worker::start(rt, e, Kind::ReadWrite);
        for id in [A, B] {
            let row = format!("insert cells.Even {{ id: {id}, n: 0, m: 0 }}");
            setup
                .call(rt, Req::Text(row))
                .expect("a row of cells inserts");
        }
        setup.call(rt, Req::Commit).expect("the rows commit");
        rt.join(setup.thread());
        let (mut one, mut two) = (Side::start(rt, e, 1), Side::start(rt, e, 2));
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
                let (reply, stopped, two) = match stop.stopped(rt, || one.worker.try_reply()) {
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
                    list: list(&v, elem),
                }
            })
            .into();
        last.call(rt, Req::Commit).expect("a reader commits");
        rt.join(last.thread());
        let history = vec![iso::Txn::new(0, Outcome::Committed, ops), one, two];
        (history, steps)
    })
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
    let mut judged = Judged::default();
    let (mut between, mut read_stops) = (0, 0);
    for write in WRITES {
        for read in READS {
            for seed in sweep(SEEDS) {
                let (history, _) = skew(write, read, At::Before, seed);
                let case = format!("{write:?} {read:?} seed {seed} before the write");
                judged.judge(&history, case);
                let changed = Step::WriteChanged(write.change());
                let stops = every_step(write, read, seed, At::Writer, &mut judged);
                if stops.contains(&(changed, Some(Step::WriteChecked))) {
                    between += 1;
                }
                let stops = every_step(write, read, seed, At::Reader, &mut judged);
                let lock = match read {
                    Read::Key => Step::VersionRead,
                    Read::Scan => Step::ScanRead,
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
    let cases = WRITES.len() * READS.len() * sweep(SEEDS).count();
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
