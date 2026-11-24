//! Read an engine script into steps, each `use` replaced by its fixture's steps.

use std::fs;
use std::path::Path;
use std::time::Duration;

use adtb_script::blocks;

use super::fault::{self, Edit, Fault};
use super::{Class, Head, Knob, Step, shown};

/// Returns the size `n` states, `Some(None)` for `default`, or `None` when it states none.
fn size(n: &str) -> Option<Option<usize>> {
    match n {
        "default" => Some(None),
        n => n.parse().ok().map(Some),
    }
}

fn option(rest: &str) -> Option<Head> {
    Some(match rest.split_once(' ').unwrap_or((rest, "")) {
        ("query_memory_bytes", "default") => Head::QueryMemory(None),
        ("query_memory_bytes", bytes) => Head::QueryMemory(Some(bytes.parse().ok()?)),
        ("workers", n) => Head::Knob(Knob::Workers, size(n)?),
        ("morsel_rows", n) => Head::Knob(Knob::MorselRows, size(n)?),
        ("range_pages", n) => Head::Knob(Knob::RangePages, size(n)?),
        ("spill_bytes", n) => Head::Knob(Knob::SpillBytes, size(n)?),
        ("fix_rounds", n) => Head::Knob(Knob::FixRounds, size(n)?),
        ("lock_tuples", n) => Head::Knob(Knob::LockTuples, size(n)?),
        ("lock_pages", n) => Head::Knob(Knob::LockPages, size(n)?),
        ("lock_ranges", n) => Head::Knob(Knob::LockRanges, size(n)?),
        ("lock_total", n) => Head::Knob(Knob::LockTotal, size(n)?),
        ("export_roots", paths) => {
            Head::ExportRoots(paths.split_whitespace().map(str::to_owned).collect())
        }
        _ => return None,
    })
}

/// A reader of the rest of a directive's line after its first word.
type Reader = fn(&str) -> Option<Head>;

/// The directives, each by its first word.
const DIRECTIVES: [(&str, Reader); 40] = [
    ("seed", |rest| Some(Head::Seed(rest.parse().ok()?))),
    ("use", |rest| valued(rest).map(Head::Use)),
    ("option", option),
    ("migrate", migrate),
    ("reopen", |rest| {
        Some(Head::Reopen {
            refused: refused(rest)?,
        })
    }),
    ("role", |rest| valued(rest).map(Head::Role)),
    ("drop", drop_role),
    ("log", |rest| rest.is_empty().then_some(Head::Log)),
    ("bundle", bundle),
    ("call", call),
    ("advance", |rest| Some(Head::Advance(duration(rest)?))),
    ("grant", |rest| grant(false, rest)),
    ("revoke", |rest| grant(true, rest)),
    ("register", register),
    ("host", |rest| (!rest.is_empty()).then(|| host(rest))),
    ("loose", |rest| named(rest).map(Head::Loose)),
    ("begin", begin),
    ("commit", commit),
    ("rollback", |rest| named(rest).map(Head::Rollback)),
    ("run", |rest| rest.is_empty().then_some(Head::Run)),
    ("query", |rest| {
        rowsort(rest).map(|rowsort| Head::Query { rowsort })
    }),
    ("command", |rest| {
        rowsort(rest).map(|rowsort| Head::Command { rowsort })
    }),
    ("plan", |rest| rest.is_empty().then_some(Head::Plan)),
    ("error", |rest| valued(rest).map(Head::Error)),
    ("adhoc", |rest| {
        Some(Head::Adhoc(rest.strip_prefix("error ")?.to_owned()))
    }),
    ("failed", |rest| valued(rest).map(Head::Failed)),
    ("fatal", |rest| valued(rest).map(Head::Fatal)),
    ("retry", |rest| valued(rest).map(Head::Retry)),
    ("mark", fault::mark),
    ("restamp", fault::restamp),
    ("damage", |rest| fault::fault(Fault::Damage, rest)),
    ("flip", |rest| fault::fault(Fault::Flip, rest)),
    ("lose", |rest| fault::fault(Fault::Lose, rest)),
    ("tear", |rest| fault::fault(Fault::Tear, rest)),
    ("free", |rest| fault::fault(Fault::Free, rest)),
    ("scribble", |rest| fault::edit(Edit::Scribble, rest)),
    ("leak", |rest| fault::edit(Edit::Leak, rest)),
    ("crash", |rest| Some(Head::Crash(fault::crash_at(rest)?))),
    ("restore", fault::restore),
    ("probe", fault::probe),
];

