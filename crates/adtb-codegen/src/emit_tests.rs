use adtb_lang::signature::replay;

use super::Module;

const LOG: &str = "module shop
add module shop
add type Tier = Free | Paid
add type Customer = { id: Key customers, tier: Tier }
add relation customers : Customer key id generate sequence
";

fn refusal(strip: impl FnOnce(&mut adtb_lang::ids::CatalogIds)) -> String {
    let source = replay([LOG]).expect("the log replays");
    let mut schema = source.schema().expect("the log builds");
    strip(&mut schema.ids);
    match Module::new(&schema).user_items() {
        Ok(()) => panic!("a schema missing an id generates"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_type_with_no_id_is_refused() {
    let e = refusal(|ids| ids.types.clear());
    assert!(e.contains("has no catalog id"), "{e}");
}

#[test]
fn a_constructor_with_no_id_is_refused() {
    let e = refusal(|ids| ids.ctors.clear());
    assert!(e.contains("has no catalog id"), "{e}");
}

#[test]
fn a_relation_with_no_id_is_refused() {
    let e = refusal(|ids| ids.relations.clear());
    assert!(e.contains("relation `customers` has no catalog id"), "{e}");
}
