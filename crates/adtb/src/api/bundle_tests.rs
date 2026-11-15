use super::*;

fn row(id: i64, name: &str, family: &str) -> Row {
    Row {
        id,
        header: Header {
            name: name.to_owned(),
            family: family.to_owned(),
            version: "1".to_owned(),
            pins: 1,
            hash: String::new(),
            catalog: None,
            edition: EDITION.to_owned(),
            lease: None,
            hides_new: false,
            priority: exec::Priority::Regular,
        },
        owner: "owner".to_owned(),
        renewed: 0,
        state: State::Active,
        latched: false,
    }
}

#[test]
fn a_file_recording_no_family_owners_gives_the_schema_owner_every_family_it_holds() {
    let mut stored = Stored::default();
    for (id, name, family) in [
        (1, "web-1", "web"),
        (2, "web-2", "web"),
        (3, "ios-1", "ios"),
    ] {
        stored
            .bundles
            .insert(name.to_owned(), row(id, name, family));
    }
    assert_eq!(
        vec![
            Fact::Deployed,
            Fact::Family("ios".to_owned(), OWNER_ID),
            Fact::Family("web".to_owned(), OWNER_ID),
        ],
        legacy(&stored)
    );
    assert!(stored.deployed.registered.is_empty());
    stored.deployed.record(&legacy(&stored));
    assert_eq!(Vec::<Fact>::new(), legacy(&stored));
}

#[test]
fn a_new_file_records_family_owners_from_its_first_open() {
    assert_eq!(vec![Fact::Deployed], legacy(&Stored::default()));
}

#[test]
fn the_bundle_writer_waited_for_under_an_index_latch_panics() {
    let mut sim = adtb_sim::Sim::new(1, adtb_sim::Config::default());
    let outcome = sim.run(|rt| {
        let stored = StoredLock::new(Stored::default());
        let _latch = crate::ranks::INDEX_LATCH.hold();
        assert_eq!(
            crate::ranks::broken("the bundle writer lock", "an index latch"),
            crate::ranks::order_broken(|| drop(stored.write(&rt)))
        );
    });
    assert_eq!(adtb_sim::Outcome::Finished, outcome);
}

mod codecs {
    use adtb_lang::ids::{CatCtorId, engine_ctor as k};
    use adtb_lang::schema::prelude::ctor;
    use adtb_lang::schema::{self, ENGINE_SRC, Schema};

    use crate::catalog::codec::Codec;
    use crate::catalog::codec::testing::{position, reread, reversed, row, tag};
    use crate::catalog::{Rot, engine_schema};
    use crate::key::Value;

    use super::super::{CallRow, Callable, Header, Registered, Row, State};
    use crate::exec;

    const SECOND: i64 = 1_000_000;

    fn some(v: Value) -> Value {
        Value::Sum {
            ctor: ctor::SOME,
            fields: vec![v],
        }
    }

    fn none() -> Value {
        Value::Sum {
            ctor: ctor::NONE,
            fields: Vec::new(),
        }
    }

    fn truth(b: bool) -> Value {
        Value::Sum {
            ctor: ctor::bool(b),
            fields: Vec::new(),
        }
    }

    /// One bundle row's variable parts: its state, priority, catalog, lease and hides_new.
    struct Case {
        state: (CatCtorId, Option<i64>),
        priority: CatCtorId,
        catalog: Option<i64>,
        lease: Option<i64>,
        hides_new: bool,
    }

