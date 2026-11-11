use std::time::Duration;

use adtb_lang::textual::parse;
use adtb_lang::types::Ty;

use crate::api::{
    CallOptions, Error, FatalReason, Fault, Limit, Log, Position, RejectReason, RetryReason, TxnId,
};

use super::tests::opened;
use super::{CallOrigin, CallRow, Form, Val, print};

fn retry(reason: RetryReason) -> Error {
    Error::Retry { reason }
}

fn reject(reason: RejectReason) -> Error {
    Error::Rejected { reason }
}

fn fatal(reason: FatalReason) -> Error {
    Error::Fatal { reason }
}

fn text(s: &str) -> String {
    s.into()
}

fn at(epoch: u64, lsn: u64) -> Position {
    Position { epoch, lsn }
}

fn retry_cases() -> Vec<(Error, &'static str)> {
    use RetryReason as R;
    let ms = Duration::from_millis;
    vec![
        (
            retry(R::Serialization {
                txn: TxnId(7),
                on: text("shop::orders"),
            }),
            r#"Retry { reason: Serialization { txn: TxnId(7), on: "shop::orders" } }"#,
        ),
        (
            retry(R::SafePointTimeout { waited: ms(1500) }),
            "Retry { reason: SafePointTimeout { waited: 1500ms } }",
        ),
        (
            retry(R::Overloaded {
                retry_after: ms(20),
            }),
            "Retry { reason: Overloaded { retry_after: 20ms } }",
        ),
        (
            retry(R::StreamIdle { limit: ms(3000) }),
            "Retry { reason: StreamIdle { limit: 3000ms } }",
        ),
        (
            retry(R::FileChanged {
                path: text("in/a.csv"),
            }),
            r#"Retry { reason: FileChanged { path: "in/a.csv" } }"#,
        ),
        (retry(R::Fenced), "Retry { reason: Fenced }"),
        (
            retry(R::ReplayConflict { waited: ms(5) }),
            "Retry { reason: ReplayConflict { waited: 5ms } }",
        ),
    ]
}

fn named_cases() -> Vec<(Error, &'static str)> {
    use RejectReason as R;
    vec![
        (
            reject(R::KeyViolation {
                relation: text("shop::customers"),
                key: text("7"),
            }),
            r#"Rejected { reason: KeyViolation { relation: "shop::customers", key: "7" } }"#,
        ),
        (
            reject(R::KeyChanged {
                relation: text("shop::customers"),
            }),
            r#"Rejected { reason: KeyChanged { relation: "shop::customers" } }"#,
        ),
        (
            reject(R::NotOpen {
                ctor: text("Card"),
                stage: text("Expanding"),
            }),
            r#"Rejected { reason: NotOpen { ctor: "Card", stage: "Expanding" } }"#,
        ),
        (
            reject(R::UserError {
                message: text("no \"stock\"\nleft"),
            }),
            r#"Rejected { reason: UserError { message: "no \"stock\"\nleft" } }"#,
        ),
        (
            reject(R::Faulted {
                fault: Fault::DivisionByZero,
            }),
            "Rejected { reason: Faulted { fault: DivisionByZero } }",
        ),
        (
            reject(R::Denied {
                detail: text("no read on shop::orders"),
            }),
            r#"Rejected { reason: Denied { detail: "no read on shop::orders" } }"#,
        ),
        (
            reject(R::Invalid {
                detail: text("no `nosuch` in scope"),
            }),
            r#"Rejected { reason: Invalid { detail: "no `nosuch` in scope" } }"#,
        ),
        (
            reject(R::SchemaRefused {
                operator: text("add type P"),
                detail: text("no type `Int`"),
            }),
            r#"Rejected { reason: SchemaRefused { operator: "add type P", detail: "no type `Int`" } }"#,
        ),
        (
            reject(R::Untranslatable {
                query: text("totals"),
                detail: text("a fixpoint"),
            }),
            r#"Rejected { reason: Untranslatable { query: "totals", detail: "a fixpoint" } }"#,
        ),
        (
            reject(R::BadInput {
                source: text("in/a.csv"),
                line: 12,
                detail: text("no field `id`"),
            }),
            r#"Rejected { reason: BadInput { source: "in/a.csv", line: 12, detail: "no field `id`" } }"#,
        ),
        (
            reject(R::SchemaMismatch {
                detail: text("no relation shop::orders"),
            }),
            r#"Rejected { reason: SchemaMismatch { detail: "no relation shop::orders" } }"#,
        ),
    ]
}

