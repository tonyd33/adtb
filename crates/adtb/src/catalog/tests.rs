use std::collections::{BTreeMap, BTreeSet};

use adtb_lang::evolve::Source;
use adtb_lang::schema::Schema;

use crate::key::Value;
use crate::mvcc::Xid;
use crate::storage::Pid;

use super::ids::{CatCtorId, CatRelId};
use super::root::{Changing, Root};
use super::{
    Error, HEAP_ROOT, Hash, MIGRATION, Named, Rows, applied, check_order_keys, declared,
    engine_schema, fold, indexes, layout, oplog, version_hash,
};

fn engine_rows(schema: &Schema) -> Rows {
    declared(schema, &Source::default())
}

/// Returns the rows of the catalog relation whose id is `id`.
fn rows_of(schema: &Schema, rows: &Rows, id: u64) -> Vec<Vec<Value>> {
    let r = *schema
        .ids
        .relations
        .iter()
        .find(|(_, i)| **i == id)
        .unwrap()
        .0;
    rows[&r].iter().map(|(_, row)| row.clone()).collect()
}

#[test]
fn the_root_page_round_trips() {
    for (changing, stored) in [
        (Changing::None, 0),
        (Changing::Schema(Xid(1025)), 1025),
        (Changing::NamingSequences, u64::MAX),
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
        let r = *schema
            .ids
            .relations
            .iter()
            .find(|(_, i)| **i == rel)
            .unwrap()
            .0;
        rows[&r].len()
    };
    assert_eq!(schema.ids.types.len(), count(219));
    assert_eq!(schema.ids.ctors.len(), count(220));
    assert_eq!(schema.ids.fields.len(), count(222));
    assert_eq!(schema.ids.relations.len(), count(223));
    assert_eq!(61, count(226));
    assert_eq!(0, count(224));
    assert_eq!(0, count(232));
}

#[test]
fn the_constructors_relation_names_its_own_row() {
    let schema = engine_schema();
    let rows = engine_rows(&schema);
    let r = *schema
        .ids
        .relations
        .iter()
        .find(|(_, i)| **i == 220)
        .unwrap()
        .0;
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
        (0..61).collect::<BTreeSet<u64>>(),
        all.keys().copied().collect()
    );
    let uniques: Vec<(u64, Vec<String>)> = all
        .values()
        .filter(|s| s.fields != schema.relation(s.relation).key)
        .map(|s| (s.id, s.fields.clone()))
        .collect();
    assert_eq!(7, uniques.len(), "{uniques:?}");
    assert!(uniques.iter().all(|(_, f)| f.len() <= 2));
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
    let Err(Error::Unaccepted(version, why)) = fold(&texts) else {
        panic!("the log folds")
    };
    assert_eq!(2, version);
    assert!(!why.is_empty());
}

#[test]
fn a_stored_migration_that_does_not_apply_is_not_accepted() {
    let texts = BTreeMap::from([(1, FIRST.to_string()), (2, FIRST.to_string())]);
    let Err(Error::Unaccepted(version, why)) = fold(&texts) else {
        panic!("the log folds")
    };
    assert_eq!(2, version);
    assert!(!why.is_empty());
}

#[test]
fn a_stored_log_that_applies_folds() {
    let texts = BTreeMap::from([(1, FIRST.to_string())]);
    let (source, _, _) = fold(&texts).expect("the log folds");
    assert_eq!(1, source.version);
}

fn sum(ctor: u32, fields: Vec<Value>) -> Vec<Value> {
    vec![Value::Sum { ctor, fields }]
}

#[test]
fn a_roots_row_of_another_shape_is_corrupt() {
    let rows = [
        vec![Value::I64(1)],
        sum(9, vec![Value::I64(1)]),
        sum(HEAP_ROOT, vec![Value::I64(1)]),
        sum(MIGRATION, vec![Value::I64(1), Value::I64(2)]),
        sum(
            HEAP_ROOT,
            vec![Value::I64(1), Value::I64(0), Value::I64(-3), Value::I64(7)],
        ),
        sum(
            HEAP_ROOT,
            vec![
                Value::I64(1),
                Value::I64(0),
                Value::I64(1 << 40),
                Value::I64(7),
            ],
        ),
        sum(
            MIGRATION,
            vec![Value::Text("1".into()), Value::Text("".into())],
        ),
    ];
    for row in rows {
        let got = Named::default().add(&row);
        assert!(matches!(got, Err(Error::Corrupt(_))), "{row:?}: {got:?}");
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
        named.heaps.get(&(CatRelId(1), CatCtorId(0)))
    );
}

#[test]
fn stored_order_keys_order_every_declared_sum() {
    let source = migrated(&Source::default(), FIRST);
    let schema = source.schema().expect("the schema builds");
    let keys = layout::new_order_keys(&schema, &layout::OrderKeys::new());
    assert_eq!(Ok(()), check_order_keys(&schema, &keys));
    let mut missing = keys.clone();
    missing.pop_last();
    let got = check_order_keys(&schema, &missing);
    assert!(matches!(got, Err(Error::Corrupt(_))), "{got:?}");
    let mut reversed = keys.clone();
    let ids: Vec<u64> = layout::declared_sums(&schema)
        .into_iter()
        .find(|cs| cs.len() > 1)
        .expect("a sum of two constructors");
    let (a, b) = (keys[&ids[0]].clone(), keys[&ids[1]].clone());
    reversed.insert(ids[0], b);
    reversed.insert(ids[1], a);
    let got = check_order_keys(&schema, &reversed);
    assert!(matches!(got, Err(Error::Corrupt(_))), "{got:?}");
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
        Box::new(|_, _| Err(super::change::Failed::Internal("broken".into()))),
    );
    match super::change::step(&sa, &sb, &mut data, moved) {
        Err(Error::Internal(why)) => assert_eq!("broken", why),
        other => panic!("not an internal failure: {other:?}"),
    }
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
    let r = *sa
        .ids
        .relations
        .keys()
        .find(|&&r| sa.relation(r).name == "ts")
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
        Err(Error::Internal(why)) => assert!(why.contains("rekeyed"), "{why}"),
        Err(e) => panic!("not an internal failure: {e:?}"),
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
        Err(super::change::Failed::Internal(why)) => assert!(why.contains("`n`"), "{why}"),
        other => panic!("not an internal failure: {other:?}"),
    }
}

#[test]
fn an_index_path_naming_no_field_is_internal() {
    let (sa, _, r, _) = gaining_a_field();
    let path = [adtb_lang::kernel::PathStep::Field("missing".into())];
    let got = super::rows::path_steps(&sa, r, 0, &path);
    assert!(
        matches!(got, Err(crate::exec::Error::Internal(_))),
        "{got:?}"
    );
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
        assert!(
            matches!(got, Err(crate::exec::Error::Corrupt(_))),
            "{row:?}: {got:?}"
        );
    }
}

#[test]
fn a_derived_index_element_without_the_normal_program_is_internal() {
    let (sa, _, r, _) = gaining_a_field();
    let elements = [crate::plan::Element::Derived {
        index: 0,
        element: 0,
    }];
    let got = super::IndexKeys::new(&sa, None, r, &[0], &elements);
    assert!(
        matches!(got, Err(crate::exec::Error::Internal(_))),
        "{got:?}"
    );
    let got = super::element_types(&sa, None, r, 0, &elements);
    assert!(
        matches!(got, Err(crate::exec::Error::Internal(_))),
        "{got:?}"
    );
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
