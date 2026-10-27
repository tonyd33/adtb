//! The shell sessions of the user manual, run in order against the `adtb` binary.
//!
//! A fenced block whose info string is `adt NAME` is a file the page asks the reader to save
//! as `NAME`. A `console` block holds `$ adtb ...` commands, each followed by the output it
//! prints. A command without `-c` reads the lines after it that start with the prompt as its
//! input.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PROMPT: &str = "adtb> ";

enum Block {
    File { name: String, text: String },
    Console(Vec<String>),
}

/// Returns the `adt NAME` and `console` blocks of markdown `page`, in order.
fn blocks(page: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut lines = page.lines();
    while let Some(line) = lines.next() {
        let Some(info) = line.strip_prefix("```") else {
            continue;
        };
        let body: Vec<String> = lines
            .by_ref()
            .take_while(|l| !l.starts_with("```"))
            .map(str::to_owned)
            .collect();
        match info.split_once(' ') {
            Some(("adt", name)) => out.push(Block::File {
                name: name.to_owned(),
                text: body.join("\n") + "\n",
            }),
            None if info == "console" => out.push(Block::Console(body)),
            _ => {}
        }
    }
    out
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
        match line.strip_prefix(PROMPT) {
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

/// Run `session` in `dir`. Returns the lines it prints on stdout, then on stderr.
fn printed(dir: &Path, session: &Session) -> Vec<String> {
    let words = words(&session.command).expect("a whole command");
    assert_eq!(
        Some("adtb"),
        words.first().map(String::as_str),
        "{}",
        session.command
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_adtb"))
        .args(&words[1..])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the shell starts");
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(session.input.as_bytes())
        .expect("the shell reads its stdin");
    let out = child.wait_with_output().expect("the shell exits");
    [out.stdout, out.stderr]
        .into_iter()
        .flat_map(|bytes| {
            let text = String::from_utf8(bytes).expect("the shell prints UTF-8");
            text.lines().map(str::to_owned).collect::<Vec<_>>()
        })
        .collect()
}

fn check(page: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/manual")
        .join(page);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
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
            Block::File { name, text } => std::fs::write(dir.join(name), text).unwrap(),
            Block::Console(lines) => {
                for session in sessions(&lines) {
                    ran += 1;
                    let got = printed(&dir, &session);
                    if got != session.shown {
                        wrong.push(format!(
                            "$ {}\nshown:\n{}\nprinted:\n{}",
                            session.command,
                            session.shown.join("\n"),
                            got.join("\n")
                        ));
                    }
                }
            }
        }
    }
    assert!(ran > 0, "{page} shows no session");
    assert!(wrong.is_empty(), "{page}:\n\n{}", wrong.join("\n\n"));
}

#[test]
fn getting_started_prints_what_it_shows() {
    check("getting-started.md");
}

#[test]
fn a_quoted_argument_is_one_word() {
    assert_eq!(
        Some(vec!["adtb".into(), "-c".into(), "a 'b' c".into()]),
        words(r#"adtb -c "a 'b' c""#)
    );
    assert_eq!(None, words("adtb -c 'from p"));
}