fn plain_cases() -> Vec<(Error, &'static str)> {
    use RejectReason as R;
    vec![
        (reject(R::Cancelled), "Rejected { reason: Cancelled }"),
        (reject(R::Terminated), "Rejected { reason: Terminated }"),
        (
            reject(R::DeadlineExceeded),
            "Rejected { reason: DeadlineExceeded }",
        ),
        (
            reject(R::StatementLimit {
                limit: Limit::Time {
                    after: Duration::from_millis(1500),
                },
            }),
            "Rejected { reason: StatementLimit { limit: Time { after: 1500ms } } }",
        ),
        (
            reject(R::StatementLimit {
                limit: Limit::Memory { bytes: 4096 },
            }),
            "Rejected { reason: StatementLimit { limit: Memory { bytes: 4096 } } }",
        ),
        (
            reject(R::StatementLimit {
                limit: Limit::Rounds { rounds: 9 },
            }),
            "Rejected { reason: StatementLimit { limit: Rounds { rounds: 9 } } }",
        ),
        (
            reject(R::TransactionExpired {
                limit: Duration::from_millis(250),
                idle: true,
            }),
            "Rejected { reason: TransactionExpired { limit: 250ms, idle: true } }",
        ),
        (
            reject(R::TransactionBusy),
            "Rejected { reason: TransactionBusy }",
        ),
        (
            reject(R::MixedVersions),
            "Rejected { reason: MixedVersions }",
        ),
        (reject(R::UnknownQuery), "Rejected { reason: UnknownQuery }"),
        (
            reject(R::UnknownTransaction),
            "Rejected { reason: UnknownTransaction }",
        ),
        (
            reject(R::RequestExpired),
            "Rejected { reason: RequestExpired }",
        ),
        (
            reject(R::RequestReused),
            "Rejected { reason: RequestReused }",
        ),
        (reject(R::AuthFailed), "Rejected { reason: AuthFailed }"),
        (
            reject(R::EditionLocked),
            "Rejected { reason: EditionLocked }",
        ),
        (
            reject(R::MigrationInProgress),
            "Rejected { reason: MigrationInProgress }",
        ),
        (reject(R::Closed), "Rejected { reason: Closed }"),
    ]
}

fn bundle_cases() -> Vec<(Error, &'static str)> {
    use RejectReason as R;
    vec![
        (
            reject(R::UnknownBundle {
                bundle: text("web-1.0"),
            }),
            r#"Rejected { reason: UnknownBundle { bundle: "web-1.0" } }"#,
        ),
        (
            reject(R::VersionMismatch {
                bundle: text("web-1.0"),
                version: 3,
            }),
            r#"Rejected { reason: VersionMismatch { bundle: "web-1.0", version: 3 } }"#,
        ),
        (
            reject(R::CatalogMismatch {
                bundle: text("web-1.0"),
                catalog: 2,
            }),
            r#"Rejected { reason: CatalogMismatch { bundle: "web-1.0", catalog: 2 } }"#,
        ),
        (
            reject(R::Retired {
                bundle: text("web-1.0"),
            }),
            r#"Rejected { reason: Retired { bundle: "web-1.0" } }"#,
        ),
        (
            reject(R::BundleExpired {
                bundle: text("web-1.0"),
                query: text("totals"),
            }),
            r#"Rejected { reason: BundleExpired { bundle: "web-1.0", query: "totals" } }"#,
        ),
        (
            reject(R::BundleStale {
                bundle: text("web-1.0"),
                ctor: text("Card"),
            }),
            r#"Rejected { reason: BundleStale { bundle: "web-1.0", ctor: "Card" } }"#,
        ),
        (
            reject(R::BundleSuperseded {
                bundle: text("web-2.0"),
            }),
            r#"Rejected { reason: BundleSuperseded { bundle: "web-2.0" } }"#,
        ),
        (
            reject(R::DowngradeRefused {
                cause: text("format 3"),
            }),
            r#"Rejected { reason: DowngradeRefused { cause: "format 3" } }"#,
        ),
        (
            reject(R::WrongVersion { version: 4 }),
            "Rejected { reason: WrongVersion { version: 4 } }",
        ),
        (
            reject(R::UnsupportedProtocol {
                min: text("1.0"),
                max: text("1.2"),
            }),
            r#"Rejected { reason: UnsupportedProtocol { min: "1.0", max: "1.2" } }"#,
        ),
        (
            reject(R::UnknownEdition {
                edition: text("2031-01"),
            }),
            r#"Rejected { reason: UnknownEdition { edition: "2031-01" } }"#,
        ),
    ]
}

