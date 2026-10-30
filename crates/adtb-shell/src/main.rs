mod schema;
mod session;
mod text;

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use adtb::{CreateOptions, Log, OpenOptions};
use clap::Parser;

use session::{Command, Session};
use text::Shape;

const PROMPT: &str = "adtb> ";
const CONTINUE: &str = "  ... ";

#[derive(Debug, Parser)]
#[command(
    name = "adtb",
    override_usage = "adtb [--create] [--schema FILE]... FILE [-c TEXT]",
    about = "Open the database FILE and run ad hoc text against it: TEXT when -c gives one, \
             else the texts read from stdin, with a prompt on a terminal.",
    after_help = "At the prompt, \\help lists the shell's commands.",
    args_override_self = true
)]
struct Args {
    #[arg(value_name = "FILE", help = "the database file")]
    file: PathBuf,
    #[arg(long, help = "create FILE when it does not exist")]
    create: bool,
    #[arg(
        long = "schema",
        value_name = "FILE",
        allow_hyphen_values = true,
        help = "the next migration of the database's log; the first may instead be a schema \
                file of declarations"
    )]
    log: Vec<PathBuf>,
    #[arg(
        short = 'c',
        value_name = "TEXT",
        allow_hyphen_values = true,
        help = "run TEXT and exit"
    )]
    text: Option<String>,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match shell(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(why) => {
            report(&why);
            ExitCode::FAILURE
        }
    }
}

fn report(message: &str) {
    if let Err(e) = writeln!(io::stderr(), "{message}") {
        panic!("stderr accepts the report {message:?}: {e}");
    }
}

/// Open the database `args` names and run its text, its script or its session. Returns
/// whether every text succeeded, or why the shell could not run.
fn shell(args: &Args) -> Result<bool, String> {
    let file = &args.file;
    let log = log(&args.log)?;
    let options = OpenOptions {
        create: args.create.then(CreateOptions::default),
        ..OpenOptions::default()
    };
    let db = adtb::open(file, &log, &options).map_err(|e| format!("{}: {e}", file.display()))?;
    let interactive = args.text.is_none() && io::stdin().is_terminal();
    let mut session = Session::new(&db, io::stdout(), io::stderr());
    let ran = match &args.text {
        Some(text) => session.run(text),
        None => read(&mut session, io::stdin().lock(), interactive),
    };
    let ran = ran.and_then(|()| session.finish());
    let ok = !session.failed() || interactive;
    drop(session);
    ran.map_err(|e| e.to_string())?;
    db.close().map_err(|e| e.to_string())?;
    Ok(ok)
}

/// Returns the log of the migration files at `paths`, the first of which may be a schema file.
fn log(paths: &[PathBuf]) -> Result<Log, String> {
    let mut texts = Vec::with_capacity(paths.len());
    for (i, path) in paths.iter().enumerate() {
        let in_file = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
        let text = std::fs::read_to_string(path).map_err(|e| in_file(&e))?;
        texts.push(match i {
            0 => schema::first_migration(&text).map_err(|e| in_file(&e))?,
            _ => text,
        });
    }
    Ok(Log::new(texts))
}

/// Read `input` a line at a time, running each text and each command, prompting on stdout
/// when `interactive`.
fn read<O: Write, E: Write>(
    session: &mut Session<'_, O, E>,
    input: impl BufRead,
    interactive: bool,
) -> io::Result<()> {
    let mut pending = String::new();
    let mut lines = input.lines().peekable();
    loop {
        if interactive {
            let prompt = if pending.is_empty() { PROMPT } else { CONTINUE };
            write!(session.out(), "{prompt}")?;
            session.out().flush()?;
        }
        let Some(line) = lines.next().transpose()? else {
            break;
        };
        if let Some(c) = Command::of(&line) {
            flush(session, &mut pending)?;
            if !session.command(c)? {
                return Ok(());
            }
        } else if line.trim().is_empty() {
            flush(session, &mut pending)?;
        } else {
            pending.push_str(&line);
            pending.push('\n');
            let ends = if interactive {
                text::shape(&pending) == Shape::Whole
            } else {
                ends_before(&pending, lines.peek().and_then(|l| l.as_deref().ok()))
            };
            if ends {
                flush(session, &mut pending)?;
            }
        }
    }
    if interactive {
        writeln!(session.out())?;
    }
    flush(session, &mut pending)
}

/// Returns whether script text `pending` ends before line `next`: an indented line continues
/// a text the parser accepts, and any line but a blank one or a command continues one it
/// wants more of.
fn ends_before(pending: &str, next: Option<&str>) -> bool {
    let Some(next) = next else {
        return true;
    };
    if next.trim().is_empty() || Command::of(next).is_some() {
        return true;
    }
    match text::shape(pending) {
        Shape::Empty | Shape::Unfinished => false,
        Shape::Whole | Shape::Block => !next.starts_with([' ', '\t']),
    }
}

/// Run the text `pending` holds, unless it has no token, and empty it.
fn flush<O: Write, E: Write>(
    session: &mut Session<'_, O, E>,
    pending: &mut String,
) -> io::Result<()> {
    let text = std::mem::take(pending);
    if text::shape(&text) == Shape::Empty {
        return Ok(());
    }
    session.run(&text)
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(line: &str) -> Args {
        Args::try_parse_from(line.split(' ')).unwrap_or_else(|e| panic!("{line}: {e}"))
    }

    #[test]
    fn the_command_is_well_formed() {
        Args::command().debug_assert();
    }

    #[test]
    fn the_manual_invocations_parse() {
        let created = parse("adtb --create --schema shelter.adt shelter.db");
        assert!(created.create);
        assert_eq!(PathBuf::from("shelter.db"), created.file);
        assert_eq!(vec![PathBuf::from("shelter.adt")], created.log);
        assert_eq!(None, created.text);
        let migrated = parse("adtb --schema crm.adt --schema plan.adt crm.db -c 1");
        assert!(!migrated.create);
        assert_eq!(
            vec![PathBuf::from("crm.adt"), PathBuf::from("plan.adt")],
            migrated.log
        );
        assert_eq!(Some("1".to_owned()), migrated.text);
        assert_eq!(PathBuf::from("app.adt"), parse("adtb app.adt").file);
    }

    #[test]
    fn flags_may_follow_the_file() {
        let args = parse("adtb shelter.db --schema shelter.adt -c 1 --create");
        assert!(args.create);
        assert_eq!(vec![PathBuf::from("shelter.adt")], args.log);
    }

    #[test]
    fn the_last_text_wins() {
        assert_eq!(Some("2".to_owned()), parse("adtb f -c 1 -c 2").text);
    }

    #[test]
    fn a_text_may_start_with_a_hyphen() {
        assert_eq!(Some("-1".to_owned()), parse("adtb f -c -1").text);
    }

    #[test]
    fn usage_errors_exit_with_two() {
        for line in [
            "adtb",
            "adtb a b",
            "adtb --nope f",
            "adtb f -c",
            "adtb f --schema",
        ] {
            let e = Args::try_parse_from(line.split(' ')).expect_err(line);
            assert_eq!(2, e.exit_code(), "{line}");
        }
    }
}
