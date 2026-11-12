use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::evolve::Source;
use adtb_lang::schema::Schema;

use crate::key::Value;
use crate::mvcc::Xid;
use crate::storage::Pid;

use super::Rot;
use super::ids::{CatCtorId, CatRelId};
use super::root::{Changing, Root};
use super::{
    Broken, Error, HEAP_ROOT, Hash, MIGRATION, Named, Rows, applied, check_order_keys, declared,
    engine_schema, fold, indexes, layout, oplog, version_hash,
};

fn engine_rows(schema: &Schema) -> Rows {
    declared(schema, &Source::default())
}

/// Returns the rows of the catalog relation whose id is `id`.
fn rows_of(schema: &Schema, rows: &Rows, id: u64) -> Vec<Vec<Value>> {
    let r = schema
        .ids
        .relation_of(CatRelId::from_raw(id))
        .next()
        .unwrap();
    rows[&r].iter().map(|(_, row)| row.clone()).collect()
}

#[test]
fn the_root_page_round_trips() {
    for (changing, stored) in [
        (Changing::None, 0),
        (Changing::Schema(Xid(1025)), 1025),
        (Changing::NamingSequences, u64::MAX),
        (Changing::Reindexing, u64::MAX - 1),
    ] {
        let root = Root {
            commit_log: Pid(7),
            large: (1, Pid(3)),
            roots: (2, Pid(9)),
            next_heap: 70,
            changing,
            sequences: Pid(77),
            history_large: (71, Pid(78)),
            record: (72, Pid(79)),
        };
        let mut page = vec![0; 4096];
        root.write(&mut page);
        assert_eq!(root, Root::read(&page));
        assert!(page[..24].iter().all(|&b| b == 0));
        let at = 24 + 8 + 4 + 4 + 8 + 8 + 8;
        assert_eq!(stored.to_le_bytes(), page[at..at + 8], "{changing:?}");
    }
}

#[test]
fn every_engine_object_with_an_id_has_a_row() {
    let schema = engine_schema();
    let rows = engine_rows(&schema);
    let count = |rel: u64| {
        let r = schema
            .ids
            .relation_of(CatRelId::from_raw(rel))
            .next()
            .unwrap();
        rows[&r].len()
    };
    assert_eq!(schema.ids.type_ids().count(), count(219));
    assert_eq!(schema.ids.ctor_ids().count(), count(220));
    assert_eq!(schema.ids.field_ids().count(), count(222));
    assert_eq!(schema.ids.relation_ids().count(), count(223));
    assert_eq!(indexes(&schema).len(), count(226));
    assert_eq!(0, count(224));
    assert_eq!(0, count(232));
}

#[test]
fn the_constructors_relation_names_its_own_row() {
    let schema = engine_schema();
    let rows = engine_rows(&schema);
    let r = schema
        .ids
        .relation_of(CatRelId::from_raw(220))
        .next()
        .unwrap();
    let own = rows[&r]
        .iter()
        .find(|(_, row)| row[0] == Value::I64(121))
        .expect("a row for constructor 121");
    let names: Vec<&Value> = own.1.iter().collect();
    assert_eq!(&Value::I64(120), names[1]);
    assert_eq!(&Value::Text("Constructor".into()), names[2]);
    assert_eq!(&Value::I32(1), names[3]);
}

#[test]
fn positions_count_from_one() {
    let schema = engine_schema();
    let rows = engine_rows(&schema);
    let positions = |id| -> BTreeSet<i32> {
        rows_of(&schema, &rows, id)
            .into_iter()
            .map(|row| match row[3] {
                Value::I32(p) => p,
                ref v => panic!("a position is an I32, not {v:?}"),
            })
            .collect()
    };
    assert_eq!(Some(&1), positions(220).first());
    assert_eq!(Some(&1), positions(222).first());
}

#[test]
fn tag_has_a_types_row_of_one_parameter() {
    let schema = engine_schema();
    let rows = engine_rows(&schema);
    let tag = rows_of(&schema, &rows, 219)
        .into_iter()
        .find(|row| row[0] == Value::I64(9))
        .expect("a row for Tag");
    assert_eq!(Value::Text("prelude".into()), tag[1]);
    assert_eq!(Value::Text("Tag".into()), tag[2]);
    assert_eq!(Value::I32(1), tag[3]);
}

