use std::path::Path;

use adtb::runtime::Runtime;
use adtb::{
    AdHocTransaction, CallOptions, CreateOptions, Database, DynRows, Error, Log, OpenOptions,
    TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::ast::{AdhocItem, TxnOption};
use adtb_lang::parse::{Options, parse_adhoc};
use adtb_lang::textual::{self, Form};
use adtb_lang::types::Ty;

use crate::{Difference, Directive, FIX_ROUNDS, Kind, Script, differ, modules, repeated};

const FILE: &str = "conformance.adt";

/// Runs `script` against a fresh database file of the engine under `rt` and returns every
/// directive whose outcome differs from the script. A file scan reads a relative path against
/// the working directory.
pub fn run_engine<R: Runtime>(rt: R, script: &Script) -> Vec<Difference> {
    run_engine_in(rt, script, Path::new(""))
}

/// Runs `script` as [`run_engine`] does, a file scan reading a relative path against `dir`.
pub fn run_engine_in<R: Runtime>(rt: R, script: &Script, dir: &Path) -> Vec<Difference> {
    let mut directives = script.directives.as_slice();
    let mut log = Log::default();
    if let [first, rest @ ..] = directives
        && first.kind == Kind::Schema
    {
        log = Log::new(migrations(&first.body));
        directives = rest;
    }
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        fix_rounds: Some(FIX_ROUNDS),
        file_root: Some(dir.to_path_buf()),
        ..OpenOptions::default()
    };
    let db = match Database::open_in(rt, FILE, &log, &options) {
        Ok(db) => db,
        Err(e) => {
            let line = script.directives.first().map_or(0, |d| d.line);
            return vec![differ(line, format!("the database does not open: {e:?}"))];
        }
    };
    let mut out = Vec::new();
    for d in directives {
        if d.kind == Kind::Schema {
            out.push(differ(d.line, "`schema` is the first directive"));
            break;
        }
        if let Err(e) = directive(&db, d) {
            out.push(e);
        }
    }
    if let Err(e) = db.close() {
        out.push(differ(
            0,
            format!("the database does not close: {}", shown(&e)),
        ));
    }
    out
}

/// Returns the migration of each module file of the schema `body`: the module added, then each
/// declaration in order, each relation of a type the module declares moved up to follow its type.
fn migrations(body: &str) -> Vec<String> {
    modules(body).iter().map(|m| migration(m)).collect()
}

fn migration(module: &str) -> String {
    let mut header = String::new();
    let mut decls: Vec<String> = Vec::new();
    for line in module.lines() {
        if let Some(name) = line.strip_prefix("module ") {
            header = format!("{line}\nadd module {name}\n");
        } else if line.starts_with(' ') || line.starts_with("--") || line.trim().is_empty() {
            let into = decls.last_mut().unwrap_or(&mut header);
            into.push_str(line);
            into.push('\n');
        } else {
            decls.push(format!("{line}\n"));
        }
    }
    let word = |d: &str, i: usize| d.split_whitespace().nth(i).unwrap_or_default().to_owned();
    let declared = |d: &str| {
        (word(d, 0) == "type").then(|| {
            let name = word(d, 1);
            name.split(['[', '=', '{'])
                .next()
                .unwrap_or_default()
                .to_owned()
        })
    };
    let types: Vec<String> = decls.iter().filter_map(|d| declared(d)).collect();
    let moved = |d: &str| word(d, 0) == "relation" && types.contains(&word(d, 3));
    let mut out = header;
    for d in decls.iter().filter(|d| !moved(d)) {
        out.push_str(match word(d, 0).as_str() {
            "fn" | "aggregate" => "define ",
            _ => "add ",
        });
        out.push_str(d);
        if let Some(ty) = declared(d) {
            for r in decls.iter().filter(|r| moved(r) && word(r, 3) == ty) {
                out.push_str("add ");
                out.push_str(r);
            }
        }
    }
    out
}