/// Returns the first word of each directive.
pub fn directive_words() -> impl Iterator<Item = &'static str> {
    DIRECTIVES.iter().map(|(word, _)| *word)
}

/// Returns the prefixes and head of the directive `line` states, or `None` when it states none.
fn directive(line: &str) -> Option<(Prefix<'_>, Head)> {
    let (prefix, line) = prefixed(line);
    let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
    let (_, read) = DIRECTIVES.iter().find(|(w, _)| *w == word)?;
    let head = read(rest)?;
    prefix.admits(&head).then_some((prefix, head))
}

/// Returns `rest` when it states something.
fn valued(rest: &str) -> Option<String> {
    (!rest.is_empty()).then(|| rest.to_owned())
}

/// Returns whether `rest` states `rowsort`, or `None` when it states anything else.
fn rowsort(rest: &str) -> Option<bool> {
    match rest {
        "" => Some(false),
        "rowsort" => Some(true),
        _ => None,
    }
}

/// Returns the refusal `rest` states as `refused VALUE`, `Some(None)` when it is empty.
fn refused(rest: &str) -> Option<Option<String>> {
    match rest.strip_prefix("refused ") {
        Some(v) => Some(Some(v.to_owned())),
        None => rest.is_empty().then_some(None),
    }
}

fn migrate(rest: &str) -> Option<Head> {
    Some(match rest.strip_prefix("at ") {
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
    })
}

fn drop_role(rest: &str) -> Option<Head> {
    let (name, error) = split(rest.strip_prefix("role ")?, " error ");
    Some(Head::DropRole {
        name: named(name)?,
        error,
    })
}

fn grant(revoke: bool, rest: &str) -> Option<Head> {
    Some(Head::Grant {
        revoke,
        error: match rest {
            "" => None,
            rest => Some(rest.strip_prefix("error ")?.to_owned()),
        },
    })
}

fn register(rest: &str) -> Option<Head> {
    let (header, fixture, error) = bundled(rest, " error ")?;
    Some(Head::Register {
        header,
        fixture,
        error,
    })
}

