use adtb_lang::ids::engine_ctor as k;
use adtb_lang::schema::prelude::ctor;
use adtb_lang::schema::{self, ENGINE_SRC};

use crate::exec::Val;
use crate::heap::FieldType;
use crate::key::{F64, Scalar, Value};

use super::super::{engine_schema, layout, row_type};
use super::testing::{position, reread, reversed, row, tag};
use super::{Codec, Decode, GrantKind, MigrationRow, RoleGrant, RoleRow, Rot, SettingRow};

const SECOND: i64 = 1_000_000;

/// Returns a value stored as `t`: seven, five seconds, a quarter, or `Some` of one of those.
fn sample(t: &FieldType) -> Value {
    match t {
        FieldType::Scalar(Scalar::I64) => Value::I64(7),
        FieldType::Scalar(Scalar::Duration) => Value::Duration(5 * SECOND),
        FieldType::Scalar(Scalar::F64) => Value::F64(F64::new(0.25)),
        FieldType::Sum(cs) => Value::Sum {
            ctor: ctor::SOME,
            fields: vec![sample(&cs[ctor::SOME as usize].fields[0])],
        },
        other => panic!("no sample of {other:?}"),
    }
}

/// Returns the setting each constructor's row of samples holds.
fn sampled(id: adtb_lang::ids::CatCtorId) -> SettingRow {
    use SettingRow as S;
    let (d, q) = (5 * SECOND, F64::new(0.25));
    match id {
        k::SETTING_STATEMENT_LIMIT => S::StatementLimit(Some(d)),
        k::SETTING_TRANSACTION_LIMIT => S::TransactionLimit(Some(d)),
        k::SETTING_IDLE_LIMIT => S::IdleLimit(Some(d)),
        k::SETTING_STREAM_IDLE_LIMIT => S::StreamIdleLimit(d),
        k::SETTING_REQUEST_WINDOW => S::RequestWindow(d),
        k::SETTING_SAFE_POINT_DELAY => S::SafePointDelay(d),
        k::SETTING_REPLICA_WAIT => S::ReplicaWait(d),
        k::SETTING_DEFAULT_LEASE => S::DefaultLease(d),
        k::SETTING_FEED_RETENTION => S::FeedRetention(d),
        k::SETTING_STATS_WINDOW => S::StatsWindow(d),
        k::SETTING_STATS_RETENTION => S::StatsRetention(d),
        k::SETTING_WAIT_SAMPLING => S::WaitSampling { every: d, keep: d },
        k::SETTING_CONFLICT_LOG => S::ConflictLog(7),
        k::SETTING_VACUUM_BUDGET => S::VacuumBudget { io: q, cpu: q },
        k::SETTING_SCRUB_INTERVAL => S::ScrubInterval(Some(d)),
        k::SETTING_LEASE_GRACE => S::LeaseGrace(d),
        k::SETTING_BLOCKING_NOTICE_PERIOD => S::BlockingNoticePeriod(d),
        k::SETTING_REPLICA_VACUUM_WAIT => S::ReplicaVacuumWait(d),
        k::SETTING_RETRY_LIMIT => S::RetryLimit(7),
        k::SETTING_RETENTION_WINDOW => S::RetentionWindow(d),
        other => panic!("{other} is no setting"),
    }
}

#[test]
fn each_codec_lists_the_constructors_and_fields_its_relation_declares() {
    let schema = engine_schema();
    Codec::<RoleRow>::new(&schema);
    Codec::<SettingRow>::new(&schema);
}

#[test]
#[should_panic(expected = "the codec of catalog::settings lists the fields of constructor 451")]
fn a_codec_listing_a_field_its_relation_does_not_declare_does_not_build() {
    let text = ENGINE_SRC.replace(
        "RetryLimit           { retries: I64 }",
        "RetryLimit { tries: I64 }",
    );
    let schema = schema::build_engine_from(&text).expect("the edited catalog builds");
    Codec::<SettingRow>::new(&schema);
}

