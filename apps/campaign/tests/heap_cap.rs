use std::path::Path;

#[test]
fn every_test_binary_has_the_heap_cap() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    assert_eq!(
        adtb_heap_cap::uncapped_test_binaries(&manifest),
        Vec::<String>::new()
    );
}
