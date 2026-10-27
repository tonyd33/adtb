//! `adtb-codegen --log DIR --bundle MODULE=FILE... [--out FILE]`: write the Rust source of a
//! schema's log and its bundles, to `FILE` or standard output.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use adtb_codegen::{BundleSource, generate, read_log};

const USAGE: &str = "usage: adtb-codegen --log DIR --bundle MODULE=FILE... [--out FILE]\n";

struct Args {
    log: PathBuf,
    bundles: Vec<(String, PathBuf)>,
    out: Option<PathBuf>,
}

fn args() -> Result<Args, String> {
    let mut log = None;
    let mut bundles = Vec::new();
    let mut out = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{a} takes a value"));
        match a.as_str() {
            "--log" => log = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            "--bundle" => {
                let v = value()?;
                let (module, file) = v
                    .split_once('=')
                    .ok_or_else(|| format!("--bundle {v}: expected MODULE=FILE"))?;
                bundles.push((module.to_owned(), PathBuf::from(file)));
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Args {
        log: log.ok_or("--log is required")?,
        bundles,
        out,
    })
}

fn run() -> Result<(), String> {
    let a = args()?;
    let log = read_log(&a.log).map_err(|e| format!("{}: {e}", a.log.display()))?;
    let mut texts = Vec::new();
    for (module, path) in &a.bundles {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        texts.push((module.as_str(), text));
    }
    let log: Vec<&str> = log.iter().map(String::as_str).collect();
    let bundles: Vec<BundleSource<'_>> = texts
        .iter()
        .map(|(module, text)| BundleSource { module, text })
        .collect();
    let code = generate(&log, &bundles).map_err(|e| e.message)?;
    match &a.out {
        Some(path) => std::fs::write(path, code).map_err(|e| format!("{}: {e}", path.display())),
        None => std::io::stdout()
            .write_all(code.as_bytes())
            .map_err(|e| e.to_string()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let mut err = std::io::stderr();
            let written = write!(err, "adtb-codegen: {e}\n{USAGE}");
            if written.is_err() {
                return ExitCode::from(2);
            }
            ExitCode::FAILURE
        }
    }
}