    /// Returns the stored row of `case` in `schema`, its fields otherwise `overrides` or fixed.
    fn bundle(schema: &Schema, case: &Case, overrides: &[(&str, Value)]) -> Vec<Value> {
        let codec = Codec::<Row>::new(schema);
        let at = (codec.relation(), position(schema, &codec, k::BUNDLE));
        let opt = |v: Option<Value>| v.map_or_else(none, some);
        let mut fields = vec![
            ("id", Value::I64(4)),
            ("name", Value::Text("web-2".into())),
            ("family", Value::Text("web".into())),
            ("owner", Value::Text("ops".into())),
            ("pins", Value::I64(3)),
            ("hash", Value::Bytes(vec![0xab, 0x01])),
            ("edition", Value::Text("2026-10".into())),
            ("catalog", opt(case.catalog.map(Value::I64))),
            ("hides_new", truth(case.hides_new)),
            ("lease", opt(case.lease.map(Value::Duration))),
            ("renewed", Value::Instant(77)),
            (
                "priority",
                tag(schema, at, "priority", case.priority, Vec::new()),
            ),
            ("memory_share", none()),
            ("workers", none()),
            (
                "state",
                tag(
                    schema,
                    at,
                    "state",
                    case.state.0,
                    case.state.1.map(Value::Instant).into_iter().collect(),
                ),
            ),
        ];
        for (name, v) in overrides {
            fields
                .iter_mut()
                .find(|(n, _)| n == name)
                .expect("an override names a field")
                .1 = v.clone();
        }
        row(schema, at.0, at.1, &fields)
    }

    fn expected(state: State, priority: exec::Priority, case: &Case) -> Row {
        Row {
            id: 4,
            header: Header {
                name: "web-2".into(),
                family: "web".into(),
                version: "2".into(),
                pins: 3,
                hash: "ab01".into(),
                catalog: case
                    .catalog
                    .map(|c| u64::try_from(c).expect("a sample catalog")),
                edition: "2026-10".into(),
                lease: case.lease,
                hides_new: case.hides_new,
                priority,
            },
            owner: "ops".into(),
            renewed: 77,
            state,
            latched: false,
        }
    }

    #[test]
    fn a_catalog_with_its_constructors_reversed_reads_each_stored_bundle_row_the_same() {
        let (shipped, reversed) = (engine_schema(), reversed());
        let (old, new) = (Codec::<Row>::new(&shipped), Codec::<Row>::new(&reversed));
        let cases = [
            (
                Case {
                    state: (k::BUNDLE_ACTIVE, None),
                    priority: k::PRIORITY_AD_HOC,
                    catalog: Some(9),
                    lease: Some(5 * SECOND),
                    hides_new: true,
                },
                State::Active,
                exec::Priority::AdHoc,
            ),
            (
                Case {
                    state: (k::BUNDLE_SUPERSEDED, Some(11)),
                    priority: k::PRIORITY_BACKGROUND,
                    catalog: None,
                    lease: None,
                    hides_new: false,
                },
                State::Superseded(11),
                exec::Priority::Background,
            ),
            (
                Case {
                    state: (k::BUNDLE_RETIRED, Some(12)),
                    priority: k::PRIORITY_REGULAR,
                    catalog: Some(0),
                    lease: None,
                    hides_new: true,
                },
                State::Retired(12),
                exec::Priority::Regular,
            ),
        ];
        let (was, now) = (
            position(&shipped, &old, k::BUNDLE),
            position(&reversed, &new, k::BUNDLE),
        );
        for (case, state, priority) in &cases {
            let stored = bundle(&shipped, case, &[]);
            let read = reread(
                (&shipped, old.relation(), was),
                (&reversed, new.relation(), now),
                &stored,
            );
            let want = expected(*state, *priority, case);
            assert_eq!(Ok(want.clone()), old.decode(was, &stored), "{state:?}");
            assert_eq!(Ok(want), new.decode(now, &read), "{state:?}");
            let nested = |s: &Schema, r: &[Value], name: &str| {
                let names = crate::catalog::layout::row(s, old.relation(), 0);
                let at = names.iter().position(|(n, _)| n == name).expect("a field");
                match &r[at] {
                    Value::Sum { ctor, .. } => *ctor,
                    other => panic!("{name} holds {other:?}"),
                }
            };
            for field in ["state", "priority"] {
                let moved = nested(&shipped, &stored, field) != nested(&reversed, &read, field);
                let middle = matches!(state, State::Superseded(_)) && field == "state"
                    || matches!(priority, exec::Priority::Background) && field == "priority";
                assert_eq!(!middle, moved, "{field} of {state:?} {priority:?}");
            }
        }
    }

