//! Generate the typed calls of the campaign manager's bundles.

use std::io::Write;
use std::path::{Path, PathBuf};

use adtb_codegen::{BundleSource, generate, read_log};

const LOG: &str = "schema/migrations";

const BUNDLES: [(&str, &str); 5] = [
    ("v1_0", "bundles/campaign-1.0.adb"),
    ("v1_3", "bundles/campaign-1.3.adb"),
    ("v1_4", "bundles/campaign-1.4.adb"),
    ("v1_5", "bundles/campaign-1.5.adb"),
    ("v1_6", "bundles/campaign-1.6.adb"),
];

fn main() {
    let mut directives = std::io::stdout();
    let mut rerun = |path: &str| {
        writeln!(directives, "cargo::rerun-if-changed={path}").expect("cargo reads directives");
    };
    rerun(LOG);
    let log = read_log(Path::new(LOG)).unwrap_or_else(|e| panic!("{LOG}: {e}"));
    let log: Vec<&str> = log.iter().map(String::as_str).collect();
    let mut texts = Vec::new();
    for (module, path) in BUNDLES {
        rerun(path);
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        texts.push((module, text));
    }
    let sources: Vec<BundleSource<'_>> = texts
        .iter()
        .map(|(module, text)| BundleSource { module, text })
        .collect();
    let code = generate(&log, &sources).unwrap_or_else(|e| panic!("{e}"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let out = out.join("campaign.rs");
    std::fs::write(&out, code).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}
