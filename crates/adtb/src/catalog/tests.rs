use std::collections::BTreeSet;

use adtb_lang::evolve::Source;
use adtb_lang::schema::Schema;

use crate::key::Value;
use crate::storage::Pid;

use super::root::Root;
use super::{Rows, declared, engine_schema, indexes, layout};

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
    let root = Root {
        commit_log: Pid(7),
        large: (1, Pid(3)),
        roots: (2, Pid(9)),
        next_heap: 70,
        changing: 1025,
    };
    let mut page = vec![0; 4096];
    root.write(&mut page);
    assert_eq!(root, Root::read(&page));
    assert!(page[..24].iter().all(|&b| b == 0));
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
