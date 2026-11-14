use std::collections::BTreeMap;

use super::*;
use crate::btree;
use crate::catalog;
use crate::exec::{self, Class, Exceeded};
use crate::mvcc::{self, Cause, SerializationFailure, Xid};
use crate::runtime::IoError;
use crate::storage::{self, Damage, Lsn, PageError, Pid, Quarantine};
use crate::write::{By, Rerun, Rot};

const NAMES: &str = "the objects";

fn page() -> PageError {
    PageError {
        pid: Pid(3),
        damage: Damage::Checksum,
    }
}

fn serialization() -> SerializationFailure {
    SerializationFailure::new(Cause::LaterCommit, Vec::new())
}

fn quarantined(by: By) -> write::Error {
    write::Error::Quarantined {
        objects: BTreeMap::from([(Quarantine::Database, None)]),
        by,
    }
}

fn storage_failure(detail: &str) -> Error {
    Error::Fatal {
        reason: FatalReason::Storage {
            detail: detail.into(),
        },
    }
}

fn reject(reason: RejectReason) -> Error {
    Error::Rejected { reason }
}

fn invalid(detail: &str) -> Error {
    reject(RejectReason::Invalid {
        detail: detail.into(),
    })
}

fn closed() -> Error {
    reject(RejectReason::Closed)
}

fn internal(what: &str) -> Error {
    reject(RejectReason::Internal { what: what.into() })
}

fn retry_serialization() -> Error {
    Error::Retry {
        reason: RetryReason::Serialization {
            txn: TxnId(7),
            on: String::new(),
        },
    }
}

fn quarantined_reply() -> Error {
    Error::Fatal {
        reason: FatalReason::Quarantined { what: NAMES.into() },
    }
}

/// One row of the class table: a failure, its class, whether it ends the transaction, and the
/// reply a client receives.
struct Row {
    error: write::Error,
    class: Class,
    ends: bool,
    /// `None` for a failure no caller maps to a reply.
    reply: Option<Error>,
}

fn row(error: write::Error, class: Class, ends: bool, reply: Error) -> Row {
    Row {
        error,
        class,
        ends,
        reply: Some(reply),
    }
}

fn exec_row(error: exec::Error, class: Class, ends: bool, reply: Error) -> Row {
    row(error.into(), class, ends, reply)
}

fn write_rows() -> Vec<Row> {
    vec![
        row(
            write::Error::Rejected(Rejection::UserError {
                message: "m".into(),
            }),
            Class::Reject,
            false,
            reject(RejectReason::UserError {
                message: "m".into(),
            }),
        ),
        row(
            quarantined(By::Met),
            Class::Damage,
            true,
            quarantined_reply(),
        ),
        row(
            quarantined(By::Transactions),
            Class::Transactions,
            true,
            quarantined_reply(),
        ),
        row(
            quarantined(By::Found),
            Class::Found,
            false,
            quarantined_reply(),
        ),
        row(
            quarantined(By::Admitted),
            Class::Refused,
            false,
            quarantined_reply(),
        ),
        row(
            write::Error::Transactions(mvcc::Error::Corrupt(page())),
            Class::Transactions,
            true,
            storage_failure("Corrupt(PageError { pid: Pid(3), damage: Checksum })"),
        ),
        row(
            write::Error::Transactions(mvcc::Error::Exhausted),
            Class::Transactions,
            true,
            storage_failure("Exhausted"),
        ),
        row(
            write::Error::Transactions(mvcc::Error::History(btree::Error::Malformed(Pid(3)))),
            Class::Transactions,
            true,
            storage_failure("History(Malformed(Pid(3)))"),
        ),
        row(
            write::Error::Transactions(mvcc::Error::Closed),
            Class::Transactions,
            true,
            closed(),
        ),
        row(
            write::Error::Corrupt(Rot::NoKeyIndex("r".into())),
            Class::Damage,
            true,
            storage_failure("relation r has no key index"),
        ),
        row(
            write::Error::Denied("d".into()),
            Class::Reject,
            false,
            reject(RejectReason::Denied { detail: "d".into() }),
        ),
        row(
            write::Error::SequenceExhausted,
            Class::Reject,
            false,
            invalid("the generated counter exhausted its integer range"),
        ),
        row(
            write::Error::Refused(vec![("c".into(), "d".into())]),
            Class::Reject,
            false,
            reject(RejectReason::SchemaRefused {
                operator: String::new(),
                detail: "constraint c is not admitted: d".into(),
            }),
        ),
        row(
            write::Error::FileChanged("f".into()),
            Class::Retry,
            true,
            Error::Retry {
                reason: RetryReason::FileChanged { path: "f".into() },
            },
        ),
        row(write::Error::Closed, Class::Reject, false, closed()),
        Row {
            error: write::Error::Blocked {
                on: Xid(5),
                rerun: Rerun::at_start(),
            },
            class: Class::Suspended,
            ends: false,
            reply: None,
        },
    ]
}

