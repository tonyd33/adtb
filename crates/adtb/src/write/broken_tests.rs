use adtb_lang::kernel::{Const, Kind};
use adtb_lang::textual::PrintError;
use adtb_lang::types::{Base, RelId, Ty};

use crate::api::{self, RejectReason};
use crate::catalog;
use crate::exec::Val;
use crate::plan::{self, IndexId};

use super::super::Error;
use super::{Broken, Broken as B, TermKind, detail};

/// Returns the text the case of `b`'s variant in `cases` reads as.
#[rustfmt::skip]
fn expected(b: &Broken) -> &'static str {
    match b {
        B::BoundedPlan(plan::Error::Internal(_)) | B::StatementPlan(plan::Error::Internal(_)) => "p",
        B::BoundedPlan(_) | B::StatementPlan(_) => "a plan computes a query, not a write",
        B::ReindexNoHeaps(_) => "reindex of RelId(3), which has no heaps",
        B::VacuumNoHeaps(_) => "vacuum of RelId(3), which has no heaps",
        B::UnstoredIndex { .. } => "index 2 of us is stored",
        B::Panicked(_) => "boom",
        B::NotAnIndex { .. } => "IndexId(5) is an index of us",
        B::DuplicateKey(_) => "a duplicate of key [1, 2] in a unique index",
        B::DenialNotSet(_) => "a denial query returns a set, found Int(1)",
        B::UniqueUnindexed { .. } => "unique 1 of relation us has an index",
        B::UndeclaredIndex { .. } => "index 7 of relation us is neither declared nor an old key",
        B::KeyNotFields(_) => "the key of us is fields",
        B::VacuumTooLarge { .. } => "a version vacuum moves fits its history heap, as it fit its current heap: 9000 bytes over the limit of 4000",
        B::ReadOnly => "a read-only transaction runs no write",
        B::ScanPathNotText => "a file scan's path is a Text",
        B::RowsNotSet(_) => "a write's rows are a set, found Int(1)",
        B::FailNotText => "`fail` takes a Text",
        B::NoArm(_) => "a case has an arm for constructor 4",
        B::Unreachable => "evaluation reaches no position that exhaustiveness or a dominating test excludes",
        B::NotAWrite(_) => "a write term is a write, found EmptySet",
        B::ReplaceNotLam => "a replace's new value is a `Lam`",
        B::RejectsWraps(_) => "`rejects into` wraps an insert or upsert, not EmptySet",
        B::NotABoundWrite(_) => "a bound write is a replace, an upsert or `rejects into`, found EmptySet",
        B::ScanUnread => "every file scan of a statement is read before it runs",
        B::RejectedRowNotRecord => "`RejectedRow` is a declared record",
        B::NoUnique(_) => "an upsert names unique 2 of its relation",
        B::UniqueIndexMissing => "a unique's index is one of its relation's",
        B::ReferenceUnindexed { .. } => "a reference field has an index: app::us.f has none",
        B::TopNotFor => "a top level read is the source of a `For`",
        B::RowsNotSetTyped(_) => "a read of rows is a set, not Base(I64)",
        B::InputNotSet(_) => "a change set input is a set, not Base(I64)",
        B::TopBinder(_) => "a top level binder is a `For` or a `Let`, not EmptySet",
        B::PriorNotSet(_) => "a prior relation is a set, not Base(I64)",
        B::Unprintable(_) => "printing a cell: PrintError { message: \"m\" }",
        B::Undeclared { .. } => "the engine declares `sys::calls`",
        B::NoValueOutcome | B::NoSettingOutcome | B::NoPointOutcome => "a statement of one item has one outcome",
        B::NoTrialOutcome => "a trial of one statement has one outcome",
        B::RefusalUnrecorded(_) | B::WitnessDenialUnrecorded(_) => "recording a denied call: Rejected { reason: Internal { what: \"w\" } }",
        B::CallKeyNotInt => "a key of `sys::calls` is an I64",
        B::ExportNotSetTyped | B::ExportNotSet => "an export's source is a set",
        B::NoViolation => "`Finding` declares `Violation`",
        B::SettingNotRecord => "a setting's value is a record",
        B::SettingField(_) => "a setting's value has field `f`",
        B::Denials(_) | B::Expiry(_) => "Missing(\"m\")",
        B::DeploymentRefused => "a deployment command was refused",
    }
}

#[test]
fn each_broken_guarantee_reads_as_its_text() {
    let int = || Ty::Base(Base::I64);
    let k = || TermKind(Kind::EmptySet);
    let r = || "us".to_string();
    let missing = || catalog::Error::Missing("m".into());
    let denial = || {
        Box::new(api::Error::Rejected {
            reason: RejectReason::Internal { what: "w".into() },
        })
    };
    let internal = || plan::Error::Internal("p".into());
    let refused = || plan::Error::NotAQuery("a write");
    #[rustfmt::skip]
    let cases = [
        B::BoundedPlan(internal()), B::BoundedPlan(refused()),
        B::StatementPlan(internal()), B::StatementPlan(refused()),
        B::ReindexNoHeaps(RelId(3)), B::VacuumNoHeaps(RelId(3)),
        B::UnstoredIndex { position: 2, relation: r() },
        B::Panicked("boom".into()),
        B::NotAnIndex { index: IndexId(5), relation: r() },
        B::DuplicateKey(vec![1, 2]), B::DenialNotSet(Val::Int(1)),
        B::UniqueUnindexed { position: 1, relation: r() },
        B::UndeclaredIndex { index: IndexId(7), relation: r() },
        B::KeyNotFields(r()), B::VacuumTooLarge { len: 9000, limit: 4000 },
        B::ReadOnly, B::ScanPathNotText, B::RowsNotSet(Val::Int(1)), B::FailNotText, B::NoArm(4),
        B::Unreachable, B::NotAWrite(k()), B::ReplaceNotLam, B::RejectsWraps(k()),
        B::NotABoundWrite(k()), B::ScanUnread, B::RejectedRowNotRecord, B::NoUnique(2),
        B::UniqueIndexMissing,
        B::ReferenceUnindexed { relation: "app::us".into(), field: "f".into() },
        B::TopNotFor, B::RowsNotSetTyped(int()), B::InputNotSet(int()), B::TopBinder(k()),
        B::PriorNotSet(int()), B::Unprintable(PrintError { message: "m".into() }),
        B::Undeclared { module: "sys".into(), name: "calls".into() },
        B::NoValueOutcome, B::NoSettingOutcome, B::NoPointOutcome, B::NoTrialOutcome,
        B::RefusalUnrecorded(denial()), B::WitnessDenialUnrecorded(denial()),
        B::CallKeyNotInt, B::ExportNotSetTyped, B::ExportNotSet, B::NoViolation,
        B::SettingNotRecord, B::SettingField("f".into()), B::Denials(missing()),
        B::Expiry(missing()), B::DeploymentRefused,
    ];
    for broken in cases {
        assert_eq!(expected(&broken), broken.to_string(), "{broken:?}");
    }
}

#[test]
fn a_term_kind_holding_a_nan_constant_equals_itself() {
    let nan = TermKind(Kind::Const(Const::Float(f64::NAN)));
    assert_eq!(nan, nan.clone());
    assert_ne!(nan, TermKind(Kind::Const(Const::Float(1.0))));
}

#[test]
fn detail_reads_an_internal_cause_as_the_debug_of_its_text() {
    let internal = Error::from(Broken::ReadOnly);
    let text = "Internal(\"a read-only transaction runs no write\")";
    assert_eq!(text, detail(&internal));
    assert_eq!("Cancelled", detail(&Error::Cancelled));
}
