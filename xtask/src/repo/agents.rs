//! The nested `AGENTS.md` files: each beside a `CLAUDE.md` holding only `@AGENTS.md`, with no em
//! dash or bold, and naming only what the tracked tree holds.
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
use std::path::{Path, PathBuf};

use super::{Tree, join};

const EXTENSIONS: [&str; 7] = ["rs", "md", "toml", "tql", "test", "fixture", "differs"];

/// Returns each claim of a nested `AGENTS.md` the tracked tree does not bear out.
pub fn problems(tree: &Tree) -> Result<Vec<String>, String> {
    let files: Vec<&Path> = tree
        .files
        .iter()
        .filter(|file| file.ends_with("AGENTS.md") && file.parent() != Some(Path::new("")))
        .map(PathBuf::as_path)
        .collect();
    if files.is_empty() {
        return Ok(vec!["no nested AGENTS.md is tracked".to_owned()]);
    }
    let mut problems = Vec::new();
    for file in files {
        let (found, checked) = stale(tree, file)?;
        if checked == 0 {
            problems.push(format!("{} names nothing checked", file.display()));
        }
        problems.extend(found);
    }
    Ok(problems)
}

/// Push the name of `path` and of each tracked file and directory under it, a file's without its
/// extension, and the text of each tracked file there but the agent files.
fn texts(tree: &Tree, path: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let under: Vec<&Path> = if tree.is_dir(path) {
        tree.under(path)
    } else {
        Vec::new()
    };
    for path in std::iter::once(path).chain(under) {
        if let Some(stem) = path.file_stem() {
            out.push(stem.to_string_lossy().into_owned());
        }
        if tree.is_file(path)
            && !path.ends_with("AGENTS.md")
            && !path.ends_with("CLAUDE.md")
            && let Ok(text) = String::from_utf8(tree.bytes(path)?)
        {
            out.push(text);
        }
    }
    Ok(())
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

/// The things one `AGENTS.md` names. A path is `None` when it leaves the checkout.
#[derive(Default)]
struct Named {
    paths: Vec<(String, Option<PathBuf>)>,
    words: Vec<String>,
    packages: Vec<String>,
    targets: Vec<String>,
    profiles: Vec<String>,
    unread: Vec<String>,
}

impl Named {
    fn of(tree: &Tree, dir: &Path, text: &str) -> Named {
        let mut named = Named::default();
        for span in text.split('`').skip(1).step_by(2) {
            if span.starts_with("cargo ") {
                named.command(span);
            } else if span.contains(char::is_whitespace) {
                named.unread.push(span.to_owned());
            } else if span.contains('/') {
                let span = span.trim_end_matches('/');
                let local = join(dir, span).filter(|local| tree.holds(local));
                named.path(span, local.or_else(|| join(Path::new(""), span)));
            } else if Path::new(span)
                .extension()
                .is_some_and(|x| EXTENSIONS.iter().any(|e| x == *e))
            {
                named.path(span, join(dir, span));
            } else {
                named.identifiers(span);
            }
        }
        for target in text.split("](").skip(1).filter_map(|t| t.split_once(')')) {
            let file = target.0.split('#').next().unwrap_or_default();
            if !file.contains("://") {
                named.path(target.0, join(dir, file));
            }
        }
        named
    }

    fn path(&mut self, shown: &str, path: Option<PathBuf>) {
        self.paths.push((shown.to_owned(), path));
    }

    fn identifiers(&mut self, span: &str) {
        let words = span.split(|c: char| !word_char(c) && c != '*');
        self.words
            .extend(words.filter(|w| !w.is_empty()).map(str::to_owned));
    }

    fn command(&mut self, span: &str) {
        let words: Vec<&str> = span
            .split_whitespace()
            .map(|w| w.trim_matches('\''))
            .collect();
        for pair in words.windows(2) {
            match pair[0] {
                "-p" | "--package" => self.packages.push(pair[1].to_owned()),
                "--test" => self.targets.push(pair[1].to_owned()),
                "--profile" => self.profiles.push(pair[1].to_owned()),
                "--file-name" => {
                    let path = join(Path::new(""), pair[1].trim_end_matches('/'));
                    self.path(pair[1], path);
                }
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

/// Returns whether some crate has a tracked test target `name`.
fn test_target(tree: &Tree, name: &str) -> bool {
    let single = format!("{name}.rs");
    tree.files.iter().any(|file| {
        let parts: Vec<&str> = file.iter().filter_map(|part| part.to_str()).collect();
        match parts.as_slice() {
            ["crates", _, "tests", file] => *file == single,
            ["crates", _, "tests", dir, "main.rs"] => *dir == name,
            _ => false,
        }
    })
}

/// Returns each claim of the `AGENTS.md` at `file` the tracked tree does not bear out, and how
/// many claims were checked.
fn stale(tree: &Tree, file: &Path) -> Result<(Vec<String>, usize), String> {
    let dir = file.parent().unwrap_or(Path::new(""));
    let text = tree.read(file)?;
    let shown = file.display().to_string();
    let named = Named::of(tree, dir, &text);
    let mut out = Vec::new();
    let mut fail = |what: String| out.push(format!("{shown}: {what}"));
    if text.contains('\u{2014}') {
        fail("holds an em dash".to_owned());
    }
    if text.split('`').step_by(2).any(|prose| prose.contains("**")) {
        fail("holds bold".to_owned());
    }
    let claude = dir.join("CLAUDE.md");
    if !tree.is_file(&claude) || tree.read(&claude)? != "@AGENTS.md\n" {
        fail("has no CLAUDE.md beside it holding only `@AGENTS.md`".to_owned());
    }
    for span in &named.unread {
        fail(format!(
            "`{span}` is neither a path, a cargo command nor a name"
        ));
    }
    let mut scope = Vec::new();
    texts(tree, dir, &mut scope)?;
    for (shown, path) in &named.paths {
        match path.as_deref().filter(|path| tree.holds(path)) {
            Some(path) => texts(tree, path, &mut scope)?,
            None => fail(format!("names `{shown}`, which the tree does not hold")),
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
        let manifest = |at: &Path| tree.is_file(&at.join(package).join("Cargo.toml"));
        if !manifest(Path::new("crates")) && !manifest(Path::new("")) {
            fail(format!(
                "names package `{package}`, which the workspace lacks"
            ));
        }
    }
    for target in &named.targets {
        if !test_target(tree, target) {
            fail(format!("names test target `{target}`, which no crate has"));
        }
    }
    let nextest = tree.read(Path::new(".config/nextest.toml"))?;
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
    Ok((out, checked))
}