#[test]
fn order_keys_follow_declaration_order_and_are_spread_once() {
    let schema = engine_schema();
    let keys = layout::new_order_keys(&schema, &Default::default());
    assert!(keys[&1] < keys[&2], "False sorts below True");
    assert!(keys[&4] < keys[&5], "None sorts below Some");
    assert!(
        !keys.contains_key(&34),
        "a newtype's constructor has no order key"
    );
    assert!(layout::new_order_keys(&schema, &keys).is_empty());
}

#[test]
fn engine_indexes_are_the_keys_then_the_uniques() {
    let schema = engine_schema();
    let all = indexes(&schema);
    assert_eq!(
        (0..all.len() as u64).collect::<BTreeSet<u64>>(),
        all.keys().copied().collect()
    );
}

#[test]
fn a_default_of_several_constructors_has_a_row_per_field() {
    let text = "module app
add module app
add type Shape { id: Key shapes }
  = Circle { r: I64, label: Text }
  | Square { s: I64, label: Text }
add relation shapes : Shape key id generate sequence default label = \"none\"
";
    let file = adtb_lang::parse::parse_migration(text, adtb_lang::parse::Options::default())
        .ok()
        .expect("the migration parses");
    let source = Source::default().migrate(&file, text).expect("it applies");
    let schema = source.schema().expect("it builds");
    let rows = declared(&schema, &source);
    let defaults = rows_of(&schema, &rows, 456);
    let texts: Vec<&Value> = defaults.iter().map(|row| &row[3]).collect();
    assert_eq!(
        vec![
            &Value::Text("generate sequence".into()),
            &Value::Text("\"none\"".into()),
            &Value::Text("\"none\"".into()),
        ],
        texts
    );
    let ids: BTreeSet<String> = defaults.iter().map(|row| format!("{:?}", row[0])).collect();
    let fields: BTreeSet<String> = defaults.iter().map(|row| format!("{:?}", row[2])).collect();
    assert_eq!((3, 3), (ids.len(), fields.len()));
}

const FIRST: &str = "module app
add module app
add type Shape { id: Key shapes }
  = Circle { r: I64 }
  | Square { s: I64 }
add relation shapes : Shape key id generate sequence
add set Round of shapes = Circle
add unique (r) in shapes.Circle
define fn area (x: Shape) -> I64 = match x
  | Circle c -> 3 * c.r * c.r
  | Square q -> q.s * q.s
define fn stamp (t: Text) -> Text = host \"stamp\" version \"1.2\"
";

fn migrated(from: &Source, text: &str) -> Source {
    let file = adtb_lang::parse::parse_migration(text, adtb_lang::parse::Options::default())
        .ok()
        .expect("the migration parses");
    from.migrate(&file, text)
        .unwrap_or_else(|r| panic!("the migration applies: {r:?}"))
}

/// Returns the rows of `catalog::{name}` that `declared` gives for `source`.
fn catalog_rows(source: &Source, name: &str) -> Vec<(u32, Vec<Value>)> {
    let schema = source.schema().expect("it builds");
    let m = schema.module_named("catalog").expect("the catalog module");
    let Some(&adtb_lang::schema::ValueDef::Relation(r)) = schema.module(m).values.get(name) else {
        panic!("the catalog declares `{name}`")
    };
    declared(&schema, source).remove(&r).unwrap_or_default()
}

fn hash_of(source: &Source) -> Hash {
    version_hash(&source.schema().expect("it builds"), source)
}

#[test]
fn the_hash_of_version_zero_is_32_zero_bytes() {
    assert_eq!([0; 32], hash_of(&Source::default()));
    assert_eq!([0; 32], version_hash(&engine_schema(), &Source::default()));
}

#[test]
fn the_hash_of_a_version_chains_on_the_one_before() {
    let one = migrated(&Source::default(), FIRST);
    let two = migrated(&one, "module app\nadd type Label = { name: Text }\n");
    let schema = two.schema().expect("it builds");
    let step = |before: Hash, number: u64| {
        let mut input = before.to_vec();
        input.extend(oplog::encoded(
            &schema,
            &oplog::operators(&schema, &two, number),
        ));
        *blake3::hash(&input).as_bytes()
    };
    let h1 = step([0; 32], 1);
    assert_eq!(h1, hash_of(&one));
    assert_eq!(step(h1, 2), hash_of(&two));
    assert_ne!(hash_of(&one), hash_of(&two));
}

