use adtb_lang::kernel::{Const, Kind};
use adtb_lang::textual::PrintError;
use adtb_lang::types::{Base, Ty};

use crate::api::{self, RejectReason};
use crate::catalog;
use crate::exec::Val;
use crate::plan;

use super::super::Error;
use super::{Broken, Broken as B, TermKind, detail};

/// Returns the case that follows `b`, the first when `b` is `None`, and the text it reads as.
/// A walk from `None` reaches every variant.
#[rustfmt::skip]
fn after(b: Option<&Broken>) -> Option<(Broken, &'static str)> {
    let int = || Ty::Base(Base::I64);
    let k = || TermKind(Kind::EmptySet);
    let r = || "us".to_string();
    let catalog_refused = || catalog::Error::Refused(vec!["r".into()]);
    let denial = || {
        Box::new(api::Error::Rejected {
            reason: RejectReason::Internal { what: "w".into() },
        })
    };
    let internal = || plan::Error::Internal(plan::Broken::GroupFieldMissing);
    let refused = || plan::Error::NotAQuery("a write");
    Some(match b {
        None => (B::BoundedPlan(internal()), "a group has a key and rows"),
        Some(B::BoundedPlan(plan::Error::Internal(_))) => (B::BoundedPlan(refused()), "a plan computes a query, not a write"),
        Some(B::BoundedPlan(_)) => (B::StatementPlan(internal()), "a group has a key and rows"),
        Some(B::StatementPlan(plan::Error::Internal(_))) => (B::StatementPlan(refused()), "a plan computes a query, not a write"),
        Some(B::StatementPlan(_)) => (B::DuplicateKey(vec![1, 2]), "a duplicate of key [1, 2] in a unique index"),
        Some(B::DuplicateKey(_)) => (B::DenialNotSet(Val::Int(1)), "a denial query returns a set, found Int(1)"),
        Some(B::DenialNotSet(_)) => (B::UniqueUnindexed { position: 1, relation: r() }, "unique 1 of relation us has an index"),
        Some(B::UniqueUnindexed { .. }) => (B::KeyNotFields(r()), "the key of us is fields"),
        Some(B::KeyNotFields(_)) => (B::ReadOnly, "a read-only transaction runs no write"),
        Some(B::ReadOnly) => (B::ScanPathNotText, "a file scan's path is a Text"),
        Some(B::ScanPathNotText) => (B::RowsNotSet(Val::Int(1)), "a write's rows are a set, found Int(1)"),
        Some(B::RowsNotSet(_)) => (B::FailNotText, "`fail` takes a Text"),
        Some(B::FailNotText) => (B::NoArm(4), "a case has an arm for constructor 4"),
        Some(B::NoArm(_)) => (B::Unreachable, "evaluation reaches no position that exhaustiveness or a dominating test excludes"),
        Some(B::Unreachable) => (B::NotAWrite(k()), "a write term is a write, found EmptySet"),
        Some(B::NotAWrite(_)) => (B::ReplaceNotLam, "a replace's new value is a `Lam`"),
        Some(B::ReplaceNotLam) => (B::RejectsWraps(k()), "`rejects into` wraps an insert or upsert, not EmptySet"),
        Some(B::RejectsWraps(_)) => (B::NotABoundWrite(k()), "a bound write is a replace, an upsert or `rejects into`, found EmptySet"),
        Some(B::NotABoundWrite(_)) => (B::ScanUnread, "every file scan of a statement is read before it runs"),
        Some(B::ScanUnread) => (B::RejectedRowNotRecord, "`RejectedRow` is a declared record"),
        Some(B::RejectedRowNotRecord) => (B::NoUnique(2), "an upsert names unique 2 of its relation"),
        Some(B::NoUnique(_)) => (B::ReferenceUnindexed { relation: "app::us".into(), field: "f".into() }, "a reference field has an index: app::us.f has none"),
        Some(B::ReferenceUnindexed { .. }) => (B::TopNotFor, "a top level read is the source of a `For`"),
        Some(B::TopNotFor) => (B::RowsNotSetTyped(int()), "a read of rows is a set, not Base(I64)"),
        Some(B::RowsNotSetTyped(_)) => (B::InputNotSet(int()), "a change set input is a set, not Base(I64)"),
        Some(B::InputNotSet(_)) => (B::TopBinder(k()), "a top level binder is a `For` or a `Let`, not EmptySet"),
        Some(B::TopBinder(_)) => (B::PriorNotSet(int()), "a prior relation is a set, not Base(I64)"),
        Some(B::PriorNotSet(_)) => (B::Unprintable(PrintError { message: "m".into() }), "printing a cell: PrintError { message: \"m\" }"),
        Some(B::Unprintable(_)) => (B::Undeclared { module: "sys".into(), name: "calls".into() }, "the engine declares `sys::calls`"),
        Some(B::Undeclared { .. }) => (B::NoValueOutcome, "a statement of one item has one outcome"),
        Some(B::NoValueOutcome) => (B::NoSettingOutcome, "a statement of one item has one outcome"),
        Some(B::NoSettingOutcome) => (B::NoPointOutcome, "a statement of one item has one outcome"),
        Some(B::NoPointOutcome) => (B::NoTrialOutcome, "a trial of one statement has one outcome"),
        Some(B::NoTrialOutcome) => (B::RefusalUnrecorded(denial()), "recording a denied call: Rejected { reason: Internal { what: \"w\" } }"),
        Some(B::RefusalUnrecorded(_)) => (B::WitnessDenialUnrecorded(denial()), "recording a denied call: Rejected { reason: Internal { what: \"w\" } }"),
        Some(B::WitnessDenialUnrecorded(_)) => (B::CallKeyNotInt, "a key of `sys::calls` is an I64"),
        Some(B::CallKeyNotInt) => (B::ExportNotSetTyped, "an export's source is a set"),
        Some(B::ExportNotSetTyped) => (B::ExportNotSet, "an export's source is a set"),
        Some(B::ExportNotSet) => (B::SettingNotRecord, "a setting's value is a record"),
        Some(B::SettingNotRecord) => (B::SettingField("f".into()), "a setting's value has field `f`"),
        Some(B::SettingField(_)) => (B::CatalogError(catalog_refused()), "Refused([\"r\"])"),
        Some(B::CatalogError(_)) => (B::DeploymentRefused, "a deployment command was refused"),
        Some(B::DeploymentRefused) => return None,
    })
}

#[test]
fn each_broken_guarantee_reads_as_its_text() {
    let mut seen = Vec::new();
    for (broken, text) in std::iter::successors(after(None), |(b, _)| after(Some(b))) {
        assert!(!seen.contains(&broken), "the walk returns to {broken:?}");
        assert_eq!(text, broken.to_string(), "{broken:?}");
        seen.push(broken);
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
    assert_eq!(
        "Cancelled",
        detail(&Error::Exec(crate::exec::Error::Cancelled))
    );
}

#[test]
fn detail_reads_a_quarantined_failure_as_the_debug_of_its_objects() {
    use crate::storage::{MarkFailure, Pid, Quarantine};
    let e = Error::Quarantined {
        objects: [
            (Quarantine::Heap(3), None),
            (
                Quarantine::Database,
                Some(MarkFailure::Unverified { page: Pid(7) }),
            ),
        ]
        .into(),
        by: super::super::By::Admitted,
    };
    let text = "Quarantined(\"Database (the mark holds only in memory: Unverified { page: Pid(7) }), Heap(3)\")";
    assert_eq!(text, detail(&e));
}
