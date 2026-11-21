//! The reader of conformance files and engine scripts.
//!
//! A script is a sequence of directives. A directive line starts in the first column; its body
//! is the lines below it indented by two spaces. The rows of a `query` follow a `----` line, one
//! per line, unindented, and end at a blank line, a comment or the next directive. Lines starting
//! with `#` between directives are comments.

#![forbid(unsafe_code)]

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Schema,
    Run,
    Query {
        rowsort: bool,
        list: bool,
        rows: Vec<String>,
    },
    Error {
        reason: String,
    },
}

/// A directive of a conformance file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    line: usize,
    kind: Kind,
    body: String,
}

impl Directive {
    pub fn new(line: usize, kind: Kind, body: String) -> Directive {
        Directive { line, kind, body }
    }

    /// Returns the 1-based line of the directive in its script.
    pub fn line(&self) -> usize {
        self.line
    }

    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    pub fn body(&self) -> &str {
        &self.body
    }
}

/// The directives of a conformance file, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    directives: Vec<Directive>,
}

impl Script {
    pub fn new(directives: Vec<Directive>) -> Script {
        Script { directives }
    }

    pub fn directives(&self) -> &[Directive] {
        &self.directives
    }
}

/// A script the reader cannot read, at the 1-based line where it fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unread {
    line: usize,
    message: String,
}

impl Unread {
    fn at(line: usize, message: impl Into<String>) -> Unread {
        Unread {
            line,
            message: message.into(),
        }
    }

    pub fn line(&self) -> usize {
        self.line
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Unread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// A directive as the reader finds it, its first line not yet read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    line: usize,
    head: String,
    body: String,
    rows: Vec<String>,
    dashes: Option<usize>,
}

impl Block {
    /// Returns the 1-based line of the directive in its script.
    pub fn line(&self) -> usize {
        self.line
    }

    /// Returns the directive's line.
    pub fn head(&self) -> &str {
        &self.head
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    /// Returns the rows after `----`, read only for a head `takes_rows` accepts.
    pub fn rows(&self) -> &[String] {
        &self.rows
    }

    /// Returns the 1-based line of the `----` before the rows, for a head `takes_rows` accepts.
    pub fn dashes(&self) -> Option<usize> {
        self.dashes
    }
}

/// Returns the blocks of a script: each directive line, its indented body and, for a head
/// `takes_rows` accepts, the rows after `----`. Rows end at a blank line, a comment or a line
/// `starts` accepts as a directive.
pub fn blocks(
    text: &str,
    starts: impl Fn(&str) -> bool,
    takes_rows: impl Fn(&str) -> bool,
) -> Result<Vec<Block>, Unread> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let n = i + 1;
        i += 1;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with(' ') {
            return Err(Unread::at(n, "an indented line outside a directive's body"));
        }
        let mut body = String::new();
        while i < lines.len() && (lines[i].starts_with("  ") || lines[i].trim().is_empty()) {
            if !lines[i].trim().is_empty() || next_is_body(&lines[i..]) {
                body.push_str(lines[i].get(2..).unwrap_or_default());
                body.push('\n');
            }
            i += 1;
        }
        let mut rows = Vec::new();
        let mut dashes = None;
        if takes_rows(line) {
            if lines.get(i) != Some(&"----") {
                let word = line.split(' ').next().unwrap_or_default();
                return Err(Unread::at(
                    n,
                    format!("`{word}` takes its rows after `----`"),
                ));
            }
            i += 1;
            dashes = Some(i);
            while i < lines.len()
                && !lines[i].trim().is_empty()
                && !lines[i].starts_with('#')
                && !starts(lines[i])
            {
                rows.push(lines[i].to_string());
                i += 1;
            }
        }
        out.push(Block {
            line: n,
            head: line.to_owned(),
            body,
            rows,
            dashes,
        });
    }
    Ok(out)
}

/// Returns whether the blank line at the head of `lines` lies inside a body.
fn next_is_body(lines: &[&str]) -> bool {
    lines
        .iter()
        .find(|l| !l.trim().is_empty())
        .is_some_and(|l| l.starts_with("  "))
}

/// Returns the blocks of the conformance file `text`.
pub fn conformance_blocks(text: &str) -> Result<Vec<Block>, Unread> {
    let takes_rows = |line: &str| matches!(kind(line), Some(Kind::Query { .. }));
    blocks(text, |line| kind(line).is_some(), takes_rows)
}