#[test]
fn a_migration_encodes_its_operator_count_then_each_operator_and_its_clauses() {
    let src = migrated(&Source::default(), "module app\nadd module app\n");
    let schema = src.schema().expect("it builds");
    let bytes = oplog::encoded(&schema, &oplog::operators(&schema, &src, 1));
    assert_eq!(
        vec![0x01, 0xfd, 0x02, 0x03, b'a', b'p', b'p', 0x00],
        bytes,
        "one operator, AddModule (381), \"app\", no clauses"
    );
}

#[test]
fn a_member_clause_is_encoded_after_its_operator() {
    let src = migrated(
        &Source::default(),
        "module app\nadd module app\nadd type S { id: Key ss } = P | Q\nadd relation ss : S key id\nadd set Ps of ss = P\n",
    );
    let schema = src.schema().expect("it builds");
    let ops = oplog::operators(&schema, &src, 1);
    let set = &ops[3];
    assert_eq!(1, set.1.len(), "{set:?}");
    let p = src.ids["ctor app::S.P"];
    let mut tail = vec![0x01];
    tail.extend([0xba, 0x03]);
    tail.extend(p.to_le_bytes());
    let bytes = oplog::encoded(&schema, &ops);
    assert!(
        bytes.ends_with(&tail),
        "the set's one Member clause (442) holds P's id {p}: {bytes:02x?}"
    );
}

#[test]
fn the_printed_text_of_a_migration_is_not_hashed() {
    let spaced = FIRST
        .replace(
            "add relation shapes : Shape key id generate sequence",
            "add relation shapes : Shape   -- every shape\n  key id generate sequence",
        )
        .replace("3 * c.r * c.r", "3*c.r*c.r");
    assert_ne!(FIRST, spaced);
    assert_eq!(
        hash_of(&migrated(&Source::default(), FIRST)),
        hash_of(&migrated(&Source::default(), &spaced))
    );
}

#[test]
fn a_changed_operand_changes_the_hash() {
    let other = FIRST.replace("3 * c.r * c.r", "4 * c.r * c.r");
    assert_ne!(
        hash_of(&migrated(&Source::default(), FIRST)),
        hash_of(&migrated(&Source::default(), &other))
    );
}

#[test]
fn every_operator_has_a_migration_ops_row_by_ordinal_from_one() {
    let src = migrated(&Source::default(), FIRST);
    let rows = catalog_rows(&src, "migration_ops");
    let keys: Vec<(Value, Value)> = rows
        .iter()
        .map(|(_, row)| (row[0].clone(), row[1].clone()))
        .collect();
    let want: Vec<(Value, Value)> = (1..=7).map(|k| (Value::I64(1), Value::I32(k))).collect();
    assert_eq!(want, keys);
    let schema = src.schema().expect("it builds");
    let ops = oplog::operators(&schema, &src, 1);
    let stored: Vec<&Value> = rows.iter().map(|(_, row)| &row[2]).collect();
    let resolved: Vec<&Value> = ops.iter().map(|(op, _)| op).collect();
    assert_eq!(resolved, stored);
}

#[test]
fn every_member_clause_has_a_migration_clauses_row() {
    let src = migrated(&Source::default(), FIRST);
    let rows = catalog_rows(&src, "migration_clauses");
    let circle = Value::I64(i64::try_from(src.ids["ctor app::Shape.Circle"]).expect("an id"));
    let keys: Vec<(Value, Value, Value)> = rows
        .iter()
        .map(|(_, row)| (row[0].clone(), row[1].clone(), row[2].clone()))
        .collect();
    assert_eq!(
        vec![
            (Value::I64(1), Value::I32(4), Value::I32(1)),
            (Value::I64(1), Value::I32(5), Value::I32(1)),
        ],
        keys
    );
    for (_, row) in &rows {
        let Value::Sum { fields, .. } = &row[3] else {
            panic!("a clause body is a sum: {row:?}")
        };
        assert_eq!(&vec![circle.clone()], fields);
    }
}

/// Returns the row of `catalog::functions` for function `name`, with its constructor's name.
fn function_row(source: &Source, name: &str) -> (String, Vec<Value>) {
    let schema = source.schema().expect("it builds");
    let m = schema.module_named("catalog").expect("the catalog module");
    let def = schema.module(m).types["Function"];
    catalog_rows(source, "functions")
        .into_iter()
        .find(|(_, row)| row[2] == Value::Text(name.into()))
        .map(|(c, row)| (schema.ty(def).ctors()[c as usize].name.clone(), row))
        .unwrap_or_else(|| panic!("a functions row for `{name}`"))
}

