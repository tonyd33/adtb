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
    let mut build = String::from("pub const BUILD: &[(&str, &str)] = &[\n");
    for (key, variable) in [
        ("profile", "PROFILE"),
        ("opt_level", "OPT_LEVEL"),
        ("debug", "DEBUG"),
        ("host", "HOST"),
        ("target", "TARGET"),
        ("rustflags", "CARGO_ENCODED_RUSTFLAGS"),
        ("cflags", "CFLAGS"),
        ("cc", "CC"),
    ] {
        writeln!(output, "cargo::rerun-if-env-changed={variable}").expect("cargo reads directives");
        let value = std::env::var(variable).unwrap_or_default();
        build.push_str(&format!("({key:?}, {value:?}),\n"));
    }
    build.push_str("];\n");
    std::fs::write(out.join("build-info.rs"), build).expect("the build context is writable");
}
