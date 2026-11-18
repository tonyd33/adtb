use adtb_testkit::scratch::Scratch;
use std::path::PathBuf;

/// Returns a new scratch directory and the path of a database file in it.
pub fn scratch(name: &str) -> (Scratch, PathBuf) {
    let dir = Scratch::new("web", name);
    let file = dir.join("campaign.adt");
    (dir, file)
}
