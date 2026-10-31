//! The exit status of the `campaign` binary on a usage error.

extern crate adtb_lang as _;

use adtb_heap_cap::test_child;

#[test]
fn a_usage_error_exits_with_one() {
    let out = test_child(env!("CARGO_BIN_EXE_campaign"), 1)
        .arg("--nope")
        .output()
        .expect("the server starts");
    assert_eq!(Some(1), out.status.code(), "{out:?}");
}
