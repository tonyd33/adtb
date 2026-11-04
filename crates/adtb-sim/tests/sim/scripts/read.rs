//! Read an engine script into steps, each `use` replaced by its fixture's steps.

use std::fs;
use std::path::Path;

use adtb_conformance::blocks;

use super::{root, shown};

#[derive(Clone, Debug)]
pub(super) enum Head {
    Seed(u64),
    Use(String),
    QueryMemory(Option<u64>),
    Migrate {
        at: Option<usize>,
        refused: Option<String>,
    },
    Reopen {
        refused: Option<String>,
    },
    Role(String),
    Run,
    Query {
        rowsort: bool,
    },
    Error(String),
}

const HEADS: [&str; 9] = [
    "seed", "use", "option", "migrate", "reopen", "role", "run", "query", "error",
];

/// One directive, with the file and line it was read from.
#[derive(Clone, Debug)]
pub(super) struct Step {
    pub(super) at: String,
    pub(super) line: usize,
    /// The fixture file and line the step comes from, when a `use` brought it in.
    pub(super) via: Option<(String, usize)>,
    pub(super) head: Head,
    pub(super) body: String,
    pub(super) rows: Vec<String>,
}

fn head(line: &str) -> Option<Head> {
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    let refused = |rest: &str| match rest.strip_prefix("refused ") {
        Some(v) => Some(Some(v.to_owned())),
        None => rest.is_empty().then_some(None),
    };
    Some(match word {
        "seed" => Head::Seed(rest.parse().ok()?),
        "use" if !rest.is_empty() => Head::Use(rest.to_owned()),
        "option" => match rest.strip_prefix("query_memory_bytes ")? {
            "default" => Head::QueryMemory(None),
            bytes => Head::QueryMemory(Some(bytes.parse().ok()?)),
        },
        "migrate" => match rest.strip_prefix("at ") {
            Some(at) => {
                let (v, rest) = at.split_once(' ').unwrap_or((at, ""));
                Head::Migrate {
                    at: Some(v.parse().ok().filter(|&v| v > 0)?),
                    refused: refused(rest)?,
                }
            }
            None => Head::Migrate {
                at: None,
                refused: refused(rest)?,
            },
        },
        "reopen" => Head::Reopen {
            refused: refused(rest)?,
        },
        "role" if !rest.is_empty() => Head::Role(rest.to_owned()),
        "run" if rest.is_empty() => Head::Run,
        "query" if rest.is_empty() || rest == "rowsort" => Head::Query {
            rowsort: !rest.is_empty(),
        },
        "error" if !rest.is_empty() => Head::Error(rest.to_owned()),
        _ => return None,
    })
}

fn starts(line: &str) -> bool {
    HEADS.contains(&line.split_whitespace().next().unwrap_or_default())
}

/// Returns the steps of the script at `path`.
///
/// Returns an error for a script holding no directive but `seed` and `use`.
pub(super) fn script(path: &Path) -> Result<Vec<Step>, String> {
    let steps = read(path, &mut Vec::new())?;
    if steps
        .iter()
        .all(|s| s.via.is_some() || matches!(s.head, Head::Seed(_)))
    {
        return Err(format!(
            "{}: the script holds no directive but `seed` and `use`",
            shown(path)
        ));
    }
    Ok(steps)
}

/// Returns the steps of the file at `path`, each `use` replaced by its fixture's steps, `uses`
/// naming the fixtures being read.
fn read(path: &Path, uses: &mut Vec<String>) -> Result<Vec<Step>, String> {
    let at = shown(path);
    let text = fs::read_to_string(path).map_err(|e| format!("{at}: {e}"))?;
    let takes_rows = |l: &str| l.split_whitespace().next() == Some("query");
    let found =
        blocks(&text, starts, takes_rows).map_err(|d| format!("{at}:{}: {}", d.line, d.message))?;
    let mut out = Vec::new();
    for b in found {
        let Some(head) = head(&b.head) else {
            return Err(format!("{at}:{}: `{}` is not a directive", b.line, b.head));
        };
        let bodied = matches!(
            head,
            Head::Migrate { .. } | Head::Run | Head::Query { .. } | Head::Error(_)
        );
        if bodied == b.body.trim().is_empty() {
            let needs = if bodied { "a body" } else { "no body" };
            return Err(format!("{at}:{}: `{}` takes {needs}", b.line, b.head));
        }
        if let Head::Use(name) = &head {
            if uses.contains(name) {
                return Err(format!(
                    "{at}:{}: `use` cycle: {} -> {name}",
                    b.line,
                    uses.join(" -> ")
                ));
            }
            uses.push(name.clone());
            let fixture = root().join("fixtures").join(format!("{name}.fixture"));
            let steps = read(&fixture, uses).map_err(|e| format!("{at}:{}: {e}", b.line))?;
            uses.pop();
            out.extend(steps.into_iter().map(|s| Step {
                via: s.via.or(Some((s.at, s.line))),
                at: at.clone(),
                line: b.line,
                ..s
            }));
            continue;
        }
        if matches!(head, Head::Seed(_)) && !out.is_empty() {
            return Err(format!("{at}:{}: `seed` is the first directive", b.line));
        }
        out.push(Step {
            at: at.clone(),
            line: b.line,
            via: None,
            head,
            body: b.body,
            rows: b.rows,
        });
    }
    Ok(out)
}

/// Returns the migrations of `body`, each starting at a `module` line.
pub(super) fn migrations(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in body.lines() {
        if line.starts_with("module ") || out.is_empty() {
            out.push(String::new());
        }
        let cur = out.last_mut().expect("a migration");
        cur.push_str(line);
        cur.push('\n');
    }
    out
}
