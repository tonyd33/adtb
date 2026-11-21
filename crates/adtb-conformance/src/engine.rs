//! Run conformance scripts on the engine and report each difference from what they state.

use std::path::Path;

use adtb::runtime::Runtime;
use adtb::{
    AdHocTransaction, CallOptions, CreateOptions, Database, DynRows, Error, Log, OpenOptions,
    TxnCallOptions, TxnOptions, Value,
};
use adtb_lang::ast::{AdhocItem, SchemaFile, TxnOption};
use adtb_lang::check::schema_migration;
use adtb_lang::parse::{Options, parse_adhoc, parse_schema};
use adtb_lang::textual::{self, Form};
use adtb_lang::types::Ty;

use crate::{
    Difference, Directive, FIX_ROUNDS, Got, Kind, Script, differ, differ_got, modules, repeated,
};

const FILE: &str = "conformance.adt";

/// Run `script` against a fresh database file of the engine under `rt`. Returns every
/// directive whose outcome differs from the script. A file scan reads a relative path against
/// the working directory.
pub fn run_engine<R: Runtime>(rt: R, script: &Script) -> Vec<Difference> {
    run_engine_in(rt, script, Path::new(""))
}

/// Run `script` as [`run_engine`] does, a file scan reading a relative path against `dir`.
pub fn run_engine_in<R: Runtime>(rt: R, script: &Script, dir: &Path) -> Vec<Difference> {
    let mut directives = script.directives();
    let mut log = Log::default();
    if let [first, rest @ ..] = directives
        && *first.kind() == Kind::Schema
    {
        match migrations(first.body()) {
            Ok(ms) => log = Log::new(ms),
            Err(e) => return vec![differ(first.line(), format!("the schema is refused: {e}"))],
        }
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
            let line = script.directives().first().map_or(0, Directive::line);
            return vec![differ(line, format!("the database does not open: {e:?}"))];
        }
    };
    let mut out = Vec::new();
    for d in directives {
        if *d.kind() == Kind::Schema {
            out.push(differ(d.line(), "`schema` is the first directive"));
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

/// Returns the migration of each module file of the schema `body`, each as
/// [`schema_migration`] replays it over the other module files, or the errors of the first that
/// does not parse or check.
pub fn migrations(body: &str) -> Result<Vec<String>, String> {
    let texts = modules(body);
    let mut files = Vec::with_capacity(texts.len());
    for t in &texts {
        let parsed = parse_schema(t, Options::default()).ok().map_err(|ds| {
            let rendered: Vec<String> = ds.iter().map(|d| d.render(t)).collect();
            rendered.join("\n")
        })?;
        files.push(parsed);
    }
    let mut out = Vec::with_capacity(files.len());
    for (i, file) in files.iter().enumerate() {
        let others: Vec<SchemaFile> = files
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .map(|(_, f)| f.clone())
            .collect();
        let m = schema_migration(&texts[i], file, &others).map_err(|ds| {
            let rendered: Vec<String> = ds.iter().map(|d| d.diag.message.clone()).collect();
            format!("module {}: {}", file.module.text, rendered.join("; "))
        })?;
        out.push(m);
    }
    Ok(out)
}

/// Run directive `d` on `db`. Returns how its outcome differs from what it states.
pub fn directive<R: Runtime>(db: &Database<R>, d: &Directive) -> Result<(), Difference> {
    match d.kind() {
        Kind::Schema => unreachable!("handled by the caller"),
        Kind::Run => db
            .query_text(d.body(), &[], &CallOptions::default())
            .map(drop)
            .map_err(|e| refusal_of(d.line(), &e)),
        Kind::Error { reason } => match ended(db, d.line(), d.body())? {
            Ended::Succeeded => Err(differ(
                d.line(),
                format!("succeeds, not refused with {reason}"),
            )),
            Ended::Early(e) => Err(differ(
                d.line(),
                format!("a statement before the last is refused with {}", shown(&e)),
            )),
            Ended::Refused(e) if name(&e) == *reason => Ok(()),
            Ended::Refused(e) => Err(differ(
                d.line(),
                format!("refused with {}, not {reason}", shown(&e)),
            )),
        },
        Kind::Query {
            rowsort,
            list,
            rows,
        } => {
            if *list {
                return Err(differ(
                    d.line(),
                    "the engine runner cannot observe List result shape",
                ));
            }
            if parse_adhoc(d.body(), Options::default())
                .ok()
                .is_ok_and(|items| {
                    items
                        .iter()
                        .filter(|i| !matches!(i, AdhocItem::Temp(_)))
                        .count()
                        != 1
                })
            {
                return Err(differ(d.line(), "a query holds one statement"));
            }
            let got = if deferrable(d.body()) {
                db.query_text(d.body(), &[], &CallOptions::default())
            } else {
                let read_only = TxnOptions {
                    read_only: true,
                    ..TxnOptions::default()
                };
                let mut t = begin(db, d.line(), &read_only)?;
                let got = t.query_text(d.body(), &[], &TxnCallOptions::default());
                t.rollback()
                    .map_err(|e| differ(d.line(), format!("the rollback fails: {}", shown(&e))))?;
                got
            };
            let mut results = got.map_err(|e| refusal_of(d.line(), &e))?;
            match results.len() {
                0 => compare(
                    d.line(),
                    &Form::of(db.schema(), &Ty::Unit),
                    vec![Value::record(Vec::new())],
                    *rowsort,
                    rows,
                ),
                1 => {
                    let got = results.remove(0);
                    let form = got.form().clone();
                    let vals = values(got)
                        .map_err(|e| differ(d.line(), format!("a row fails: {}", shown(&e))))?;
                    compare(d.line(), &form, vals, *rowsort, rows)
                }
                _ => Err(differ(d.line(), "a query holds one statement")),
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

/// Run the statements of `body` in one transaction of `db` and roll it back.
///
/// Returns how they ended, or how the transaction failed, as the difference of the directive on
/// `line`.
pub fn ended<R: Runtime>(db: &Database<R>, line: usize, body: &str) -> Result<Ended, Difference> {
    let mut t = begin(db, line, &TxnOptions::default())?;
    let ended = refusal(&mut t, body);
    t.rollback()
        .map_err(|e| differ(line, format!("the rollback fails: {}", shown(&e))))?;
    Ok(ended)
}

/// How the statements of an `error` directive ended.
#[derive(Debug)]
pub enum Ended {
    Succeeded,
    /// A statement before the last is refused.
    Early(Error),
    /// The last statement, or the text as a whole, is refused.
    Refused(Error),
}

/// Run the statements of `body` in `t`: every statement but the last as one call, then the
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

/// Compare result `got`, printed at `form`, with the expected `rows` of the directive on `line`:
/// in order, or as a multiset under `rowsort`.
pub fn compare(
    line: usize,
    form: &Form,
    mut got: Vec<Value>,
    rowsort: bool,
    rows: &[String],
) -> Result<(), Difference> {
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
    let mut want = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        match textual::parse::<Value>(r, form) {
            Ok(v) => want.push(v),
            Err(e) => {
                return Err(differ(
                    line,
                    format!(
                        "expected row {} `{r}` does not read: {e}; got [{}]",
                        i + 1,
                        printed.join(", ")
                    ),
                ));
            }
        }
    }
    if let Some(d) = repeated(line, &want) {
        return Err(d);
    }
    if rowsort {
        got.sort();
        want.sort();
    }
    if got == want {
        Ok(())
    } else {
        let message = format!("got [{}]", printed.join(", "));
        Err(differ_got(line, message, Got::Rows(printed)))
    }
}

fn refusal_of(line: usize, e: &Error) -> Difference {
    crate::refused(
        line,
        format!("refused with {}", shown(e)),
        &name(e),
        &e.to_string(),
    )
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
