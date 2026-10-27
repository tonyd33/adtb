//! The conformance runner: reads test scripts and runs them against the reference interpreter
//! or the engine.
//!
//! A script is a sequence of directives. A directive line starts in the first column; its body
//! is the lines below it indented by two spaces. The rows of a `query` follow a `----` line, one
//! per line, unindented, and end at a blank line, a comment or the next directive. Lines starting with `#`
//! between directives are comments.

#![forbid(unsafe_code)]

mod engine;

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::panic;
use std::path::{Path, PathBuf};

use adtb_lang::ast::SchemaFile;
use adtb_lang::check::{Program, check_adhoc, check_schema};
use adtb_lang::parse::{Options, parse_adhoc, parse_schema};
use adtb_lang::schema::{Schema, build};
use adtb_lang::span::FileId;
use adtb_lang::types::Ty;
use adtb_reference::{Abort, Database, Disk, Interpreter, Rejection, Supply, Value, textual};

pub use engine::run_engine;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Schema,
    Run,
    Query { rowsort: bool, rows: Vec<String> },
    Error { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directive {
    /// The 1-based line of the directive in its script.
    pub line: usize,
    pub kind: Kind,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    pub directives: Vec<Directive>,
}

/// A directive whose outcome differs from its script, or a script the runner cannot read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

fn differ(line: usize, message: impl Into<String>) -> Difference {
    Difference {
        line,
        message: message.into(),
    }
}

/// Reads a script.
pub fn parse(text: &str) -> Result<Script, Difference> {
    let lines: Vec<&str> = text.lines().collect();
    let mut directives = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let n = i + 1;
        i += 1;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with(' ') {
            return Err(differ(n, "an indented line outside a directive's body"));
        }
        let mut words = line.split_whitespace();
        let head = words.next().unwrap_or_default();
        let rest: Vec<&str> = words.collect();
        let mut kind = match (head, rest.as_slice()) {
            ("schema", []) => Kind::Schema,
            ("run", []) => Kind::Run,
            ("query", []) => Kind::Query {
                rowsort: false,
                rows: Vec::new(),
            },
            ("query", ["rowsort"]) => Kind::Query {
                rowsort: true,
                rows: Vec::new(),
            },
            ("error", [reason]) => Kind::Error {
                reason: reason.to_string(),
            },
            _ => return Err(differ(n, format!("`{line}` is not a directive"))),
        };
        let mut body = String::new();
        while i < lines.len() && (lines[i].starts_with("  ") || lines[i].trim().is_empty()) {
            if !lines[i].trim().is_empty() || next_is_body(&lines[i..]) {
                body.push_str(lines[i].get(2..).unwrap_or_default());
                body.push('\n');
            }
            i += 1;
        }
        if let Kind::Query { rows, .. } = &mut kind {
            if lines.get(i) != Some(&"----") {
                return Err(differ(n, "a query's rows follow `----`"));
            }
            i += 1;
            while i < lines.len() && !lines[i].trim().is_empty() && !starts_directive(lines[i]) {
                rows.push(lines[i].to_string());
                i += 1;
            }
        }
        if body.trim().is_empty() {
            return Err(differ(n, "a directive with no body"));
        }
        directives.push(Directive {
            line: n,
            kind,
            body,
        });
    }
    Ok(Script { directives })
}

fn starts_directive(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    line.starts_with('#')
        || matches!(
            words.as_slice(),
            ["schema" | "run" | "query"] | ["query", "rowsort"]
        )
        || matches!(words.as_slice(), ["error", c] if c.chars().all(char::is_alphanumeric))
}

/// Returns whether the blank line at the head of `lines` lies inside a body.
fn next_is_body(lines: &[&str]) -> bool {
    lines
        .iter()
        .find(|l| !l.trim().is_empty())
        .is_some_and(|l| l.starts_with("  "))
}

/// The fixpoint round limit of a statement. The step at which a statement exceeds its limits is
/// free, so a low limit keeps an unbounded `fix` quick.
pub(crate) const FIX_ROUNDS: usize = 200;