#[test]
fn a_language_function_is_hashed_over_its_body_in_canonical_form() {
    let src = migrated(&Source::default(), FIRST);
    let (ctor, row) = function_row(&src, "area");
    assert_eq!("Language", ctor);
    let canonical = "match x | app::Circle c -> 3 * c . r * c . r | app::Square q -> q . s * q . s";
    assert_eq!(
        vec![
            Value::I64(i64::try_from(src.ids["fn app::area"]).expect("an id")),
            Value::Text("app".into()),
            Value::Text("area".into()),
            Value::Text("(x: Shape) -> I64".into()),
            Value::Sum {
                ctor: 1,
                fields: Vec::new()
            },
            Value::Bytes(blake3::hash(canonical.as_bytes()).as_bytes().to_vec()),
            Value::Sum {
                ctor: 0,
                fields: Vec::new()
            },
            Value::Text("match x\n  | Circle c -> 3 * c.r * c.r\n  | Square q -> q.s * q.s".into()),
        ],
        row
    );
}

#[test]
fn a_function_hash_is_stable_across_whitespace_and_comments() {
    let respaced = FIRST.replace(
        "match x\n  | Circle c -> 3 * c.r * c.r",
        "-- by kind\n  match x | Circle c ->\n      3*c.r*c.r  -- pi, roughly",
    );
    assert_ne!(FIRST, respaced);
    let hash = |text: &str| function_row(&migrated(&Source::default(), text), "area").1[5].clone();
    assert_eq!(hash(FIRST), hash(&respaced));
    assert_ne!(hash(FIRST), hash(&FIRST.replace("3 * c.r", "4 * c.r")));
}

#[test]
fn a_host_function_is_hashed_over_its_declared_version() {
    let src = migrated(&Source::default(), FIRST);
    let (ctor, row) = function_row(&src, "stamp");
    assert_eq!("Host", ctor);
    assert_eq!(
        Value::Bytes(blake3::hash(b"1.2").as_bytes().to_vec()),
        row[5]
    );
    assert_eq!(
        &[Value::Text("stamp".into()), Value::Text("1.2".into())],
        &row[7..]
    );
    assert_eq!(
        Value::Sum {
            ctor: 0,
            fields: Vec::new()
        },
        row[4],
        "not declared deterministic"
    );
}

#[test]
fn a_language_function_calling_a_nondeterministic_one_is_not_deterministic() {
    let src = migrated(
        &migrated(&Source::default(), FIRST),
        "module app\ndefine fn label (t: Text) -> Text = stamp(t)\n",
    );
    let deterministic = |name: &str| function_row(&src, name).1[4].clone();
    let (falsity, truth) = (
        Value::Sum {
            ctor: 0,
            fields: Vec::new(),
        },
        Value::Sum {
            ctor: 1,
            fields: Vec::new(),
        },
    );
    assert_eq!(falsity, deterministic("label"));
    assert_eq!(truth, deterministic("area"));
}

#[test]
fn the_migrations_row_holds_the_version_its_hash_and_its_text() {
    let src = migrated(&Source::default(), FIRST);
    let schema = src.schema().expect("it builds");
    let rows: Vec<(u32, Vec<Value>)> = applied(&schema, &src, 1_767_225_600_000_000)
        .into_values()
        .flatten()
        .collect();
    assert_eq!(
        vec![(
            0,
            vec![
                Value::I64(1),
                Value::Bytes(hash_of(&src).to_vec()),
                Value::Text(FIRST.into()),
                Value::Instant(1_767_225_600_000_000),
            ]
        )],
        rows
    );
    assert!(applied(&engine_schema(), &Source::default(), 0).is_empty());
}

#[test]
fn a_stored_migration_that_does_not_parse_is_not_accepted() {
    let texts = BTreeMap::from([(1, FIRST.to_string()), (2, "module app\nadd".to_string())]);
    let Err(Error::Unaccepted(version, why)) = fold(&texts, &BTreeMap::new()) else {
        panic!("the log folds")
    };
    assert_eq!(2, version);
    assert_eq!(vec!["expected what to add, found the end of the text"], why);
}