#[test]
fn a_catalog_with_its_constructors_reversed_reads_each_stored_setting_row_the_same() {
    let shipped = engine_schema();
    let reversed = reversed();
    let (old, new) = (
        Codec::<SettingRow>::new(&shipped),
        Codec::<SettingRow>::new(&reversed),
    );
    let mut moved = 0;
    for id in SettingRow::CTORS.iter().map(|c| c.id) {
        let (was, now) = (position(&shipped, &old, id), position(&reversed, &new, id));
        let names = layout::row(&shipped, old.relation(), was);
        let types = row_type(&shipped, old.relation(), was);
        let row: Vec<Value> = names
            .iter()
            .zip(&types)
            .map(|((n, _), t)| match n.as_str() {
                "id" => Value::I64(i64::try_from(id.raw()).expect("an id fits 63 bits")),
                _ => sample(t),
            })
            .collect();
        let read = reread(
            (&shipped, old.relation(), was),
            (&reversed, new.relation(), now),
            &row,
        );
        assert_eq!(Ok(sampled(id)), old.decode(was, &row), "{id}");
        assert_eq!(Ok(sampled(id)), new.decode(now, &read), "{id}");
        moved += usize::from(was != now);
    }
    assert_eq!(SettingRow::CTORS.len(), moved, "every setting moves");
}

#[test]
fn a_setting_field_holding_another_type_is_refused_as_damage_naming_it() {
    let schema = engine_schema();
    let codec = Codec::<SettingRow>::new(&schema);
    let at = position(&schema, &codec, k::SETTING_RETRY_LIMIT);
    let found = codec.decode(at, &[Value::I64(451), Value::Text("x".into())]);
    let damage = Rot::CatalogRow {
        row: "catalog::settings.RetryLimit".into(),
        detail: "holds Text(\"x\") in field `retries`".into(),
    };
    assert_eq!(
        "a row of catalog::settings.RetryLimit holds Text(\"x\") in field `retries`",
        damage.to_string()
    );
    assert_eq!(Err(damage), found);
}

#[test]
fn a_scrub_interval_holding_no_option_is_refused_as_damage() {
    let schema = engine_schema();
    let codec = Codec::<SettingRow>::new(&schema);
    let at = position(&schema, &codec, k::SETTING_SCRUB_INTERVAL);
    let found = codec.decode(at, &[Value::I64(208), Value::Duration(5)]);
    let damage = Rot::CatalogRow {
        row: "catalog::settings.ScrubInterval".into(),
        detail: "holds Duration(5) in field `every`".into(),
    };
    assert_eq!(Err(damage), found);
}

#[test]
fn a_setting_row_short_of_a_field_is_refused_as_damage() {
    let schema = engine_schema();
    let codec = Codec::<SettingRow>::new(&schema);
    let at = position(&schema, &codec, k::SETTING_VACUUM_BUDGET);
    let found = codec.decode(at, &[Value::I64(207), Value::F64(F64::new(0.5))]);
    let damage = Rot::CatalogRow {
        row: "catalog::settings.VacuumBudget".into(),
        detail: "holds 2 fields, not 3".into(),
    };
    assert_eq!(
        "a row of catalog::settings.VacuumBudget holds 2 fields, not 3",
        damage.to_string()
    );
    assert_eq!(Err(damage), found);
}

#[test]
fn a_row_past_the_relations_constructors_is_refused_as_damage() {
    let schema = engine_schema();
    let codec = Codec::<SettingRow>::new(&schema);
    let past = u32::try_from(SettingRow::CTORS.len()).expect("the count fits 32 bits");
    let damage = Rot::CatalogRow {
        row: "catalog::settings".into(),
        detail: "has no constructor at position 20".into(),
    };
    assert_eq!(
        "a row of catalog::settings has no constructor at position 20",
        damage.to_string()
    );
    assert_eq!(Err(damage), codec.decode(past, &[Value::I64(1)]));
}