struct Run {
    schema: Schema,
    program: Program,
    db: Database,
    supply: Supply,
    files: Disk,
}

/// Runs `script` against a fresh database of the reference interpreter and returns every
/// directive whose outcome differs from the script. A file scan reads a relative path against
/// the working directory.
pub fn run_reference(script: &Script) -> Vec<Difference> {
    run_reference_in(script, Path::new(""))
}

/// Runs `script` as [`run_reference`] does, a file scan reading a relative path against `dir`.
pub fn run_reference_in(script: &Script, dir: &Path) -> Vec<Difference> {
    let mut out = Vec::new();
    let mut state: Option<Run> = None;
    for (k, d) in script.directives.iter().enumerate() {
        if let Kind::Schema = d.kind {
            if k != 0 {
                out.push(differ(d.line, "`schema` is the first directive"));
                return out;
            }
            match schema(&d.body, dir) {
                Ok(s) => state = Some(s),
                Err(e) => {
                    out.push(differ(d.line, format!("the schema is refused: {e}")));
                    return out;
                }
            }
            continue;
        }
        let st = match &mut state {
            Some(s) => s,
            None => match schema("", dir) {
                Ok(s) => state.insert(s),
                Err(e) => {
                    out.push(differ(d.line, format!("the prelude is refused: {e}")));
                    return out;
                }
            },
        };
        if let Err(e) = st.directive(d) {
            out.push(e);
        }
    }
    out
}

/// Builds the schema whose module files `body` holds, each starting with its `module` line, its
/// file scans reading under `dir`.
fn schema(body: &str, dir: &Path) -> Result<Run, String> {
    let mut files: Vec<(FileId, SchemaFile)> = Vec::new();
    for (i, text) in modules(body).iter().enumerate() {
        let file = parse_schema(text, Options::default())
            .ok()
            .map_err(|ds| messages(text, ds.iter().map(|d| (d.span.range().start, &d.message))))?;
        files.push((FileId(i as u32), file));
    }
    let schema = build(&files).map_err(|ds| format!("{ds:?}"))?;
    let program = check_schema(&schema).map_err(|ds| format!("{ds:?}"))?;
    Ok(Run {
        schema,
        program,
        db: Database::new(),
        supply: Supply::default(),
        files: Disk::new(dir),
    })
}

/// Splits `body` before each `module` line.
fn modules(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in body.lines() {
        if line.starts_with("module ") || out.is_empty() {
            out.push(String::new());
        }
        let cur = out.last_mut().expect("a module");
        cur.push_str(line);
        cur.push('\n');
    }
    out.retain(|m| !m.trim().is_empty());
    out
}

fn messages<'a>(text: &str, ds: impl Iterator<Item = (usize, &'a String)>) -> String {
    ds.map(|(at, m)| {
        let line = text[..at.min(text.len())].lines().count().max(1);
        format!("body line {line}: {m}")
    })
    .collect::<Vec<_>>()
    .join("; ")
}

/// How a statement ended: its result and type, or the reason constructor it was refused with.
enum Ended {
    Ok(Value, Ty),
    Refused(String, String),
}

impl Run {
    fn show(&self, v: &Value, ty: &Ty) -> String {
        textual::print(&self.schema, v, ty)
            .unwrap_or_else(|e| format!("a value that does not print: {e}"))
    }