#[test]
fn a_stored_migration_that_does_not_apply_is_not_accepted() {
    let texts = BTreeMap::from([(1, FIRST.to_string()), (2, FIRST.to_string())]);
    let Err(Error::Unaccepted(version, why)) = fold(&texts, &BTreeMap::new()) else {
        panic!("the log folds")
    };
    assert_eq!(2, version);
    assert_eq!(
        vec![
            "module `app` already exists",
            "module `app` already declares type `Shape`",
            "module `app` already declares relation `shapes`",
            "module `app` already declares set `shapes.Round`",
            "module `app` already declares fn `area`",
            "module `app` already declares fn `stamp`",
        ],
        why
    );
}

#[test]
fn a_stored_log_that_applies_folds() {
    let texts = BTreeMap::from([(1, FIRST.to_string())]);
    let (source, _, _) = fold(&texts, &BTreeMap::new()).expect("the log folds");
    assert_eq!(1, source.version);
}

fn sum(ctor: u32, fields: Vec<Value>) -> Vec<Value> {
    vec![Value::Sum { ctor, fields }]
}

#[test]
fn a_roots_row_of_another_shape_is_corrupt() {
    let ctor = |ctor, fields: &[Value]| Rot::RootsCtor {
        ctor,
        fields: fields.to_vec(),
    };
    let heap = |id| vec![Value::I64(1), Value::I64(0), Value::I64(id), Value::I64(7)];
    let text = || Value::Text("1".into());
    let rows = [
        (vec![Value::I64(1)], Rot::RootsRow(vec![Value::I64(1)])),
        (
            sum(9, vec![Value::I64(1)]),
            Rot::PendingRow {
                ctor: 9,
                fields: vec![Value::I64(1)],
            },
        ),
        (
            sum(HEAP_ROOT, vec![Value::I64(1)]),
            ctor(HEAP_ROOT, &[Value::I64(1)]),
        ),
        (
            sum(MIGRATION, vec![Value::I64(1), Value::I64(2)]),
            ctor(MIGRATION, &[Value::I64(1), Value::I64(2)]),
        ),
        (sum(HEAP_ROOT, heap(-3)), Rot::NegativeId(-3)),
        (sum(HEAP_ROOT, heap(1 << 40)), Rot::HeapIdWide(1 << 40)),
        (
            sum(MIGRATION, vec![text(), Value::Text("".into())]),
            Rot::NotAnId(text()),
        ),
    ];
    for (row, rot) in rows {
        let got = Named::default().add(&row);
        assert_eq!(Err(Error::Corrupt(rot)), got, "{row:?}");
    }
}

#[test]
fn a_roots_row_names_its_structure() {
    let mut named = Named::default();
    let row = sum(
        HEAP_ROOT,
        vec![Value::I64(1), Value::I64(0), Value::I64(3), Value::I64(7)],
    );
    named.add(&row).expect("the row reads");
    assert_eq!(
        Some(&(3, Pid(7))),
        named
            .heaps
            .get(&(CatRelId::from_raw(1), CatCtorId::from_raw(0)))
    );
}

#[test]
fn stored_order_keys_order_every_declared_sum() {
    let source = migrated(&Source::default(), FIRST);
    let schema = source.schema().expect("the schema builds");
    let keys = layout::new_order_keys(&schema, &layout::OrderKeys::new());
    assert_eq!(Ok(()), check_order_keys(&schema, &keys));
    let mut missing = keys.clone();
    let (last, _) = missing.pop_last().expect("an order key");
    let got = check_order_keys(&schema, &missing);
    assert_eq!(Err(Rot::NoOrderKey(last).into()), got);
    let mut reversed = keys.clone();
    let ids: Vec<u64> = layout::declared_sums(&schema)
        .into_iter()
        .find(|cs| cs.len() > 1)
        .expect("a sum of two constructors");
    let (a, b) = (keys[&ids[0]].clone(), keys[&ids[1]].clone());
    reversed.insert(ids[0], b);
    reversed.insert(ids[1], a);
    let got = check_order_keys(&schema, &reversed);
    assert_eq!(Err(Rot::OrderKeyOrder(ids[1]).into()), got);
}

