//! Check a schema and an ad hoc text, then run the text on the reference interpreter and on the
//! engine over the simulator, each on a fresh database, and print both outcomes.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, Read};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use adtb::runtime::Runtime;
use adtb::write::Part;
use adtb::{CreateOptions, Database, Log, OpenOptions, Value};
use adtb_conformance::{
    Difference, Directive, Got, Kind, Script, migrations, panic_message, run_engine_in,
    run_reference_in,
};
use adtb_lang::ast::AdhocItem;
use adtb_lang::check::{check_adhoc, check_schema};
use adtb_lang::parse::{Options, parse_adhoc, parse_schema};
use adtb_lang::schema::build;
use adtb_lang::span::{Diag, FileDiag, FileId};
use adtb_lang::textual::{self, Form};
use adtb_lang::types::Base;
use adtb_sim::{Config, Outcome, Sim};
use clap::Parser;

const SCHEMA: usize = 1;
const DATA: usize = 2;
const BEFORE: usize = 3;
const TEXT: usize = 4;
const EXPLAIN: usize = 5;

#[derive(Debug, Parser)]
#[command(
    name = "adtb-probe",
    about = "Check the schema FILE and the ad hoc TEXT, printing each refusal with its line and \
             column. Then run TEXT on a fresh database of the reference interpreter and of the \
             engine over the simulator, print each outcome and whether they agree. TEXT `-` reads \
             the text from stdin.",
    after_help = "Exits 0 when the sides agree, 1 when they differ or a side fails to load, 2 \
                  when the schema or the text is refused or NAME names nothing.",
    args_override_self = true
)]
struct Args {
    #[arg(
        long,
        value_name = "FILE",
        allow_hyphen_values = true,
        help = "the schema: module files, each from its `module` line"
    )]
    schema: PathBuf,
    #[arg(
        long,
        value_name = "FILE",
        allow_hyphen_values = true,
        help = "ad hoc text run before TEXT, to load the database"
    )]
    data: Option<PathBuf>,
    #[arg(long, help = "also print the engine's plan of TEXT")]
    explain: bool,
    #[arg(
        long,
        value_name = "NAME",
        allow_hyphen_values = true,
        help = "also print the engine's plan of each incremental check of the constraint or \
                check NAME, by qualified name (`app::adults`); TEXT is then optional"
    )]
    checks: Option<String>,
    #[arg(long, help = "run on the reference interpreter only")]
    reference: bool,
    #[arg(long, help = "run on the engine only")]
    engine: bool,
    #[arg(
        value_name = "TEXT",
        required_unless_present = "checks",
        allow_hyphen_values = true,
        value_parser = text,
        help = "the ad hoc text, or `-` for stdin"
    )]
    text: Option<String>,
}

fn text(v: &str) -> Result<String, String> {
    if v.starts_with("--") {
        return Err("a text cannot start with `--`".to_owned());
    }
    Ok(v.to_owned())
}

impl Args {
    /// Read a text `-` from stdin, and run on both sides when neither is named.
    fn resolve(mut self) -> Result<Args, String> {
        if self.text.as_deref() == Some("-") {
            self.text = Some(stdin()?);
        }
        if !self.reference && !self.engine {
            (self.reference, self.engine) = (true, true);
        }
        Ok(self)
    }
}

fn stdin() -> Result<String, String> {
    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .map_err(|e| format!("stdin: {e}"))?;
    Ok(text)
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Returns each module file of `text` with the 0-based line it starts on.
fn modules(text: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.starts_with("module ") || out.is_empty() {
            out.push((n, String::new()));
        }
        let (_, cur) = out.last_mut().expect("a module was pushed");
        cur.push_str(line);
        cur.push('\n');
    }
    out.retain(|(_, m)| !m.trim().is_empty());
    out
}

/// Returns `d` as `LABEL:LINE:COL: MESSAGE`, then its source line and a caret under its start.
fn render(label: &str, src: &str, first_line: usize, d: &Diag) -> String {
    let (line, col) = d.line_col(src);
    let source = src.lines().nth(line - 1).unwrap_or_default();
    format!(
        "{label}:{}:{col}: {}\n  {source}\n  {:>col$}\n",
        line + first_line,
        d.message,
        "^"
    )
}

