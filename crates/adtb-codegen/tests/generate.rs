//! What the generator emits and refuses, from the text it writes; the code it writes is compiled
//! and called by `adtb-codegen-tests`.

use adtb_codegen::{BundleSource, Error, generate};

const V1: &str = "module shop
add module shop
add type Tier = Free | Paid
add type Customer = { id: Key customers, name: Text, tier: Tier }
add relation customers : Customer key id generate sequence
define fn shown (c: Customer) -> Text = c.name
define fn tiered (c: Customer) -> Bool = match c.tier
  | Free -> false
  | Paid -> true
";

const V2: &str = "module shop
add type Note = { id: Key notes, body: Text }
add relation notes : Note key id generate sequence
";

const HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn file(pins: usize, edition: &str, body: &str) -> String {
    format!("bundle shop-1.0 pins {pins} ({HASH}) edition \"{edition}\"\nimport shop\n{body}")
}

fn generated(log: &[&str], text: &str) -> Result<String, Error> {
    generate(
        log,
        &[BundleSource {
            module: "queries",
            text,
        }],
    )
}

fn refusal(log: &[&str], text: &str) -> String {
    match generated(log, text) {
        Ok(_) => panic!("`{text}` generates"),
        Err(e) => e.message,
    }
}

#[test]
fn the_schema_module_holds_the_log_and_the_user_types() {
    let code = generated(&[V1, V2], &file(2, "2026-10", "")).expect("the bundle generates");
    assert!(code.contains("pub mod schema {"), "{code}");
    assert!(
        code.contains(&format!("{V1:?}")),
        "the log holds the first migration"
    );
    assert!(
        code.contains(&format!("{V2:?}")),
        "the log holds the second migration"
    );
    for item in [
        "pub enum Tier",
        "pub struct Customer",
        "pub struct Note",
        "pub struct Customers;",
        "pub struct Notes;",
    ] {
        assert!(code.contains(item), "the schema module holds `{item}`");
    }
}

#[test]
fn ordinals_count_imported_functions_then_items_in_declaration_order() {
    let body = "import fn tiered
import fn shown
fn local (c: Customer) -> Text = c.name
query names () -> Set Text = from c in customers select local(c)
txn add (n: Text) -> Key customers =
  transact
    let c = insert customers { name: n, tier: Free }
    return key c
";
    let code = generated(&[V1], &file(1, "2026-10", body)).expect("the bundle generates");
    for (name, ordinal) in [("tiered", 1), ("shown", 2), ("names", 3), ("add", 4)] {
        let at = code
            .find(&format!("impl ::adtb::Callable for {name} {{"))
            .unwrap_or_else(|| panic!("{name} is a callable"));
        let tail = &code[at..];
        let o = tail
            .find("const ORDINAL")
            .expect("a callable states its ordinal");
        assert!(
            tail[o..].starts_with(&format!("const ORDINAL: u32 = {ordinal};")),
            "{name} takes ordinal {ordinal}"
        );
    }
    assert!(
        !code.contains("for local {"),
        "a bundle function takes no ordinal"
    );
}

#[test]
fn a_bundle_at_the_last_version_reexports_the_schema_module() {
    let code = generated(&[V1, V2], &file(2, "2026-10", "")).expect("the bundle generates");
    assert!(code.contains("pub use super::schema::*;"), "{code}");
    assert!(!code.contains("pub mod types"), "{code}");
}

#[test]
fn a_bundle_pinned_earlier_holds_the_types_of_its_pin() {
    let code = generated(&[V1, V2], &file(1, "2026-10", "")).expect("the bundle generates");
    let at = code.find("pub mod queries").expect("the bundle module");
    let bundle = &code[at..];
    assert!(bundle.contains("pub mod types {"), "{bundle}");
    assert!(bundle.contains("pub struct Customer"), "{bundle}");
    assert!(
        !bundle.contains("pub struct Note"),
        "a type of a later version is not in the bundle's"
    );
}

#[test]
fn a_bundle_of_another_edition_is_refused() {
    let got = refusal(&[V1], &file(1, "2030-01", ""));
    assert!(got.contains("edition"), "{got}");
}

#[test]
fn a_pin_past_the_log_is_refused() {
    let got = refusal(&[V1], &file(2, "2026-10", ""));
    assert!(got.contains("pins version 2"), "{got}");
}

#[test]
fn a_callable_that_takes_a_type_name_is_refused() {
    let body = "query Customer () -> Set Text = from c in customers select c.name\n";
    let got = refusal(&[V1], &file(1, "2026-10", body));
    assert!(got.contains("`Customer`"), "{got}");
}

#[test]
fn a_callable_named_as_a_generated_item_is_refused() {
    let body = "query Error () -> Set Text = from c in customers select c.name\n";
    let got = refusal(&[V1], &file(1, "2026-10", body));
    assert!(got.contains("`Error`"), "{got}");
}

#[test]
fn a_query_that_does_not_check_is_refused_with_its_name() {
    let body = "query broken () -> Set Text = from c in customers select c.nothing\n";
    let got = refusal(&[V1], &file(1, "2026-10", body));
    assert!(got.contains("query `broken`"), "{got}");
}

#[test]
fn a_declared_limit_is_refused_as_registration_refuses_it() {
    let body = "query slow () limit 30s -> Set Text = from c in customers select c.name\n";
    let got = refusal(&[V1], &file(1, "2026-10", body));
    assert!(got.contains("query `slow`"), "{got}");
    assert!(got.contains("limit"), "{got}");
}

#[test]
fn a_feed_is_refused() {
    let got = refusal(&[V1], &file(1, "2026-10", "feed changes of customers\n"));
    assert!(got.contains("register"), "{got}");
}

#[test]
fn a_log_that_does_not_replay_is_refused_with_its_migration() {
    let got = refusal(
        &[V1, "module shop\ndrop relation nothing\n"],
        &file(1, "2026-10", ""),
    );
    assert!(got.contains("migration 2"), "{got}");
}
