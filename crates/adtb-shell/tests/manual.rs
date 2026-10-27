//! The shell sessions of the user manual, run in order against the `adtb` binary.
//!
//! A fenced block whose info string is a language and a path, such as `adt NAME` or `csv
//! dir/NAME`, is a file the page asks the reader to save at that path; `adt fragment` is an
//! excerpt, not a file. A `console` block holds `$ adtb ...` commands, each followed by the output it prints
//! on stdout and stderr, interleaved as printed. A line `<!-- exit N... -->` just before a
//! `console` block lists the status each of its sessions exits with; without one, each exits 0.
//! A command without `-c` reads the lines after it that start with the prompt or the
//! continuation prompt as its input; a bare continuation prompt is a blank line. A command
//! ending in `< FILE` reads that file instead.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

const PROMPT: &str = "adtb> ";
const CONTINUE: &str = "  ... ";

enum Block {
    File { name: String, text: String },
    Console { lines: Vec<String>, exits: Vec<i32> },
}

/// Returns the file and `console` blocks of markdown `page`, in order.
fn blocks(page: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut lines = page.lines();
    let mut before = "";
    while let Some(line) = lines.next() {
        let Some(info) = line.strip_prefix("```") else {
            before = line;
            continue;
        };
        let body: Vec<String> = lines
            .by_ref()
            .take_while(|l| !l.starts_with("```"))
            .map(str::to_owned)
            .collect();
        match info.split_once(' ') {
            Some((_, name)) if name != "fragment" => out.push(Block::File {
                name: name.to_owned(),
                text: body.join("\n") + "\n",
            }),
            None if info == "console" => out.push(Block::Console {
                lines: body,
                exits: exits(before),
            }),
            _ => {}
        }
        before = "";
    }
    out
}

/// Returns the statuses a `<!-- exit N... -->` line lists, or none for any other line.
fn exits(line: &str) -> Vec<i32> {
    line.strip_prefix("<!-- exit ")
        .and_then(|l| l.strip_suffix(" -->"))
        .map(|codes| {
            codes
                .split(' ')
                .map(|c| c.parse().unwrap_or_else(|e| panic!("{line}: {c}: {e}")))
                .collect()
        })
        .unwrap_or_default()
}

/// A command of a console block, the input it reads and the output the page shows for it.
struct Session {
    command: String,
    input: String,
    shown: Vec<String>,
}

/// Returns the sessions of console block `lines`. A command whose quote is open at the end of
/// a line runs on to the next.
fn sessions(lines: &[String]) -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    let mut lines = lines.iter();
    while let Some(line) = lines.next() {
        if let Some(command) = line.strip_prefix("$ ") {
            let mut command = command.to_owned();
            while words(&command).is_none() {
                let more = lines.next().expect("a quoted argument closes in its block");
                command.push('\n');
                command.push_str(more);
            }
            out.push(Session {
                command,
                input: String::new(),
                shown: Vec::new(),
            });
            continue;
        }
        let session = out
            .last_mut()
            .expect("a console block starts with a command");
        let input = match line.strip_prefix(PROMPT) {
            None if line == CONTINUE.trim_end() => Some(""),
            None => line.strip_prefix(CONTINUE),
            prompted => prompted,
        };
        match input {
            Some(input) => {
                session.input.push_str(input);
                session.input.push('\n');
            }
            None => session.shown.push(line.clone()),
        }
    }
    out
}

/// Returns the words of `command` as a POSIX shell splits them, for quotes without escapes,
/// or `None` while a quote is open.
fn words(command: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut word: Option<String> = None;
    let mut quote = None;
    for c in command.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            (None, c) if c.is_whitespace() => out.extend(word.take()),
            (_, c) => word.get_or_insert_default().push(c),
        }
    }
    if quote.is_some() {
        return None;
    }
    out.extend(word);
    Some(out)
}

/// What a session printed on stdout and stderr, in the order printed, and how it exited.
struct Printed {
    lines: Vec<String>,
    status: ExitStatus,
}

