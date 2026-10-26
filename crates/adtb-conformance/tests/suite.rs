use std::path::{Path, PathBuf};

fn part(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(name)
}

fn passes(name: &str) {
    let dir = part(name);
    assert!(
        !adtb_conformance::scripts(&dir).is_empty(),
        "no scripts in {}",
        dir.display()
    );
    let found = adtb_conformance::run_dir(&dir);
    assert!(
        found.is_empty(),
        "{} directives differ:\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn model() {
    passes("model");
}

#[test]
fn queries() {
    passes("queries");
}

#[test]
fn writes() {
    passes("writes");
}

#[test]
fn constraints() {
    passes("constraints");
}

#[test]
fn conformance() {
    passes("conformance");
}
