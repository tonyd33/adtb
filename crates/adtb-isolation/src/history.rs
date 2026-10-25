/// A transaction's id, unique within one history.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TxnId(pub u64);

/// A transaction's end as the client saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Committed,
    Aborted,
    /// Submitted with no answer; it may or may not have committed.
    InDoubt,
}

/// One operation on a list key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Append `elem` to the list at `key`.
    Append { key: i64, elem: i64 },
    /// Read the list at `key`, oldest element first.
    Read { key: i64, list: Vec<i64> },
}

/// A transaction: its operations in the order it ran them, and its outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Txn {
    pub id: TxnId,
    pub outcome: Outcome,
    pub ops: Vec<Op>,
}

impl Txn {
    pub fn new(id: u64, outcome: Outcome, ops: Vec<Op>) -> Txn {
        Txn {
            id: TxnId(id),
            outcome,
            ops,
        }
    }
}

/// A history the checker refuses to analyse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Malformed {
    DuplicateTxn(TxnId),
    /// Two appends of one element to one key.
    DuplicateAppend {
        key: i64,
        elem: i64,
        first: TxnId,
        second: TxnId,
    },
}

/// Why `to` follows `from` on one key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dep {
    /// `to` appended `next` directly after `from` appended `prior`.
    Ww { prior: i64, next: i64 },
    /// `to` read a list ending in `elem`, which `from` appended.
    Wr { elem: i64 },
    /// `from` read a list ending in `read`, or an empty list, and `to` wrote the next version of
    /// the key, starting with `missed`.
    Rw { read: Option<i64>, missed: i64 },
}

/// A dependency between two transactions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: TxnId,
    pub to: TxnId,
    pub key: i64,
    pub dep: Dep,
}

/// What a transaction's own operations say its read must return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expected {
    /// Exactly this list.
    List(Vec<i64>),
    /// A list ending in these elements.
    Suffix(Vec<i64>),
}

/// An anomaly, with the cycle or the read that shows it.
///
/// A cycle lists its edges in order; each edge's `to` is the next edge's `from`, and the last
/// edge's `to` is the first edge's `from`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anomaly {
    /// A cycle of write dependencies.
    G0 { cycle: Vec<Edge> },
    /// A committed transaction read an element an aborted transaction appended.
    G1a {
        reader: TxnId,
        writer: TxnId,
        key: i64,
        elem: i64,
    },
    /// A committed transaction read a list ending in an element its writer later followed with
    /// another append to the same key.
    G1b {
        reader: TxnId,
        writer: TxnId,
        key: i64,
        elem: i64,
    },
    /// A cycle of write and read dependencies with at least one read dependency.
    G1c { cycle: Vec<Edge> },
    /// A cycle with exactly one anti-dependency.
    GSingle { cycle: Vec<Edge> },
    /// A cycle with more than one anti-dependency, in a component with no cycle of exactly one.
    G2 { cycle: Vec<Edge> },
    /// A read that contradicts the transaction's own earlier operations on the key.
    Internal {
        txn: TxnId,
        key: i64,
        expected: Expected,
        read: Vec<i64>,
    },
    /// A read holding an element no transaction appended.
    Garbage { reader: TxnId, key: i64, elem: i64 },
    /// A read holding one element twice.
    Duplicate { reader: TxnId, key: i64, elem: i64 },
    /// Two reads of one key, neither a prefix of the other.
    IncompatibleOrder {
        key: i64,
        left: Vec<i64>,
        right: Vec<i64>,
    },
}
