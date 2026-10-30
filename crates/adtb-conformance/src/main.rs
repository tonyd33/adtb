use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "adtb-conformance",
    about = "Run each conformance file, or each file under a directory, on the reference \
             interpreter and print every difference from what the file expects."
)]
struct Args {
    #[arg(
        value_name = "PATH",
        help = "a conformance file or a directory of them"
    )]
    paths: Vec<PathBuf>,
}

#[expect(
    clippy::print_stdout,
    reason = "goes if the runner stops being a binary that reports on its streams"
)]
fn main() -> ExitCode {
    let mut failed = false;
    for path in Args::parse().paths {
        let found = if path.is_dir() {
            adtb_conformance::run_dir(&path)
        } else {
            adtb_conformance::run_file(&path)
        };
        for d in &found {
            println!("{d}");
        }
        failed |= !found.is_empty();
    }
    if failed {
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
    fn the_conf_invocation_parses() {
        let a = Args::try_parse_from(["adtb-conformance", "conformance/queries/match.test"])
            .expect("a file parses");
        assert_eq!(
            vec![PathBuf::from("conformance/queries/match.test")],
            a.paths
        );
        let none = Args::try_parse_from(["adtb-conformance"]).expect("no path parses");
        assert!(none.paths.is_empty());
    }
}