    #[test]
    fn a_catalog_with_its_constructors_reversed_reads_each_stored_call_row_the_same() {
        let (shipped, reversed) = (engine_schema(), reversed());
        let (old, new) = (
            Codec::<Registered>::new(&shipped),
            Codec::<Registered>::new(&reversed),
        );
        let mut moved = Vec::new();
        for (id, kind) in [
            (k::REGISTERED_QUERY, Callable::Query),
            (k::REGISTERED_TXN, Callable::Txn),
            (k::REGISTERED_IMPORTED_FN, Callable::Fn),
        ] {
            let (was, now) = (position(&shipped, &old, id), position(&reversed, &new, id));
            let stored = row(
                &shipped,
                old.relation(),
                was,
                &[
                    ("id", Value::I64(8)),
                    ("bundle", Value::I64(4)),
                    ("name", Value::Text("q".into())),
                    ("ordinal", Value::I32(2)),
                    ("signature", Value::Text("q : I64".into())),
                    ("text", Value::Text("query q = 1".into())),
                    ("limit", none()),
                ],
            );
            let read = reread(
                (&shipped, old.relation(), was),
                (&reversed, new.relation(), now),
                &stored,
            );
            let want = Registered {
                id: 8,
                bundle: 4,
                call: CallRow {
                    kind,
                    name: "q".into(),
                    ordinal: 2,
                    signature: "q : I64".into(),
                    text: "query q = 1".into(),
                },
            };
            assert_eq!(Ok(want.clone()), old.decode(was, &stored), "{id}");
            assert_eq!(Ok(want), new.decode(now, &read), "{id}");
            if was != now {
                moved.push(kind);
            }
        }
        assert_eq!(vec![Callable::Query, Callable::Fn], moved);
    }

    fn active() -> Case {
        Case {
            state: (k::BUNDLE_ACTIVE, None),
            priority: k::PRIORITY_REGULAR,
            catalog: None,
            lease: None,
            hides_new: false,
        }
    }

    /// Returns the refusal of the bundle row of `active` with `overrides`.
    fn refusal(overrides: &[(&str, Value)]) -> Result<Row, Rot> {
        let schema = engine_schema();
        let codec = Codec::<Row>::new(&schema);
        let at = position(&schema, &codec, k::BUNDLE);
        codec.decode(at, &bundle(&schema, &active(), overrides))
    }

    fn damage(detail: &str) -> Rot {
        Rot::CatalogRow {
            row: "catalog::bundles.Bundle".into(),
            detail: detail.into(),
        }
    }

    #[test]
    fn a_bundle_name_without_a_version_is_refused_as_damage() {
        let detail = "names `web`, not a bundle name of a family, a hyphen and a version";
        assert_eq!(
            "a row of catalog::bundles.Bundle names `web`, not a bundle name of a family, a hyphen and a version",
            damage(detail).to_string()
        );
        assert_eq!(
            Err(damage(detail)),
            refusal(&[("name", Value::Text("web".into()))])
        );
        assert_eq!(
            Err(damage(
                "names `web-x`, not a bundle name of a family, a hyphen and a version"
            )),
            refusal(&[("name", Value::Text("web-x".into()))])
        );
    }

    #[test]
    fn a_bundle_name_of_another_family_than_its_row_is_refused_as_damage() {
        let detail = "names bundle `web-2` under family `ios`, not `web`";
        assert_eq!(
            "a row of catalog::bundles.Bundle names bundle `web-2` under family `ios`, not `web`",
            damage(detail).to_string()
        );
        assert_eq!(
            Err(damage(detail)),
            refusal(&[("family", Value::Text("ios".into()))])
        );
    }

