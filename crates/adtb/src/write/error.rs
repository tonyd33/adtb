use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::types::RelId;

use crate::exec::{self, Val};
use crate::mvcc::{Conflict, Xid};
use crate::storage::{MarkFailure, PageError, Quarantine};
use crate::vacuum;

use super::{Broken, By, Rerun, Rot, Stage, Violator};

/// Why a statement was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// `constraint` is the violated constraint's name; `report` the rows its query returns.
    Violated {
        violator: Violator,
        constraint: String,
        report: BTreeSet<Val>,
    },
    /// Two rows of `relation` hold `key`, the least such key.
    KeyViolation {
        relation: RelId,
        key: Val,
    },
    KeyChanged {
        relation: RelId,
    },
    /// `ctor` is qualified by its relation (`identities.Apple`) or by its sum (`Provider.Okta`).
    NotOpen {
        ctor: String,
        stage: Stage,
    },
    UserError {
        message: String,
    },
    /// An argument of a command is outside its parameter's domain, as `detail` states.
    Invalid {
        detail: String,
    },
    /// A record of the statement's source failed: record `line` of relation `source`, or `""`
    /// and 0 when the source is not one relation.
    BadInput {
        source: String,
        line: i64,
        detail: String,
    },
    /// A new row of constructor `ctor` of `relation` stores in `size` bytes, over the heap's
    /// `limit`.
    RowTooLarge {
        relation: RelId,
        ctor: u32,
        size: usize,
        limit: usize,
    },
    KeyTooLarge {
        relation: RelId,
        ctor: u32,
        index: String,
        size: usize,
        limit: usize,
    },
}

/// Why a statement or a commit failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Rejected(Rejection),
    /// The call reads, or writes under, `objects`, which are quarantined, each with why its mark
    /// holds only in memory if it does. `by` states what made the call fail.
    Quarantined {
        objects: BTreeMap<Quarantine, Option<MarkFailure>>,
        by: By,
    },
    /// The transactions layer refuses the call.
    Transactions(crate::mvcc::Error),
    /// The open could not arm the read-in gate, so it refuses the call.
    Unarmed(crate::storage::Unarmed),
    Corrupt(Rot),
    /// The role the statement runs for lacks a privilege, as this text states.
    Denied(String),
    SequenceExhausted,
    Internal(Box<Broken>),
    Exec(exec::Error),
    /// The schema has a constraint without an index-bounded incremental check; each constraint
    /// refused, with why.
    Refused(Vec<(String, String)>),
    /// The files a path names differ from those an earlier scan of it in the transaction read.
    FileChanged(String),
    /// The engine closed while the statement waited for a transaction.
    Closed,
    /// A statement run with [`Bounds::suspend`](super::Bounds::suspend) must wait for
    /// transaction `on` to end, to write a version of `heap`, or at a gate for `None`. A rerun of
    /// the statements passes `rerun` as [`Bounds::rerun`](super::Bounds::rerun).
    Blocked {
        on: Xid,
        heap: Option<u32>,
        rerun: Rerun,
    },
}

impl From<PageError> for Error {
    fn from(e: PageError) -> Error {
        Error::Exec(exec::Error::Page(e))
    }
}

impl From<crate::heap::OrphanEntry> for Error {
    fn from(e: crate::heap::OrphanEntry) -> Error {
        Error::Exec(e.into())
    }
}

impl From<Conflict> for Error {
    fn from(f: Conflict) -> Error {
        Error::Exec(exec::Error::Serialization(f))
    }
}

impl From<crate::heap::Error> for Error {
    fn from(e: crate::heap::Error) -> Error {
        match e {
            crate::heap::Error::Page(p) => p.into(),
            crate::heap::Error::Changed(c) => c.into(),
            crate::heap::Error::TooLarge { len, limit } => {
                Error::Internal(Box::new(Broken::Unadmitted { len, limit }))
            }
            crate::heap::Error::NoDirectory => Broken::NoDirectory.into(),
            crate::heap::Error::Unencodable(e) => Broken::Unencodable(e).into(),
        }
    }
}

impl From<vacuum::Error> for Error {
    fn from(e: vacuum::Error) -> Error {
        match e {
            vacuum::Error::Page(p) | vacuum::Error::Heap(crate::heap::Error::Page(p)) => p.into(),
            vacuum::Error::Index(i) => Error::from(i),
            vacuum::Error::Heap(e @ crate::heap::Error::Changed { .. }) => Error::from(e),
            vacuum::Error::Heap(crate::heap::Error::NoDirectory) => Broken::NoDirectory.into(),
            vacuum::Error::Heap(crate::heap::Error::Unencodable(e)) => {
                Broken::Unencodable(e).into()
            }
            vacuum::Error::Heap(crate::heap::Error::TooLarge { len, limit }) => {
                Broken::KeptTooLarge { len, limit }.into()
            }
            vacuum::Error::Stopped => Error::Exec(exec::Error::Cancelled),
            vacuum::Error::Unkeyed(c) => Error::Exec(exec::Broken::UnkeyedRow(c.0).into()),
        }
    }
}

impl From<crate::btree::Error> for Error {
    fn from(e: crate::btree::Error) -> Error {
        use crate::btree::Error as B;
        match e {
            B::Page(p) => p.into(),
            B::Key(e) => e.into(),
            e @ (B::Malformed(_) | B::Entry(..)) => Rot::Index(e).into(),
            B::TooLong { len, max } => Broken::EntryTooLong { len, max }.into(),
        }
    }
}

impl From<crate::key::EncodeError> for Error {
    fn from(e: crate::key::EncodeError) -> Error {
        Error::Exec(e.into())
    }
}

impl From<crate::mvcc::Error> for Error {
    fn from(e: crate::mvcc::Error) -> Error {
        Error::Transactions(e)
    }
}

impl From<crate::catalog::Error> for Error {
    fn from(e: crate::catalog::Error) -> Error {
        use crate::catalog::Error as C;
        let refused = |e: &C| {
            Error::Rejected(Rejection::Invalid {
                detail: crate::catalog::detail(e),
            })
        };
        match e {
            C::Page(p) => p.into(),
            C::Heap(e) => e.into(),
            C::Index(e) => e.into(),
            C::Corrupt(r) => Rot::Catalog(r).into(),
            C::Missing(what) => Rot::NoRoot(what).into(),
            C::Txn(e) => Error::Transactions(e),
            C::Spill(io) => Error::Exec(exec::Error::Spill(io)),
            C::Conflict(c) => c.into(),
            C::Internal(b) => Broken::Catalog(b).into(),
            C::Exec(e) => Error::Exec(e.into_error()),
            C::MissingFunction(name) => Error::Exec(exec::Error::MissingFunction(name)),
            C::OtherVersion(name) => Error::Exec(exec::Error::OtherVersion(name)),
            C::Quarantined(object, _) => Error::admitted(object),
            e @ (C::Refused(_)
            | C::RefusedBy { .. }
            | C::Unaccepted(..)
            | C::PastLimits(_)
            | C::FormatTooOld(_)) => refused(&e),
        }
    }
}

impl From<super::CounterError> for Error {
    fn from(e: super::CounterError) -> Error {
        use super::CounterError as C;
        match e {
            C::Page(p) => p.into(),
            C::Index(e) => e.into(),
            C::Exhausted => Error::SequenceExhausted,
            C::Full(room) => {
                Error::Exec(exec::Error::Unsupported(super::sequence::past_room(room)))
            }
        }
    }
}

impl From<exec::Error> for Error {
    fn from(e: exec::Error) -> Error {
        Error::Exec(e)
    }
}