fn render_files(label: &str, files: &[(usize, String)], ds: &[FileDiag]) -> String {
    ds.iter()
        .map(|fd| match files.get(fd.file.0 as usize) {
            Some((first, src)) if !fd.file.is_engine() => render(label, src, *first, &fd.diag),
            _ => format!("{label}: in the prelude: {}\n", fd.diag.message),
        })
        .collect()
}

/// Check the schema and the text, when there is one, as the reference does, and return the
/// text's items.
fn check(label: &str, schema: &str, text: Option<&str>) -> Result<Vec<AdhocItem>, String> {
    let files = modules(schema);
    let mut parsed = Vec::new();
    for (i, (first, src)) in files.iter().enumerate() {
        let file = parse_schema(src, Options::default()).ok().map_err(|ds| {
            ds.iter()
                .map(|d| render(label, src, *first, d))
                .collect::<String>()
        })?;
        parsed.push((FileId(i as u32), file));
    }
    let built = build(&parsed).map_err(|ds| render_files(label, &files, &ds))?;
    let program = check_schema(&built).map_err(|ds| render_files(label, &files, &ds))?;
    let Some(text) = text else {
        return Ok(Vec::new());
    };
    let rendered = |ds: Vec<Diag>| {
        ds.iter()
            .map(|d| render("TEXT", text, 0, d))
            .collect::<String>()
    };
    let items = parse_adhoc(text, Options::default())
        .ok()
        .map_err(rendered)?;
    check_adhoc(&built, &program, &items, &BTreeMap::new()).map_err(rendered)?;
    Ok(items)
}

/// Returns where the query that ends `text` starts: at the start of the text when every item
/// before it is a `temp` declaration, else at the start of its line. Returns `None` when the last
/// item is not a query.
fn query_start(text: &str, items: &[AdhocItem]) -> Option<usize> {
    let (AdhocItem::Expr(last), before) = items.split_last()? else {
        return None;
    };
    if before.iter().all(|i| matches!(i, AdhocItem::Temp(_))) {
        return Some(0);
    }
    let start = last.span.start as usize;
    Some(text[..start].rfind('\n').map_or(0, |n| n + 1))
}

fn directive(line: usize, kind: Kind, body: &str) -> Directive {
    let mut body = body.to_owned();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    Directive { line, kind, body }
}

fn query() -> Kind {
    Kind::Query {
        rowsort: true,
        rows: Vec::new(),
    }
}

/// Returns the script that loads the schema and the data, then runs the text: as a `query` when
/// its last item is a query, the items before it as a `run`, else all of it as a `run`.
fn script(schema: &str, data: Option<&str>, text: &str, items: &[AdhocItem]) -> Script {
    let mut directives = vec![directive(SCHEMA, Kind::Schema, schema)];
    if let Some(data) = data.filter(|d| !d.trim().is_empty()) {
        directives.push(directive(DATA, Kind::Run, data));
    }
    match query_start(text, items) {
        Some(at) => {
            let (before, last) = text.split_at(at);
            if !before.trim().is_empty() {
                directives.push(directive(BEFORE, Kind::Run, before));
            }
            directives.push(directive(TEXT, query(), last));
        }
        None => directives.push(directive(TEXT, Kind::Run, text)),
    }
    Script { directives }
}

fn failed(message: String) -> Vec<Difference> {
    vec![Difference {
        line: 0,
        message,
        got: None,
    }]
}

fn on_reference(script: &Script, dir: &Path) -> Vec<Difference> {
    panic::catch_unwind(AssertUnwindSafe(|| run_reference_in(script, dir)))
        .unwrap_or_else(|p| failed(format!("the reference panics: {}", panic_message(&*p))))
}

fn on_engine(script: &Script, dir: &Path) -> Vec<Difference> {
    let mut found = Vec::new();
    let ran = panic::catch_unwind(AssertUnwindSafe(|| {
        Sim::new(0, Config::default()).run(|rt| found = run_engine_in(rt, script, dir))
    }));
    match ran {
        Ok(Outcome::Finished) => found,
        Ok(Outcome::Crashed) => failed("the simulated process crashes".into()),
        Err(p) => failed(format!("the engine panics: {}", panic_message(&*p))),
    }
}