    fn directive(&mut self, d: &Directive) -> Result<(), Difference> {
        match &d.kind {
            Kind::Schema => unreachable!("handled by the caller"),
            Kind::Run => {
                for (i, ended) in self.statements(&d.body, true)?.0.into_iter().enumerate() {
                    if let Ended::Refused(reason, detail) = ended {
                        return Err(differ(
                            d.line,
                            format!("statement {} is refused with {reason}: {detail}", i + 1),
                        ));
                    }
                }
                Ok(())
            }
            Kind::Error { reason } => {
                let before = self.db.clone();
                let outcome = self.statements(&d.body, true);
                self.db = before;
                let (ended, total) = outcome?;
                if ended.len() < total {
                    let Some(Ended::Refused(r, detail)) = ended.last() else {
                        unreachable!("a run stops early only at a refusal")
                    };
                    return Err(differ(
                        d.line,
                        format!(
                            "statement {} of {total} is refused with {r}: {detail}",
                            ended.len()
                        ),
                    ));
                }
                match ended.last() {
                    Some(Ended::Refused(r, _)) if r == reason => Ok(()),
                    Some(Ended::Refused(r, detail)) => Err(differ(
                        d.line,
                        format!("refused with {r}, not {reason}: {detail}"),
                    )),
                    Some(Ended::Ok(v, ty)) => Err(differ(
                        d.line,
                        format!(
                            "succeeds with {}, not refused with {reason}",
                            self.show(v, ty)
                        ),
                    )),
                    None => Err(differ(d.line, "no statement")),
                }
            }
            Kind::Query { rowsort, rows } => {
                let (ended, _) = self
                    .statements(&d.body, false)
                    .map_err(|e| differ(d.line, e.message))?;
                let [one] = ended.as_slice() else {
                    return Err(differ(d.line, "a query holds one statement"));
                };
                match one {
                    Ended::Refused(r, detail) => {
                        Err(differ(d.line, format!("refused with {r}: {detail}")))
                    }
                    Ended::Ok(v, ty) => self.compare(d.line, v, ty, *rowsort, rows),
                }
            }
        }
    }

    /// Runs the statements of ad hoc text `body` in order, stopping at the first refused, and
    /// keeps the state of each that succeeds when `keep`. Returns how each ended and how many
    /// the text holds.
    fn statements(&mut self, body: &str, keep: bool) -> Result<(Vec<Ended>, usize), Difference> {
        let items = match parse_adhoc(body, Options::default()).ok() {
            Ok(items) => items,
            Err(ds) => {
                let detail = messages(body, ds.iter().map(|d| (d.span.range().start, &d.message)));
                return Ok((vec![Ended::Refused("Invalid".into(), detail)], 1));
            }
        };
        let adhoc = match check_adhoc(&self.schema, &self.program, &items, &BTreeMap::new()) {
            Ok(a) => a,
            Err(ds) => {
                let detail = messages(body, ds.iter().map(|d| (d.span.range().start, &d.message)));
                return Ok((vec![Ended::Refused("Invalid".into(), detail)], 1));
            }
        };
        let schema = adhoc.schema(&self.schema);
        let mut out = Vec::new();
        for &item in &adhoc.items {
            let ty = match adhoc.terms.ty(item) {
                Ty::Change => Ty::Unit,
                t => t.clone(),
            };
            let it = Interpreter::new(schema, &self.program, &self.db)
                .with_fix_limit(FIX_ROUNDS)
                .with_files(&self.files);
            match it.run(&adhoc.terms, item, &[], &mut self.supply) {
                Ok(o) => {
                    let changed = !o.change.inserted.is_empty() || !o.change.deleted.is_empty();
                    if changed && !keep {
                        return Err(differ(0, "a query changes the database"));
                    }
                    if keep {
                        self.db = o.db;
                    }
                    out.push(Ended::Ok(o.result, ty));
                }
                Err(a) => {
                    let (reason, detail) = refusal(a);
                    out.push(Ended::Refused(reason, detail));
                    break;
                }
            }
        }
        let declared = self.schema.relations.len();
        self.db.retain(|(r, _), _| (r.0 as usize) < declared);
        Ok((out, adhoc.items.len()))
    }

