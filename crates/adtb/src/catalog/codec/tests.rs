use adtb_lang::evolve::Privilege;
use adtb_lang::ids::{CatCtorId, engine_ctor as k};
use adtb_lang::schema::prelude::ctor;
use adtb_lang::schema::{self, ENGINE_SRC};

use crate::exec::Val;
use crate::key::{F64, Value};

use super::super::{engine_schema, layout};
use super::testing::{Sample, Sampled, position, reversed, row, tag};
use super::{
    Codec, Decode, GrantKind, MigrationRow, NonNegative, RoleGrant, RoleRow, Rot, SettingRow,
};

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
fn a_setting_field_holding_another_type_is_refused_as_damage_naming_it() {
    let schema = engine_schema();
    let codec = Codec::<SettingRow>::new(&schema);
    let at = position(&schema, &codec, k::SETTING_RETRY_LIMIT);
    let found = codec.decode(at, &[Value::I64(451), Value::Text("x".into())]);
    let damage = Rot::CatalogRow {
        row: "catalog::settings.RetryLimit".into(),
        detail: "holds `x` in field `retries`".into(),
    };
    assert_eq!(
        "a row of catalog::settings.RetryLimit holds `x` in field `retries`",
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
        detail: "holds duration 5 in field `every`".into(),
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
fn a_role_row_holding_another_type_is_refused() {
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
    let damage = Rot::CatalogRow {
        row: "catalog::roles.Role".into(),
        detail: "holds 4 in field `name`".into(),
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
        detail: "holds 207 in field `id`".into(),
    };
    assert_eq!(
        "a row of catalog::settings.RetryLimit holds 207 in field `id`",
        damage.to_string()
    );
    assert_eq!(Err(damage), other);
    let text = codec.decode(at, &[Value::Text("x".into()), Value::I64(3)]);
    let damage = Rot::CatalogRow {
        row: "catalog::settings.RetryLimit".into(),
        detail: "holds `x` in field `id`".into(),
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
        detail: "holds `x` in field `created`".into(),
    };
    assert_eq!(
        "a row of catalog::roles.Role holds `x` in field `created`",
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
        k::GRANT_LOG,
        k::GRANT_DEPLOYMENT,
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
    kind: CatCtorId,
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
fn a_grant_row_holding_another_type_or_constructor_is_refused_as_damage_naming_it() {
    let schema = engine_schema();
    let codec = Codec::<RoleGrant>::new(&schema);
    let c = position(&schema, &codec, k::ROLE_GRANT);
    for (field, v, text) in [
        (
            "target",
            some(Value::I64(3)),
            "a row of catalog::grants.RoleGrant holds constructor 1 of (3) in field `target`",
        ),
        (
            "kind",
            Value::Sum {
                ctor: 2,
                fields: Vec::new(),
            },
            "a row of catalog::grants.RoleGrant holds constructor 2 of () in field `kind`",
        ),
        (
            "dormant",
            Value::I64(1),
            "a row of catalog::grants.RoleGrant holds 1 in field `dormant`",
        ),
        (
            "grantee",
            Value::I64(1),
            "a row of catalog::grants.RoleGrant holds 1 in field `grantee`",
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
            "a row of catalog::migrations.Migration holds -1 in field `version`",
        ),
    ] {
        let refused = codec.decode(c, &stored).map_err(|r| r.to_string());
        assert_eq!(Err(text.to_owned()), refused);
    }
}

const SECOND: i64 = 1_000_000;

impl Sampled for RoleRow {
    fn samples(_: &schema::Schema) -> Vec<Sample<RoleRow>> {
        let fields = vec![
            ("id", Value::I64(3)),
            ("name", Value::Text("ops".into())),
            ("created", Value::Instant(9)),
        ];
        let want = RoleRow {
            id: 3,
            name: "ops".into(),
            created: 9,
        };
        vec![(k::ROLE, fields, want)]
    }
}

impl Sampled for RoleGrant {
    fn samples(schema: &schema::Schema) -> Vec<Sample<RoleGrant>> {
        let codec = Codec::<RoleGrant>::new(schema);
        let at = (codec.relation(), position(schema, &codec, k::ROLE_GRANT));
        let sample = |id, kind, target: Option<&str>, option: bool| {
            let fields = vec![
                ("id", Value::I64(5)),
                ("kind", tag(schema, at, "kind", id, Vec::new())),
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
            let want = RoleGrant {
                id: 5,
                kind,
                privilege: Privilege::Read,
                target: target.map(str::to_owned),
                grantee: "web".into(),
                grantor: "owner".into(),
                grant_option: option,
                dormant: !option,
            };
            (k::ROLE_GRANT, fields, want)
        };
        vec![
            sample(k::GRANT_LOG, GrantKind::Log, Some("app::t"), true),
            sample(k::GRANT_DEPLOYMENT, GrantKind::Deployment, None, false),
        ]
    }
}

impl Sampled for MigrationRow {
    fn samples(_: &schema::Schema) -> Vec<Sample<MigrationRow>> {
        let fields = vec![
            ("version", Value::I64(3)),
            ("hash", Value::Bytes(vec![7; 32])),
            ("applied", Value::Instant(41)),
        ];
        let want = MigrationRow {
            version: 3,
            hash: [7; 32],
            applied: 41,
        };
        vec![(k::MIGRATION, fields, want)]
    }
}

impl Sampled for SettingRow {
    fn samples(_: &schema::Schema) -> Vec<Sample<SettingRow>> {
        use SettingRow as S;
        let d = |n: i64| Value::Duration(n * SECOND);
        let f = |x: f64| Value::F64(F64::new(x));
        let nn = |n: i64| NonNegative::new(n).expect("a count of the table is from 0");
        let one = |id: CatCtorId, field: &'static str, v: Value, want: S| {
            let n = i64::try_from(id.raw()).expect("an id fits 63 bits");
            (id, vec![("id", Value::I64(n)), (field, v)], want)
        };
        let two = |id: CatCtorId,
                   (a, x): (&'static str, Value),
                   (b, y): (&'static str, Value),
                   want: S| {
            let n = i64::try_from(id.raw()).expect("an id fits 63 bits");
            (id, vec![("id", Value::I64(n)), (a, x), (b, y)], want)
        };
        vec![
            one(
                k::SETTING_STATEMENT_LIMIT,
                "limit",
                some(d(1)),
                S::StatementLimit(Some(nn(SECOND))),
            ),
            one(
                k::SETTING_TRANSACTION_LIMIT,
                "limit",
                none(),
                S::TransactionLimit(None),
            ),
            one(
                k::SETTING_IDLE_LIMIT,
                "limit",
                some(d(3)),
                S::IdleLimit(Some(nn(3 * SECOND))),
            ),
            one(
                k::SETTING_STREAM_IDLE_LIMIT,
                "limit",
                d(4),
                S::StreamIdleLimit(nn(4 * SECOND)),
            ),
            one(
                k::SETTING_REQUEST_WINDOW,
                "keep",
                d(5),
                S::RequestWindow(nn(5 * SECOND)),
            ),
            one(
                k::SETTING_SAFE_POINT_DELAY,
                "delay",
                d(6),
                S::SafePointDelay(nn(6 * SECOND)),
            ),
            one(
                k::SETTING_REPLICA_WAIT,
                "wait",
                d(7),
                S::ReplicaWait(nn(7 * SECOND)),
            ),
            one(
                k::SETTING_DEFAULT_LEASE,
                "lease",
                d(8),
                S::DefaultLease(nn(8 * SECOND)),
            ),
            one(
                k::SETTING_FEED_RETENTION,
                "keep",
                d(9),
                S::FeedRetention(nn(9 * SECOND)),
            ),
            one(
                k::SETTING_STATS_WINDOW,
                "length",
                d(10),
                S::StatsWindow(nn(10 * SECOND)),
            ),
            one(
                k::SETTING_STATS_RETENTION,
                "keep",
                d(11),
                S::StatsRetention(nn(11 * SECOND)),
            ),
            two(
                k::SETTING_WAIT_SAMPLING,
                ("every", d(12)),
                ("keep", d(13)),
                S::WaitSampling {
                    every: nn(12 * SECOND),
                    keep: nn(13 * SECOND),
                },
            ),
            one(
                k::SETTING_CONFLICT_LOG,
                "rows",
                Value::I64(14),
                S::ConflictLog(nn(14)),
            ),
            two(
                k::SETTING_VACUUM_BUDGET,
                ("io", f(0.25)),
                ("cpu", f(0.5)),
                S::VacuumBudget {
                    io: F64::new(0.25),
                    cpu: F64::new(0.5),
                },
            ),
            one(
                k::SETTING_SCRUB_INTERVAL,
                "every",
                some(d(15)),
                S::ScrubInterval(Some(nn(15 * SECOND))),
            ),
            one(
                k::SETTING_LEASE_GRACE,
                "grace",
                d(16),
                S::LeaseGrace(nn(16 * SECOND)),
            ),
            one(
                k::SETTING_BLOCKING_NOTICE_PERIOD,
                "period",
                d(17),
                S::BlockingNoticePeriod(nn(17 * SECOND)),
            ),
            one(
                k::SETTING_REPLICA_VACUUM_WAIT,
                "wait",
                d(18),
                S::ReplicaVacuumWait(nn(18 * SECOND)),
            ),
            one(
                k::SETTING_RETRY_LIMIT,
                "retries",
                Value::I64(19),
                S::RetryLimit(nn(19)),
            ),
            one(
                k::SETTING_RETENTION_WINDOW,
                "length",
                d(20),
                S::RetentionWindow(nn(20 * SECOND)),
            ),
        ]
    }
}