/// How a side's text ended, as two sides compare it: rows as a multiset, a refusal by its reason.
#[derive(Debug, PartialEq, Eq)]
enum Ended {
    Ok,
    Rows(Vec<String>),
    Refused(String),
    Failed,
}

/// How one side ran the script: what loading the database reported, how the text ended and how
/// that reads, and the plan of its explain.
struct Ran {
    setup: Vec<String>,
    ended: Ended,
    text: String,
    explain: Option<String>,
}

fn outcome(found: &[Difference], line: usize, kind: &Kind) -> (Ended, String) {
    match found.iter().find(|d| d.line == line) {
        Some(d) => match &d.got {
            Some(Got::Rows(rows)) => {
                let mut sorted = rows.clone();
                sorted.sort();
                (Ended::Rows(sorted), format!("[{}]", rows.join(", ")))
            }
            Some(Got::Refused { reason, detail }) => (
                Ended::Refused(reason.clone()),
                format!("refused with {reason}: {detail}"),
            ),
            None => (Ended::Failed, d.message.clone()),
        },
        None if matches!(kind, Kind::Query { .. }) => (Ended::Rows(Vec::new()), "[]".into()),
        None => (Ended::Ok, "ok".into()),
    }
}

/// Returns the `text` member of the row an `explain` returns, or all of `row` when it does not
/// read as one.
fn plan_text(row: &str) -> String {
    let text = || Form::Base(Base::Text);
    let form = Form::Record(vec![("document".into(), text()), ("text".into(), text())]);
    match textual::parse::<Value>(row, &form) {
        Ok(Value::Record(fields)) => match &fields[..] {
            [_, Value::Text(plan)] => plan.to_string(),
            _ => row.to_owned(),
        },
        _ => row.to_owned(),
    }
}

fn plan(found: &[Difference]) -> String {
    match found.iter().find(|d| d.line == EXPLAIN) {
        Some(Difference {
            got: Some(Got::Rows(rows)),
            ..
        }) => rows.iter().map(|r| plan_text(r)).collect(),
        Some(d) => d.message.clone(),
        None => "no plan".into(),
    }
}

fn ran(script: &Script, found: &[Difference]) -> Ran {
    let at = |line| script.directives.iter().find(|d| d.line == line);
    let text = at(TEXT).expect("a probe's script runs its text");
    let (ended, shown) = outcome(found, TEXT, &text.kind);
    Ran {
        setup: found
            .iter()
            .filter(|d| ![TEXT, EXPLAIN].contains(&d.line))
            .map(|d| match d.line {
                SCHEMA => format!("the schema: {}", d.message),
                DATA => format!("the data: {}", d.message),
                BEFORE => format!("the text before its query: {}", d.message),
                _ => d.message.clone(),
            })
            .collect(),
        ended,
        text: shown,
        explain: at(EXPLAIN).map(|_| plan(found)),
    }
}

fn agree(a: &Ran, b: &Ran) -> bool {
    a.ended != Ended::Failed && a.ended == b.ended
}

fn report(out: &mut String, side: &str, r: &Ran) {
    for s in &r.setup {
        writeln!(out, "{side}: {s}").expect("a string is written");
    }
    writeln!(out, "{side}: {}", r.text).expect("a string is written");
    if let Some(plan) = &r.explain {
        let plan = plan.trim_end_matches('\n').replace('\n', "\n  ");
        writeln!(out, "{side} plan:\n  {plan}").expect("a string is written");
    }
}

/// Returns the plan of each incremental check of the constraint or check `name` of a fresh
/// database of the engine over `schema`, each under the change that runs it.
fn checks<R: Runtime>(rt: R, schema: &str, dir: &Path, name: &str) -> Result<String, String> {
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        file_root: Some(dir.to_path_buf()),
        ..OpenOptions::default()
    };
    let log = Log::new(migrations(schema));
    let db = Database::open_in(rt, "probe.adt", &log, &options)
        .map_err(|e| format!("the database does not open: {e:?}"))?;
    let plans = db.explain_checks(name).map_err(|e| e.to_string());
    db.close()
        .map_err(|e| format!("the database does not close: {e}"))?;
    let mut out = String::new();
    for p in plans? {
        let on = match &p.trigger {
            None => "any change".to_owned(),
            Some((rel, Part::Inserted)) => format!("insert into {rel}"),
            Some((rel, Part::Deleted)) => format!("delete from {rel}"),
            Some((rel, Part::Changed)) => format!("change to {rel}"),
        };
        let plan = p.plan.text();
        let plan = plan.trim_end_matches('\n').replace('\n', "\n  ");
        writeln!(out, "check {name} on {on}:\n  {plan}").expect("a string is written");
    }
    Ok(out)
}