fn place_cases() -> Vec<(Error, &'static str)> {
    use RejectReason as R;
    vec![
        (
            reject(R::TooOld { oldest: at(1, 40) }),
            "Rejected { reason: TooOld { oldest: Position { epoch: 1, lsn: 40 } } }",
        ),
        (
            reject(R::HistoryUnavailable { oldest: at(1, 40) }),
            "Rejected { reason: HistoryUnavailable { oldest: Position { epoch: 1, lsn: 40 } } }",
        ),
        (
            reject(R::PositionNotOnHistory { position: at(2, 9) }),
            "Rejected { reason: PositionNotOnHistory { position: Position { epoch: 2, lsn: 9 } } }",
        ),
        (
            reject(R::RestorePointUnavailable {
                oldest: at(1, 40),
                newest: at(2, 9),
            }),
            "Rejected { reason: RestorePointUnavailable { oldest: Position { epoch: 1, lsn: 40 }, \
             newest: Position { epoch: 2, lsn: 9 } } }",
        ),
        (
            reject(R::ReadOnlyReplica {
                primary: text("db-1:7000"),
            }),
            r#"Rejected { reason: ReadOnlyReplica { primary: "db-1:7000" } }"#,
        ),
        (
            reject(R::UnknownDatabase {
                database: text("shop"),
            }),
            r#"Rejected { reason: UnknownDatabase { database: "shop" } }"#,
        ),
        (
            reject(R::DumpDamaged {
                part: text("manifest"),
            }),
            r#"Rejected { reason: DumpDamaged { part: "manifest" } }"#,
        ),
        (
            reject(R::BackupDamaged {
                part: text("segment 3"),
            }),
            r#"Rejected { reason: BackupDamaged { part: "segment 3" } }"#,
        ),
        (
            reject(R::Internal {
                what: text("a write term is a write"),
            }),
            r#"Rejected { reason: Internal { what: "a write term is a write" } }"#,
        ),
        (
            reject(R::RowTooLarge {
                relation: text("shop::orders"),
                ctor: text("Order"),
                size: 9000,
                limit: 8000,
            }),
            r#"Rejected { reason: RowTooLarge { relation: "shop::orders", ctor: "Order", size: 9000, limit: 8000 } }"#,
        ),
        (
            reject(R::KeyTooLarge {
                relation: text("shop::orders"),
                ctor: text("Order"),
                index: text("by_note"),
                size: 5000,
                limit: 4000,
            }),
            r#"Rejected { reason: KeyTooLarge { relation: "shop::orders", ctor: "Order", index: "by_note", size: 5000, limit: 4000 } }"#,
        ),
    ]
}

fn fatal_cases() -> Vec<(Error, &'static str)> {
    use FatalReason as F;
    vec![
        (
            fatal(F::Malformed {
                detail: text("a frame past its stream"),
            }),
            r#"Fatal { reason: Malformed { detail: "a frame past its stream" } }"#,
        ),
        (
            fatal(F::Quarantined {
                what: text("shop::orders"),
            }),
            r#"Fatal { reason: Quarantined { what: "shop::orders" } }"#,
        ),
        (
            fatal(F::MissingFunction {
                function: text("shop::rate"),
            }),
            r#"Fatal { reason: MissingFunction { function: "shop::rate" } }"#,
        ),
        (
            fatal(F::Storage {
                detail: text("the disk is full"),
            }),
            r#"Fatal { reason: Storage { detail: "the disk is full" } }"#,
        ),
    ]
}