fn exec_rows() -> Vec<Row> {
    vec![
        exec_row(
            exec::Error::Fault(Fault::DivisionByZero),
            Class::Reject,
            false,
            reject(RejectReason::Faulted {
                fault: Fault::DivisionByZero,
            }),
        ),
        exec_row(
            exec::Error::StatementLimit(Exceeded::Spill {
                limit: 20,
                grant: 10,
            }),
            Class::Reject,
            false,
            reject(RejectReason::StatementLimit {
                limit: Limit::Memory { bytes: 10 },
            }),
        ),
        exec_row(
            exec::Error::Serialization(serialization()),
            Class::Retry,
            true,
            retry_serialization(),
        ),
        exec_row(
            exec::Error::Page(page()),
            Class::Damage,
            true,
            storage_failure("PageError { pid: Pid(3), damage: Checksum }"),
        ),
        exec_row(
            exec::Error::OrphanEntry(9),
            Class::Found,
            false,
            storage_failure("an entry of index 9 names no version"),
        ),
        exec_row(
            exec::Error::Spill(IoError::Full),
            Class::Reject,
            false,
            storage_failure("the spill file failed: Full"),
        ),
        exec_row(
            exec::Error::Corrupt(exec::Rot::SpillShort),
            Class::Damage,
            true,
            storage_failure("a spilled extent reads back short"),
        ),
        exec_row(
            exec::Error::Unsupported("x".into()),
            Class::Reject,
            false,
            invalid("x does not run in this engine yet"),
        ),
        exec_row(
            exec::Error::NoCaller,
            Class::Reject,
            false,
            invalid("`caller` in a statement does not run in this engine yet"),
        ),
        exec_row(
            exec::Error::Rejected("r".into()),
            Class::Reject,
            false,
            reject(RejectReason::BadInput {
                source: String::new(),
                line: 0,
                detail: "r".into(),
            }),
        ),
        exec_row(
            exec::Error::Cancelled,
            Class::Reject,
            false,
            reject(RejectReason::Cancelled),
        ),
        exec_row(
            exec::Error::DeadlineExceeded,
            Class::Reject,
            false,
            reject(RejectReason::DeadlineExceeded),
        ),
        exec_row(
            exec::Error::MissingFunction("h".into()),
            Class::Reject,
            false,
            Error::Fatal {
                reason: FatalReason::MissingFunction {
                    function: "h".into(),
                },
            },
        ),
        exec_row(
            exec::Error::OtherVersion("h".into()),
            Class::Checker,
            false,
            internal("host function `h` is bound at the version its declaration names"),
        ),
        exec_row(
            exec::Broken::Unreachable.into(),
            Class::Checker,
            false,
            internal("an evaluation reaches no `Unreachable`"),
        ),
        exec_row(
            exec::Error::HistoryUnavailable {
                oldest: storage::Position {
                    epoch: 1,
                    lsn: Lsn(2),
                },
            },
            Class::Reject,
            false,
            reject(RejectReason::HistoryUnavailable {
                oldest: Position { epoch: 1, lsn: 2 },
            }),
        ),
        exec_row(
            exec::Error::WrongVersion { version: 3 },
            Class::Reject,
            false,
            reject(RejectReason::WrongVersion { version: 3 }),
        ),
    ]
}

fn broken_row(b: write::Broken, class: Class, what: &str) -> Row {
    row(b.into(), class, false, internal(what))
}

/// Returns a row for each variant of `write::Broken`.
fn broken_rows() -> Vec<Row> {
    let mut rows = store_broken_rows();
    rows.extend(term_broken_rows());
    rows.extend(call_broken_rows());
    rows
}

fn store_broken_rows() -> Vec<Row> {
    use write::Broken as B;
    let plan = || crate::plan::Error::Internal(crate::plan::Broken::GroupFieldMissing);
    let c = Class::Checker;
    vec![
        broken_row(B::BoundedPlan(plan()), c, "a group has a key and rows"),
        broken_row(B::StatementPlan(plan()), c, "a group has a key and rows"),
        broken_row(
            B::DuplicateKey(vec![1]),
            c,
            "a duplicate of key [1] in a unique index",
        ),
        broken_row(
            B::DenialNotSet(Val::Int(1)),
            c,
            "a denial query returns a set, found Int(1)",
        ),
        broken_row(
            B::UniqueUnindexed {
                position: 1,
                relation: "r".into(),
            },
            c,
            "unique 1 of relation r has an index",
        ),
        broken_row(B::KeyNotFields("r".into()), c, "the key of r is fields"),
    ]
}

