use std::collections::BTreeSet;

use crate::key::Value;
use crate::storage::Pid;

use super::root::Root;
use super::{declared, engine_schema, indexes};

#[test]
fn the_root_page_round_trips() {
    let root = Root {
        commit_log: Pid(7),
        large: (1, Pid(3)),
        roots: (2, Pid(9)),
    };
    let mut page = vec![0; 4096];
    root.write(&mut page);
    assert_eq!(root, Root::read(&page));
    assert!(page[..24].iter().all(|&b| b == 0));
}

#[test]
fn every_engine_object_with_an_id_has_a_row() {
    let schema = engine_schema();
    let rows = declared(&schema);
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
    let rows = declared(&schema);
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
    assert_eq!(&Value::I32(0), names[3]);
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