/// Returns the script the conformance file `text` holds.
pub fn parse(text: &str) -> Result<Script, Unread> {
    let mut directives = Vec::new();
    for b in conformance_blocks(text)? {
        let kind = match kind(&b.head) {
            Some(Kind::Query { rowsort, list, .. }) => Kind::Query {
                rowsort,
                list,
                rows: b.rows,
            },
            Some(k) => k,
            None => {
                return Err(Unread::at(
                    b.line,
                    format!("`{}` is not a directive", b.head),
                ));
            }
        };
        if b.body.trim().is_empty() {
            return Err(Unread::at(b.line, "a directive with no body"));
        }
        directives.push(Directive {
            line: b.line,
            kind,
            body: b.body,
        });
    }
    Ok(Script { directives })
}

/// A reader of the words of a directive's line after its first.
type Reader = fn(&[&str]) -> Option<Kind>;

/// The directives of a conformance file, each by its first word.
const DIRECTIVES: [(&str, Reader); 4] = [
    ("schema", |rest| rest.is_empty().then_some(Kind::Schema)),
    ("run", |rest| rest.is_empty().then_some(Kind::Run)),
    ("query", |rest| {
        let (rowsort, list) = match rest {
            [] => (false, false),
            ["rowsort"] => (true, false),
            ["list"] => (false, true),
            _ => return None,
        };
        Some(Kind::Query {
            rowsort,
            list,
            rows: Vec::new(),
        })
    }),
    ("error", |rest| match rest {
        [reason] if reason.chars().all(char::is_alphanumeric) => Some(Kind::Error {
            reason: (*reason).to_owned(),
        }),
        _ => None,
    }),
];

/// Returns the first word of each directive of a conformance file.
pub fn directive_words() -> impl Iterator<Item = &'static str> {
    DIRECTIVES.iter().map(|(word, _)| *word)
}

/// Returns the directive `line` starts, its rows not yet read.
fn kind(line: &str) -> Option<Kind> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let (first, rest) = words.split_first()?;
    let (_, read) = DIRECTIVES.iter().find(|(word, _)| word == first)?;
    read(rest)
}

/// What a walk of a corpus directory meets.
#[derive(Debug)]
pub enum Entry {
    /// A `.test` file.
    Case(PathBuf),
    /// A file of another name.
    Other(PathBuf),
    /// A directory that does not read.
    Unread(PathBuf, io::Error),
}

/// Walk `dir` and every directory below it that `skip` does not hold, passing each entry met to
/// `found`.
pub fn walk(dir: &Path, skip: &dyn Fn(&Path) -> bool, found: &mut dyn FnMut(Entry)) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => return found(Entry::Unread(dir.to_path_buf(), e)),
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(e) => {
                found(Entry::Unread(dir.to_path_buf(), e));
                continue;
            }
        };
        if path.is_dir() {
            if !skip(&path) {
                walk(&path, skip, found);
            }
        } else if path.extension().is_some_and(|x| x == "test") {
            found(Entry::Case(path));
        } else {
            found(Entry::Other(path));
        }
    }
}

/// Returns `path` relative to the directory that holds the corpus `root`, when it lies under it.
pub fn shown(root: &Path, path: &Path) -> String {
    let base = root.parent().unwrap_or(root);
    path.strip_prefix(base)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Returns the name of the case file `path` of the corpus `root`: its path under the root without
/// `.test`.
pub fn case_name(root: &Path, path: &Path) -> String {
    let under = path.strip_prefix(root).unwrap_or(path);
    under.with_extension("").display().to_string()
}

/// Returns the name and script of each conformance file under `root`, in path order, or why one
/// does not read.
pub fn conformance_scripts(root: &Path) -> Result<Vec<(String, Script)>, String> {
    let mut files = Vec::new();
    let mut unread = None;
    walk(root, &|_| false, &mut |entry| match entry {
        Entry::Case(path) => files.push(path),
        Entry::Other(_) => {}
        Entry::Unread(dir, e) => {
            unread.get_or_insert_with(|| format!("{}: {e}", shown(root, &dir)));
        }
    });
    if let Some(unread) = unread {
        return Err(unread);
    }
    if files.is_empty() {
        return Err(format!("no cases under {}", shown(root, root)));
    }
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let at = shown(root, &path);
            let text = fs::read_to_string(&path).map_err(|e| format!("{at}: {e}"))?;
            let script = parse(&text).map_err(|u| format!("{at}:{}: {}", u.line, u.message))?;
            Ok((case_name(root, &path), script))
        })
        .collect()
}