fn directive<R: Runtime>(db: &Database<R>, d: &Directive) -> Result<(), Difference> {
    match &d.kind {
        Kind::Schema => unreachable!("handled by the caller"),
        Kind::Run => db
            .query_text(&d.body, &[], &CallOptions::default())
            .map(drop)
            .map_err(|e| differ(d.line, format!("refused with {}", shown(&e)))),
        Kind::Error { reason } => {
            let mut t = begin(db, d.line, &TxnOptions::default())?;
            let ended = refusal(&mut t, &d.body);
            t.rollback()
                .map_err(|e| differ(d.line, format!("the rollback fails: {}", shown(&e))))?;
            match ended {
                Ended::Succeeded => Err(differ(
                    d.line,
                    format!("succeeds, not refused with {reason}"),
                )),
                Ended::Early(e) => Err(differ(
                    d.line,
                    format!("a statement before the last is refused with {}", shown(&e)),
                )),
                Ended::Refused(e) if name(&e) == *reason => Ok(()),
                Ended::Refused(e) => Err(differ(
                    d.line,
                    format!("refused with {}, not {reason}", shown(&e)),
                )),
            }
        }
        Kind::Query { rowsort, rows } => {
            if parse_adhoc(&d.body, Options::default())
                .ok()
                .is_ok_and(|items| {
                    items
                        .iter()
                        .filter(|i| !matches!(i, AdhocItem::Temp(_)))
                        .count()
                        != 1
                })
            {
                return Err(differ(d.line, "a query holds one statement"));
            }
            let got = if deferrable(&d.body) {
                db.query_text(&d.body, &[], &CallOptions::default())
            } else {
                let read_only = TxnOptions {
                    read_only: true,
                    ..TxnOptions::default()
                };
                let mut t = begin(db, d.line, &read_only)?;
                let got = t.query_text(&d.body, &[], &TxnCallOptions::default());
                t.rollback()
                    .map_err(|e| differ(d.line, format!("the rollback fails: {}", shown(&e))))?;
                got
            };
            let mut results =
                got.map_err(|e| differ(d.line, format!("refused with {}", shown(&e))))?;
            match results.len() {
                0 => compare(
                    d.line,
                    &Form::of(db.schema(), &Ty::Unit),
                    vec![Value::record(Vec::new())],
                    *rowsort,
                    rows,
                ),
                1 => {
                    let got = results.remove(0);
                    let form = got.form().clone();
                    let vals = values(got)
                        .map_err(|e| differ(d.line, format!("a row fails: {}", shown(&e))))?;
                    compare(d.line, &form, vals, *rowsort, rows)
                }
                _ => Err(differ(d.line, "a query holds one statement")),
            }
        }
    }
}

/// Returns whether `body` is one `read_only deferrable` block, which writes nothing and does not
/// run inside an interactive transaction.
fn deferrable(body: &str) -> bool {
    parse_adhoc(body, Options::default())
        .ok()
        .is_ok_and(|items| match items.as_slice() {
            [AdhocItem::Transact(b)] => b
                .options
                .contains(&TxnOption::ReadOnly { deferrable: true }),
            _ => false,
        })
}

fn begin<'a, R: Runtime>(
    db: &'a Database<R>,
    line: usize,
    opts: &TxnOptions,
) -> Result<AdHocTransaction<'a, R>, Difference> {
    db.begin(opts)
        .map_err(|e| differ(line, format!("no transaction begins: {}", shown(&e))))
}

fn values(rows: DynRows) -> Result<Vec<Value>, Error> {
    rows.collect()
}

/// How the statements of an `error` directive ended.
enum Ended {
    Succeeded,
    /// A statement before the last is refused.
    Early(Error),
    /// The last statement, or the text as a whole, is refused.
    Refused(Error),
}

/// Runs the statements of `body` in `t`: every statement but the last as one call, then the
/// last.
///
/// Text refused as `Invalid` before its last statement counts as text refused as a whole.
fn refusal<R: Runtime>(t: &mut AdHocTransaction<'_, R>, body: &str) -> Ended {
    let mut call = |text: &str| t.query_text(text, &[], &TxnCallOptions::default());
    let (prefix, last) = match last_start(body) {
        Some(at) => body.split_at(at),
        None => ("", body),
    };
    if !prefix.trim().is_empty()
        && let Err(e) = call(prefix)
    {
        return if name(&e) == "Invalid" {
            Ended::Refused(e)
        } else {
            Ended::Early(e)
        };
    }
    match call(last) {
        Ok(_) => Ended::Succeeded,
        Err(e) => Ended::Refused(e),
    }
}

/// Returns the offset of the line the last item of ad hoc text `body` starts on, when the text
/// parses and that item is a query, a write or a `transact` block.
fn last_start(body: &str) -> Option<usize> {
    let items = parse_adhoc(body, Options::default()).ok().ok()?;
    let start = match items.last()? {
        AdhocItem::Expr(e) => e.span.start,
        AdhocItem::Write(w) => w.span.start,
        AdhocItem::Transact(b) => b.span.start,
        _ => return None,
    } as usize;
    Some(body[..start].rfind('\n').map_or(0, |n| n + 1))
}

fn compare(
    line: usize,
    form: &Form,
    mut got: Vec<Value>,
    rowsort: bool,
    rows: &[String],
) -> Result<(), Difference> {
    let mut want = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        match textual::parse::<Value>(r, form) {
            Ok(v) => want.push(v),
            Err(e) => {
                return Err(differ(
                    line,
                    format!("expected row {} `{r}` does not read: {e}", i + 1),
                ));
            }
        }
    }
    if let Some(d) = repeated(line, &want) {
        return Err(d);
    }
    let printed = match got
        .iter()
        .map(|v| textual::print(v, form))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(p) => p,
        Err(e) => {
            return Err(differ(
                line,
                format!("a result row does not print: {}", e.message),
            ));
        }
    };
    if rowsort {
        got.sort();
        want.sort();
    }
    if got == want {
        Ok(())
    } else {
        Err(differ(line, format!("got [{}]", printed.join(", "))))
    }
}

/// Returns the reason constructor `e` is refused with.
fn name(e: &Error) -> String {
    let reason = match e {
        Error::Retry { reason } => format!("{reason:?}"),
        Error::Rejected { reason } => format!("{reason:?}"),
        Error::Fatal { reason } => format!("{reason:?}"),
    };
    reason
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or_default()
        .to_owned()
}

fn shown(e: &Error) -> String {
    format!("{}: {e}", name(e))
}
