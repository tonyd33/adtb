#[cfg(not(feature = "native"))]
fn main() {}

#[cfg(feature = "native")]
fn main() {
    use std::io::Write;
    use std::path::PathBuf;
    let mut output = std::io::stdout();
    for path in ["fixtures/log/0001.adt", "fixtures/baseline-1.0.adb"] {
        writeln!(output, "cargo::rerun-if-changed={path}").expect("cargo reads directives");
    }
    let migration = std::fs::read_to_string("fixtures/log/0001.adt")
        .expect("the benchmark migration is readable");
    let bundle = std::fs::read_to_string("fixtures/baseline-1.0.adb")
        .expect("the benchmark bundle is readable");
    let code = adtb_codegen::generate(
        &[&migration],
        &[adtb_codegen::BundleSource {
            module: "calls",
            text: &bundle,
        }],
    )
    .unwrap_or_else(|error| panic!("benchmark declarations: {error}"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    std::fs::write(out.join("baseline.rs"), code).expect("the generated code is writable");
}
