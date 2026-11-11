//! Reads of the catalog root page the reindex tests share.

use adtb::Database;
use adtb::testing::Object;
use adtb_sim::SimRuntime;

/// Returns how many words of the catalog root page read as a reindex under way.
pub fn changing_words(db: &Database<SimRuntime>) -> u64 {
    let t = db.testing();
    let page = t.probe().page(t.root(Object::Catalog).unwrap()).unwrap();
    let mut found = 0;
    for at in (0..page.len() - 8).step_by(8) {
        let v = u64::from_le_bytes(page[at..at + 8].try_into().unwrap());
        if v == u64::MAX - 1 {
            found += 1;
        }
    }
    found
}