#[test]
fn a_fill_that_fails_internally_fails_the_step_as_internal() {
    let (sa, sb, r, mut data) = gaining_a_field();
    let rid = super::ids::rel_id(&sa, r);
    let cid = super::ctor_id(&sa, r, 0);
    let rb = super::ids::relation(&sb, rid).expect("ts after");
    let fid = super::change::fields(&sb, rb, 0)
        .iter()
        .find(|f| f.name == "n")
        .and_then(|f| f.id)
        .expect("n has an id");
    let mut moved = super::change::Moved::default();
    moved.fills.insert(
        (rid, cid, fid),
        Box::new(|_, _| Err(Broken::LastNotSum.into())),
    );
    assert_eq!(
        Err(Error::from(Broken::LastNotSum)),
        super::change::step(&sa, &sb, &mut data, moved)
    );
}

/// Returns the schemas of a relation `ts` of a one-field record before and after it gains a
/// field `n`, the relation in the first, and data holding one row of it.
fn gaining_a_field() -> (Schema, Schema, adtb_lang::types::RelId, super::change::Data) {
    let a = migrated(
        &Source::default(),
        "module m\nadd module m\nadd type T = { id: Key ts }\nadd relation ts : T key id\n",
    );
    let b = migrated(&a, "module m\nadd field T.n: I64 fill 0\n");
    let (sa, sb) = (
        a.schema().expect("it builds"),
        b.schema().expect("it builds"),
    );
    let (r, _) = sa
        .ids
        .relation_ids()
        .find(|&(r, _)| sa.relation(r).name == "ts")
        .expect("ts");
    let rid = super::ids::rel_id(&sa, r);
    let cid = super::ctor_id(&sa, r, 0);
    let mut data = super::change::Data::new();
    data.insert(rid, [(cid, vec![vec![Value::I64(1)]])].into());
    (sa, sb, r, data)
}

#[test]
fn a_rekey_of_a_relation_the_schema_after_lacks_is_internal() {
    let (sa, _, r, data) = gaining_a_field();
    let maps = [adtb_lang::evolve::RowMap::Rekey { relation: r }];
    match super::moved::moved(&sa, &engine_schema(), &maps, &data, &Default::default()) {
        Err(e) => assert_eq!(
            Error::from(Broken::RekeyedGone(super::ids::rel_id(&sa, r))),
            e
        ),
        Ok(_) => panic!("the rekey maps"),
    }
}

#[test]
fn a_copy_from_a_row_without_the_field_is_internal() {
    let (sa, sb, r, data) = gaining_a_field();
    let maps = [adtb_lang::evolve::RowMap::Copy {
        def: sa.relation(r).elem_def,
        ctors: vec![0],
        field: "n".into(),
    }];
    let moved =
        super::moved::moved(&sa, &sb, &maps, &data, &Default::default()).expect("the copy maps");
    let fill = moved.fills.values().next().expect("the copy fills `n`");
    match fill(&[], &[]) {
        Err(super::change::Failed::Internal(why)) => {
            assert_eq!(Broken::NotCopied("n".into()), *why);
        }
        other => panic!("not an internal failure: {other:?}"),
    }
}

#[test]
fn an_index_path_naming_no_field_is_internal() {
    let (sa, _, r, _) = gaining_a_field();
    let path = [adtb_lang::kernel::PathStep::Field("missing".into())];
    let got = super::rows::path_steps(&sa, r, 0, &path);
    let want = crate::exec::Broken::PathRowField {
        path: path.to_vec(),
        field: "missing".into(),
    };
    assert_eq!(Err(want.into()), got.map(|_| ()));
}

#[test]
fn an_executor_damage_in_index_keying_stays_damage_and_its_internal_stays_internal() {
    let rot = crate::exec::Rot::SpillCut;
    let got = super::broken(rot.clone().into());
    assert_eq!(Error::Corrupt(Rot::Exec(rot)), got);
    let internal = || crate::exec::Error::from(crate::exec::Broken::EmptyRange);
    let want = Error::from(Broken::IndexKey(internal()));
    assert_eq!(want, super::broken(internal()));
}

#[test]
fn a_stored_row_without_the_shape_of_an_index_path_is_corrupt() {
    use super::rows::{Step, at_path};
    let steps = [Step::Field(0), Step::Ctor(1), Step::Field(0)];
    let held = [sum(1, vec![Value::I64(7)]).remove(0)];
    assert_eq!(Ok(Some(&Value::I64(7))), at_path(&held, &steps));
    let other = [sum(0, vec![]).remove(0)];
    assert_eq!(Ok(None), at_path(&other, &steps));
    for row in [vec![], vec![Value::I64(1)], vec![sum(1, vec![]).remove(0)]] {
        let got = at_path(&row, &steps);
        let rot = crate::exec::Rot::StoredPath(steps.to_vec());
        assert_eq!(Err(crate::exec::Error::Corrupt(rot)), got, "{row:?}");
    }
}

