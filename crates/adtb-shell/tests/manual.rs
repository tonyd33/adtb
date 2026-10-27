//! The shell sessions of the user manual, run in order against the `adtb` binary.
//!
//! A fenced block whose info string is a language and a name, such as `adt NAME` or `csv NAME`,
//! is a file the page asks the reader to save as `NAME`; `adt fragment` is an excerpt, not a
//! file. A `console` block holds `$ adtb ...` commands, each followed by the output it prints
//! on stdout and stderr, interleaved as printed.
//! A command without `-c` reads the lines after it that start with the prompt or the
//! continuation prompt as its input; a bare continuation prompt is a blank line. A command
//! ending in `< FILE` reads that file instead.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PROMPT: &str = "adtb> ";
const CONTINUE: &str = "  ... ";

enum Block {
    File { name: String, text: String },
    Console(Vec<String>),
}

/// Returns the file and `console` blocks of markdown `page`, in order.
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
            Some((_, name)) if name != "fragment" => out.push(Block::File {
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

/// Run `session` in `dir`. Returns the lines it prints on stdout and stderr, in the order printed.
fn printed(dir: &Path, session: &Session) -> Vec<String> {
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
    child
        .stdin
        .take()
        .expect("a piped stdin")
        .write_all(input.as_bytes())
        .expect("the shell reads its stdin");
    let mut text = String::new();
    printed
        .read_to_string(&mut text)
        .expect("the shell prints UTF-8");
    child.wait().expect("the shell exits");
    text.lines().map(str::to_owned).collect()
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
fn tutorial_schema_prints_what_it_shows() {
    check("tutorial/1-schema.md");
}

#[test]
fn tutorial_sums_prints_what_it_shows() {
    check("tutorial/2-sums.md");
}

#[test]
fn tutorial_queries_prints_what_it_shows() {
    check("tutorial/3-queries.md");
}

#[test]
fn tutorial_match_prints_what_it_shows() {
    check("tutorial/4-match.md");
}

#[test]
fn tutorial_references_and_constraints_prints_what_it_shows() {
    check("tutorial/5-references-and-constraints.md");
}

#[test]
fn tutorial_transactions_prints_what_it_shows() {
    check("tutorial/6-transactions.md");
}

#[test]
fn tutorial_migrations_prints_what_it_shows() {
    check("tutorial/7-migrations.md");
}

#[test]
fn guide_modelling_with_sums_prints_what_it_shows() {
    check("guides/modelling-with-sums.md");
}

#[test]
fn guide_evolving_a_schema_prints_what_it_shows() {
    check("guides/evolving-a-schema.md");
}

#[test]
fn guide_writing_constraints_prints_what_it_shows() {
    check("guides/writing-constraints.md");
}

#[test]
fn guide_files_and_csv_prints_what_it_shows() {
    check("guides/files-and-csv.md");
}

#[test]
fn concept_relations_as_sums_prints_what_it_shows() {
    check("concepts/relations-as-sums.md");
}

#[test]
fn concept_no_null_prints_what_it_shows() {
    check("concepts/no-null.md");
}

#[test]
fn concept_keys_and_references_prints_what_it_shows() {
    check("concepts/keys-and-references.md");
}

#[test]
fn concept_serializable_transactions_prints_what_it_shows() {
    check("concepts/serializable-transactions.md");
}

#[test]
fn concept_migration_log_prints_what_it_shows() {
    check("concepts/migration-log.md");
}

#[test]
fn reference_shell_prints_what_it_shows() {
    check("reference/shell.md");
}

#[test]
fn reference_language_prints_what_it_shows() {
    check("reference/language.md");
}

#[test]
fn reference_prelude_prints_what_it_shows() {
    check("reference/prelude.md");
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