fn on_engine_checks(schema: &str, dir: &Path, name: &str) -> Result<String, String> {
    let mut found = Err("the simulation does not run".to_owned());
    let ran = panic::catch_unwind(AssertUnwindSafe(|| {
        Sim::new(0, Config::default()).run(|rt| found = checks(rt, schema, dir, name))
    }));
    match ran {
        Ok(Outcome::Finished) => found,
        Ok(Outcome::Crashed) => Err("the simulated process crashes".into()),
        Err(p) => Err(format!("the engine panics: {}", panic_message(&*p))),
    }
}

fn probe(args: &Args) -> Result<(String, ExitCode), String> {
    let schema = read(&args.schema)?;
    let data = args.data.as_deref().map(read).transpose()?;
    let label = args.schema.display().to_string();
    let items = match check(&label, &schema, args.text.as_deref()) {
        Ok(items) => items,
        Err(refused) => return Ok((refused, ExitCode::from(2))),
    };
    let dir = args.schema.parent().unwrap_or(Path::new(""));
    let (mut out, code) = match &args.text {
        Some(text) => run(args, &schema, data.as_deref(), text, &items, dir),
        None => (String::new(), ExitCode::SUCCESS),
    };
    if let Some(name) = &args.checks {
        out.push_str(&on_engine_checks(&schema, dir, name)?);
    }
    Ok((out, code))
}

/// Run `text` on each side `args` names. Returns the report with its exit code.
fn run(
    args: &Args,
    schema: &str,
    data: Option<&str>,
    text: &str,
    items: &[AdhocItem],
    dir: &Path,
) -> (String, ExitCode) {
    let script = script(schema, data, text, items);
    let mut explained = script.clone();
    if args.explain {
        let text = format!("explain {text}");
        explained
            .directives
            .push(directive(EXPLAIN, query(), &text));
    }
    let mut sides = Vec::new();
    if args.reference {
        sides.push(("reference", ran(&script, &on_reference(&script, dir))));
    }
    if args.engine {
        sides.push(("engine", ran(&explained, &on_engine(&explained, dir))));
    }
    let mut out = String::new();
    for (side, r) in &sides {
        report(&mut out, side, r);
    }
    let loaded = sides.iter().all(|(_, r)| r.setup.is_empty());
    let agree = loaded && sides.windows(2).all(|w| agree(&w[0].1, &w[1].1));
    if sides.len() == 2 {
        out.push_str(if agree { "agree\n" } else { "differ\n" });
    }
    let code = if agree {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };
    (out, code)
}