/// Run `session` in `dir`.
fn printed(dir: &Path, session: &Session) -> Printed {
    let mut words = words(&session.command).expect("a whole command");
    let input = match words.as_slice() {
        [.., lt, file] if lt == "<" => {
            let path = dir.join(file);
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            words.truncate(words.len() - 2);
            text
        }
        _ => session.input.clone(),
    };
    assert_eq!(
        Some("adtb"),
        words.first().map(String::as_str),
        "{}",
        session.command
    );
    let (mut printed, out) = std::io::pipe().expect("a pipe for the shell's output");
    let err = out.try_clone().expect("a second end for stderr");
    let mut child = Command::new(env!("CARGO_BIN_EXE_adtb"))
        .args(&words[1..])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(out)
        .stderr(err)
        .spawn()
        .expect("the shell starts");
    let mut stdin = child.stdin.take().expect("a piped stdin");
    let mut text = String::new();
    std::thread::scope(|s| {
        let writer = s.spawn(move || stdin.write_all(input.as_bytes()));
        printed
            .read_to_string(&mut text)
            .expect("the shell prints UTF-8");
        writer
            .join()
            .expect("the input writer does not panic")
            .expect("the shell reads its stdin");
    });
    let status = child.wait().expect("the shell exits");
    Printed {
        lines: text.lines().map(str::to_owned).collect(),
        status,
    }
}

/// Runs every session of `page`, a path under `docs/manual`, and returns what each that prints
/// other than the page shows printed. Returns `None` when the page shows no session.
fn check(manual: &Path, page: &Path) -> Option<Vec<String>> {
    let text =
        std::fs::read_to_string(manual.join(page)).unwrap_or_else(|e| panic!("{page:?}: {e}"));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("manual")
        .join(page);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("clear {dir:?}: {e}"),
        _ => {}
    }
    std::fs::create_dir_all(&dir).unwrap();
    let mut wrong = Vec::new();
    let mut ran = 0;
    for block in blocks(&text) {
        match block {
            Block::File { name, text } => {
                let path = dir.join(name);
                std::fs::create_dir_all(path.parent().expect("a file's directory")).unwrap();
                std::fs::write(path, text).unwrap();
            }
            Block::Console { lines, exits } => {
                let sessions = sessions(&lines);
                if !exits.is_empty() && exits.len() != sessions.len() {
                    wrong.push(format!(
                        "{}: {} statuses for the {} sessions of the block at `{}`",
                        page.display(),
                        exits.len(),
                        sessions.len(),
                        lines[0]
                    ));
                }
                for (i, session) in sessions.iter().enumerate() {
                    ran += 1;
                    let got = printed(&dir, session);
                    let status = got.status.code();
                    let expected = exits.get(i).copied().unwrap_or(0);
                    if got.lines != session.shown || status != Some(expected) {
                        wrong.push(format!(
                            "{}: $ {}\nshown, exit {expected}:\n{}\nprinted, {status:?}:\n{}",
                            page.display(),
                            session.command,
                            session.shown.join("\n"),
                            got.lines.join("\n")
                        ));
                    }
                }
            }
        }
    }
    (ran > 0).then_some(wrong)
}

/// Returns the markdown pages under `dir`, as paths relative to `root`, sorted.
fn pages(root: &Path, dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{dir:?}: {e}")) {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            out.extend(pages(root, &path));
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(
                path.strip_prefix(root)
                    .expect("under the root")
                    .to_path_buf(),
            );
        }
    }
    out.sort();
    out
}

#[test]
fn every_page_prints_what_it_shows() {
    let manual = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/manual");
    let pages = pages(&manual, &manual);
    assert!(!pages.is_empty(), "the manual has pages");
    let checked: Vec<(PathBuf, Option<Vec<String>>)> = std::thread::scope(|s| {
        let runs: Vec<_> = pages
            .iter()
            .map(|page| (page, s.spawn(|| check(&manual, page))))
            .collect();
        runs.into_iter()
            .map(|(page, run)| {
                (
                    page.clone(),
                    run.join().expect("a page's check does not panic"),
                )
            })
            .collect()
    });
    let shows: Vec<&PathBuf> = checked
        .iter()
        .filter(|(_, c)| c.is_some())
        .map(|(p, _)| p)
        .collect();
    assert!(
        shows.contains(&&PathBuf::from("getting-started.md")),
        "the walk finds the pages: {shows:?}"
    );
    let wrong: Vec<String> = checked
        .into_iter()
        .flat_map(|(_, c)| c.unwrap_or_default())
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

#[test]
fn a_quoted_argument_is_one_word() {
    assert_eq!(
        Some(vec!["adtb".into(), "-c".into(), "a 'b' c".into()]),
        words(r#"adtb -c "a 'b' c""#)
    );
    assert_eq!(None, words("adtb -c 'from p"));
}

#[test]
fn continuation_prompts_are_input_and_the_rest_is_output() {
    let lines: Vec<String> = [
        "$ adtb db",
        "adtb> transact",
        "  ...   skip",
        "  ...",
        "done",
    ]
    .map(str::to_owned)
    .into();
    let [session] = sessions(&lines).try_into().ok().expect("one session");
    assert_eq!("transact\n  skip\n\n", session.input);
    assert_eq!(vec!["done".to_owned()], session.shown);
}