/// Returns the head of `host` with names `rest`, `none` naming none.
fn host(rest: &str) -> Head {
    match rest {
        "none" => Head::Host(Vec::new()),
        names => Head::Host(names.split(' ').map(str::to_owned).collect()),
    }
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

fn begin(rest: &str) -> Option<Head> {
    let (rest, error) = split(rest, " error ");
    let (read_only, rest) = flag(rest, "read only ");
    let (deferrable, rest) = flag(rest, "deferrable ");
    let (name, id) = split(rest, " id ");
    Some(Head::Begin {
        name: named(name)?,
        read_only,
        deferrable,
        id: match id {
            Some(i) => Some(named(&i)?),
            None => None,
        },
        error,
    })
}

fn commit(rest: &str) -> Option<Head> {
    let (rest, error) = split(rest, " error ");
    let (rest, retry) = split(rest, " retry ");
    let (rest, fatal) = split(rest, " fatal ");
    let (name, position) = split(rest, " position ");
    let (class, error) = match (error, retry, fatal) {
        (e, None, None) => (Class::Rejected, e),
        (None, Some(r), None) => (Class::Retry, Some(r)),
        (None, None, Some(f)) => (Class::Fatal, Some(f)),
        _ => return None,
    };
    Some(Head::Commit {
        name: named(name)?,
        position: match position {
            Some(p) => Some(named(&p)?),
            None => None,
        },
        class,
        error,
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

impl Prefix<'_> {
    /// Returns whether these prefixes may stand before `head`.
    fn admits(&self, head: &Head) -> bool {
        let sent = matches!(
            head,
            Head::Run | Head::Query { .. } | Head::Error(_) | Head::Failed(_)
        );
        let call = matches!(
            head,
            Head::Call {
                ordinal: Some(_),
                ..
            }
        );
        let role = sent
            || matches!(
                head,
                Head::Call { .. } | Head::Grant { .. } | Head::Register { .. }
            );
        let txn = self.role.is_none()
            && self.assert.is_none()
            && (sent
                || call
                || matches!(
                    head,
                    Head::Plan | Head::ProbePlan | Head::Fatal(_) | Head::Retry(_)
                ));
        let assert = sent || call || matches!(head, Head::Begin { .. });
        let failed = !matches!(head, Head::Failed(_)) || self.role.is_some();
        failed
            && (self.role.is_none() || role)
            && (self.txn.is_none() || txn)
            && (self.assert.is_none() || assert)
    }
}

impl Head {
    /// Returns whether expected rows follow the directive.
    fn takes_rows(&self) -> bool {
        match self {
            Head::Query { .. }
            | Head::Command { .. }
            | Head::Plan
            | Head::ProbePlan
            | Head::ProbeChecks(_)
            | Head::ProbeLocks(_)
            | Head::Probe(_)
            | Head::Call {
                ordinal: Some(_),
                error: None,
                ..
            } => true,
            Head::Restore(r) => !r.refused(),
            _ => false,
        }
    }
}

/// Returns whether the directive `line` states is followed by expected rows.
pub(super) fn takes_rows(line: &str) -> bool {
    directive(line).is_some_and(|(_, head)| head.takes_rows())
}

/// Returns whether `line` states a directive.
pub(super) fn starts(line: &str) -> bool {
    directive(line).is_some()
}

/// Returns the steps of the script at `path`, its fixtures under `root`.
///
/// Returns an error for a script holding no directive but `seed` and `use`.
pub(super) fn script(root: &Path, path: &Path) -> Result<Vec<Step>, String> {
    let steps = read(root, path, &mut Vec::new())?;
    if steps
        .iter()
        .all(|s| s.via.is_some() || matches!(s.head, Head::Seed(_)))
    {
        return Err(format!(
            "{}: the script holds no directive but `seed` and `use`",
            shown(root, path)
        ));
    }
    Ok(steps)
}

/// Returns the steps of the file at `path`, each `use` replaced by its fixture's steps under
/// `root`, `uses` naming the fixtures being read.
fn read(root: &Path, path: &Path, uses: &mut Vec<String>) -> Result<Vec<Step>, String> {
    let at = shown(root, path);
    let text = fs::read_to_string(path).map_err(|e| format!("{at}: {e}"))?;
    let found = blocks(&text, starts, takes_rows)
        .map_err(|u| format!("{at}:{}: {}", u.line(), u.message()))?;
    let mut out = Vec::new();
    for b in found {
        let Some((prefix, head)) = directive(b.head()) else {
            return Err(format!(
                "{at}:{}: `{}` is not a directive",
                b.line(),
                b.head()
            ));
        };
        let bodied = matches!(
            head,
            Head::Migrate { .. }
                | Head::Log
                | Head::Run
                | Head::Query { .. }
                | Head::Command { .. }
                | Head::Plan
                | Head::ProbePlan
                | Head::Error(_)
                | Head::Adhoc(_)
                | Head::Failed(_)
                | Head::Fatal(_)
                | Head::Retry(_)
                | Head::Grant { .. }
                | Head::Loose(_)
                | Head::Edit {
                    edit: Edit::Scribble,
                    ..
                }
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
        if !free && bodied == b.body().trim().is_empty() {
            let needs = if bodied { "a body" } else { "no body" };
            return Err(format!("{at}:{}: `{}` takes {needs}", b.line(), b.head()));
        }
        let line = b.line();
        let mut body = b.body().to_owned();
        if let Head::Bundle {
            fixture: Some(name),
            ..
        }
        | Head::Register {
            fixture: Some(name),
            ..
        } = &head
        {
            let fixture = root.join("fixtures").join(format!("{name}.bundle"));
            body = fs::read_to_string(&fixture)
                .map_err(|e| format!("{at}:{line}: {}: {e}", shown(root, &fixture)))?;
        }
        if let Head::Use(name) = &head {
            if uses.contains(name) {
                return Err(format!(
                    "{at}:{}: `use` cycle: {} -> {name}",
                    line,
                    uses.join(" -> ")
                ));
            }
            uses.push(name.clone());
            let fixture = root.join("fixtures").join(format!("{name}.fixture"));
            let steps = read(root, &fixture, uses).map_err(|e| format!("{at}:{line}: {e}"))?;
            uses.pop();
            out.extend(steps.into_iter().map(|s| Step {
                via: s.via.or(Some((s.at, s.line))),
                at: at.clone(),
                line,
                ..s
            }));
            continue;
        }
        if matches!(head, Head::Seed(_)) && !out.is_empty() {
            return Err(format!("{at}:{line}: `seed` is the first directive"));
        }
        out.push(Step {
            at: at.clone(),
            line,
            via: None,
            role: prefix.role.map(str::to_owned),
            txn: prefix.txn.map(str::to_owned),
            assert: prefix.assert.map(str::to_owned),
            head,
            body,
            rows: b.rows().to_vec(),
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
