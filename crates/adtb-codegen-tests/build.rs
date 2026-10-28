//! Generate the typed calls of the fixture shop and of the campaign app.

use std::io::Write;
use std::path::{Path, PathBuf};

use adtb_codegen::{BundleSource, generate, read_log};

fn emit(log_dir: &str, bundles: &[(&str, &str)], out: &Path) {
    let mut directives = std::io::stdout();
    let mut rerun = |path: &str| {
        writeln!(directives, "cargo::rerun-if-changed={path}").expect("cargo reads directives");
    };
    rerun(log_dir);
    let log = read_log(Path::new(log_dir)).unwrap_or_else(|e| panic!("{log_dir}: {e}"));
    let log: Vec<&str> = log.iter().map(String::as_str).collect();
    let mut texts = Vec::new();
    for (module, path) in bundles {
        rerun(path);
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        texts.push((*module, text));
    }
    let sources: Vec<BundleSource<'_>> = texts
        .iter()
        .map(|(module, text)| BundleSource { module, text })
        .collect();
    let code = generate(&log, &sources).unwrap_or_else(|e| panic!("{log_dir}: {e}"));
    std::fs::write(out, code).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    emit(
        "fixtures/shop/log",
        &[("queries", "fixtures/shop/shop-1.0.adb")],
        &out.join("shop.rs"),
    );
    emit(
        "../../apps/campaign/schema/migrations",
        &[("queries", "../../apps/campaign/bundles/campaign-1.0.adb")],
        &out.join("campaign.rs"),
    );
}
