//! The nested `AGENTS.md` files: each beside a `CLAUDE.md` holding only `@AGENTS.md`, with no em
//! dash or bold, and naming only what the tree holds.
//!
//! A backticked span is one of three things:
//!
//! - a path: it holds a `/` and is read in the file's directory, else from the repository's root,
//!   or it is a bare file name with an extension and is read in the file's directory;
//! - a cargo command: it starts with `cargo `, and its packages, test targets, profiles, file
//!   names and filter names are checked;
//! - a name: anything else without whitespace. Each identifier in it must occur as a whole word
//!   in a file under the `AGENTS.md`'s directory or under a path the file names, or name a file
//!   or directory there. A filter's regex names a prefix.
//!
//! A Markdown link's target is a path read from the file's directory.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::{read, repo};

const EXTENSIONS: [&str; 7] = ["rs", "md", "toml", "tql", "test", "fixture", "differs"];

/// Returns every `AGENTS.md` under `dir` but the root's, skipping hidden and build directories.
fn nested(dir: &Path, root: &Path, found: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let name = name.unwrap_or_default();
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                nested(&path, root, found);
            }
        } else if name == "AGENTS.md" && dir != root {
            found.push(path);
        }
    }
}

/// Returns the text of every file under `path`, a file or a directory, but the agent files, and
/// the name of each file and directory there, a file's without its extension.
fn texts(path: &Path, out: &mut Vec<String>) {
    if let Some(stem) = path.file_stem() {
        out.push(stem.to_string_lossy().into_owned());
    }
    if path.is_dir() {
        let entries = fs::read_dir(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        for entry in entries {
            texts(
                &entry
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                    .path(),
                out,
            );
        }
    } else if !path.ends_with("AGENTS.md") && !path.ends_with("CLAUDE.md") {
        let bytes = fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if let Ok(text) = String::from_utf8(bytes) {
            out.push(text);
        }
    }
}

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Returns whether `word` occurs in `text` with no word character either side; a trailing `*`
/// matches any continuation.
fn occurs(text: &str, word: &str) -> bool {
    let (word, prefix) = match word.strip_suffix('*') {
        Some(w) => (w, true),
        None => (word, false),
    };
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back().is_none_or(|c| !word_char(c));
        let after = text[at + word.len()..].chars().next();
        before && (prefix || after.is_none_or(|c| !word_char(c)))
    })
}

/// The things one `AGENTS.md` names.
#[derive(Default)]
struct Named {
    paths: Vec<(String, PathBuf)>,
    words: Vec<String>,
    packages: Vec<String>,
    targets: Vec<String>,
    profiles: Vec<String>,
    unread: Vec<String>,
}

impl Named {
    fn of(root: &Path, dir: &Path, text: &str) -> Named {
        let mut named = Named::default();
        for span in text.split('`').skip(1).step_by(2) {
            if span.starts_with("cargo ") {
                named.command(root, span);
            } else if span.contains(char::is_whitespace) {
                named.unread.push(span.to_owned());
            } else if span.contains('/') {
                let span = span.trim_end_matches('/');
                let local = dir.join(span);
                named.path(
                    span,
                    if local.exists() {
                        local
                    } else {
                        root.join(span)
                    },
                );
            } else if Path::new(span)
                .extension()
                .is_some_and(|x| EXTENSIONS.iter().any(|e| x == *e))
            {
                named.path(span, dir.join(span));
            } else {
                named.identifiers(span);
            }
        }
        for target in text.split("](").skip(1).filter_map(|t| t.split_once(')')) {
            let file = target.0.split('#').next().unwrap_or_default();
            if !file.contains("://") {
                named.path(target.0, dir.join(file));
            }
        }
        named
    }

    fn path(&mut self, shown: &str, path: PathBuf) {
        self.paths.push((shown.to_owned(), path));
    }

    fn identifiers(&mut self, span: &str) {
        let words = span.split(|c: char| !word_char(c) && c != '*');
        self.words
            .extend(words.filter(|w| !w.is_empty()).map(str::to_owned));
    }

