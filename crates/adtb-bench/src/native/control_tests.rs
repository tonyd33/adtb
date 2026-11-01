use super::*;
use adtb::{Error, RejectReason, Value};
use std::sync::atomic::AtomicU64;

static NEXT: AtomicU64 = AtomicU64::new(0);

#[path = "control_fixture.rs"]
mod fixture;

fn native(a: &Adtb, case: &fixture::Case) -> Result<Vec<Value>, Error> {
    let schema = a.db.schema();
    let sessions = schema.relation(fixture::relation(schema, "sessions"));
    let party_type = schema
        .ty(sessions.elem_def)
        .header()
        .iter()
        .find(|field| field.name == "party")
        .expect("declared party field")
        .ty
        .clone();
    let party = Value::Int(case.field("party").parse().expect("runtime party key"));
    a.control(case.field("adtb"), &[("control_party", party_type, party)])
}

fn refusal(case: &fixture::Case, schema: &adtb_lang::schema::Schema) -> Result<Vec<Value>, Error> {
    let parts: Vec<_> = case.field("native").split('|').collect();
    let reason = match parts.as_slice() {
        ["ok"] => return Ok(Vec::new()),
        ["key", relation, key] => RejectReason::KeyViolation {
            relation: (*relation).into(),
            key: (*key).into(),
        },
        ["violation", constraint] => RejectReason::Violated {
            constraint: (*constraint).into(),
            report: fixture::report(schema, case.field("report")),
            redacted: 0,
        },
        ["fault", "NoSuchRow"] => RejectReason::Faulted {
            fault: adtb::Fault::NoSuchRow,
        },
        ["invalid", detail] => RejectReason::Invalid {
            detail: (*detail).into(),
        },
        other => panic!("unknown native expectation {other:?}"),
    };
    Err(Error::Rejected { reason })
}

fn actual_schema() -> adtb_lang::schema::Schema {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/sqlite-controls/schema-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("fresh schema directory");
    let peer = Adtb::open(&root, 1, true).expect("actual declaration schema");
    let schema = peer.db.schema().clone();
    peer.close().expect("close schema peer");
    schema
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Native(Result<Vec<Value>, Error>),
    Sqlite(Result<Value, (i32, String)>),
}

fn raw(adapter: &Adapter) -> Option<Vec<super::super::sqlite::ControlRows>> {
    match adapter {
        Adapter::Sqlite(a) => Some(a.control_snapshot().expect("all stored SQLite columns")),
        Adapter::Adtb(_) => None,
    }
}

