mod cli;
mod engine;
mod engine_runner;
mod index_elements;
mod probe;
mod runner;
mod suite;
mod unchecked_state;
mod upsert_results;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[test]
fn every_source_of_the_binary_is_a_module() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut targets = BTreeSet::new();
    for entry in fs::read_dir(&tests).expect("the tests directory reads") {
        let path = entry.expect("a directory entry").path();
        if path.extension().is_some_and(|extension| extension == "rs")
            || path.join("main.rs").exists()
        {
            targets.insert(path);
        }
    }
    let root = tests.join("conformance");
    assert_eq!(targets, BTreeSet::from([root.clone()]));
    let main = fs::read_to_string(root.join("main.rs")).expect("main.rs reads");
    let declared: BTreeSet<String> = main
        .lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .map(|name| format!("{name}.rs"))
        .collect();
    let mut sources = BTreeSet::new();
    for entry in fs::read_dir(&root).expect("the binary's directory reads") {
        let name = entry.expect("a directory entry").file_name();
        sources.insert(name.into_string().expect("a UTF-8 file name"));
    }
    sources.remove("main.rs");
    assert_eq!(sources, declared);
}