#[test]
fn a_role_row_reads_by_field_name_and_one_holding_another_type_is_refused() {
    let schema = engine_schema();
    let codec = Codec::<RoleRow>::new(&schema);
    let names: Vec<String> = layout::row(&schema, codec.relation(), 0)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let row = |name: Value| -> Vec<Value> {
        names
            .iter()
            .map(|n| match n.as_str() {
                "id" => Value::I64(3),
                "name" => name.clone(),
                "created" => Value::Instant(9),
                other => panic!("a role has no field `{other}`"),
            })
            .collect()
    };
    assert_eq!(
        Ok(RoleRow {
            id: 3,
            name: "ops".into(),
            created: 9,
        }),
        codec.decode(0, &row(Value::Text("ops".into())))
    );
    let damage = Rot::CatalogRow {
        row: "catalog::roles.Role".into(),
        detail: "holds I64(4) in field `name`".into(),
    };
    assert_eq!(Err(damage), codec.decode(0, &row(Value::I64(4))));
}

#[test]
fn a_value_past_the_relations_constructors_is_refused_before_its_fields_are_read() {
    let schema = engine_schema();
    let codec = Codec::<SettingRow>::new(&schema);
    let past = u32::try_from(SettingRow::CTORS.len()).expect("the count fits 32 bits");
    let damage = Rot::CatalogRow {
        row: "catalog::settings".into(),
        detail: "has no constructor at position 20".into(),
    };
    let v = Val::ctor(past, vec![Val::Int(1)]);
    assert_eq!(Err(damage), codec.decode_val(&schema, &v));
}

#[test]
fn a_setting_row_whose_id_is_not_its_constructors_is_refused_as_damage() {
    let schema = engine_schema();
    let codec = Codec::<SettingRow>::new(&schema);
    let at = position(&schema, &codec, k::SETTING_RETRY_LIMIT);
    let other = codec.decode(at, &[Value::I64(207), Value::I64(3)]);
    let damage = Rot::CatalogRow {
        row: "catalog::settings.RetryLimit".into(),
        detail: "holds I64(207) in field `id`".into(),
    };
    assert_eq!(
        "a row of catalog::settings.RetryLimit holds I64(207) in field `id`",
        damage.to_string()
    );
    assert_eq!(Err(damage), other);
    let text = codec.decode(at, &[Value::Text("x".into()), Value::I64(3)]);
    let damage = Rot::CatalogRow {
        row: "catalog::settings.RetryLimit".into(),
        detail: "holds Text(\"x\") in field `id`".into(),
    };
    assert_eq!(Err(damage), text);
}

#[test]
fn a_role_row_holding_another_type_in_a_field_no_reader_reads_is_refused() {
    let schema = engine_schema();
    let codec = Codec::<RoleRow>::new(&schema);
    let row = row(
        &schema,
        codec.relation(),
        0,
        &[
            ("id", Value::I64(3)),
            ("name", Value::Text("ops".into())),
            ("created", Value::Text("x".into())),
        ],
    );
    let damage = Rot::CatalogRow {
        row: "catalog::roles.Role".into(),
        detail: "holds Text(\"x\") in field `created`".into(),
    };
    assert_eq!(
        "a row of catalog::roles.Role holds Text(\"x\") in field `created`",
        damage.to_string()
    );
    assert_eq!(Err(damage), codec.decode(0, &row));
}

