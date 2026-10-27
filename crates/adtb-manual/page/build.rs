use std::path::Path;

use adtb_codegen::{BundleSource, generate, read_log};

fn main() {
    let log = read_log(Path::new("migrations")).expect("the migrations read");
    let log: Vec<&str> = log.iter().map(String::as_str).collect();
    let text = std::fs::read_to_string("shelter-1.0.adb").expect("the bundle reads");
    let bundle = BundleSource {
        module: "queries",
        text: &text,
    };
    let code = generate(&log, &[bundle]).unwrap_or_else(|e| panic!("{e}"));
    let out = std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR");
    std::fs::write(Path::new(&out).join("shelter.rs"), code).expect("the code writes");
}
