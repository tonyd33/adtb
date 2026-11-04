//! Run the steps of an engine script on the simulator, one process per open.

use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::time::Duration;

use adtb::catalog::version_hash;
use adtb::runtime::{Clock, Done, Io, Op, Runtime, Threads};
use adtb::{CallOptions, Database, Error, Log, OpenError, OpenOptions, Registration, Value};
use adtb_conformance::engine::{self, Ended};
use adtb_conformance::{Difference, Directive, Got, Kind, panic_message};
use adtb_lang::evolve::Source;
use adtb_lang::parse::{Options, parse_migration};
use adtb_lang::textual::{Form, print};
use adtb_lang::types::Base;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_simkit::api::options;

use super::read::{self, Head, Step};
use super::role::{self, Handles, OWNER, called};
use super::shown;
use super::txn::{self, Txns};

const FILE: &str = "app.adt";

fn text(s: &str) -> String {
    print(&Value::text(s), &Form::Base(Base::Text)).expect("text prints")
}

/// Returns open refusal `e` in textual syntax, or the difference naming a refusal that has none.
pub(super) fn open_value(e: &OpenError) -> Result<String, String> {
    let field =
        |name: &str, field: &str, value: &str| Ok(format!("{name} {{ {field}: {} }}", text(value)));
    match e {
        OpenError::AlreadyOpen => Ok("AlreadyOpen".into()),
        OpenError::NotFound => Ok("NotFound".into()),
        OpenError::KeyRequired => Ok("KeyRequired".into()),
        OpenError::WrongKey => Ok("WrongKey".into()),
        OpenError::FormatTooNew { written_by } => field("FormatTooNew", "written_by", written_by),
        OpenError::Corrupt { detail } => field("Corrupt", "detail", detail),
        OpenError::Diverged { version } => Ok(format!("Diverged {{ version: {version} }}")),
        OpenError::MigrationRefused { version, reason } => Ok(format!(
            "MigrationRefused {{ version: {version}, reason: {} }}",
            reason.value()
        )),
        OpenError::RegistrationRefused { bundle, reason } => Ok(format!(
            "RegistrationRefused {{ bundle: {}, reason: {} }}",
            text(bundle),
            reason.value()
        )),
        OpenError::HostFnMissing { name } => field("HostFnMissing", "name", name),
        OpenError::HostFnMismatch { name } => field("HostFnMismatch", "name", name),
        OpenError::DowngradeRefused { cause } => field("DowngradeRefused", "cause", cause),
        OpenError::Locked { .. } => {
            Err("the open is refused with `Locked`, which has no textual value".into())
        }
        OpenError::Io(_) => Err("the open is refused with `Io`, which has no textual value".into()),
    }
}

pub(super) struct Differs {
    at: String,
    message: String,
    expected: Option<String>,
    actual: Option<String>,
}

impl Differs {
    pub(super) fn at(step: &Step, message: impl Into<String>) -> Differs {
        let mut at = format!("{}:{}", step.at, step.line);
        if let Some((file, line)) = &step.via {
            write!(at, " (via {file}:{line})").expect("a string takes text");
        }
        Differs {
            at,
            message: message.into(),
            expected: None,
            actual: None,
        }
    }

    pub(super) fn values(mut self, expected: String, actual: String) -> Differs {
        self.expected = Some(expected);
        self.actual = Some(actual);
        self
    }

    fn shown(&self) -> String {
        let mut s = format!("{}: {}", self.at, self.message);
        if let (Some(e), Some(a)) = (&self.expected, &self.actual) {
            write!(s, "\n  expected: {e}\n  actual:   {a}").expect("a string takes text");
        }
        s
    }
}