#[test]
fn a_value_with_another_count_of_fields_than_its_constructor_is_refused() {
    let schema = engine_schema();
    let codec = Codec::<SettingRow>::new(&schema);
    let at = position(&schema, &codec, k::SETTING_RETRY_LIMIT);
    let surplus = Val::ctor(at, vec![Val::Int(451), Val::Int(3), Val::Int(4)]);
    let damage = Rot::CatalogRow {
        row: "catalog::settings.RetryLimit".into(),
        detail: "holds 3 fields, not 2".into(),
    };
    assert_eq!(
        "a row of catalog::settings.RetryLimit holds 3 fields, not 2",
        damage.to_string()
    );
    assert_eq!(Err(damage), codec.decode_val(&schema, &surplus));
    let short = Val::ctor(at, vec![Val::Int(451)]);
    let damage = Rot::CatalogRow {
        row: "catalog::settings.RetryLimit".into(),
        detail: "holds 1 field, not 2".into(),
    };
    assert_eq!(
        "a row of catalog::settings.RetryLimit holds 1 field, not 2",
        damage.to_string()
    );
    assert_eq!(Err(damage), codec.decode_val(&schema, &short));
}

#[test]
fn reversing_the_catalogs_sums_moves_the_constructors_of_one_line_sums() {
    let (shipped, reversed) = (engine_schema(), reversed());
    for id in [
        k::PRIORITY_AD_HOC,
        k::PRIORITY_REGULAR,
        k::BUNDLE_ACTIVE,
        k::BUNDLE_RETIRED,
    ] {
        let at = |s: &schema::Schema| {
            s.ids()
                .ctor_of(id)
                .unwrap_or_else(|| panic!("{id} is declared"))
                .index()
        };
        assert_ne!(at(&shipped), at(&reversed), "{id}");
    }
}

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