const SLOTS: usize = 62;

fn slot(e: &Error) -> usize {
    use RejectReason as R;
    match e {
        Error::Retry { reason } => match reason {
            RetryReason::Serialization { .. } => 0,
            RetryReason::SafePointTimeout { .. } => 1,
            RetryReason::Overloaded { .. } => 2,
            RetryReason::StreamIdle { .. } => 3,
            RetryReason::FileChanged { .. } => 4,
            RetryReason::Fenced => 5,
            RetryReason::ReplayConflict { .. } => 6,
        },
        Error::Rejected { reason } => match reason {
            R::Violated { .. } => 7,
            R::KeyViolation { .. } => 8,
            R::KeyChanged { .. } => 9,
            R::NotOpen { .. } => 10,
            R::UserError { .. } => 11,
            R::Faulted { .. } => 12,
            R::Denied { .. } => 13,
            R::Invalid { .. } => 14,
            R::SchemaRefused { .. } => 15,
            R::Untranslatable { .. } => 16,
            R::BadInput { .. } => 17,
            R::SchemaMismatch { .. } => 18,
            R::Cancelled => 19,
            R::Terminated => 20,
            R::DeadlineExceeded => 21,
            R::StatementLimit { limit } => match limit {
                Limit::Time { .. } => 22,
                Limit::Memory { .. } => 23,
                Limit::Rounds { .. } => 24,
            },
            R::TransactionExpired { .. } => 25,
            R::TransactionBusy => 26,
            R::MixedVersions => 27,
            R::UnknownQuery => 28,
            R::UnknownTransaction => 29,
            R::UnknownBundle { .. } => 30,
            R::VersionMismatch { .. } => 31,
            R::CatalogMismatch { .. } => 32,
            R::Retired { .. } => 33,
            R::BundleExpired { .. } => 34,
            R::BundleStale { .. } => 35,
            R::BundleSuperseded { .. } => 36,
            R::DowngradeRefused { .. } => 37,
            R::TooOld { .. } => 38,
            R::HistoryUnavailable { .. } => 39,
            R::PositionNotOnHistory { .. } => 40,
            R::WrongVersion { .. } => 41,
            R::ReadOnlyReplica { .. } => 42,
            R::RequestExpired => 43,
            R::RequestReused => 44,
            R::AuthFailed => 45,
            R::UnsupportedProtocol { .. } => 46,
            R::UnknownDatabase { .. } => 47,
            R::EditionLocked => 48,
            R::UnknownEdition { .. } => 49,
            R::MigrationInProgress => 50,
            R::DumpDamaged { .. } => 51,
            R::BackupDamaged { .. } => 52,
            R::RestorePointUnavailable { .. } => 53,
            R::Internal { .. } => 54,
            R::RowTooLarge { .. } => 55,
            R::Closed => 56,
            R::KeyTooLarge { .. } => 57,
        },
        Error::Fatal { reason } => match reason {
            FatalReason::Malformed { .. } => 58,
            FatalReason::Quarantined { .. } => 59,
            FatalReason::MissingFunction { .. } => 60,
            FatalReason::Storage { .. } => 61,
        },
    }
}

/// Returns each error but `Violated` with the value of the prelude sum `Error` a client receives
/// for it.
fn cases() -> Vec<(Error, &'static str)> {
    let mut cases = retry_cases();
    cases.extend(named_cases());
    cases.extend(plain_cases());
    cases.extend(bundle_cases());
    cases.extend(place_cases());
    cases.extend(fatal_cases());
    cases
}