    fn command(&mut self, root: &Path, span: &str) {
        let words: Vec<&str> = span
            .split_whitespace()
            .map(|w| w.trim_matches('\''))
            .collect();
        for pair in words.windows(2) {
            match pair[0] {
                "-p" | "--package" => self.packages.push(pair[1].to_owned()),
                "--test" => self.targets.push(pair[1].to_owned()),
                "--profile" => self.profiles.push(pair[1].to_owned()),
                "--file-name" => self.path(pair[1], root.join(pair[1].trim_end_matches('/'))),
                _ => {}
            }
        }
        for (term, into) in [
            ("binary(=", 0),
            ("package(=", 1),
            ("test(=", 2),
            ("test(/", 2),
        ] {
            for (at, _) in span.match_indices(term) {
                let rest = &span[at + term.len()..];
                let name = rest.split([')', '/']).next().unwrap_or_default();
                let name = name.trim_start_matches('^');
                match into {
                    0 => self.targets.push(name.to_owned()),
                    1 => self.packages.push(name.to_owned()),
                    _ if term == "test(/" && !name.ends_with(':') => {
                        self.identifiers(&format!("{name}*"));
                    }
                    _ => self.identifiers(name.trim_end_matches(':')),
                }
            }
        }
    }
}

/// Returns whether some crate has a test target `name`.
fn test_target(root: &Path, name: &str) -> bool {
    let crates = root.join("crates");
    let entries = fs::read_dir(&crates).unwrap_or_else(|e| panic!("{}: {e}", crates.display()));
    entries.flatten().any(|entry| {
        let tests = entry.path().join("tests");
        tests.join(format!("{name}.rs")).is_file() || tests.join(name).join("main.rs").is_file()
    })
}

/// Returns each claim of the `AGENTS.md` at `file` the tree does not bear out, and how many
/// claims were checked.
fn stale(root: &Path, file: &Path) -> (Vec<String>, usize) {
    let dir = file.parent().expect("a file lies in a directory");
    let text = read(file);
    let shown = file
        .strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string();
    let named = Named::of(root, dir, &text);
    let mut out = Vec::new();
    let mut fail = |what: String| out.push(format!("{shown}: {what}"));
    if text.contains('\u{2014}') {
        fail("holds an em dash".to_owned());
    }
    if text.split('`').step_by(2).any(|prose| prose.contains("**")) {
        fail("holds bold".to_owned());
    }
    let claude = dir.join("CLAUDE.md");
    if fs::read_to_string(&claude).ok().as_deref() != Some("@AGENTS.md\n") {
        fail("has no CLAUDE.md beside it holding only `@AGENTS.md`".to_owned());
    }
    for span in &named.unread {
        fail(format!(
            "`{span}` is neither a path, a cargo command nor a name"
        ));
    }
    let mut scope = Vec::new();
    texts(dir, &mut scope);
    for (shown, path) in &named.paths {
        if path.exists() {
            texts(path, &mut scope);
        } else {
            fail(format!("names `{shown}`, which the tree does not hold"));
        }
    }
    let words: BTreeSet<&String> = named.words.iter().collect();
    for word in &words {
        if !scope.iter().any(|t| occurs(t, word)) {
            fail(format!(
                "names `{word}`, which no file under its directory or a path it names holds"
            ));
        }
    }
    for package in &named.packages {
        let manifest = |at: PathBuf| at.join(package).join("Cargo.toml").is_file();
        if !manifest(root.join("crates")) && !manifest(root.to_path_buf()) {
            fail(format!(
                "names package `{package}`, which the workspace lacks"
            ));
        }
    }
    for target in &named.targets {
        if !test_target(root, target) {
            fail(format!("names test target `{target}`, which no crate has"));
        }
    }
    let nextest = read(&root.join(".config/nextest.toml"));
    for profile in &named.profiles {
        if !nextest.contains(&format!("[profile.{profile}]")) {
            fail(format!(
                "names nextest profile `{profile}`, which is not declared"
            ));
        }
    }
    let checked = named.paths.len()
        + words.len()
        + named.packages.len()
        + named.targets.len()
        + named.profiles.len();
    (out, checked)
}

#[test]
fn every_nested_agents_file_names_only_what_the_tree_holds() {
    let root = repo();
    let root = root.canonicalize().unwrap_or(root);
    let mut files = Vec::new();
    nested(&root, &root, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no nested AGENTS.md under {}",
        root.display()
    );
    let mut problems = Vec::new();
    for file in &files {
        let (found, checked) = stale(&root, file);
        assert!(checked > 0, "{} names nothing checked", file.display());
        problems.extend(found);
    }
    assert_eq!(problems, Vec::<String>::new());
}
