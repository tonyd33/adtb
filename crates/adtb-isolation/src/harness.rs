use crate::history::{Op, Outcome, Txn};

/// The database refused the operation and aborted the transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aborted;

/// A database running the append workload.
///
/// After an operation returns `Aborted`, the harness drops the transaction without calling
/// anything else on it.
pub trait Database {
    type Txn;

    fn begin(&mut self) -> Self::Txn;
    fn append(&mut self, txn: &mut Self::Txn, key: i64, elem: i64) -> Result<(), Aborted>;
    /// Returns the list at `key`, oldest element first.
    fn read(&mut self, txn: &mut Self::Txn, key: i64) -> Result<Vec<i64>, Aborted>;
    fn commit(&mut self, txn: Self::Txn) -> Result<(), Aborted>;
    fn abort(&mut self, txn: Self::Txn);
}

/// One step of a script; `txn` numbers a transaction within the script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Begin(u64),
    Append(u64, i64, i64),
    Read(u64, i64),
    Commit(u64),
    Abort(u64),
}

impl Step {
    fn txn(self) -> u64 {
        match self {
            Step::Begin(t)
            | Step::Append(t, ..)
            | Step::Read(t, _)
            | Step::Commit(t)
            | Step::Abort(t) => t,
        }
    }
}

/// Run `steps` against `db` in order and return the history they produce, one transaction per
/// script number, in order of `Begin`.
///
/// Steps of a transaction the database aborted are skipped. A transaction still open after the
/// last step is aborted.
///
/// Panics if a step names a transaction that has not begun, or begins one twice.
pub fn run<D: Database>(db: &mut D, steps: &[Step]) -> Vec<Txn> {
    let mut history: Vec<Txn> = Vec::new();
    let mut open: Vec<Option<D::Txn>> = Vec::new();
    for &step in steps {
        let t = step.txn();
        let found = history.iter().position(|x| x.id.0 == t);
        if let Step::Begin(_) = step {
            assert!(found.is_none(), "transaction {t} begun twice");
            history.push(Txn::new(t, Outcome::InDoubt, vec![]));
            open.push(Some(db.begin()));
            continue;
        }
        let i = found.unwrap_or_else(|| panic!("transaction {t} has not begun"));
        let record = &mut history[i];
        let Some(txn) = open[i].as_mut() else {
            continue;
        };
        let ended = match step {
            Step::Begin(_) => unreachable!(),
            Step::Append(_, key, elem) => match db.append(txn, key, elem) {
                Ok(()) => {
                    record.ops.push(Op::Append { key, elem });
                    None
                }
                Err(Aborted) => Some(Outcome::Aborted),
            },
            Step::Read(_, key) => match db.read(txn, key) {
                Ok(list) => {
                    record.ops.push(Op::Read { key, list });
                    None
                }
                Err(Aborted) => Some(Outcome::Aborted),
            },
            Step::Commit(_) => match db.commit(open[i].take().expect("open")) {
                Ok(()) => Some(Outcome::Committed),
                Err(Aborted) => Some(Outcome::Aborted),
            },
            Step::Abort(_) => {
                db.abort(open[i].take().expect("open"));
                Some(Outcome::Aborted)
            }
        };
        if let Some(outcome) = ended {
            open[i] = None;
            record.outcome = outcome;
        }
    }
    for (record, txn) in history.iter_mut().zip(open) {
        if let Some(txn) = txn {
            db.abort(txn);
            record.outcome = Outcome::Aborted;
        }
    }
    history
}

/// A scripted interleaving a serializable database must run without anomaly.
///
/// No step appends to a key another open transaction has appended to, so a database whose
/// appends wait on a conflicting writer never blocks the script. The last transaction reads
/// every key after the others end, revealing each key's version order.
#[derive(Clone, Copy, Debug)]
pub struct Scenario {
    pub name: &'static str,
    pub steps: &'static [Step],
}

use Step::{Abort, Append, Begin, Commit, Read};

/// Two transactions each read both keys and append to a different one.
pub const WRITE_SKEW: Scenario = Scenario {
    name: "write skew",
    steps: &[
        Begin(1),
        Begin(2),
        Read(1, 1),
        Read(1, 2),
        Read(2, 1),
        Read(2, 2),
        Append(1, 1, 1),
        Append(2, 2, 1),
        Commit(1),
        Commit(2),
        Begin(3),
        Read(3, 1),
        Read(3, 2),
        Commit(3),
    ],
};

/// A reader sees one key before and the other after a writer that appends to both.
pub const READ_SKEW: Scenario = Scenario {
    name: "read skew",
    steps: &[
        Begin(1),
        Append(1, 1, 1),
        Append(1, 2, 1),
        Commit(1),
        Begin(2),
        Read(2, 1),
        Begin(3),
        Append(3, 1, 2),
        Append(3, 2, 2),
        Commit(3),
        Read(2, 2),
        Commit(2),
        Begin(4),
        Read(4, 1),
        Read(4, 2),
        Commit(4),
    ],
};

/// Two transactions read a key and both append to it.
pub const LOST_UPDATE: Scenario = Scenario {
    name: "lost update",
    steps: &[
        Begin(1),
        Begin(2),
        Read(1, 1),
        Read(2, 1),
        Append(1, 1, 1),
        Commit(1),
        Append(2, 1, 2),
        Commit(2),
        Begin(3),
        Read(3, 1),
        Commit(3),
    ],
};

/// A reader runs while a writer that later aborts has appended.
pub const ABORTED_READ: Scenario = Scenario {
    name: "aborted read",
    steps: &[
        Begin(1),
        Append(1, 1, 1),
        Begin(2),
        Read(2, 1),
        Abort(1),
        Commit(2),
        Begin(3),
        Read(3, 1),
        Commit(3),
    ],
};

/// A reader runs between two appends of one writer to one key.
pub const INTERMEDIATE_READ: Scenario = Scenario {
    name: "intermediate read",
    steps: &[
        Begin(1),
        Append(1, 1, 1),
        Begin(2),
        Read(2, 1),
        Append(1, 1, 2),
        Commit(1),
        Commit(2),
        Begin(3),
        Read(3, 1),
        Commit(3),
    ],
};

/// Each of two transactions appends to one key and reads the other's.
pub const CIRCULAR_FLOW: Scenario = Scenario {
    name: "circular information flow",
    steps: &[
        Begin(1),
        Begin(2),
        Append(1, 1, 1),
        Append(2, 2, 1),
        Read(1, 2),
        Read(2, 1),
        Commit(1),
        Commit(2),
        Begin(3),
        Read(3, 1),
        Read(3, 2),
        Commit(3),
    ],
};

pub const SCENARIOS: &[Scenario] = &[
    WRITE_SKEW,
    READ_SKEW,
    LOST_UPDATE,
    ABORTED_READ,
    INTERMEDIATE_READ,
    CIRCULAR_FLOW,
];
