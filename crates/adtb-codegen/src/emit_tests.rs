use adtb_lang::ids::{CatalogIds, Slot};
use adtb_lang::schema::{Schema, States, build_with};
use adtb_lang::signature::replay;

use super::Module;

const LOG: &str = "module shop
add module shop
add type Tier = Free | Paid
add type Customer = { id: Key customers, tier: Tier }
add relation customers : Customer key id generate sequence
";

fn refusal(stripped: impl Fn(&CatalogIds) -> Vec<Slot>) -> String {
    let source = replay([LOG]).expect("the log replays");
    let full = source.schema().expect("the log builds");
    let files = source.files().expect("the log's declarations parse");
    let without = |_: &Schema| {
        let mut ids = full.ids().clone();
        for slot in stripped(full.ids()) {
            ids.forget(&slot);
        }
        ids
    };
    let schema = build_with(&files, &[], &without, &States::default()).expect("the log builds");
    match Module::new(&schema).user_items() {
        Ok(()) => panic!("a schema missing an id generates"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_type_with_no_id_is_refused() {
    let e = refusal(|ids| ids.type_ids().map(|(t, _)| Slot::Type(t)).collect());
    assert!(e.contains("has no catalog id"), "{e}");
}

#[test]
fn a_constructor_with_no_id_is_refused() {
    let e = refusal(|ids| ids.ctor_ids().map(|(c, _)| Slot::Ctor(c)).collect());
    assert!(e.contains("has no catalog id"), "{e}");
}

#[test]
fn a_relation_with_no_id_is_refused() {
    let e = refusal(|ids| ids.relation_ids().map(|(r, _)| Slot::Relation(r)).collect());
    assert!(e.contains("relation `customers` has no catalog id"), "{e}");
}
