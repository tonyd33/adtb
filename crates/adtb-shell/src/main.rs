mod schema;
mod session;
mod text;

use std::ffi::OsString;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use adtb::{CreateOptions, Log, OpenOptions};

use session::{Command, Session};
use text::Shape;

const USAGE: &str = "usage: adtb [--create] [--schema FILE]... FILE [-c TEXT]";

const HELP: &str = "usage: adtb [--create] [--schema FILE]... FILE [-c TEXT]

Open the database FILE and run ad hoc text against it: TEXT when -c
gives one, else the texts read from stdin, with a prompt on a terminal.

  --create         create FILE when it does not exist
  --schema FILE    the next migration of the database's log; the first
                   may instead be a schema file of declarations
  -c TEXT          run TEXT and exit
  -h, --help       print this help

At the prompt, \\help lists the shell's commands.";

const PROMPT: &str = "adtb> ";
const CONTINUE: &str = "  ... ";

#[derive(Debug)]
struct Args {
    file: PathBuf,
    create: bool,
    help: bool,
    log: Vec<PathBuf>,
    text: Option<String>,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Args, String> {
        let (mut file, mut create, mut log, mut text) = (None, false, Vec::new(), None);
        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("-h" | "--help") => {
                    return Ok(Args {
                        file: PathBuf::new(),
                        create: false,
                        help: true,
                        log: Vec::new(),
                        text: None,
                    });
                }
                Some("--create") => create = true,
                Some("--schema") => log.push(args.next().ok_or("--schema needs a file")?.into()),
                Some("-c") => {
                    let t = args.next().ok_or("-c needs a text")?;
                    text = Some(
                        t.into_string()
                            .map_err(|t| format!("-c text {} is not UTF-8", t.display()))?,
                    );
                }
                Some(flag) if flag.starts_with('-') => return Err(format!("unknown flag {flag}")),
                _ if file.is_some() => return Err("more than one FILE".into()),
                _ => file = Some(arg),
            }
        }
        Ok(Args {
            file: file.ok_or("no FILE")?.into(),
            create,
            help: false,
            log,
            text,
        })
    }
}

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(why) => {
            report(&format!("{why}\n{USAGE}"));
            return ExitCode::from(2);
        }
    };
    if args.help {
        return match writeln!(io::stdout(), "{HELP}") {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        };
    }
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
