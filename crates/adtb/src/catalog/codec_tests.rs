use adtb_lang::ids::engine_ctor as k;
use adtb_lang::schema::prelude::ctor;
use adtb_lang::schema::{self, ENGINE_SRC, Schema};

use crate::exec::Val;
use crate::heap::{FieldType, Layout};
use crate::key::{F64, Scalar, Value};

use super::super::ids::ctor_index;
use super::super::{engine_schema, row_type};
use super::{Codec, Decode, RoleRow, Rot, SettingRow};

const SECOND: i64 = 1_000_000;

/// Returns `text` with the constructors of each sum written one per line in reverse order.
fn reversed_sums(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("  = ") {
            out.push(lines[i].to_owned());
            i += 1;
            continue;
        }
        let mut ctors: Vec<Vec<&str>> = Vec::new();
        while i < lines.len() {
            let line = lines[i];
            if line.starts_with("  = ") || line.starts_with("  | ") {
                ctors.push(vec![&line[4..]]);
            } else if line.starts_with("    ") {
                ctors
                    .last_mut()
                    .expect("a continuation follows a constructor")
                    .push(line);
            } else {
                break;
            }
            i += 1;
        }
        ctors.reverse();
        for (n, ctor) in ctors.iter().enumerate() {
            let lead = if n == 0 { "  = " } else { "  | " };
            out.push(format!("{lead}{}", ctor[0]));
            out.extend(ctor[1..].iter().map(|l| (*l).to_owned()));
        }
    }
    out.join("\n")
}

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

/// Returns the position of `id` among the constructors of `T`'s relation in `schema`.
fn position<T: Decode>(schema: &Schema, codec: &Codec<T>, id: adtb_lang::ids::CatCtorId) -> u32 {
    ctor_index(schema, codec.relation(), id)
        .unwrap_or_else(|| panic!("{id} is a constructor of the relation"))
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
    let reversed =
        schema::build_engine_from(&reversed_sums(ENGINE_SRC)).expect("the reversed catalog builds");
    let (old, new) = (
        Codec::<SettingRow>::new(&shipped),
        Codec::<SettingRow>::new(&reversed),
    );
    let mut moved = 0;
    for id in SettingRow::CTORS.iter().map(|c| c.id) {
        let (was, now) = (position(&shipped, &old, id), position(&reversed, &new, id));
        let types = row_type(&shipped, old.relation(), was);
        let row: Vec<Value> = types.iter().map(sample).collect();
        let (fixed, vars) = Layout::new(types).encode(&row);
        let read = Layout::new(row_type(&reversed, new.relation(), now)).decode(&fixed, &vars);
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
    let names: Vec<String> = super::super::layout::row(&schema, codec.relation(), 0)
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