#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "goes if the probe stops being a binary that reports on its streams"
)]
fn main() -> ExitCode {
    let args = match Args::parse().resolve() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("adtb-probe: {e}");
            return ExitCode::from(2);
        }
    };
    match probe(&args) {
        Ok((out, code)) => {
            print!("{out}");
            code
        }
        Err(e) => {
            eprintln!("adtb-probe: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(args: &[&str]) -> Result<Args, clap::Error> {
        Args::try_parse_from(std::iter::once("adtb-probe").chain(args.iter().copied()))
    }

    #[test]
    fn the_command_is_well_formed() {
        Args::command().debug_assert();
    }

    #[test]
    fn a_full_invocation_parses() {
        let a = parse(&[
            "--schema",
            "s.adt",
            "--data",
            "d.adt",
            "--explain",
            "--checks",
            "app::c",
            "users",
        ])
        .expect("the invocation parses")
        .resolve()
        .expect("no stdin is read");
        assert_eq!(PathBuf::from("s.adt"), a.schema);
        assert_eq!(Some(PathBuf::from("d.adt")), a.data);
        assert!(a.explain && a.reference && a.engine);
        assert_eq!(Some("app::c"), a.checks.as_deref());
        assert_eq!(Some("users"), a.text.as_deref());
    }

    #[test]
    fn the_text_is_optional_only_with_checks() {
        let a = parse(&["--schema", "s.adt", "--checks", "app::adults"]).expect("checks alone");
        assert_eq!(None, a.text);
        let e = parse(&["--schema", "s.adt"]).expect_err("no text");
        assert_eq!(2, e.exit_code());
    }

    #[test]
    fn one_side_runs_only_that_side() {
        let a = parse(&["--schema", "s.adt", "--engine", "1"])
            .expect("the invocation parses")
            .resolve()
            .expect("no stdin is read");
        assert!(a.engine && !a.reference);
    }

    #[test]
    fn a_text_may_start_with_a_hyphen() {
        let a = parse(&["--schema", "s.adt", "-1"]).expect("a negative text parses");
        assert_eq!(Some("-1"), a.text.as_deref());
    }

    #[test]
    fn a_misspelt_flag_is_not_a_text() {
        let e = parse(&["--schema", "s.adt", "--explian"]).expect_err("--explian");
        assert_eq!(2, e.exit_code());
    }

    #[test]
    fn help_is_not_a_text() {
        for flag in ["-h", "--help"] {
            let e = parse(&["--schema", "s.adt", flag]).expect_err(flag);
            assert_eq!(clap::error::ErrorKind::DisplayHelp, e.kind(), "{flag}");
        }
    }

    #[test]
    fn usage_errors_exit_with_two() {
        for args in [
            &["1"][..],
            &["--schema"],
            &["--schema", "s", "1", "2"],
            &["--schema", "s", "--nope"],
        ] {
            let e = parse(args).expect_err(&format!("{args:?}"));
            assert_eq!(2, e.exit_code(), "{args:?}");
        }
    }

    fn side(got: Option<Got>) -> Ran {
        let found = vec![Difference {
            line: TEXT,
            message: "a message: with a colon".into(),
            got,
        }];
        let (ended, text) = outcome(&found, TEXT, &query());
        Ran {
            setup: Vec::new(),
            ended,
            text,
            explain: None,
        }
    }

    fn rows(rows: &[&str]) -> Option<Got> {
        Some(Got::Rows(rows.iter().map(ToString::to_string).collect()))
    }

    fn refused(reason: &str, detail: &str) -> Option<Got> {
        Some(Got::Refused {
            reason: reason.into(),
            detail: detail.into(),
        })
    }

    #[test]
    fn rows_agree_as_a_multiset() {
        assert!(agree(
            &side(rows(&["b", "a: 1"])),
            &side(rows(&["a: 1", "b"]))
        ));
        assert!(!agree(&side(rows(&["a", "a"])), &side(rows(&["a"]))));
    }

    #[test]
    fn refusals_agree_by_reason_whatever_their_detail() {
        let a = side(refused("KeyViolation", "Violated: `x`"));
        let b = side(refused("KeyViolation", "holds a row: with key"));
        assert!(agree(&a, &b));
        assert!(!agree(&a, &side(refused("Violated", "KeyViolation: x"))));
    }

    #[test]
    fn rows_and_a_refusal_differ() {
        assert!(!agree(&side(rows(&[])), &side(refused("Invalid", ""))));
    }

    #[test]
    fn a_failure_agrees_with_nothing() {
        assert!(!agree(&side(None), &side(None)));
    }

    #[test]
    fn no_difference_is_no_rows_for_a_query_and_ok_for_a_run() {
        assert_eq!(
            (Ended::Rows(Vec::new()), "[]".into()),
            outcome(&[], TEXT, &query())
        );
        assert_eq!((Ended::Ok, "ok".into()), outcome(&[], TEXT, &Kind::Run));
    }

    #[test]
    fn the_plan_is_the_text_member_of_the_explain_row() {
        assert_eq!(
            "Set\n  Scan \"us\"\n",
            plan_text(r#"{ document: "{}", text: "Set\n  Scan \"us\"\n" }"#)
        );
        assert_eq!("not a row", plan_text("not a row"));
    }
}