/// Returns the stored grant row of kind `kind` in `schema`, its fields otherwise `overrides` or
/// fixed: id 5, `read` on `target` given by `owner` to `web`, with grant option `option` and
/// dormant as `option` is not.
fn grant(
    schema: &schema::Schema,
    kind: adtb_lang::ids::CatCtorId,
    target: Option<&str>,
    option: bool,
    overrides: &[(&str, Value)],
) -> Vec<Value> {
    let codec = Codec::<RoleGrant>::new(schema);
    let at = (codec.relation(), position(schema, &codec, k::ROLE_GRANT));
    let mut fields = vec![
        ("id", Value::I64(5)),
        ("kind", tag(schema, at, "kind", kind, Vec::new())),
        ("privilege", Value::Text("read".into())),
        (
            "target",
            target.map_or_else(none, |t| some(Value::Text(t.into()))),
        ),
        ("grantee", Value::Text("web".into())),
        ("grantor", Value::Text("owner".into())),
        ("grant_option", truth(option)),
        ("dormant", truth(!option)),
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

#[test]
fn a_catalog_with_its_constructors_reversed_reads_each_stored_grant_row_the_same() {
    let (shipped, reversed) = (engine_schema(), reversed());
    let (old, new) = (
        Codec::<RoleGrant>::new(&shipped),
        Codec::<RoleGrant>::new(&reversed),
    );
    let (was, now) = (
        position(&shipped, &old, k::ROLE_GRANT),
        position(&reversed, &new, k::ROLE_GRANT),
    );
    let kind_at = |s: &schema::Schema, r: &[Value]| {
        let names = layout::row(s, old.relation(), 0);
        let at = names.iter().position(|(n, _)| n == "kind").expect("a kind");
        match &r[at] {
            Value::Sum { ctor, .. } => *ctor,
            other => panic!("kind holds {other:?}"),
        }
    };
    let cases = [
        (k::GRANT_LOG, Some("app::t"), true, GrantKind::Log),
        (k::GRANT_DEPLOYMENT, None, false, GrantKind::Deployment),
    ];
    for (id, target, option, kind) in cases {
        let stored = grant(&shipped, id, target, option, &[]);
        let read = reread(
            (&shipped, old.relation(), was),
            (&reversed, new.relation(), now),
            &stored,
        );
        let want = RoleGrant {
            id: 5,
            kind,
            privilege: "read".into(),
            target: target.map(str::to_owned),
            grantee: "web".into(),
            grantor: "owner".into(),
            grant_option: option,
            dormant: !option,
        };
        assert_eq!(Ok(want.clone()), old.decode(was, &stored), "{kind:?}");
        assert_eq!(Ok(want), new.decode(now, &read), "{kind:?}");
        assert_ne!(
            kind_at(&shipped, &stored),
            kind_at(&reversed, &read),
            "{kind:?} moves"
        );
    }
}

#[test]
fn a_grant_row_holding_another_type_or_constructor_is_refused_as_damage_naming_it() {
    let schema = engine_schema();
    let codec = Codec::<RoleGrant>::new(&schema);
    let c = position(&schema, &codec, k::ROLE_GRANT);
    for (field, v, text) in [
        (
            "target",
            some(Value::I64(3)),
            "a row of catalog::grants.RoleGrant holds Sum { ctor: 1, fields: [I64(3)] } in field `target`",
        ),
        (
            "kind",
            Value::Sum {
                ctor: 2,
                fields: Vec::new(),
            },
            "a row of catalog::grants.RoleGrant holds Sum { ctor: 2, fields: [] } in field `kind`",
        ),
        (
            "dormant",
            Value::I64(1),
            "a row of catalog::grants.RoleGrant holds I64(1) in field `dormant`",
        ),
        (
            "grantee",
            Value::I64(1),
            "a row of catalog::grants.RoleGrant holds I64(1) in field `grantee`",
        ),
    ] {
        let stored = grant(&schema, k::GRANT_LOG, Some("app::t"), true, &[(field, v)]);
        let refused = codec.decode(c, &stored).map_err(|r| r.to_string());
        assert_eq!(Err(text.to_owned()), refused, "{field}");
    }
}

/// Returns the stored migration row of `schema` with version `version` and hash `hash`.
fn migration(schema: &schema::Schema, version: i64, hash: Vec<u8>) -> Vec<Value> {
    let codec = Codec::<MigrationRow>::new(schema);
    let c = position(schema, &codec, k::MIGRATION);
    row(
        schema,
        codec.relation(),
        c,
        &[
            ("version", Value::I64(version)),
            ("hash", Value::Bytes(hash)),
            ("text", Value::Text("create relation t".into())),
            ("applied", Value::Instant(41)),
        ],
    )
}

#[test]
fn a_migration_row_reads_its_fields_under_the_shipped_and_the_reversed_catalog() {
    let (shipped, reversed) = (engine_schema(), reversed());
    let (old, new) = (
        Codec::<MigrationRow>::new(&shipped),
        Codec::<MigrationRow>::new(&reversed),
    );
    let (was, now) = (
        position(&shipped, &old, k::MIGRATION),
        position(&reversed, &new, k::MIGRATION),
    );
    let stored = migration(&shipped, 3, vec![7; 32]);
    let read = reread(
        (&shipped, old.relation(), was),
        (&reversed, new.relation(), now),
        &stored,
    );
    let want = MigrationRow {
        version: 3,
        hash: [7; 32],
        applied: 41,
    };
    assert_eq!(Ok(&want), old.decode(was, &stored).as_ref());
    assert_eq!(Ok(want), new.decode(now, &read));
}

#[test]
fn a_migration_row_with_a_short_hash_or_a_negative_version_is_refused_as_damage() {
    let schema = engine_schema();
    let codec = Codec::<MigrationRow>::new(&schema);
    let c = position(&schema, &codec, k::MIGRATION);
    for (stored, text) in [
        (
            migration(&schema, 3, vec![7; 31]),
            "a row of catalog::migrations.Migration holds a hash of 31 bytes, not 32",
        ),
        (
            migration(&schema, -1, vec![7; 32]),
            "a row of catalog::migrations.Migration holds I64(-1) in field `version`",
        ),
    ] {
        let refused = codec.decode(c, &stored).map_err(|r| r.to_string());
        assert_eq!(Err(text.to_owned()), refused);
    }
}