/// Returns the version hash of `migrations` as 64 hex digits, or why it has none.
fn hash(migrations: &[String]) -> Result<String, String> {
    let mut source = Source::default();
    for (v, text) in (1..).zip(migrations) {
        let file = parse_migration(text, Options::default())
            .ok()
            .map_err(|_| format!("migration {v} of the pin does not parse"))?;
        source = source
            .migrate(&file, text)
            .map_err(|_| format!("migration {v} of the pin does not apply"))?;
    }
    let schema = source
        .schema()
        .map_err(|_| "the pinned schema does not build")?;
    Ok(version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Returns the bundle file of `header` and `body`, the hash of the first N migrations of `log`
/// written after `pins N` when the header states none.
fn bundle_file(header: &str, body: &str, log: &[String]) -> Result<String, String> {
    let mut words: Vec<String> = header.split(' ').map(str::to_owned).collect();
    if let Some(i) = words.iter().position(|w| w == "pins")
        && let Some(n) = words.get(i + 1).and_then(|n| n.parse::<usize>().ok())
        && !words.get(i + 2).is_some_and(|w| w.starts_with('('))
    {
        let past = || format!("`pins {n}` is past the log's {} migrations", log.len());
        words.insert(
            i + 2,
            format!("({})", hash(log.get(..n).ok_or_else(past)?)?),
        );
    }
    Ok(match header {
        "" => body.to_owned(),
        _ => format!("bundle {}\n{body}", words.join(" ")),
    })
}

/// Advance the simulated clock by `d`.
fn sleep(rt: &SimRuntime, d: Duration) {
    let mut io = rt.io();
    let ticket = io.submit(Op::Timer { at: rt.now() + d });
    loop {
        if let Some(c) = io.poll() {
            assert_eq!(ticket, c.ticket, "the one timer completes");
            assert_eq!(Ok(Done::Fired), c.result, "the timer fires");
            return;
        }
        rt.park();
    }
}

pub(super) fn refused(e: &Error) -> String {
    e.value().to_string()
}

pub(super) fn rejected(reason: &str) -> String {
    format!("Rejected {{ reason: {reason} }}")
}

/// Compare outcome `got` with `want`, a refusal, else the step's rows.
pub(super) fn compare(
    step: &Step,
    want: Option<&str>,
    got: Result<Vec<String>, String>,
) -> Result<(), Differs> {
    let want = want.map_or_else(|| Ok(step.rows.clone()), |w| Err(rejected(w)));
    if got == want {
        return Ok(());
    }
    Err(Differs::at(step, "the outcome differs").values(listed(want), listed(got)))
}

pub(super) fn listed(r: Result<Vec<String>, String>) -> String {
    r.map_or_else(|e| e, |rows| format!("[{}]", rows.join(", ")))
}

/// What a `call` step ended with.
pub(super) enum Answer {
    Bound,
    Rows(Form, Vec<Value>),
    /// Refused at the bind or the submit.
    Refused(String),
    /// Admitted through a role's binding, and failed when it completed.
    Failed(String),
}

/// Returns the difference of a step whose rows `d` states differ.
fn rows_differ(step: &Step, d: Difference) -> Differs {
    match d.got {
        Some(Got::Rows(rows)) => Differs::at(step, "the rows differ").values(
            format!("[{}]", step.rows.join(", ")),
            format!("[{}]", rows.join(", ")),
        ),
        _ => Differs::at(step, d.message),
    }
}

/// Compare the rows of a call with the step's, a set result in printed order, or as a
/// multiset under `rowsort`.
pub(super) fn call_rows(
    step: &Step,
    form: &Form,
    mut got: Vec<Value>,
    rowsort: bool,
) -> Result<(), Differs> {
    got.sort();
    engine::compare(step.line, form, got, rowsort, &step.rows).map_err(|d| rows_differ(step, d))
}

/// Returns the values the queries of the step's body evaluate to, one per line.
pub(super) fn arguments(db: &Database<SimRuntime>, step: &Step) -> Result<Vec<Value>, Differs> {
    let mut args = Vec::new();
    for line in step.body.lines() {
        let mut got = db
            .query_text(line, &[], &CallOptions::default())
            .map_err(|e| {
                Differs::at(
                    step,
                    format!("argument `{line}` is refused: {}", refused(&e)),
                )
            })?;
        let (1, Some((Some(Ok(value)), None))) =
            (got.len(), got.pop().map(|mut r| (r.next(), r.next())))
        else {
            let shown = format!("argument `{line}` is no one value");
            return Err(Differs::at(step, shown));
        };
        args.push(value);
    }
    Ok(args)
}

struct Script {
    steps: Vec<Step>,
    next: usize,
    log: Vec<String>,
    options: OpenOptions,
    bundles: Vec<Step>,
    handles: Handles,
    out: Vec<Differs>,
}

impl Script {
    /// Run one process: the open the next step states, then each step up to the next open.
    /// A next step that sets an option or needs an open database runs alone.
    fn session(&mut self, rt: &SimRuntime) {
        let first = self.steps[self.next].clone();
        self.next += 1;
        let (log, refused) = match &first.head {
            Head::Migrate { at, refused } => {
                let mut log = self.log.clone();
                if let Some(v) = at
                    && v - 1 > log.len()
                {
                    self.out.push(Differs::at(
                        &first,
                        format!("`at {v}` is past the log's {} migrations", log.len()),
                    ));
                    self.next = self.steps.len();
                    return;
                }
                log.truncate(at.map_or(log.len(), |v| v - 1));
                log.extend(read::migrations(&first.body));
                (log, refused.clone())
            }
            Head::Reopen { refused } => (self.log.clone(), refused.clone()),
            Head::QueryMemory(_)
            | Head::Log
            | Head::Bundle { .. }
            | Head::Host(_)
            | Head::Loose(_) => {
                self.closed(&first);
                return;
            }
            _ => {
                let message = "no database is open: `migrate` or `reopen` first";
                self.out.push(Differs::at(&first, message));
                return;
            }
        };
        let options = OpenOptions {
            registrations: self.registrations(&log),
            ..self.options.clone()
        };
        let opened = Database::open_in(rt.clone(), FILE, &Log::new(log.clone()), &options);
        let db = match (opened, refused) {
            (Ok(db), want) => {
                self.log = log;
                if let Some(want) = want {
                    let open = Differs::at(&first, "the open succeeds");
                    self.out.push(open.values(want, "an open database".into()));
                }
                db
            }
            (Err(e), want) => {
                if let OpenError::MigrationRefused { version, .. } = e {
                    self.log = log;
                    self.log
                        .truncate(usize::try_from(version).expect("a version") - 1);
                }
                match (open_value(&e), want) {
                    (Err(unprintable), _) => self.out.push(Differs::at(&first, unprintable)),
                    (Ok(got), Some(want)) if want == got => {}
                    (Ok(got), Some(want)) => self.out.push(
                        Differs::at(&first, "the open is refused otherwise").values(want, got),
                    ),
                    (Ok(got), None) => self
                        .out
                        .push(Differs::at(&first, format!("the open is refused: {got}"))),
                }
                return;
            }
        };
        let mut txns = Txns::new();
        while let Some(step) = self.steps.get(self.next).cloned() {
            if matches!(step.head, Head::Migrate { .. } | Head::Reopen { .. }) {
                break;
            }
            self.next += 1;
            if let Err(d) = self.step(rt, &db, &mut txns, &step) {
                self.out.push(d);
            }
        }
        for name in txns.into_keys() {
            let open = format!("transaction `{name}` is open at the next open");
            self.out.push(Differs::at(&first, open));
        }
        self.handles = Handles::default();
        if let Err(e) = db.close() {
            let closed = format!("the database does not close: {}", e.value());
            self.out.push(Differs::at(&first, closed));
        }
    }

    /// Returns the registrations of the bundles stated since the last open, each bundle file that
    /// differs from its step a difference.
    fn registrations(&mut self, log: &[String]) -> Vec<Registration> {
        let mut out = Vec::new();
        for step in std::mem::take(&mut self.bundles) {
            let (header, want) = match &step.head {
                Head::Bundle {
                    header, refused, ..
                } => (header, refused),
                Head::Loose(name) => {
                    out.push(Registration::new(name, step.body.clone()));
                    continue;
                }
                _ => unreachable!("only bundle and loose steps are pending"),
            };
            let got = bundle_file(header, &step.body, log)
                .map(|text| Registration::bundle(text).map_err(|e| refused(&e)));
            match got.map(|r| r.map(|reg| out.push(reg)).map(|()| Vec::new())) {
                Err(e) => self.out.push(Differs::at(&step, e)),
                Ok(got) => self.out.extend(compare(&step, want.as_deref(), got).err()),
            }
        }
        out
    }

    /// Run a step that needs no open database.
    fn closed(&mut self, step: &Step) {
        match &step.head {
            Head::QueryMemory(bytes) => self.options.query_memory_bytes = *bytes,
            Head::Log => self.log.extend(read::migrations(&step.body)),
            Head::Bundle { .. } | Head::Loose(_) => self.bundles.push(step.clone()),
            Head::Host(names) => names.iter().for_each(|n| role::host(&mut self.options, n)),
            _ => unreachable!("session runs only these before an open"),
        }
    }

    fn step<'a>(
        &mut self,
        rt: &SimRuntime,
        db: &'a Database<SimRuntime>,
        txns: &mut Txns<'a>,
        step: &Step,
    ) -> Result<(), Differs> {
        let conformance = |kind| Directive {
            line: step.line,
            kind,
            body: step.body.clone(),
        };
        match &step.head {
            Head::Seed(_) | Head::Use(_) | Head::Migrate { .. } | Head::Reopen { .. } => {
                unreachable!("read takes seeds and uses, and session takes opens")
            }
            Head::QueryMemory(_)
            | Head::Log
            | Head::Bundle { .. }
            | Head::Host(_)
            | Head::Loose(_) => {
                self.closed(step);
                Ok(())
            }
            Head::Begin { .. } | Head::Commit { .. } | Head::Rollback(_) => {
                txn::ended(db, txns, step)
            }
            Head::Advance(d) => {
                sleep(rt, *d);
                Ok(())
            }
            Head::Call { rowsort, error, .. } => {
                match called(rt, db, &mut self.handles, txns, step)? {
                    Answer::Rows(form, rows) if error.is_none() => {
                        call_rows(step, &form, rows, *rowsort)
                    }
                    Answer::Rows(form, rows) => {
                        let shown = rows
                            .iter()
                            .map(|v| print(v, &form).unwrap_or_else(|e| e.message));
                        compare(step, error.as_deref(), Ok(shown.collect()))
                    }
                    Answer::Bound => compare(step, error.as_deref(), Ok(Vec::new())),
                    Answer::Refused(got) => compare(step, error.as_deref(), Err(got)),
                    Answer::Failed(got) => {
                        let want = error
                            .as_deref()
                            .map_or_else(|| Ok(step.rows.clone()), |w| Err(rejected(w)));
                        Err(Differs::at(step, "admitted, and fails at completion")
                            .values(listed(want), got))
                    }
                }
            }
            Head::Grant { revoke, error } => role::grant(db, step, *revoke, error.as_deref()),
            Head::Register { header, error, .. } => {
                let text =
                    bundle_file(header, &step.body, &self.log).map_err(|e| Differs::at(step, e))?;
                let authority = db.role_authority();
                let got = Registration::bundle(text).and_then(|reg| {
                    let session = authority.session(step.role.as_deref().unwrap_or(OWNER))?;
                    authority.register(&session, &reg)
                });
                compare(
                    step,
                    error.as_deref(),
                    got.map(|_| Vec::new()).map_err(|e| refused(&e)),
                )
            }
            Head::Run | Head::Query { .. } | Head::Error(_) | Head::Failed(_)
                if step.role.is_some() =>
            {
                role::sent_step(step, role::sent(rt, db, step)?)
            }
            Head::Run | Head::Query { .. } | Head::Error(_)
                if step.txn.is_some() || step.assert.is_some() =>
            {
                role::sent_step(step, txn::texted(db, txns, step)?)
            }
            Head::Failed(_) => unreachable!("read gives `failed` an `as` prefix"),
            Head::DropRole { name, error } => {
                let got = db.role_authority().drop_role(name);
                compare(
                    step,
                    error.as_deref(),
                    got.map(|()| Vec::new()).map_err(|e| refused(&e)),
                )
            }
            Head::Role(name) => db
                .role_authority()
                .create_role(name)
                .map_err(|e| Differs::at(step, format!("the role is refused: {}", e.value()))),
            Head::Run => engine::directive(db, &conformance(Kind::Run))
                .map_err(|d| Differs::at(step, d.message)),
            Head::Query { rowsort } => {
                let kind = Kind::Query {
                    rowsort: *rowsort,
                    list: false,
                    rows: step.rows.clone(),
                };
                engine::directive(db, &conformance(kind)).map_err(|d| rows_differ(step, d))
            }
            Head::Error(reason) => {
                let want = rejected(reason);
                match engine::ended(db, step.line, &step.body)
                    .map_err(|d| Differs::at(step, d.message))?
                {
                    Ended::Refused(e) if e.value().to_string() == want => Ok(()),
                    Ended::Refused(e) => {
                        Err(Differs::at(step, "refused otherwise")
                            .values(want, e.value().to_string()))
                    }
                    Ended::Succeeded => {
                        Err(Differs::at(step, "succeeds").values(want, "success".into()))
                    }
                    Ended::Early(e) => Err(Differs::at(
                        step,
                        format!("a statement before the last is refused with {}", e.value()),
                    )),
                }
            }
        }
    }
}

/// Run the script at `path`. Returns each directive that differs from it, shown.
pub(super) fn run_file(path: &Path) -> Vec<String> {
    let steps = match read::script(path) {
        Ok(s) => s,
        Err(e) => return vec![e],
    };
    let (seed, start) = match steps.first().map(|s| &s.head) {
        Some(Head::Seed(n)) => (*n, 1),
        _ => (0, 0),
    };
    let mut script = Script {
        steps,
        next: start,
        log: Vec::new(),
        options: OpenOptions {
            no_background_vacuum: true,
            ..options(4)
        },
        bundles: Vec::new(),
        handles: Handles::default(),
        out: Vec::new(),
    };
    let ran = panic::catch_unwind(AssertUnwindSafe(|| {
        let mut sim = Sim::new(seed, Config::default());
        while script.next < script.steps.len() {
            if sim.run(|rt| script.session(&rt)) != Outcome::Finished {
                return Some(format!("{}: the simulated process crashes", shown(path)));
            }
        }
        None
    }));
    let mut out: Vec<String> = script.out.iter().map(Differs::shown).collect();
    let unopened = |s| Differs::at(s, "no open registers the bundle").shown();
    out.extend(script.bundles.iter().map(unopened));
    match ran {
        Ok(None) => {}
        Ok(Some(crash)) => out.push(crash),
        Err(p) => out.push(format!(
            "{}: the engine panics: {}",
            shown(path),
            panic_message(&*p)
        )),
    }
    out
}