/// Returns the error each call's audit row holds, one call failing with each of `errors`, and the
/// form of the prelude sum `Error`.
fn recorded(errors: &[Error]) -> (Vec<Val>, Form) {
    let (_scratch, db) = opened(&Log::new(["module shop\nadd module shop\n"]));
    let engine = db.engine.clone();
    let caller = engine.principal(None).expect("principal");
    let origin = CallOrigin::Function { name: text("f") };
    for error in errors {
        engine
            .audit_call(&CallRow {
                role: "",
                caller: &caller,
                origin: &origin,
                args: &[0; 32],
                failed: Some(error),
            })
            .expect("the call's audit row is written");
    }
    let mut results = db
        .query_text(
            "from a in catalog::audit.Call select { n: a.seq, outcome: a.outcome }",
            &[],
            &CallOptions::default(),
        )
        .expect("read the audit rows");
    let rows = results.remove(0);
    let Form::Record(fields) = rows.form().clone() else {
        panic!("the select is a record")
    };
    let mut got: Vec<Val> = rows.map(|r| r.expect("audit row")).collect();
    got.sort();
    assert_eq!(errors.len(), got.len());
    let outcome_form = &fields[1].1;
    let failed = prelude_ctor(&engine.schema, "Outcome", "Failed");
    let held = errors
        .iter()
        .zip(&got)
        .map(|(error, row)| {
            let Val::Record(row) = row else {
                panic!("an audit row is a record, found {row:?}")
            };
            let outcome = &row[1];
            let shown = print(outcome, outcome_form).expect("the outcome prints");
            let Val::Ctor(c, fields) = outcome else {
                panic!("an outcome is a constructor, found {shown}")
            };
            assert_eq!(failed, *c, "{error:?} is recorded as {shown}");
            fields
                .first()
                .expect("a failed outcome holds its error")
                .clone()
        })
        .collect();
    let error_form = Form::of(
        &engine.schema,
        &Ty::Data {
            def: engine.schema.prelude_type("Error"),
            args: Vec::new(),
            ctors: None,
        },
    );
    (held, error_form)
}

#[test]
fn a_failed_calls_audit_row_holds_the_error_value_returned_for_every_reason_but_violated() {
    let cases = cases();
    let mut slots: Vec<usize> = cases.iter().map(|(e, _)| slot(e)).collect();
    slots.sort_unstable();
    let violated = 7;
    let all_but_violated: Vec<usize> = (0..SLOTS).filter(|s| *s != violated).collect();
    assert_eq!(all_but_violated, slots);
    let errors: Vec<Error> = cases.iter().map(|(e, _)| e.clone()).collect();
    let (held, error_form) = recorded(&errors);
    let mut wrong = Vec::new();
    for ((error, want), value) in cases.iter().zip(&held) {
        let expected: Val = parse(want, &error_form)
            .unwrap_or_else(|e| panic!("the expected value `{want}` reads: {e:?}"));
        if *value != expected {
            let shown = print(value, &error_form).expect("the error prints");
            wrong.push(format!("{error:?}\n  holds {shown}\n  want  {want}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
#[ignore = "a constraint report has no value form yet"]
fn a_violated_calls_audit_row_holds_the_violation_returned_with_its_report() {
    let error = reject(RejectReason::Violated {
        constraint: text("shop::paid"),
        report: vec![Val::Int(1)],
        redacted: 2,
    });
    assert_eq!(7, slot(&error));
    let (held, error_form) = recorded(std::slice::from_ref(&error));
    let schema = crate::catalog::engine_schema();
    let report: Vec<u8> = [0x04, 0x01].into_iter().chain(1i64.to_le_bytes()).collect();
    let expected = Val::ctor(
        prelude_ctor(&schema, "Error", "Rejected"),
        vec![Val::ctor(
            prelude_ctor(&schema, "RejectReason", "Violated"),
            vec![
                Val::text("shop::paid"),
                Val::Bytes(report.into()),
                Val::Int(2),
            ],
        )],
    );
    assert_eq!(
        expected,
        held[0],
        "holds {}",
        print(&held[0], &error_form).expect("the error prints")
    );
}

/// Returns the index of constructor `name` of the prelude type `ty`.
fn prelude_ctor(schema: &adtb_lang::schema::Schema, ty: &str, name: &str) -> u32 {
    schema
        .ty(schema.prelude_type(ty))
        .ctor_index(name)
        .unwrap_or_else(|| panic!("the prelude's {ty} declares {name}"))
}
