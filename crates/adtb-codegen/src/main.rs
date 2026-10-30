//! `adtb-codegen --log DIR --bundle MODULE=FILE... [--out FILE]`: write the Rust source of a
//! schema's log and its bundles, to `FILE` or standard output.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use adtb_codegen::{BundleSource, generate, read_log};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "adtb-codegen",
    about = "Write the Rust source of a schema's log and its bundles, to FILE or standard output.",
    args_override_self = true
)]
struct Args {
    #[arg(
        long,
        value_name = "DIR",
        allow_hyphen_values = true,
        help = "the migration log"
    )]
    log: PathBuf,
    #[arg(
        long = "bundle",
        value_name = "MODULE=FILE",
        allow_hyphen_values = true,
        value_parser = bundle,
        help = "a bundle file and the module its code goes in"
    )]
    bundles: Vec<(String, PathBuf)>,
    #[arg(
        long,
        value_name = "FILE",
        allow_hyphen_values = true,
        help = "the file to write"
    )]
    out: Option<PathBuf>,
}

fn bundle(v: &str) -> Result<(String, PathBuf), String> {
    let (module, file) = v.split_once('=').ok_or("expected MODULE=FILE")?;
    Ok((module.to_owned(), PathBuf::from(file)))
}

fn run(a: &Args) -> Result<(), String> {
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
    let a = match Args::try_parse() {
        Ok(a) => a,
        Err(e) => return usage(&e),
    };
    match run(&a) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => match writeln!(std::io::stderr(), "adtb-codegen: {e}") {
            Ok(()) => ExitCode::FAILURE,
            Err(_) => ExitCode::from(2),
        },
    }
}

/// Print `e`. Returns 1 for a usage error and 0 for help.
fn usage(e: &clap::Error) -> ExitCode {
    if e.print().is_err() {
        return ExitCode::from(2);
    }
    if e.use_stderr() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_is_well_formed() {
        Args::command().debug_assert();
    }

    #[test]
    fn an_invocation_parses() {
        let a = Args::try_parse_from([
            "adtb-codegen",
            "--log",
            "log",
            "--bundle",
            "app=app.bundle",
            "--bundle",
            "web=web.bundle",
            "--out",
            "gen.rs",
        ])
        .expect("the invocation parses");
        assert_eq!(PathBuf::from("log"), a.log);
        assert_eq!(
            vec![
                ("app".to_owned(), PathBuf::from("app.bundle")),
                ("web".to_owned(), PathBuf::from("web.bundle"))
            ],
            a.bundles
        );
        assert_eq!(Some(PathBuf::from("gen.rs")), a.out);
    }

    #[test]
    fn the_log_is_required_and_bundles_and_out_are_not() {
        assert!(Args::try_parse_from(["adtb-codegen", "--out", "f"]).is_err());
        let a = Args::try_parse_from(["adtb-codegen", "--log", "log"]).expect("--log alone");
        assert!(a.bundles.is_empty());
        assert_eq!(None, a.out);
    }

    #[test]
    fn a_bundle_needs_a_module() {
        assert!(Args::try_parse_from(["adtb-codegen", "--log", "l", "--bundle", "f"]).is_err());
    }
}