#[test]
fn a_derived_index_element_without_the_normal_program_is_internal() {
    let (sa, _, r, _) = gaining_a_field();
    let elements = [crate::plan::Element::Derived {
        index: 0,
        element: 0,
    }];
    let none = crate::exec::Broken::NoNormal {
        index: 0,
        element: 0,
    };
    let got = super::IndexKeys::new(&sa, None, r, &[0], &elements);
    assert_eq!(Some(none.clone().into()), got.err());
    let got = super::element_types(&sa, None, r, 0, &elements);
    assert_eq!(Err(none.into()), got);
}

#[test]
fn a_function_with_a_bounded_type_parameter_is_hashed() {
    let src = migrated(
        &Source::default(),
        "module app\nadd module app\ndefine fn least [a: Ord] (x: a, y: a) -> a = if x < y then x else y\n",
    );
    let (ctor, row) = function_row(&src, "least");
    assert_eq!("Language", ctor);
    assert_eq!(Value::Text("[a: Ord] (x: a, y: a) -> a".into()), row[3]);
}

/// Returns whether `line` reads a relation's key fields.
fn reads_key(line: &str) -> bool {
    [".key.", ".key;", ".key)", ".key_fields"]
        .iter()
        .any(|k| line.contains(k))
}

/// Returns whether `line` sorts: a sort method of any form, or an ordered collection built.
fn sorts(line: &str) -> bool {
    [
        ".sort(",
        ".sort_by",
        ".sort_unstable",
        ".sort_by_key",
        ".sort_by_cached_key",
    ]
    .iter()
    .any(|s| line.contains(s))
}

/// Add to `out` each line of the Rust sources under `dir` that orders a relation's key fields,
/// as `PATH: LINE TEXT`, and count in `seen` each sort it reads: a sort within 12 lines of a
/// read of the key, or a `BTreeSet` or `BTreeMap` collected in a statement that reads it.
fn key_sorts(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<String>,
    seen: &mut usize,
) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("the sources are readable")
        .map(|e| e.expect("a directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if !path.ends_with("target") {
                key_sorts(root, &path, out, seen);
            }
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs") || path.ends_with("catalog/tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a source is readable");
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let statement = || {
                let end = lines[i..].iter().position(|l| l.contains(';')).unwrap_or(0);
                &lines[i..=(i + end).min(i + 8)]
            };
            let hit = if sorts(l) {
                *seen += 1;
                lines[i.saturating_sub(12)..=i].iter().any(|l| reads_key(l))
            } else if l.contains("BTreeSet") || l.contains("BTreeMap") {
                *seen += 1;
                let st = statement();
                st.iter().any(|l| reads_key(l)) && st.iter().any(|l| l.contains("collect"))
            } else {
                false
            };
            if hit {
                let at = path.strip_prefix(root).expect("under the root").display();
                out.push(format!("{at}: {}", l.trim()));
            }
        }
    }
}

#[test]
fn only_key_names_sorts_the_fields_of_a_key() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (mut found, mut seen, mut scanned) = (Vec::new(), 0, 0);
    for group in ["crates", "apps"] {
        let mut dirs: Vec<_> = std::fs::read_dir(root.join(group))
            .expect("the crates are readable")
            .map(|e| e.expect("a directory entry").path())
            .collect();
        dirs.sort();
        for dir in dirs {
            let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
            if manifest.contains("adtb-lang") {
                scanned += 1;
                key_sorts(&root, &dir, &mut found, &mut seen);
            }
        }
    }
    assert!(
        scanned >= 15,
        "the search reached every crate on adtb-lang: {scanned}"
    );
    assert!(
        seen > 100,
        "the search read the sorts of those crates: {seen}"
    );
    let membership = [
        "crates/adtb-lang/src/evolve/ctor/mod.rs: \
         fn key_fields(schema: &Schema, def: TypeId) -> BTreeSet<String> {",
        "crates/adtb-lang/src/evolve/ids.rs: let keys: BTreeSet<&String> = schema",
    ];
    let rule = "crates/adtb-lang/src/textual/form.rs: names.sort();";
    assert_eq!(vec![membership[0], membership[1], rule], found);
}
