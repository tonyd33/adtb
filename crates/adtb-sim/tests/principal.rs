#[expect(
    dead_code,
    reason = "split the shared API helpers into independently imported modules"
)]
#[path = "support/api.rs"]
mod api;

use adtb::{
    CallOptions, Database, Error, Log, OpenOptions, Registration, RejectReason, TxnOptions, Value,
};
use adtb_sim::SimRuntime;

use api::{once, options, result};

fn open(rt: SimRuntime, field: &str) -> Database<SimRuntime> {
    open_with(rt, "", field)
}

fn open_with(rt: SimRuntime, declarations: &str, field: &str) -> Database<SimRuntime> {
    let schema = format!(
        "module identity\nadd module identity\n{declarations}\nadd type Principal = EndUser {{ {field} }} | Service {{ name: Text }} | Anonymous\n"
    );
    Database::open_in(
        rt,
        "principal.adt",
        &Log::new([schema]),
        &OpenOptions {
            registrations: vec![Registration::new(
                "principal",
                "import identity\nquery me () -> Principal = caller",
            )],
            ..options(2)
        },
    )
    .expect("the custom principal schema opens")
}

fn assertion(db: &Database<SimRuntime>, field: Value) -> Value {
    let end_user = db
        .schema()
        .ty(db.schema().principal)
        .ctor_index("EndUser")
        .unwrap();
    Value::ctor(end_user, vec![field])
}

fn outcomes(db: &Database<SimRuntime>, assertion: Value) -> [Result<(), Error>; 3] {
    let opts = CallOptions {
        assert: Some(assertion.clone()),
        ..CallOptions::default()
    };
    [
        db.query_text("caller", &[], &opts).map(|_| ()),
        db.registered("principal")
            .unwrap()
            .call(1, &[], &opts)
            .map(|_| ()),
        db.begin(&TxnOptions {
            assert: Some(assertion),
            ..TxnOptions::default()
        })
        .map(|_| ()),
    ]
}

fn refused_by_every_entry(db: &Database<SimRuntime>, field: Value) {
    let outcomes = outcomes(db, assertion(db, field));
    assert!(
        outcomes.iter().all(|got| matches!(
            got,
            Err(Error::Rejected {
                reason: RejectReason::Invalid { .. }
            })
        )),
        "ad hoc, registered and Begin must refuse an invalid principal: {outcomes:?}"
    );
}

fn values(mut results: Vec<adtb::DynRows>) -> Vec<Value> {
    assert_eq!(1, results.len());
    results.remove(0).collect::<Result<_, _>>().unwrap()
}

fn accepted_by_every_entry(db: &Database<SimRuntime>, field: Value) {
    let asserted = assertion(db, field);
    let opts = CallOptions {
        assert: Some(asserted.clone()),
        ..CallOptions::default()
    };
    assert_eq!(
        vec![asserted.clone()],
        values(db.query_text("caller", &[], &opts).unwrap())
    );
    let adtb::Called::Rows(rows) = db
        .registered("principal")
        .unwrap()
        .call(1, &[], &opts)
        .unwrap()
    else {
        panic!("the caller query returns rows");
    };
    assert_eq!(
        vec![asserted.clone()],
        rows.collect::<Result<Vec<_>, _>>().unwrap()
    );
    let mut transaction = db
        .begin(&TxnOptions {
            assert: Some(asserted.clone()),
            ..TxnOptions::default()
        })
        .unwrap();
    assert_eq!(
        vec![asserted],
        values(
            transaction
                .query_text("caller", &[], &adtb::TxnCallOptions::default())
                .unwrap()
        )
    );
    transaction.rollback().unwrap();
}

#[test]
fn an_out_of_range_asserted_integer_is_invalid_at_every_entry() {
    once(421, |rt| {
        let db = open(rt, "level: I8");
        refused_by_every_entry(&db, Value::Int(256));
    });
}

#[test]
fn a_nested_nullary_constructor_with_fields_is_an_invalid_assertion() {
    once(422, |rt| {
        let db = open(rt, "ready: Bool");
        refused_by_every_entry(&db, Value::ctor(0, vec![Value::Int(7)]));
    });
}

#[test]
fn text_construction_normalizes_canonical_equivalents() {
    assert_eq!(Value::text("\u{e9}"), Value::text("e\u{301}"));
}