fn term_broken_rows() -> Vec<Row> {
    use adtb_lang::kernel::Kind;
    use adtb_lang::textual::PrintError;
    use adtb_lang::types::{Base, Ty};
    use write::{Broken as B, TermKind};
    let kind = || TermKind(Kind::Record(Vec::new()));
    let int = || Ty::Base(Base::I64);
    let c = Class::Checker;
    vec![
        broken_row(B::ReadOnly, c, "a read-only transaction runs no write"),
        broken_row(B::ScanPathNotText, c, "a file scan's path is a Text"),
        broken_row(
            B::RowsNotSet(Val::Int(1)),
            c,
            "a write's rows are a set, found Int(1)",
        ),
        broken_row(B::FailNotText, c, "`fail` takes a Text"),
        broken_row(B::NoArm(2), c, "a case has an arm for constructor 2"),
        broken_row(
            B::Unreachable,
            c,
            "evaluation reaches no position that exhaustiveness or a dominating test excludes",
        ),
        broken_row(
            B::NotAWrite(kind()),
            c,
            "a write term is a write, found Record([])",
        ),
        broken_row(B::ReplaceNotLam, c, "a replace's new value is a `Lam`"),
        broken_row(
            B::RejectsWraps(kind()),
            c,
            "`rejects into` wraps an insert or upsert, not Record([])",
        ),
        broken_row(
            B::NotABoundWrite(kind()),
            c,
            "a bound write is a replace, an upsert or `rejects into`, found Record([])",
        ),
        broken_row(
            B::ScanUnread,
            c,
            "every file scan of a statement is read before it runs",
        ),
        broken_row(
            B::RejectedRowNotRecord,
            c,
            "`RejectedRow` is a declared record",
        ),
        broken_row(
            B::NoUnique(2),
            c,
            "an upsert names unique 2 of its relation",
        ),
        broken_row(
            B::ReferenceUnindexed {
                relation: "r".into(),
                field: "f".into(),
            },
            c,
            "a reference field has an index: r.f has none",
        ),
        broken_row(B::TopNotFor, c, "a top level read is the source of a `For`"),
        broken_row(
            B::RowsNotSetTyped(int()),
            c,
            "a read of rows is a set, not Base(I64)",
        ),
        broken_row(
            B::InputNotSet(int()),
            c,
            "a change set input is a set, not Base(I64)",
        ),
        broken_row(
            B::TopBinder(kind()),
            c,
            "a top level binder is a `For` or a `Let`, not Record([])",
        ),
        broken_row(
            B::PriorNotSet(int()),
            c,
            "a prior relation is a set, not Base(I64)",
        ),
        broken_row(
            B::Unprintable(PrintError {
                message: "p".into(),
            }),
            c,
            "printing a cell: PrintError { message: \"p\" }",
        ),
        broken_row(
            B::Undeclared {
                module: "m".into(),
                name: "n".into(),
            },
            c,
            "the engine declares `m::n`",
        ),
    ]
}

fn call_broken_rows() -> Vec<Row> {
    use write::Broken as B;
    let closed_reply = || Box::new(closed());
    let c = Class::Checker;
    vec![
        broken_row(
            B::NoValueOutcome,
            c,
            "a statement of one item has one outcome",
        ),
        broken_row(
            B::NoSettingOutcome,
            c,
            "a statement of one item has one outcome",
        ),
        broken_row(
            B::NoPointOutcome,
            c,
            "a statement of one item has one outcome",
        ),
        broken_row(
            B::NoTrialOutcome,
            c,
            "a trial of one statement has one outcome",
        ),
        broken_row(
            B::RefusalUnrecorded(closed_reply()),
            c,
            "recording a denied call: Rejected { reason: Closed }",
        ),
        broken_row(
            B::WitnessDenialUnrecorded(closed_reply()),
            c,
            "recording a denied call: Rejected { reason: Closed }",
        ),
        broken_row(B::CallKeyNotInt, c, "a key of `sys::calls` is an I64"),
        broken_row(B::ExportNotSetTyped, c, "an export's source is a set"),
        broken_row(B::ExportNotSet, c, "an export's source is a set"),
        broken_row(B::SettingNotRecord, c, "a setting's value is a record"),
        broken_row(
            B::SettingField("f".into()),
            c,
            "a setting's value has field `f`",
        ),
        broken_row(
            B::CatalogError(catalog::Error::Refused(vec!["r".into()])),
            c,
            "Refused([\"r\"])",
        ),
        broken_row(B::DeploymentRefused, c, "a deployment command was refused"),
        broken_row(
            B::NoDenial("app::c".into()),
            c,
            "constraint or check app::c has a normal form",
        ),
    ]
}

/// The number of slots [`slot`] gives.
const SLOTS: usize = 74;