fn run_case(case: &fixture::Case, peer: Peer, schema: &adtb_lang::schema::Schema) {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sqlite-controls/tests");
    std::fs::create_dir_all(&scratch).expect("create control scratch");
    let root = scratch.join(format!(
        "{}-{peer:?}-{}-{}",
        case.field("name"),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).expect("fresh control directory");
    let initial = fixture::snapshot(schema, "planned.rows");
    let mut adapter = Adapter::open(peer, &root, 1, true).expect("open actual peer");
    adapter.seed(&initial).expect("seed actual peer");
    for action in match case.field("setup") {
        "planned" => Vec::new(),
        "live" => vec![Action::Start(Id(1))],
        "played" => vec![Action::Start(Id(1)), Action::Finish(Id(1))],
        other => panic!("unknown setup {other}"),
    } {
        let mut prepared = adapter.prepare().expect("prepare setup");
        assert_eq!(
            prepared
                .call(action, Duration::from_secs(10))
                .expect("setup call")
                .0,
            model::expected_output(action)
        );
    }
    let before = fixture::snapshot(schema, &format!("{}.rows", case.field("setup")));
    assert_eq!(
        adapter.snapshot().expect("before state"),
        before,
        "{} {peer:?}",
        case.field("name")
    );
    let raw_before = raw(&adapter);
    let (actual, expected_outcome) = match &adapter {
        Adapter::Adtb(a) => {
            let mut actual = native(a, case);
            if let Err(Error::Rejected {
                reason: RejectReason::Violated { report, .. },
            }) = &mut actual
            {
                report.sort();
            }
            (
                Outcome::Native(actual),
                Outcome::Native(refusal(case, schema)),
            )
        }
        Adapter::Sqlite(a) => {
            let actual = a
                .control(case.field("sqlite"))
                .map(|changed| Value::Int(i64::try_from(changed).expect("affected count fits")))
                .map_err(|e| match e {
                    rusqlite::Error::SqliteFailure(code, message) => {
                        (code.extended_code, message.expect("SQLite refusal message"))
                    }
                    other => panic!("unexpected SQLite error {other:?}"),
                });
            let (code, payload) = case.field("sql").split_once('|').expect("code|payload");
            let expected = if code == "ok" {
                Ok(Value::Int(
                    payload.parse().expect("expected affected count"),
                ))
            } else {
                Err((code.parse().expect("extended code"), payload.to_owned()))
            };
            (Outcome::Sqlite(actual), Outcome::Sqlite(expected))
        }
    };
    let raw_after = raw(&adapter);
    let expected = fixture::snapshot(schema, case.field("after"));
    if case.field("native") != "ok" {
        assert_eq!(expected, before, "refused case preserves entire state");
        assert_eq!(
            raw_after, raw_before,
            "refused case preserves every stored SQLite column"
        );
    }
    assert_eq!(
        adapter.snapshot().expect("after state"),
        expected,
        "{} {peer:?}",
        case.field("name")
    );
    adapter.close().expect("close control peer");
    let reopened = Adapter::open(peer, &root, 1, false).expect("reopen actual file");
    assert_eq!(
        reopened.snapshot().expect("reopened state"),
        expected,
        "{} {peer:?}",
        case.field("name")
    );
    assert_eq!(
        raw(&reopened),
        raw_after,
        "every stored SQLite column survives reopen"
    );
    reopened.close().expect("close reopened file");
    assert_eq!(actual, expected_outcome, "{} {peer:?}", case.field("name"));
}

fn family(name: &str) {
    let schema = actual_schema();
    let cases = fixture::cases("passing.txt");
    let coverage = fixture::read("coverage.txt");
    let mut expected = BTreeMap::new();
    for line in coverage.lines() {
        let (family, count) = line.split_once('|').expect("family|count");
        assert!(
            expected
                .insert(family, count.parse::<usize>().expect("case count"))
                .is_none(),
            "duplicate family"
        );
    }
    let families = [
        "atomic",
        "incoming",
        "key-positive",
        "keys",
        "lifecycle-positive",
        "membership",
        "outgoing",
        "scope-positive",
        "scoped-unique",
        "selection",
        "shape-domains",
        "shape-fields",
        "transition",
    ];
    assert_eq!(expected.keys().copied().collect::<Vec<_>>(), families);
    assert_eq!(
        expected.values().sum::<usize>(),
        31,
        "all matrix cases are required"
    );
    let mut found = BTreeMap::new();
    for case in &cases {
        *found.entry(case.field("family")).or_insert(0usize) += 1;
    }
    assert_eq!(
        found, expected,
        "no missing, duplicated or unused matrix family"
    );
    let mut reached = 0;
    for case in cases.iter().filter(|case| case.field("family") == name) {
        for peer in [Peer::Adtb, Peer::Sqlite] {
            run_case(case, peer, &schema);
        }
        reached += 1;
    }
    assert_eq!(reached, expected[name]);
    assert!(reached > 0);
}

macro_rules! control_families {
    ($($test:ident => $family:literal),+ $(,)?) => {$(
        #[test]
        fn $test() { family($family); }
    )+};
}

control_families! {
    declaration_keys => "keys",
    outgoing_references => "outgoing",
    incoming_references => "incoming",
    campaign_membership => "membership",
    scoped_live_unique => "scoped-unique",
    played_transition => "transition",
    shape_field_admission => "shape-fields",
    shape_domain_admission => "shape-domains",
    key_and_reference_positive => "key-positive",
    lifecycle_positive => "lifecycle-positive",
    constructor_scope_positive => "scope-positive",
    least_constraint_selection => "selection",
    whole_change_set_atomicity => "atomic",
}

#[test]
fn sqlite_unconstructible_tagged_shapes() {
    let schema = actual_schema();
    let cases = fixture::cases("sqlite-shapes.txt");
    assert_eq!(cases.len(), 10);
    for case in &cases {
        assert_eq!(case.field("native"), "excluded|generated-enum");
        run_case(case, Peer::Sqlite, &schema);
    }
}

#[test]
fn sqlite_unscoped_uniques() {
    let schema = actual_schema();
    let cases = fixture::cases("canonical-names.txt");
    assert_eq!(cases.len(), 2);
    for case in &cases {
        run_case(case, Peer::Sqlite, &schema);
    }
}

fn canonical(family: &str) {
    let schema = actual_schema();
    let cases = fixture::cases("canonical-names.txt");
    assert_eq!(cases.len(), 2);
    let selected: Vec<_> = cases
        .iter()
        .filter(|case| case.field("family") == family)
        .collect();
    assert_eq!(selected.len(), 1);
    run_case(selected[0], Peer::Adtb, &schema);
}

#[test]
#[ignore = "work/issues/unscoped-unique-canonical-name.md: unscoped unique names include a constructor scope"]
fn entity_unique_reports_canonical_name() {
    canonical("entity-unique");
}

#[test]
#[ignore = "work/issues/unscoped-unique-canonical-name.md: unscoped unique names include a constructor scope"]
fn session_unique_reports_canonical_name() {
    canonical("session-unique");
}
