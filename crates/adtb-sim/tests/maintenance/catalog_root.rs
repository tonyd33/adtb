//! The reads of the catalog root page the reindex tests share.

use adtb::Database;
use adtb_sim::SimRuntime;

/// Returns what the catalog root of `db` reads as changing, as the catalog probe prints it:
/// `change Reindexing` while rebuilds of indexes are under way, `change None` with none.
pub fn changing(db: &Database<SimRuntime>) -> String {
    let lines = db.testing().catalog(false).unwrap();
    lines
        .into_iter()
        .find(|l| l.starts_with("change "))
        .expect("the catalog probe prints what its root reads as changing")
}
