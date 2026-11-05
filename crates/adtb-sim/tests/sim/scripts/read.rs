//! Read an engine script into steps, each `use` replaced by its fixture's steps.

use std::fs;
use std::path::Path;
use std::time::Duration;

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
    DropRole {
        name: String,
        error: Option<String>,
    },
    Log,
    Bundle {
        header: String,
        fixture: Option<String>,
        refused: Option<String>,
    },
    Call {
        name: String,
        ordinal: Option<u32>,
        committed: bool,
        rowsort: bool,
        error: Option<String>,
    },
    Advance(Duration),
    Grant {
        revoke: bool,
        error: Option<String>,
    },
    Register {
        header: String,
        fixture: Option<String>,
        error: Option<String>,
    },
    Host(Vec<String>),
    Loose(String),
    Begin {
        name: String,
        read_only: bool,
        deferrable: bool,
        error: Option<String>,
    },
    Commit {
        name: String,
        position: Option<String>,
        error: Option<String>,
    },
    Rollback(String),
    Run,
    Query {
        rowsort: bool,
    },
    Command {
        rowsort: bool,
    },
    Error(String),
    Adhoc(String),
    Failed(String),
}

const HEADS: [&str; 28] = [
    "seed", "use", "option", "migrate", "reopen", "role", "log", "bundle", "call", "advance",
    "run", "query", "error", "as", "grant", "revoke", "register", "host", "loose", "begin",
    "commit", "rollback", "in", "assert", "failed", "drop", "command", "adhoc",
];

/// One directive, with the file and line it was read from.
#[derive(Clone, Debug)]
pub(super) struct Step {
    pub(super) at: String,
    pub(super) line: usize,
    /// The fixture file and line the step comes from, when a `use` brought it in.
    pub(super) via: Option<(String, usize)>,
    /// The role an `as` prefix names, the step then running through a session of it.
    pub(super) role: Option<String>,
    /// The transaction an `in` prefix names, the step then one call of it.
    pub(super) txn: Option<String>,
    /// The principal an `assert` prefix states, in textual syntax.
    pub(super) assert: Option<String>,
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
        "drop" => {
            let (name, error) = split(rest.strip_prefix("role ")?, " error ");
            Head::DropRole {
                name: named(name)?,
                error,
            }
        }
        "log" if rest.is_empty() => Head::Log,
        "bundle" => bundle(rest)?,
        "call" => call(rest)?,
        "advance" => Head::Advance(duration(rest)?),
        "grant" | "revoke" => Head::Grant {
            revoke: word == "revoke",
            error: match rest {
                "" => None,
                rest => Some(rest.strip_prefix("error ")?.to_owned()),
            },
        },
        "register" => {
            let (header, fixture, error) = bundled(rest, " error ")?;
            Head::Register {
                header,
                fixture,
                error,
            }
        }
        "host" if !rest.is_empty() => Head::Host(rest.split(' ').map(str::to_owned).collect()),
        "loose" if !rest.is_empty() && !rest.contains(' ') => Head::Loose(rest.to_owned()),
        "begin" => {
            let (rest, error) = split(rest, " error ");
            let (read_only, rest) = flag(rest, "read only ");
            let (deferrable, name) = flag(rest, "deferrable ");
            Head::Begin {
                name: named(name)?,
                read_only,
                deferrable,
                error,
            }
        }
        "commit" => {
            let (rest, error) = split(rest, " error ");
            let (name, position) = split(rest, " position ");
            Head::Commit {
                name: named(name)?,
                position: match position {
                    Some(p) => Some(named(&p)?),
                    None => None,
                },
                error,
            }
        }
        "rollback" => Head::Rollback(named(rest)?),
        "run" if rest.is_empty() => Head::Run,
        "query" if rest.is_empty() || rest == "rowsort" => Head::Query {
            rowsort: !rest.is_empty(),
        },
        "command" if rest.is_empty() || rest == "rowsort" => Head::Command {
            rowsort: !rest.is_empty(),
        },
        "error" if !rest.is_empty() => Head::Error(rest.to_owned()),
        "adhoc" => Head::Adhoc(rest.strip_prefix("error ")?.to_owned()),
        "failed" if !rest.is_empty() => Head::Failed(rest.to_owned()),
        _ => return None,
    })
}

/// Returns `rest` before `sep` and the text after it, or `rest` whole.
fn split<'a>(rest: &'a str, sep: &str) -> (&'a str, Option<String>) {
    match rest.split_once(sep) {
        Some((rest, v)) => (rest, Some(v.to_owned())),
        None => (rest, None),
    }
}

/// Returns the header, the fixture `use` names and the value after `sep` of a bundle's directive.
fn bundled(rest: &str, sep: &str) -> Option<(String, Option<String>, Option<String>)> {
    let (rest, value) = split(rest, sep);
    let (header, fixture) = match rest.strip_prefix("use ") {
        Some(name) => ("", Some(name.to_owned())),
        None => split(rest, " use "),
    };
    (!fixture.as_ref().is_some_and(|f| f.contains(' ')))
        .then(|| (header.to_owned(), fixture, value))
}

fn bundle(rest: &str) -> Option<Head> {
    let (header, fixture, refused) = bundled(rest, " refused ")?;
    Some(Head::Bundle {
        header,
        fixture,
        refused,
    })
}

