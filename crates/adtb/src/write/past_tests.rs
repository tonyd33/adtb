//! The reads of the past in process over the simulator: a registered `at` block pinned past the
//! database's version.

use adtb_testkit::process;

use crate::catalog::version_hash;
use crate::{Database, Log, OpenError, OpenOptions, Registration};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};

use crate::test_world::load::options;

const STOCK: &str = r"module stock
add module stock
add type Item { id: Key items, name: Text }
  = Open   { qty: I64 }
  | Closed
add relation items : Item key id generate sequence
add type Order = { id: Key orders, item: Key items }
add relation orders : Order key id generate sequence
";

/// A migration after [`STOCK`] that rewrites `items`.
const RENAME: &str = r"module stock
rename field Item.Open.qty to count
";

const NAMES: &str = "from i in items select i.name";

/// Returns `pins` naming the version `migrations` make, with its hash.
fn pins(migrations: &[&str]) -> String {
    let mut source = Source::default();
    for text in migrations {
        let file = parse_migration(text, Options::default())
            .ok()
            .unwrap_or_else(|ds| panic!("`{text}` parses: {ds:?}"));
        source = source
            .migrate(&file, text)
            .unwrap_or_else(|r| panic!("`{text}` applies: {r:?}"));
    }
    let hash: String = version_hash(&source)
        .expect("the log resolves")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("pins {} ({hash})", migrations.len())
}

/// Returns bundle `reports-1.0`, pinned at the version `migrations` make, whose query
/// `names_then` reads the names of items at a position, its block pinned at the version
/// `block` makes.
fn reports(migrations: &[&str], block: &[&str]) -> Registration {
    let text = format!(
        "bundle reports-1.0 {} edition \"2026-10\"\nimport stock\nquery names_then (c: Position) -> Set Text =\n  at commit c {} {{ {NAMES} }}\n",
        pins(migrations),
        pins(block)
    );
    Registration::bundle(&text).unwrap_or_else(|e| panic!("`{text}` is a bundle file: {e:?}"))
}

#[test]
fn a_registered_block_pinned_past_the_databases_version_is_refused_at_open() {
    process::once(22, |rt| {
        let opts = OpenOptions {
            registrations: vec![reports(&[STOCK], &[STOCK, RENAME])],
            ..options(2)
        };
        match Database::open_in(rt, "stock.adt", &Log::new([STOCK]), &opts) {
            Err(OpenError::RegistrationRefused { bundle, .. }) => assert_eq!("reports-1.0", bundle),
            other => panic!("the registration is refused: {other:?}"),
        }
    });
}