    fn compare(
        &self,
        line: usize,
        got: &Value,
        ty: &Ty,
        rowsort: bool,
        rows: &[String],
    ) -> Result<(), Difference> {
        let elem = match ty {
            Ty::Set(e) | Ty::NonEmpty(e) => (**e).clone(),
            _ => ty.clone(),
        };
        let mut want = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            match textual::parse(&self.schema, r, &elem) {
                Ok(v) => want.push(v),
                Err(e) => {
                    return Err(differ(
                        line,
                        format!("expected row {} `{r}` does not read: {e}", i + 1),
                    ));
                }
            }
        }
        let shown = || match got {
            Value::List(l) => {
                let rows: Vec<String> = l.iter().map(|v| self.show(v, &elem)).collect();
                format!("in order [{}]", rows.join(", "))
            }
            v => self.show(v, ty),
        };
        if let Some(d) = repeated(line, &want) {
            return Err(d);
        }
        let mismatch = || differ(line, format!("got {}", shown()));
        match got {
            Value::Set(s) => {
                let want_set: std::collections::BTreeSet<Value> = want.iter().cloned().collect();
                if &want_set != s {
                    return Err(mismatch());
                }
                if !rowsort {
                    let canonical: Vec<String> = want.iter().map(|v| self.show(v, &elem)).collect();
                    if format!("[{}]", canonical.join(", ")) != shown() {
                        return Err(differ(
                            line,
                            format!(
                                "the rows are not in printed order; got {}; use `rowsort`",
                                shown()
                            ),
                        ));
                    }
                }
                Ok(())
            }
            Value::List(l) => {
                let (mut a, mut b) = (l.clone(), want);
                if rowsort {
                    a.sort();
                    b.sort();
                }
                if a == b { Ok(()) } else { Err(mismatch()) }
            }
            v => {
                if want.len() == 1 && &want[0] == v {
                    Ok(())
                } else {
                    Err(mismatch())
                }
            }
        }
    }
}

/// Returns the reason constructor an abort is refused with, and its detail.
fn refusal(a: Abort) -> (String, String) {
    match a {
        Abort::Fault(f) => ("Faulted".into(), format!("{f:?}")),
        Abort::Limit(l) => ("StatementLimit".into(), format!("{l:?}")),
        Abort::Reject(r) => {
            let name = match &r {
                Rejection::Violated { .. } => "Violated",
                Rejection::KeyViolation { .. } => "KeyViolation",
                Rejection::KeyChanged { .. } => "KeyChanged",
                Rejection::NotOpen { .. } => "NotOpen",
                Rejection::UserError { .. } => "UserError",
                Rejection::BadInput { .. } => "BadInput",
                Rejection::RowTooLarge { .. } => "RowTooLarge",
            };
            (name.into(), format!("{r:?}"))
        }
        Abort::NotYet(form) => (
            "NotYet".into(),
            format!("the interpreter does not evaluate {form}"),
        ),
        Abort::Internal(what) => ("Internal".into(), what),
    }
}

/// Returns the scripts under `dir`, `.test` files at any depth, sorted by path.
pub fn scripts(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "test") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Runs every script under `dir` against the reference interpreter and returns each difference,
/// prefixed by its script's path and line.
pub fn run_dir(dir: &Path) -> Vec<String> {
    scripts(dir).iter().flat_map(|p| run_file(p)).collect()
}

/// Runs the script at `path` against the reference interpreter and returns each difference,
/// prefixed by the path and line.
pub fn run_file(path: &Path) -> Vec<String> {
    let shown = path.display();
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return vec![format!("{shown}: {e}")],
    };
    let found = match parse(&text) {
        Ok(s) => panic::catch_unwind(|| {
            run_reference_in(&s, path.parent().unwrap_or_else(|| Path::new("")))
        })
        .unwrap_or_else(|p| {
            vec![differ(
                0,
                format!("the interpreter panics: {}", panic_message(&*p)),
            )]
        }),
        Err(d) => vec![d],
    };
    found
        .into_iter()
        .map(|d| format!("{shown}:{}: {}", d.line, d.message))
        .collect()
}

/// Returns the script error of an expected row that repeats an earlier one.
fn repeated<V: Ord>(line: usize, want: &[V]) -> Option<Difference> {
    let mut seen = BTreeMap::new();
    want.iter().enumerate().find_map(|(i, v)| {
        seen.insert(v, i).map(|first| {
            differ(
                line,
                format!(
                    "expected row {} repeats row {}; a result holds each row once",
                    i + 1,
                    first + 1
                ),
            )
        })
    })
}

/// Returns the message of a panic's payload, or the empty string when it holds none.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(ToString::to_string))
        .unwrap_or_default()
}
