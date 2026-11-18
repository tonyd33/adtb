//! The exit status of the `adtb-codegen` binary on a usage error.

extern crate adtb_lang as _;

#[test]
fn a_usage_error_exits_with_one() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_adtb-codegen"))
        .arg("--nope")
        .output()
        .expect("the generator starts");
    assert_eq!(Some(1), out.status.code(), "{out:?}");
}