    #[test]
    #[should_panic(
        expected = "the codec of catalog::bundles lists the payload of constructor 170 of field `state`"
    )]
    fn a_codec_listing_a_nested_payload_of_other_types_than_declared_does_not_build() {
        let text = ENGINE_SRC.replace(
            "type BundleState = Active | Superseded { at: Instant } | Retired { at: Instant }",
            "type BundleState = Active | Superseded { at: I64 } | Retired { at: Instant }",
        );
        let schema = schema::build_engine_from(&text).expect("the edited catalog builds");
        Codec::<Row>::new(&schema);
    }

    #[test]
    fn a_hides_new_that_is_no_bool_is_refused_as_damage() {
        let found = refusal(&[("hides_new", Value::I64(1))]);
        assert_eq!(
            "a row of catalog::bundles.Bundle holds I64(1) in field `hides_new`",
            damage("holds I64(1) in field `hides_new`").to_string()
        );
        assert_eq!(Err(damage("holds I64(1) in field `hides_new`")), found);
    }

    #[test]
    fn an_option_holding_another_type_is_refused_as_damage() {
        let text = some(Value::Text("x".into()));
        assert_eq!(
            Err(damage(
                "holds Sum { ctor: 1, fields: [Text(\"x\")] } in field `catalog`"
            )),
            refusal(&[("catalog", text.clone())])
        );
        assert_eq!(
            Err(damage(
                "holds Sum { ctor: 1, fields: [Text(\"x\")] } in field `lease`"
            )),
            refusal(&[("lease", text)])
        );
    }

    #[test]
    fn a_negative_pins_or_catalog_is_refused_as_damage() {
        assert_eq!(
            Err(damage("holds I64(-1) in field `pins`")),
            refusal(&[("pins", Value::I64(-1))])
        );
        assert_eq!(
            Err(damage(
                "holds Sum { ctor: 1, fields: [I64(-1)] } in field `catalog`"
            )),
            refusal(&[("catalog", some(Value::I64(-1)))])
        );
    }

    #[test]
    fn a_nested_sum_field_holding_another_type_is_refused_as_damage() {
        assert_eq!(
            Err(damage("holds I64(3) in field `state`")),
            refusal(&[("state", Value::I64(3))])
        );
        let past = Value::Sum {
            ctor: 3,
            fields: Vec::new(),
        };
        assert_eq!(
            Err(damage(
                "holds Sum { ctor: 3, fields: [] } in field `priority`"
            )),
            refusal(&[("priority", past)])
        );
        let schema = engine_schema();
        let codec = Codec::<Row>::new(&schema);
        let at = (codec.relation(), position(&schema, &codec, k::BUNDLE));
        let bare = tag(&schema, at, "state", k::BUNDLE_SUPERSEDED, Vec::new());
        let shown = format!("holds {bare:?} in field `state`");
        assert_eq!(Err(damage(&shown)), refusal(&[("state", bare)]));
    }

    #[test]
    fn a_call_row_past_the_registered_constructors_is_refused_as_damage() {
        let schema = engine_schema();
        let codec = Codec::<Registered>::new(&schema);
        let damage = Rot::CatalogRow {
            row: "catalog::registered".into(),
            detail: "has no constructor at position 3".into(),
        };
        assert_eq!(
            "a row of catalog::registered has no constructor at position 3",
            damage.to_string()
        );
        assert_eq!(Err(damage), codec.decode(3, &[Value::I64(1)]));
    }

    #[test]
    #[should_panic(
        expected = "the codec of catalog::bundles lists the constructors of field `state` of constructor 173"
    )]
    fn a_codec_listing_a_constructor_its_nested_sum_does_not_declare_does_not_build() {
        let text = ENGINE_SRC.replace(
            "type BundleState = Active | Superseded { at: Instant } | Retired { at: Instant }",
            "type BundleState = Active | Superseded { at: Instant }",
        );
        let schema = schema::build_engine_from(&text).expect("the edited catalog builds");
        Codec::<Row>::new(&schema);
    }

    #[test]
    fn each_bundle_codec_lists_the_constructors_and_fields_its_relation_declares() {
        let schema = engine_schema();
        Codec::<Row>::new(&schema);
        Codec::<Registered>::new(&schema);
    }
}
