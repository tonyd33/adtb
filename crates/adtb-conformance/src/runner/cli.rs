//! The command line of the `adtb-conformance` binary.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;

use clap::{Parser, Subcommand};

use super::{CONFORMANCE, Corpus, Format, Options, SCRIPTS, Selection, Sides};
use crate::probe;
use crate::{Found, runner};

#[derive(Debug, Parser)]
#[command(
    name = "adtb-conformance",
    about = "Run the conformance files under `conformance/` on the reference interpreter and on \
             the engine, and the engine scripts under `crates/adtb-sim/scripts/` on the engine \
             over the simulator, from the workspace root. Print each case that fails or is \
             updated, a summary, and each difference with its rows as a diff.",
    after_help = "Exits 0 when every case passes or is updated, 1 when one fails, 2 on a usage \
                  error.",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[command(flatten)]
    run: Run,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, clap::Args)]
struct Run {
    #[arg(
        long,
        value_name = "PATH",
        help = "run only this case file, or every case under this directory, of either corpus"
    )]
    file_name: Vec<PathBuf>,
    #[arg(
        short,
        long,
        value_name = "PATTERN",
        help = "run only the cases whose name, their path under the corpus without `.test`, \
                holds PATTERN"
    )]
    include: Option<String>,
    #[arg(
        short,
        long,
        value_name = "N",
        help = "run N cases at a time (default: half the logical CPUs)"
    )]
    jobs: Option<usize>,
    #[arg(
        long,
        help = "stop at the first case that fails, running one case at a time"
    )]
    fail_fast: bool,
    #[arg(
        short,
        long,
        help = "write the rows each differing directive got in place of its expected rows: the \
                reference's for a conformance file, unless only the engine runs"
    )]
    update: bool,
    #[arg(
        long,
        conflicts_with = "engine",
        help = "run the conformance files on the reference only, and no engine script"
    )]
    reference: bool,
    #[arg(long, help = "run the conformance files on the engine only")]
    engine: bool,
    #[arg(long, help = "print no colour")]
    no_color: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(
        about = "Check the schema FILE and the ad hoc TEXT, printing each refusal with its line \
                 and column. Then run TEXT on a fresh database of the reference interpreter and \
                 of the engine over the simulator, print each outcome and whether they agree. \
                 TEXT `-` reads the text from stdin.",
        after_help = "Exits 0 when the sides agree, 1 when they differ or a side fails to load, \
                      2 when the schema or the text is refused or NAME names nothing.",
        args_override_self = true
    )]
    Probe(probe::Args),
    /// Run one conformance file on the engine and print its differences as one JSON line.
    #[command(hide = true)]
    EngineCase { root: PathBuf, file: PathBuf },
}

/// Run the command `std::env::args` names.
#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "goes if main stops printing the report, the probe's output and usage errors"
)]
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Probe(args)) => {
            let ran = args.resolve().and_then(|a| probe::probe(&a));
            match ran {
                Ok((out, code)) => {
                    print!("{out}");
                    code
                }
                Err(e) => {
                    eprintln!("adtb-conformance probe: {e}");
                    ExitCode::from(2)
                }
            }
        }
        Some(Command::EngineCase { root, file }) => {
            let corpus = Corpus {
                format: Format::Conformance,
                root,
            };
            let selection = runner::select(&corpus, &file);
            match (selection.cases.as_slice(), selection.problems.as_slice()) {
                ([case], []) => {
                    println!("{}", runner::encode(&runner::on_engine(case)));
                    ExitCode::SUCCESS
                }
                (cases, problems) => {
                    let problems: Vec<String> = problems.iter().map(Found::shown).collect();
                    eprintln!(
                        "adtb-conformance engine-case: {} selects {} cases, not one: {}",
                        file.display(),
                        cases.len(),
                        problems.join("; ")
                    );
                    ExitCode::from(2)
                }
            }
        }
        None => match corpora(&cli.run) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("adtb-conformance: {e}");
                ExitCode::from(2)
            }
        },
    }
}

/// The corpora under the working directory, their roots made absolute.
fn roots() -> Result<[Corpus; 2], String> {
    let root = |format, dir: &str| {
        Path::new(dir)
            .canonicalize()
            .map(|root| Corpus { format, root })
            .map_err(|e| format!("{dir}: {e}; run from the workspace root"))
    };
    Ok([
        root(Format::Conformance, CONFORMANCE)?,
        root(Format::Scripts, SCRIPTS)?,
    ])
}

/// Returns the selection of each path `run` names, or of each corpus when it names none.
fn selections(run: &Run, sides: Sides) -> Result<Vec<Selection>, String> {
    let corpora = roots()?;
    let wanted = |c: &Corpus| {
        c.format == Format::Conformance || (sides != Sides::Reference && cfg!(feature = "scripts"))
    };
    if run.file_name.is_empty() {
        return Ok(corpora
            .iter()
            .filter(|c| wanted(c))
            .map(|c| runner::select(c, &c.root))
            .collect());
    }
    let mut out = Vec::new();
    for path in &run.file_name {
        let full = path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let corpus = corpora
            .iter()
            .find(|c| full.starts_with(&c.root))
            .ok_or_else(|| {
                format!(
                    "{}: under neither `{CONFORMANCE}` nor `{SCRIPTS}` of the working directory",
                    path.display()
                )
            })?;
        if corpus.format == Format::Scripts && !cfg!(feature = "scripts") {
            return Err(format!(
                "{}: the runner is built without the `scripts` feature; run `cargo conformance`",
                path.display()
            ));
        }
        if wanted(corpus) {
            out.push(runner::select(corpus, &full));
        }
    }
    let conformance = run.file_name.is_empty()
        || out
            .iter()
            .any(|s| s.cases.iter().any(|c| c.format == Format::Conformance));
    if run.update && sides == Sides::Engine && conformance {
        return Err(
            "`--update` takes the reference's rows; the engine never writes the expected rows \
             of a conformance file"
                .into(),
        );
    }
    Ok(out)
}

fn corpora(run: &Run) -> Result<ExitCode, String> {
    let sides = match (run.reference, run.engine) {
        (true, _) => Sides::Reference,
        (_, true) => Sides::Engine,
        _ => Sides::Both,
    };
    let selections = selections(run, sides)?;
    let program = std::env::current_exe().map_err(|e| format!("this program's path: {e}"))?;
    let options = Options {
        sides,
        update: run.update,
        include: run.include.clone(),
        jobs: run
            .jobs
            .unwrap_or_else(|| thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))),
        fail_fast: run.fail_fast,
        color: !run.no_color
            && io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none(),
        isolate: Some(program),
    };
    let mut all = Selection::default();
    for s in selections {
        all.cases.extend(s.cases);
        all.problems.extend(s.problems);
    }
    let mut out = io::stdout();
    let report = runner::run(all, &options, &mut out).map_err(|e| format!("stdout: {e}"))?;
    out.flush().map_err(|e| format!("stdout: {e}"))?;
    Ok(if report.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Returns the probe's arguments when `args`, after the program name, name the probe.
#[cfg(test)]
pub(crate) fn probe_args(args: &[&str]) -> Result<probe::Args, clap::Error> {
    let all = ["adtb-conformance", "probe"]
        .into_iter()
        .chain(args.iter().copied());
    match Cli::try_parse_from(all)?.command {
        Some(Command::Probe(a)) => Ok(a),
        other => panic!("the probe parses as another command: {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_is_well_formed() {
        Cli::command().debug_assert();
    }
}