/// Returns the slot of `e`'s variant, of the variant it holds where its class depends on it.
/// Every arm has its own slot, numbered from 0 with none skipped.
fn slot(e: &write::Error) -> usize {
    use write::Broken as B;
    match e {
        write::Error::Rejected(_) => 0,
        write::Error::Quarantined { by, .. } => match by {
            By::Met => 1,
            By::Found => 2,
            By::Admitted => 3,
            By::Transactions => 4,
        },
        write::Error::Transactions(e) => match e {
            mvcc::Error::Corrupt(_) => 5,
            mvcc::Error::Exhausted => 6,
            mvcc::Error::History(_) => 7,
            mvcc::Error::Closed => 8,
        },
        write::Error::Corrupt(_) => 9,
        write::Error::Denied(_) => 10,
        write::Error::SequenceExhausted => 11,
        write::Error::Refused(_) => 12,
        write::Error::FileChanged(_) => 13,
        write::Error::Closed => 14,
        write::Error::Blocked { .. } => 15,
        write::Error::Exec(e) => match e {
            exec::Error::Fault(_) => 16,
            exec::Error::StatementLimit(_) => 17,
            exec::Error::Serialization(_) => 18,
            exec::Error::Page(_) => 19,
            exec::Error::OrphanEntry(_) => 20,
            exec::Error::Spill(_) => 21,
            exec::Error::Corrupt(_) => 22,
            exec::Error::Unsupported(_) => 23,
            exec::Error::NoCaller => 24,
            exec::Error::Rejected(_) => 25,
            exec::Error::Cancelled => 26,
            exec::Error::DeadlineExceeded => 27,
            exec::Error::MissingFunction(_) => 28,
            exec::Error::OtherVersion(_) => 29,
            exec::Error::Internal(_) => 30,
            exec::Error::HistoryUnavailable { .. } => 31,
            exec::Error::WrongVersion { .. } => 32,
        },
        write::Error::Internal(b) => match b.as_ref() {
            B::BoundedPlan(_) => 33,
            B::StatementPlan(_) => 34,
            B::DuplicateKey(_) => 35,
            B::DenialNotSet(_) => 36,
            B::UniqueUnindexed { .. } => 37,
            B::KeyNotFields(_) => 38,
            B::ReadOnly => 39,
            B::ScanPathNotText => 40,
            B::RowsNotSet(_) => 41,
            B::FailNotText => 42,
            B::NoArm(_) => 43,
            B::Unreachable => 44,
            B::NotAWrite(_) => 45,
            B::ReplaceNotLam => 46,
            B::RejectsWraps(_) => 47,
            B::NotABoundWrite(_) => 48,
            B::ScanUnread => 49,
            B::RejectedRowNotRecord => 50,
            B::NoUnique(_) => 51,
            B::ReferenceUnindexed { .. } => 52,
            B::TopNotFor => 53,
            B::RowsNotSetTyped(_) => 54,
            B::InputNotSet(_) => 55,
            B::TopBinder(_) => 56,
            B::PriorNotSet(_) => 57,
            B::Unprintable(_) => 58,
            B::Undeclared { .. } => 59,
            B::NoValueOutcome => 60,
            B::NoSettingOutcome => 61,
            B::NoPointOutcome => 62,
            B::NoTrialOutcome => 63,
            B::RefusalUnrecorded(_) => 64,
            B::WitnessDenialUnrecorded(_) => 65,
            B::CallKeyNotInt => 66,
            B::ExportNotSetTyped => 67,
            B::ExportNotSet => 68,
            B::SettingNotRecord => 69,
            B::SettingField(_) => 70,
            B::CatalogError(_) => 71,
            B::DeploymentRefused => 72,
            B::NoDenial(_) => 73,
        },
    }
}

#[test]
fn each_failure_is_of_its_class_and_ends_and_replies_as_its_row_states() {
    let schema = catalog::engine_schema();
    let rows: Vec<Row> = (write_rows().into_iter())
        .chain(exec_rows())
        .chain(broken_rows())
        .collect();
    let mut slots: Vec<usize> = rows.iter().map(|r| slot(&r.error)).collect();
    slots.sort_unstable();
    assert_eq!((0..SLOTS).collect::<Vec<_>>(), slots);
    let mut wrong = Vec::new();
    for Row {
        error,
        class,
        ends,
        reply,
    } in rows
    {
        let got_class = error.class();
        let got_ends = got_class.ends();
        let shown = format!("{error:?}");
        let got = reply.is_some().then(|| {
            Error::of_mapped(
                &schema,
                TxnId(7),
                write::Mapped::unmapped_for_test(error),
                |_| NAMES.into(),
            )
        });
        if got_class != class || got_ends != ends || got != reply {
            wrong.push(format!(
                "{shown}: {got_class:?}, want {class:?}; ends {got_ends}, want {ends}; \
                 reply {got:?}, want {reply:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