#[test]
fn valid_integer_limits_and_anonymous_and_owner_controls_remain_accepted() {
    once(423, |rt| {
        let db = open(rt, "level: I8");
        accepted_by_every_entry(&db, Value::Int(-128));
        accepted_by_every_entry(&db, Value::Int(127));
        let principal = db.schema().ty(db.schema().principal);
        let anonymous = Value::tag(principal.ctor_index("Anonymous").unwrap());
        assert!(outcomes(&db, anonymous).iter().all(Result::is_ok));
        let expected = Value::ctor(
            principal.ctor_index("Service").unwrap(),
            vec![Value::text("owner")],
        );
        assert_eq!(
            vec![expected],
            values(
                db.query_text("caller", &[], &CallOptions::default())
                    .unwrap()
            )
        );
    });
}

#[test]
fn service_assertion_denial_precedes_nested_field_validation() {
    once(424, |rt| {
        let db = open(rt, "level: I8");
        let service = db
            .schema()
            .ty(db.schema().principal)
            .ctor_index("Service")
            .unwrap();
        let got = outcomes(&db, Value::ctor(service, vec![Value::Int(7)]));
        assert!(
            got.iter().all(|got| matches!(
                got,
                Err(Error::Rejected {
                    reason: RejectReason::Denied { .. }
                })
            )),
            "{got:?}"
        );
    });
}

#[test]
fn text_construction_canonicalizes_but_raw_noncanonical_assertions_are_refused() {
    once(425, |rt| {
        let db = open(rt, "name: Text");
        accepted_by_every_entry(&db, Value::text("e\u{301}"));
        refused_by_every_entry(&db, Value::Text(std::sync::Arc::from("e\u{301}")));
    });
}

#[test]
fn shared_headers_generic_fields_and_exact_arities_are_validated() {
    once(426, |rt| {
        let db = open_with(
            rt,
            "add type Envelope a { code: I16 } = Wrap { value: Option a }",
            "payload: Envelope I8",
        );
        let wrap = |code, value| Value::ctor(0, vec![Value::Int(code), value]);
        accepted_by_every_entry(&db, wrap(32767, Value::some(Value::Int(127))));
        refused_by_every_entry(&db, wrap(32768, Value::some(Value::Int(127))));
        refused_by_every_entry(&db, wrap(1, Value::some(Value::Int(128))));
        refused_by_every_entry(&db, Value::ctor(0, vec![Value::none()]));
        refused_by_every_entry(
            &db,
            Value::ctor(0, vec![Value::Int(1), Value::none(), Value::Int(2)]),
        );
    });
}

#[test]
fn newtype_predicates_remain_write_only_while_representations_are_checked() {
    once(427, |rt| {
        let declarations = "add type Positive = new I8 where self > 0\nadd type Item = { id: Key items, number: Positive }\nadd relation items : Item key id generate sequence";
        let db = open_with(rt, declarations, "number: Positive");
        accepted_by_every_entry(&db, Value::Int(-1));
        refused_by_every_entry(&db, Value::Int(128));
        assert!(matches!(
            db.query_text(
                "insert identity::items { number: identity::Positive(-1) }",
                &[],
                &CallOptions::default()
            ),
            Err(Error::Rejected {
                reason: RejectReason::Violated { .. }
            })
        ));
        let opts = CallOptions {
            assert: Some(assertion(&db, Value::Int(128))),
            ..CallOptions::default()
        };
        assert!(matches!(
            db.query_text(
                "insert identity::items { number: identity::Positive(1) }",
                &[],
                &opts
            ),
            Err(Error::Rejected {
                reason: RejectReason::Invalid { .. }
            })
        ));
        assert_eq!(
            vec![Value::Int(0)],
            result(&db, "count(i for i in identity::items)")
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        );
    });
}

#[test]
fn key_shape_validation_does_not_authorize_or_require_a_target_row() {
    once(428, |rt| {
        let declarations = "add type Member = { id: Key members, name: Text }\nadd relation members : Member key id generate sequence";
        let db = open_with(rt, declarations, "member: Key members");
        accepted_by_every_entry(&db, Value::Int(999));
        refused_by_every_entry(&db, Value::text("999"));
    });
}

#[test]
fn spans_require_valid_ordered_endpoints_and_dates_keep_their_range() {
    once(429, |rt| {
        let db = open(rt, "during: Span I8");
        accepted_by_every_entry(&db, Value::ctor(0, vec![Value::Int(-1), Value::Int(1)]));
        refused_by_every_entry(&db, Value::ctor(0, vec![Value::Int(2), Value::Int(1)]));
        refused_by_every_entry(&db, Value::ctor(0, vec![Value::Int(-129), Value::Int(1)]));
    });
    once(430, |rt| {
        let db = open(rt, "at: Date");
        accepted_by_every_entry(&db, Value::Date(0));
        refused_by_every_entry(&db, Value::Date(i32::MAX));
    });
}