fn call(rest: &str) -> Option<Head> {
    let (rest, error) = split(rest, " error ");
    let mut words = rest.split(' ');
    let name = words.next().filter(|n| !n.is_empty())?;
    let ordinal = words.next().map(str::parse).transpose().ok()?;
    let (committed, rowsort) = match words.collect::<Vec<_>>()[..] {
        [] => (false, false),
        ["committed"] => (true, false),
        ["rowsort"] => (false, true),
        ["committed", "rowsort"] => (true, true),
        _ => return None,
    };
    let rows = ordinal.is_some() && error.is_none();
    (rows || !(committed || rowsort)).then(|| Head::Call {
        name: name.to_owned(),
        ordinal,
        committed,
        rowsort,
        error,
    })
}

fn flag<'a>(rest: &'a str, word: &str) -> (bool, &'a str) {
    rest.strip_prefix(word)
        .map_or((false, rest), |rest| (true, rest))
}

fn named(name: &str) -> Option<String> {
    (!name.is_empty() && !name.contains(' ')).then(|| name.to_owned())
}

/// Returns the duration `text` states as a count of `s`, `m`, `h` or `d`.
fn duration(text: &str) -> Option<Duration> {
    let unit = match text.chars().last()? {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return None,
    };
    let count: u64 = text[..text.len() - 1].parse().ok()?;
    Some(Duration::from_secs(count.checked_mul(unit)?))
}

/// The prefixes of a directive.
#[derive(Default)]
struct Prefix<'a> {
    txn: Option<&'a str>,
    role: Option<&'a str>,
    assert: Option<&'a str>,
}

/// Returns the `in NAME`, `as ROLE` and `assert VALUE` prefixes of `line`, in that order, and the
/// directive after them.
fn prefixed(line: &str) -> (Prefix<'_>, &str) {
    let mut p = Prefix::default();
    let mut line = line;
    if let Some((txn, rest)) = line.strip_prefix("in ").and_then(|r| r.split_once(' ')) {
        (p.txn, line) = (Some(txn), rest);
    }
    if let Some((role, rest)) = line.strip_prefix("as ").and_then(|r| r.split_once(' ')) {
        (p.role, line) = (Some(role), rest);
    }
    if let Some(rest) = line.strip_prefix("assert ") {
        let end = value_end(rest);
        (p.assert, line) = (Some(&rest[..end]), rest[end..].trim_start());
    }
    (p, line)
}

/// Returns the length of the value `text` begins: up to the first space outside brackets and
/// quotes that no `{` follows.
fn value_end(text: &str) -> usize {
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for (i, c) in text.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            _ if quoted => {}
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth = depth.saturating_sub(1),
            ' ' if depth == 0 && !text[i..].trim_start().starts_with('{') => return i,
            _ => {}
        }
    }
    text.len()
}

/// Returns whether the directive `line` begins is followed by expected rows.
fn takes_rows(line: &str) -> bool {
    let line = prefixed(line).1;
    let call = line.strip_prefix("call ").and_then(call);
    ["query", "command"].contains(&line.split(' ').next().unwrap_or_default())
        || matches!(
            call,
            Some(Head::Call {
                ordinal: Some(_),
                error: None,
                ..
            })
        )
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
    let found =
        blocks(&text, starts, takes_rows).map_err(|d| format!("{at}:{}: {}", d.line, d.message))?;
    let mut out = Vec::new();
    for b in found {
        let (prefix, line) = prefixed(&b.head);
        let sent = |h: &Head| {
            matches!(
                h,
                Head::Run | Head::Query { .. } | Head::Error(_) | Head::Failed(_)
            )
        };
        let call = |h: &Head| {
            matches!(
                h,
                Head::Call {
                    ordinal: Some(_),
                    ..
                }
            )
        };
        let allowed = |h: &Head| {
            let role = sent(h)
                || matches!(
                    h,
                    Head::Call { .. } | Head::Grant { .. } | Head::Register { .. }
                );
            let txn = prefix.role.is_none() && prefix.assert.is_none() && (sent(h) || call(h));
            let assert = sent(h) || call(h) || matches!(h, Head::Begin { .. });
            let failed = !matches!(h, Head::Failed(_)) || prefix.role.is_some();
            failed
                && (prefix.role.is_none() || role)
                && (prefix.txn.is_none() || txn)
                && (prefix.assert.is_none() || assert)
        };
        let Some(head) = head(line).filter(allowed) else {
            return Err(format!("{at}:{}: `{}` is not a directive", b.line, b.head));
        };
        let bodied = matches!(
            head,
            Head::Migrate { .. }
                | Head::Log
                | Head::Run
                | Head::Query { .. }
                | Head::Command { .. }
                | Head::Error(_)
                | Head::Adhoc(_)
                | Head::Failed(_)
                | Head::Grant { .. }
                | Head::Loose(_)
        ) || matches!(
            &head,
            Head::Bundle { fixture: None, .. } | Head::Register { fixture: None, .. }
        );
        let free = matches!(
            head,
            Head::Call {
                ordinal: Some(_),
                ..
            } | Head::Migrate { at: Some(_), .. }
        );
        if !free && bodied == b.body.trim().is_empty() {
            let needs = if bodied { "a body" } else { "no body" };
            return Err(format!("{at}:{}: `{}` takes {needs}", b.line, b.head));
        }
        let mut body = b.body;
        if let Head::Bundle {
            fixture: Some(name),
            ..
        }
        | Head::Register {
            fixture: Some(name),
            ..
        } = &head
        {
            let fixture = root().join("fixtures").join(format!("{name}.bundle"));
            body = fs::read_to_string(&fixture)
                .map_err(|e| format!("{at}:{}: {}: {e}", b.line, shown(&fixture)))?;
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
            role: prefix.role.map(str::to_owned),
            txn: prefix.txn.map(str::to_owned),
            assert: prefix.assert.map(str::to_owned),
            head,
            body,
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
