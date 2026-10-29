//! The engine over generated cases: its results against the reference interpreter's, a query
//! against its partitions by constructor and by predicate, and every planner rewrite setting
//! against the default. A disagreement is shrunk to a small program before it is reported.

use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::OnceLock;

use adtb::plan::{Rewrites, with_rewrites};
use adtb::{
    CallOptions, Called, CreateOptions, Database, Error, Log, OpenError, OpenOptions, Registration,
    RejectReason,
};
use adtb_lang::textual;
use adtb_reference::{Abort, Rejection, Value};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::cases::{Case, Forms, Inverse, Operator, Query, Walk, case, walk, walk_of};
use adtb_testkit::corpus::{self, Script};
use adtb_testkit::fixture::Fixture;
use adtb_testkit::{Rng, profile};

/// Cases per default run, [`profile::deep`] sized, unless `ADTB_DIFF_CASES` says otherwise.
const CASES: u64 = 12;

/// The queries of each case, over one database.
const QUERIES: usize = 24;

/// Returns the seeds of one run of a drawn or corpus case.
fn seeds() -> Range<u64> {
    seeds_of(CASES)
}

/// Returns the seeds of one run, from `ADTB_DIFF_SEED`, else zero, `ADTB_DIFF_CASES` of them,
/// else `cases` as [`profile::deep`] sizes it.
fn seeds_of(cases: u64) -> Range<u64> {
    let var = |name: &str, default: u64| {
        std::env::var(name).map_or(default, |v| {
            v.parse()
                .unwrap_or_else(|_| panic!("{name} is not a number: {v}"))
        })
    };
    let first = var("ADTB_DIFF_SEED", 0);
    first..first + var("ADTB_DIFF_CASES", profile::deep(cases))
}

/// The disagreements printed whole with their program; the rest are listed by seed.
const SHOWN: usize = 3;

/// The evaluations of a law a shrink of the queries of a case may try.
const QUERY_TRIES: usize = 400;

/// The engine databases a shrink of the database of a case may open.
const DATABASE_TRIES: usize = 24;

/// How a query ended: its rows, ascending, or a fault, or a rejection both sides name, or a
/// refusal of another kind.
#[derive(Clone, Debug, PartialEq)]
enum Got {
    Rows(Vec<Value>),
    Faulted,
    Rejected(Reason),
    Refused(String),
}

/// A rejection reason the engine and the reference both give, by the fields that identify it.
#[derive(Clone, Debug, PartialEq)]
enum Reason {
    Violated { constraint: String },
    KeyViolation { relation: String, key: String },
    KeyChanged { relation: String },
    NotOpen { ctor: String },
    UserError { message: String },
    BadInput { source: String, line: i64 },
}

impl Reason {
    fn of_engine(r: RejectReason) -> Option<Reason> {
        Some(match r {
            RejectReason::Violated { constraint, .. } => Reason::Violated { constraint },
            RejectReason::KeyViolation { relation, key } => Reason::KeyViolation { relation, key },
            RejectReason::KeyChanged { relation } => Reason::KeyChanged { relation },
            RejectReason::NotOpen { ctor, .. } => Reason::NotOpen { ctor },
            RejectReason::UserError { message } => Reason::UserError { message },
            RejectReason::BadInput { source, line, .. } => Reason::BadInput { source, line },
            _ => return None,
        })
    }

    fn of_reference(r: Rejection) -> Option<Reason> {
        Some(match r {
            Rejection::Violated { constraint, .. } => Reason::Violated { constraint },
            Rejection::KeyViolation { relation, key } => Reason::KeyViolation { relation, key },
            Rejection::KeyChanged { relation } => Reason::KeyChanged { relation },
            Rejection::NotOpen { ctor, .. } => Reason::NotOpen { ctor },
            Rejection::UserError { message } => Reason::UserError { message },
            Rejection::BadInput { source, line, .. } => Reason::BadInput { source, line },
            Rejection::RowTooLarge { .. } | Rejection::KeyTooLarge { .. } => return None,
        })
    }
}

/// Returns whether `a` and `b` are equal, floats within `tolerance` of each other.
fn within(a: &Value, b: &Value, tolerance: f64) -> bool {
    let all = |a: &[Value], b: &[Value]| {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| within(x, y, tolerance))
    };
    let fields = |a: &[(String, Value)], b: &[(String, Value)]| {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|((m, x), (n, y))| m == n && within(x, y, tolerance))
    };
    match (a, b) {
        (Value::F64(x), Value::F64(y)) => {
            x == y || (x.is_nan() && y.is_nan()) || (x - y).abs() <= tolerance
        }
        (Value::Ctor(c, xs), Value::Ctor(d, ys)) => c == d && fields(xs, ys),
        (Value::Record(xs), Value::Record(ys)) => fields(xs, ys),
        (Value::Key(x), Value::Key(y)) => within(x, y, tolerance),
        (Value::Set(xs), Value::Set(ys)) => {
            let (xs, ys): (Vec<Value>, Vec<Value>) =
                (xs.iter().cloned().collect(), ys.iter().cloned().collect());
            all(&xs, &ys)
        }
        (Value::List(xs), Value::List(ys)) => all(xs, ys),
        _ => a == b,
    }
}

impl Got {
    fn agrees(&self, other: &Got, tolerance: f64) -> bool {
        match (self, other) {
            (Got::Rows(a), Got::Rows(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| within(x, y, tolerance))
            }
            (Got::Faulted, Got::Faulted) => true,
            (Got::Rejected(a), Got::Rejected(b)) => a == b,
            _ => false,
        }
    }

    /// Returns the union of the rows of `parts`, or the first that does not end with rows.
    fn union(parts: Vec<Got>) -> Got {
        let mut rows = Vec::new();
        for p in parts {
            match p {
                Got::Rows(r) => rows.extend(r),
                other => return other,
            }
        }
        rows.sort();
        rows.dedup();
        Got::Rows(rows)
    }

    /// Returns the sum of `parts`, each one integer row.
    fn sum(parts: Vec<Got>) -> Got {
        let mut total = 0;
        for p in parts {
            match p {
                Got::Rows(r) => match r.as_slice() {
                    [Value::Int(n)] => total += n,
                    _ => return Got::Refused(format!("a count is not one integer: {r:?}")),
                },
                other => return other,
            }
        }
        Got::Rows(vec![Value::Int(total)])
    }
}

fn reference(case: &Case) -> Fixture {
    let src = case
        .schema
        .strip_prefix("module app\n")
        .expect("a case's schema is module `app`");
    let mut fx = Fixture::new(src);
    for w in &case.load {
        if let Err(a) = fx.run(w) {
            panic!("the reference refuses `{w}`: {a:?}");
        }
    }
    fx
}

fn reference_eval(fx: &Fixture, text: &str) -> Got {
    match fx.eval(text) {
        Ok(Value::Set(s)) => Got::Rows(s.into_iter().collect()),
        Ok(v) => Got::Rows(vec![v]),
        Err(Abort::Fault(_)) => Got::Faulted,
        Err(Abort::Reject(r)) => match Reason::of_reference(r.clone()) {
            Some(reason) => Got::Rejected(reason),
            None => Got::Refused(format!("{:?}", Abort::Reject(r))),
        },
        Err(a) => Got::Refused(format!("{a:?}")),
    }
}

/// Returns how a query ends that the engine fails with `e`.
fn engine_failed(e: Error) -> Got {
    let message = e.to_string();
    match e {
        Error::Rejected {
            reason: RejectReason::Faulted { .. },
        } => Got::Faulted,
        Error::Rejected { reason } => {
            Reason::of_engine(reason).map_or(Got::Refused(message), Got::Rejected)
        }
        _ => Got::Refused(message),
    }
}

fn engine_eval(db: &Database<SimRuntime>, text: &str) -> Got {
    let mut results = match db.query_text(text, &[], &CallOptions::default()) {
        Ok(r) => r,
        Err(e) => return engine_failed(e),
    };
    if results.len() != 1 {
        return Got::Refused(format!("{} results of one query", results.len()));
    }
    let got = results.remove(0);
    let form = got.form().clone();
    let mut rows = Vec::new();
    for row in got {
        let v = match row {
            Ok(v) => v,
            Err(e) => return engine_failed(e),
        };
        let read = textual::print(&v, &form)
            .map_err(|e| e.message)
            .and_then(|t| textual::parse::<Value>(&t, &form).map_err(|e| e.to_string()));
        match read {
            Ok(v) => rows.push(v),
            Err(e) => return Got::Refused(format!("a row does not read back: {e}")),
        }
    }
    rows.sort();
    Got::Rows(rows)
}

/// Returns the engine database of the simulated file, created if it does not exist, at the
/// version the migrations of `log` leave.
fn open(rt: SimRuntime, log: &[String]) -> Database<SimRuntime> {
    try_open(rt, log).unwrap_or_else(|e| {
        let last = log.last().map_or("", String::as_str);
        panic!(
            "the engine does not open at migration {}: {e:?}\n{last}",
            log.len()
        )
    })
}

fn try_open(rt: SimRuntime, log: &[String]) -> Result<Database<SimRuntime>, OpenError> {
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        ..OpenOptions::default()
    };
    Database::open_in(rt, "differential.adt", &Log::new(log.to_vec()), &options)
}

fn write(db: &Database<SimRuntime>, writes: &[String]) {
    for w in writes {
        if let Err(e) = db.query_text(w, &[], &CallOptions::default()) {
            panic!("the engine refuses `{w}`: {e}");
        }
    }
}

fn close(db: Database<SimRuntime>) {
    if let Err(e) = db.close() {
        panic!("the engine does not close: {e}");
    }
}

/// Runs `body` as one process of `sim` and returns its result.
fn process<T>(sim: &mut Sim, body: impl FnOnce(SimRuntime) -> T) -> T {
    let mut out = None;
    let outcome = sim.run(|rt| out = Some(body(rt)));
    match (outcome, out) {
        (Outcome::Finished, Some(v)) => v,
        (o, _) => panic!("the simulated run ends {o:?}"),
    }
}

/// Runs `f` on a fresh simulated engine database holding the case, loaded.
fn on_engine<T>(seed: u64, case: &Case, f: impl FnOnce(&Database<SimRuntime>) -> T) -> T {
    process(&mut Sim::new(seed, Config::default()), |rt| {
        let db = open(rt, std::slice::from_ref(&case.migration));
        write(&db, &case.load);
        let out = f(&db);
        close(db);
        out
    })
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

/// What a law reads a query against: the case's seed and tolerance, the reference and the
/// engine, each holding the case's database.
struct Env<'a> {
    seed: u64,
    tolerance: f64,
    fx: &'a Fixture,
    db: &'a Database<SimRuntime>,
}

/// A law of one query: the disagreement it shows, if any.
type Law = fn(&Env, &Query) -> Option<String>;

/// How a law fails on a case: a disagreement it states, or a panic.
#[derive(Clone, Debug)]
enum Failure {
    Disagrees(String),
    Panics(String),
}

impl Failure {
    fn same_kind(&self, other: &Failure) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Disagrees(m) => f.write_str(m),
            Failure::Panics(m) => write!(f, "panics: {m}"),
        }
    }
}

/// Returns how `law` fails on `q`, if it does.
fn fails(env: &Env, law: Law, q: &Query) -> Option<Failure> {
    match panic::catch_unwind(AssertUnwindSafe(|| law(env, q))) {
        Ok(m) => m.map(Failure::Disagrees),
        Err(p) => Some(Failure::Panics(panic_message(&*p))),
    }
}

/// Run `body` over the reference and the engine holding `case`, a panic reported as a failure.
fn with_case<T>(seed: u64, case: &Case, body: impl FnOnce(&Env) -> T) -> Result<T, Failure> {
    panic::catch_unwind(AssertUnwindSafe(|| {
        let fx = reference(case);
        on_engine(seed, case, |db| {
            body(&Env {
                seed,
                tolerance: case.tolerance,
                fx: &fx,
                db,
            })
        })
    }))
    .map_err(|p| Failure::Panics(panic_message(&*p)))
}

/// Returns the first query of `case` that breaks `law`, by index, and how.
fn first_failure(seed: u64, case: &Case, law: Law) -> Option<(Option<usize>, Failure)> {
    let ran = with_case(seed, case, |env| {
        case.queries
            .iter()
            .enumerate()
            .find_map(|(i, q)| fails(env, law, q).map(|m| (Some(i), m)))
    });
    match ran {
        Ok(found) => found,
        Err(m) => Some((None, m)),
    }
}

/// Returns `q` shrunk greedily against `law` over the database `env` reads, each step failing
/// as `failure` does.
fn shrink_query(env: &Env, law: Law, q: Query, failure: Failure) -> (Query, Failure) {
    let (mut q, mut failure) = (q, failure);
    let mut tries = 0;
    'outer: while tries < QUERY_TRIES {
        for s in q.shrinks() {
            tries += 1;
            if let Some(f) = fails(env, law, &s).filter(|f| f.same_kind(&failure)) {
                (q, failure) = (s, f);
                continue 'outer;
            }
            if tries >= QUERY_TRIES {
                break;
            }
        }
        break;
    }
    (q, failure)
}

/// Returns `case` shrunk against `law`, from its failing query `at`: that query alone, shrunk,
/// over a database shrunk, then shrunk again over that database, each step failing as
/// `failure` does.
fn shrink(
    seed: u64,
    case: &Case,
    law: Law,
    at: Option<usize>,
    failure: &Failure,
) -> (Case, Failure) {
    let mut c = match at {
        Some(i) => case.with_queries(vec![case.queries[i].clone()]),
        None => case.clone(),
    };
    let mut failure = failure.clone();
    let queries = |c: &Case, failure: &Failure| {
        let [q] = c.queries.as_slice() else {
            return None;
        };
        with_case(seed, c, |env| {
            shrink_query(env, law, q.clone(), failure.clone())
        })
        .ok()
    };
    if let Some((q, f)) = queries(&c, &failure) {
        (c, failure) = (c.with_queries(vec![q]), f);
    }
    let mut opened = 0;
    'outer: while opened < DATABASE_TRIES {
        for s in c.smaller() {
            if panic::catch_unwind(AssertUnwindSafe(|| reference(&s))).is_err() {
                continue;
            }
            opened += 1;
            if let Some((_, f)) =
                first_failure(seed, &s, law).filter(|(_, f)| f.same_kind(&failure))
            {
                (c, failure) = (s, f);
                continue 'outer;
            }
            if opened >= DATABASE_TRIES {
                break;
            }
        }
        break;
    }
    if let Some((q, f)) = queries(&c, &failure) {
        (c, failure) = (c.with_queries(vec![q]), f);
    }
    (c, failure)
}

/// Runs `law` over every query of every seed's drawn case and fails with each disagreement, or
/// panic, by seed, the first shrunk.
fn sweep(name: &str, law: Law) {
    sweep_cases(name, law, |seed| (String::new(), case(seed, QUERIES)));
}

/// Runs `law` over every query of the case `make` returns for each seed, with a label, and
/// fails with each disagreement, or panic, by seed and label, the first shrunk.
fn sweep_cases(name: &str, law: Law, make: impl Fn(u64) -> (String, Case)) {
    let mut found = Vec::new();
    let seeds = seeds();
    for seed in seeds.clone() {
        let (label, c) = make(seed);
        if let Some((at, failure)) = first_failure(seed, &c, law) {
            found.push((seed, label, c, at, failure));
        }
    }
    let Some((seed, _, c, at, failure)) = found.first() else {
        return;
    };
    let mut report = format!(
        "{name}: {} of {} seeds disagree\n",
        found.len(),
        seeds.end - seeds.start
    );
    let (small, why) = shrink(*seed, c, law, *at, failure);
    report.push_str(&format!("seed {seed}, shrunk: {why}\n{}", small.program()));
    for q in &small.queries {
        report.push_str(&format!("query: {}\n", q.text()));
    }
    report.push('\n');
    for (i, (seed, label, c, _, failure)) in found.iter().enumerate() {
        report.push_str(&format!("seed {seed}: {label}{failure}\n"));
        if 0 < i && i < SHOWN {
            report.push_str(&format!("{}\n\n", c.program()));
        }
    }
    panic!("{report}");
}

/// Returns the texts of `q` a law compares across the engine and the reference.
fn texts(q: &Query) -> Vec<String> {
    let mut out = vec![q.text()];
    out.extend(q.counts().map(|c| c.whole));
    out
}

#[test]
fn generated_cases_check_and_load() {
    for seed in seeds() {
        let c = case(seed, QUERIES);
        let fx = panic::catch_unwind(AssertUnwindSafe(|| reference(&c)))
            .unwrap_or_else(|p| panic!("seed {seed}: {}", panic_message(&*p)));
        for q in &c.queries {
            let mut all = texts(q);
            all.extend(q.by_ctor().unwrap_or_default());
            if let Some(split) = q.by_predicate() {
                all.push(split.whole);
                all.extend(split.parts);
            }
            all.extend(q.counts().into_iter().flat_map(|c| c.parts));
            for t in all {
                let got = panic::catch_unwind(AssertUnwindSafe(|| reference_eval(&fx, &t)))
                    .unwrap_or_else(|p| Got::Refused(panic_message(&*p)));
                if let Got::Refused(why) = got {
                    panic!("seed {seed}: the reference refuses `{t}`: {why}");
                }
            }
        }
    }
}

/// The least share, in percent, of generated queries with a non-empty result.
const NONEMPTY: usize = 45;

/// Returns whether a query holds a form.
type Holds = fn(&Forms) -> bool;

/// The least share, in percent, of generated queries holding each form.
const FORMS: [(&str, Holds, usize); 5] = [
    ("fix", |f| f.fix, 8),
    ("group", |f| f.group, 12),
    ("match", |f| f.matches, 35),
    ("join", |f| f.join, 65),
    ("float", |f| f.float, 12),
];

#[test]
fn generated_queries_cover_forms() {
    let (mut total, mut nonempty) = (0, 0);
    let mut held = [0; FORMS.len()];
    for seed in seeds() {
        let c = case(seed, QUERIES);
        let fx = reference(&c);
        for q in &c.queries {
            total += 1;
            if matches!(reference_eval(&fx, &q.text()), Got::Rows(r) if !r.is_empty()) {
                nonempty += 1;
            }
            let forms = q.forms();
            for (k, (_, holds, _)) in FORMS.iter().enumerate() {
                held[k] += usize::from(holds(&forms));
            }
        }
    }
    let mut short = Vec::new();
    if nonempty * 100 < NONEMPTY * total {
        short.push(format!(
            "non-empty {nonempty} of {total}, under {NONEMPTY}%"
        ));
    }
    for (k, (name, _, floor)) in FORMS.iter().enumerate() {
        if held[k] * 100 < floor * total {
            short.push(format!("{name} {} of {total}, under {floor}%", held[k]));
        }
    }
    assert!(
        short.is_empty(),
        "seeds {:?}: {}",
        seeds(),
        short.join("; ")
    );
}

fn differential(env: &Env, q: &Query) -> Option<String> {
    texts(q).into_iter().find_map(|t| {
        let (want, got) = (reference_eval(env.fx, &t), engine_eval(env.db, &t));
        (!got.agrees(&want, env.tolerance))
            .then(|| format!("`{t}`: the engine gives {got:?}, the reference {want:?}"))
    })
}

#[test]
fn a_nan_agrees_with_a_nan() {
    assert!(within(&Value::F64(f64::NAN), &Value::F64(f64::NAN), 0.0));
    assert!(!within(&Value::F64(f64::NAN), &Value::F64(0.0), 0.0));
}

#[test]
fn a_rejection_both_sides_give_alike_is_agreement() {
    let c = Case::of_text(
        "module app\ntype Small = { id: I64, name: Text }\n".to_owned(),
        "module app\nadd module app\nadd type Small = { id: I64, name: Text }\n".to_owned(),
        Vec::new(),
        vec![r#"read_csv("nope.csv", Small)"#.to_owned()],
    );
    assert_eq!(
        None,
        first_failure(0, &c, differential).map(|(_, f)| f.to_string())
    );
}

#[test]
fn engine_equals_reference() {
    sweep("differential", differential);
}

/// The scripts of the conformance suite a case can be made of.
fn scripts() -> &'static [Script] {
    static SCRIPTS: OnceLock<Vec<Script>> = OnceLock::new();
    SCRIPTS.get_or_init(|| {
        corpus::scripts(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance"))
    })
}

/// The least number of conformance scripts a case can be made of.
const SCRIPTS: usize = 40;

#[test]
fn corpus_mutants_equal_reference() {
    let all = scripts();
    assert!(
        all.len() >= SCRIPTS,
        "{} conformance scripts make cases, under {SCRIPTS}",
        all.len()
    );
    sweep_cases("corpus mutation", differential, |seed| {
        let s = &all[Rng::new(seed).below(all.len() as u64) as usize];
        (format!("{}: ", s.name), s.case(seed, QUERIES))
    });
}

#[test]
fn sum_is_union_of_constructors() {
    sweep("constructor partition", |env, q| {
        let parts = q.by_ctor()?;
        let text = q.text();
        let whole = engine_eval(env.db, &text);
        let union = Got::union(parts.iter().map(|p| engine_eval(env.db, p)).collect());
        (!whole.agrees(&union, env.tolerance))
            .then(|| format!("`{text}` gives {whole:?}, the union of {parts:?} gives {union:?}"))
    });
}

#[test]
fn predicate_partitions_bindings() {
    sweep("predicate partition", |env, q| {
        let split = q.by_predicate()?;
        let whole = engine_eval(env.db, &split.whole);
        let union = Got::union(split.parts.iter().map(|p| engine_eval(env.db, p)).collect());
        if !whole.agrees(&union, env.tolerance) {
            return Some(format!(
                "`{}` gives {whole:?}, the union of {:?} gives {union:?}",
                split.whole, split.parts
            ));
        }
        let counts = q.counts()?;
        let total = engine_eval(env.db, &counts.whole);
        let sum = Got::sum(
            counts
                .parts
                .iter()
                .map(|p| engine_eval(env.db, p))
                .collect(),
        );
        (!total.agrees(&sum, 0.0)).then(|| {
            format!(
                "`{}` gives {total:?}, the sum of {:?} gives {sum:?}",
                counts.whole, counts.parts
            )
        })
    });
}

const REWRITES: usize = 12;

fn flag(r: &mut Rewrites, i: usize) -> &mut bool {
    match i {
        0 => &mut r.index_scan,
        1 => &mut r.index_join,
        2 => &mut r.hash_join,
        3 => &mut r.fuse_probe,
        4 => &mut r.static_route,
        5 => &mut r.prune,
        6 => &mut r.unswitch,
        7 => &mut r.group_agg,
        8 => &mut r.semi_naive,
        9 => &mut r.range_probe,
        10 => &mut r.early_stop,
        _ => &mut r.flatten,
    }
}

/// Returns every rewrite off, one rewrite drawn from `seed` and `text` turned off alone and
/// turned on alone, and two subsets drawn from them.
fn settings(seed: u64, text: &str) -> Vec<Rewrites> {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    let mut rng = Rng::new(seed ^ hasher.finish());
    let i = rng.below(REWRITES as u64) as usize;
    let mut off = Rewrites::ALL;
    *flag(&mut off, i) = false;
    let mut on = Rewrites::NONE;
    *flag(&mut on, i) = true;
    let mut out = vec![Rewrites::NONE, off, on];
    for _ in 0..2 {
        let mut r = Rewrites::NONE;
        for i in 0..REWRITES {
            *flag(&mut r, i) = rng.below(2) == 0;
        }
        out.push(r);
    }
    out
}

#[test]
fn rewrites_keep_results() {
    sweep("plan toggles", |env, q| {
        for t in texts(q) {
            let want = engine_eval(env.db, &t);
            for r in settings(env.seed, &t) {
                let got = with_rewrites(r, || engine_eval(env.db, &t));
                if !got.agrees(&want, env.tolerance) {
                    return Some(format!(
                        "`{t}` gives {got:?} under {r:?}, {want:?} under every rewrite"
                    ));
                }
            }
        }
        None
    });
}

/// Walks per default run, [`profile::deep`] sized, unless `ADTB_DIFF_CASES` says otherwise.
const WALKS: u64 = 4;

/// The migrations of each walk of a default sweep.
const STEPS: usize = 2;

/// The migrations of each walk of a sweep over every operator, enough for the operators that
/// take back what an earlier one left.
const LONG_STEPS: usize = 6;

/// The operators whose laws depend on how removals and constraint drops stage.
const STAGED: [Operator; 3] = [
    Operator::RemoveConstructor,
    Operator::DropUnique,
    Operator::DropCheck,
];

/// Returns the first query of `case` on which `db` disagrees with the reference, named by
/// where in the walk it runs.
fn walk_check(seed: u64, db: &Database<SimRuntime>, case: &Case, at: &str) -> Option<String> {
    let fx = reference(case);
    let env = Env {
        seed,
        tolerance: case.tolerance,
        fx: &fx,
        db,
    };
    case.queries
        .iter()
        .find_map(|q| fails(&env, differential, q))
        .map(|m| format!("{at}: {m}"))
}

/// Returns the first disagreement of the engine and the reference along walk `w`: before its
/// first migration, then after each migration and after the writes that follow it.
fn walked(seed: u64, w: &Walk) -> Option<String> {
    let log = w.log();
    panic::catch_unwind(AssertUnwindSafe(|| {
        let mut sim = Sim::new(seed, Config::default());
        let found = process(&mut sim, |rt| {
            let db = open(rt, &log[..1]);
            write(&db, &w.start.load);
            let found = walk_check(seed, &db, &w.start, "before the walk");
            close(db);
            found
        });
        if found.is_some() {
            return found;
        }
        for (k, step) in w.steps.iter().enumerate() {
            let found = process(&mut sim, |rt| {
                let db = open(rt, &log[..k + 2]);
                let at = format!("after migration {}", k + 2);
                let mut found = walk_check(seed, &db, &step.migrated, &at);
                if found.is_none() {
                    write(&db, &step.writes);
                    found = walk_check(seed, &db, &step.written, &format!("{at} and its writes"));
                }
                close(db);
                found
            });
            if found.is_some() {
                return found;
            }
        }
        None
    }))
    .unwrap_or_else(|p| Some(format!("panics: {}", panic_message(&*p))))
}

/// Returns the first disagreement of `db` at the version `log` leaves with the reference
/// holding `case`.
fn checked(sim: &mut Sim, seed: u64, log: &[String], case: &Case, at: &str) -> Option<String> {
    process(sim, |rt| {
        let db = open(rt, log);
        let found = walk_check(seed, &db, case, at);
        close(db);
        found
    })
}

/// Returns the first law of its operators that migration `k` of walk `w` breaks, from the
/// database the walk holds before it: its revert, which restores the database before it or is
/// refused, then the revert of that revert, which applies the migration again.
fn step_laws(seed: u64, w: &Walk, k: usize) -> Option<String> {
    let log = w.log();
    let step = &w.steps[k];
    let mut sim = Sim::new(seed, Config::default());
    process(&mut sim, |rt| {
        let db = open(rt, &log[..1]);
        write(&db, &w.start.load);
        close(db);
    });
    for (j, before) in w.steps[..k].iter().enumerate() {
        process(&mut sim, |rt| {
            let db = open(rt, &log[..j + 2]);
            write(&db, &before.writes);
            close(db);
        });
    }
    let mut at = log[..k + 2].to_vec();
    at.push(step.inverse.revert().to_owned());
    let reverted = match &step.inverse {
        Inverse::Refused { .. } => {
            return process(&mut sim, |rt| match try_open(rt, &at) {
                Err(OpenError::MigrationRefused { version, .. }) if version == at.len() as u64 => {
                    None
                }
                Err(e) => Some(format!("its revert fails, not refused: {e:?}")),
                Ok(db) => {
                    close(db);
                    Some("its revert applies, though it discards data with no inverse".into())
                }
            });
        }
        Inverse::Restores { reverted, .. } => reverted,
    };
    let found = checked(&mut sim, seed, &at, reverted, "its revert");
    if found.is_some() {
        return found;
    }
    at.push(format!("module app\nrevert {:04}\n", at.len()));
    checked(
        &mut sim,
        seed,
        &at,
        &step.migrated,
        "the revert of its revert",
    )
}

/// A bundle of the pinned-query law, pinned at the version before a migration.
struct Pinned {
    name: String,
    text: String,
    registration: Registration,
    /// The query it registers, by its position among the case's, else none for the bundle whose
    /// one transaction writes every row of a constructor relation again unchanged.
    query: Option<(usize, String)>,
}

/// Returns the bundles of the pinned-query law at the version `log` leaves that translate to the
/// version `to` leaves: one for each query of `case` that checks there, and one for each
/// constructor relation of module `app` whose transaction writes each of its rows again, by
/// upsert, as a value stating its every field.
fn law_bundles(log: &[String], to: &[String], case: &Case) -> Result<Vec<Pinned>, String> {
    use adtb_lang::check::check_adhoc;
    use adtb_lang::parse::{Options, parse_adhoc};
    let replayed = |log: &[String]| {
        let source = adtb_lang::signature::replay(log.iter().map(String::as_str))
            .map_err(|e| format!("the log does not replay: {e:?}"))?;
        let (schema, program) = source
            .check()
            .map_err(|e| format!("the version does not check: {e:?}"))?;
        Ok::<_, String>((source, schema, program))
    };
    let (source, schema, program) = replayed(log)?;
    let later = replayed(to)?;
    let translates = |body: &str| {
        let p = adtb_lang::translate::Pinned::new(
            &schema, &source, &later.1, &later.0, &later.2, false,
        );
        p.is_ok_and(|p| normal(&schema, &program, body).is_some_and(|q| p.query(&q).is_ok()))
    };
    let hash: String = adtb::catalog::version_hash(&schema, &source)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let header = |name: &str| {
        format!(
            "bundle {name} pins {} ({hash}) edition \"2026-10\"\nimport app\n",
            log.len()
        )
    };
    let mut texts = Vec::new();
    for (i, q) in case.queries.iter().enumerate() {
        let body = q.text();
        let Ok(items) = parse_adhoc(&body, Options::default()).ok() else {
            continue;
        };
        let Ok(adhoc) = check_adhoc(&schema, &program, &items, &Default::default()) else {
            continue;
        };
        let [item] = adhoc.items.as_slice() else {
            continue;
        };
        let ty = schema.show_ty(adhoc.terms.ty(*item));
        let indented = body.replace('\n', "\n  ");
        let item = format!("query q () -> {ty} =\n  {indented}\n");
        if !translates(&item) {
            continue;
        }
        let name = format!("law{i}-1.0");
        let text = format!("{}{item}", header(&name));
        texts.push((name, text, Some((i, body))));
    }
    for (j, write) in rewrites(&schema).into_iter().enumerate() {
        let item = format!("txn w () -> I64 =\n  transact\n    {write}\n    return 0\n");
        if !translates(&item) {
            continue;
        }
        let name = format!("put{j}-1.0");
        let text = format!("{}{item}", header(&name));
        texts.push((name, text, None));
    }
    texts
        .into_iter()
        .map(|(name, text, query)| {
            let registration = Registration::bundle(&text)
                .map_err(|e| format!("`{text}` is no bundle file: {e:?}"))?;
            Ok(Pinned {
                name,
                text,
                registration,
                query,
            })
        })
        .collect()
}

/// Returns the normal form of the one query or transaction of bundle body `item`, which imports
/// module `app`, over `schema` checked as `program`.
fn normal(
    schema: &adtb_lang::schema::Schema,
    program: &adtb_lang::check::Program,
    item: &str,
) -> Option<adtb_lang::norm::NormalQuery> {
    use adtb_lang::parse::{Options, parse_bundle_body};
    use adtb_lang::signature::{self, Item};
    let text = format!("import app\n{item}");
    let (imports, items) = parse_bundle_body(&text, Options::default()).ok().ok()?;
    let vis = signature::visible(schema, &imports).ok()?;
    let calls = signature::calls(schema, program, &vis, &imports, &items).ok()?;
    match &calls.first()?.item {
        Item::Query(_, q) => adtb_lang::norm::query(schema, program, q).ok(),
        Item::Txn(_, t) => adtb_lang::norm::txn(schema, program, t).ok(),
        Item::Fn(..) => None,
    }
}

/// Returns, for each constructor relation of module `app` in `schema`, an upsert of each of its
/// rows as a value stating its every field, which leaves the row as it is.
fn rewrites(schema: &adtb_lang::schema::Schema) -> Vec<String> {
    let mut out = Vec::new();
    for r in &schema.relations {
        if schema.module(r.module).name != "app" {
            continue;
        }
        let t = schema.ty(r.elem_def);
        let record = matches!(
            t.kind,
            adtb_lang::schema::TypeDefKind::Sum { record: true, .. }
        );
        for c in t.ctors() {
            let fields: Vec<String> = t
                .header()
                .iter()
                .chain(&c.fields)
                .map(|f| format!("{0}: x.{0}", f.name))
                .collect();
            let rel = if record {
                r.name.clone()
            } else {
                format!("{}.{}", r.name, c.name)
            };
            out.push(format!(
                "insert {rel} from (from x in {rel} select {{ {} }}) unless conflict on key else update old set new",
                fields.join(", ")
            ));
        }
    }
    out
}

/// Returns each row of `rows`, its value debug printed, sorted.
fn rows_of(rows: adtb::DynRows) -> Result<Vec<String>, Error> {
    let mut out = rows
        .map(|r| r.map(|v| format!("{v:?}")))
        .collect::<Result<Vec<_>, _>>()?;
    out.sort();
    Ok(out)
}

/// Returns the rows `text` returns on `db`, sorted, or `None` when it fails or returns other
/// than one result.
fn ad_hoc(db: &Database<SimRuntime>, text: &str) -> Option<Vec<String>> {
    let mut got = db.query_text(text, &[], &CallOptions::default()).ok()?;
    (got.len() == 1).then(|| rows_of(got.remove(0)).ok())?
}

/// Returns the rows of every relation of module `app` on `db`, as one line each, sorted.
fn contents(db: &Database<SimRuntime>, names: &[String]) -> Vec<String> {
    names
        .iter()
        .flat_map(|r| {
            ad_hoc(db, &format!("from x in {r} select x"))
                .unwrap_or_default()
                .into_iter()
                .map(move |row| format!("{r}: {row}"))
        })
        .collect()
}

/// Open `log` registering every bundle of `laws`, each one the library translates: one the
/// engine refuses as stale, or as reading a constructor enabled since its pin, is dropped from
/// `laws` and the open retried. Fail when the engine refuses one as untranslatable.
fn open_translated(
    rt: &SimRuntime,
    log: &[String],
    laws: &mut Vec<Pinned>,
) -> Result<Database<SimRuntime>, String> {
    loop {
        let options = OpenOptions {
            create: Some(CreateOptions::default()),
            registrations: laws.iter().map(|l| l.registration.clone()).collect(),
            ..OpenOptions::default()
        };
        let opened = Database::open_in(
            rt.clone(),
            "differential.adt",
            &Log::new(log.to_vec()),
            &options,
        );
        match opened {
            Ok(db) => return Ok(db),
            Err(OpenError::RegistrationRefused {
                bundle,
                reason:
                    Error::Rejected {
                        reason: RejectReason::BundleStale { .. },
                    },
            }) if laws.iter().any(|l| l.name == bundle) => laws.retain(|l| l.name != bundle),
            Err(OpenError::DowngradeRefused { cause })
                if laws
                    .iter()
                    .any(|l| cause.starts_with(&format!("bundle {},", l.name))) =>
            {
                laws.retain(|l| !cause.starts_with(&format!("bundle {},", l.name)));
            }
            Err(e) => return Err(format!("a bundle pinned before it does not open: {e:?}")),
        }
    }
}

/// Returns the rows of a call of the one query or transaction of bundle `name` on `db`.
fn called(db: &Database<SimRuntime>, name: &str) -> Result<Vec<String>, String> {
    let bound = db
        .registered(name)
        .map_err(|e| format!("{name} does not bind: {e:?}"))?;
    match bound.call(1, &[], &CallOptions::default()) {
        Ok(Called::Rows(rows)) => rows_of(rows).map_err(|e| format!("{name} fails: {e:?}")),
        Ok(Called::Committed(c)) => rows_of(c.value).map_err(|e| format!("{name} fails: {e:?}")),
        Err(e) => Err(format!("{name} fails: {e:?}")),
    }
}

/// The queries the pinned-query law translated, by the operators of the migration translated
/// across, over one sweep.
static TRANSLATED: std::sync::Mutex<Vec<(Operator, usize)>> = std::sync::Mutex::new(Vec::new());

/// Count `n` translated queries for each operator of `ops`.
fn translated(ops: &[Operator], n: usize) {
    let mut counts = TRANSLATED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for &op in ops {
        match counts.iter_mut().find(|(o, _)| *o == op) {
            Some((_, c)) => *c += n,
            None => counts.push((op, n)),
        }
    }
}

/// The operators the translation of a pinned query crosses rather than refuses.
const TRANSLATES: [Operator; 18] = [
    Operator::AddRelation,
    Operator::AddConstructor,
    Operator::Enable,
    Operator::Deprecate,
    Operator::Disable,
    Operator::RenameType,
    Operator::RenameRelation,
    Operator::RenameConstructor,
    Operator::AddField,
    Operator::DropField,
    Operator::RenameField,
    Operator::AddIndex,
    Operator::DropIndex,
    Operator::AddUnique,
    Operator::AddDefault,
    Operator::DropDefault,
    Operator::AddCheck,
    Operator::RedefineFn,
];

/// Returns where a bundle of walk `w` pinned at the version before migration `k` breaks the law
/// of a pinned query once translated across it: over the data before it, carried by the
/// migration, a query returns what it returns at its pin, and a transaction writing every row
/// again unchanged leaves the data as it is; over the data the walk's writes leave after it, a
/// query returns what it returns at its pin over that data reverted. A query that fails at its
/// pin, and data whose revert is refused, hold no law.
fn pinned_law(seed: u64, w: &Walk, k: usize) -> Option<String> {
    let log = w.log();
    let step = &w.steps[k];
    let before = if k == 0 {
        &w.start
    } else {
        &w.steps[k - 1].written
    };
    let mut laws = match law_bundles(&log[..=k], &log[..k + 2], before) {
        Ok(laws) => laws,
        Err(e) => return Some(e),
    };
    let mut sim = Sim::new(seed, Config::default());
    process(&mut sim, |rt| {
        let db = open(rt, &log[..1]);
        write(&db, &w.start.load);
        close(db);
    });
    for (j, step) in w.steps[..k].iter().enumerate() {
        process(&mut sim, |rt| {
            let db = open(rt, &log[..j + 2]);
            write(&db, &step.writes);
            close(db);
        });
    }
    let expected: Vec<(String, Vec<String>)> = process(&mut sim, |rt| {
        let db = open(rt, &log[..=k]);
        let out = laws
            .iter()
            .filter_map(|l| Some((l.name.clone(), ad_hoc(&db, &l.query.as_ref()?.1)?)))
            .collect();
        close(db);
        out
    });
    let names = match relations(&log[..k + 2]) {
        Ok(names) => names,
        Err(e) => return Some(e),
    };
    let after = process(&mut sim, |rt| {
        let db = open_translated(&rt, &log[..k + 2], &mut laws)?;
        if let Some(found) = carried(&db, &laws, &expected, &names) {
            close(db);
            return Err(found);
        }
        let queries = laws.iter().filter(|l| l.query.is_some()).count();
        translated(&step.operators, queries);
        write(&db, &step.writes);
        let rewrites: Vec<&Pinned> = laws.iter().filter(|l| l.query.is_none()).collect();
        if let Some(found) = rewritten(&db, &rewrites, &names) {
            close(db);
            return Err(format!("after the writes, {found}"));
        }
        let got: Vec<(String, Result<Vec<String>, String>)> = laws
            .iter()
            .filter(|l| l.query.is_some())
            .map(|l| (l.name.clone(), called(&db, &l.name)))
            .collect();
        close(db);
        Ok(got)
    });
    let got = match after {
        Ok(got) => got,
        Err(found) => return Some(found),
    };
    let Inverse::Restores { revert, .. } = &step.inverse else {
        return None;
    };
    let mut at = log[..k + 2].to_vec();
    at.push(revert.clone());
    process(&mut sim, |rt| match try_open(rt, &at) {
        Err(OpenError::MigrationRefused { .. }) => None,
        Err(e) => Some(format!("its revert after the writes does not open: {e:?}")),
        Ok(db) => {
            let found = got.iter().find_map(|(name, got)| {
                let (i, q) = laws.iter().find(|l| l.name == *name)?.query.as_ref()?;
                let want = ad_hoc(&db, q)?;
                match got {
                    Ok(got) if *got == want => None,
                    Ok(got) => Some(format!(
                        "query {i} pinned before it returns {got:?} after the writes, not \
                         {want:?} as the data reverted: {q}"
                    )),
                    Err(e) => Some(format!(
                        "query {i} pinned before it fails after the writes: {e}: {q}"
                    )),
                }
            });
            close(db);
            found
        }
    })
}

/// Returns the names of the relations of module `app` at the version `log` leaves.
fn relations(log: &[String]) -> Result<Vec<String>, String> {
    let source = adtb_lang::signature::replay(log.iter().map(String::as_str))
        .map_err(|e| format!("the log does not replay: {e:?}"))?;
    let schema = source
        .schema()
        .map_err(|e| format!("the version does not build: {e:?}"))?;
    Ok(schema
        .relations
        .iter()
        .filter(|r| schema.module(r.module).name == "app")
        .map(|r| r.name.clone())
        .collect())
}

/// Returns where a bundle of `laws` registered on `db` breaks the law over the data carried by
/// the migration: a query returns other than its `expected` rows at its pin, or a transaction
/// changes the rows of the relations `names`.
fn carried(
    db: &Database<SimRuntime>,
    laws: &[Pinned],
    expected: &[(String, Vec<String>)],
    names: &[String],
) -> Option<String> {
    laws.iter().find_map(|l| match &l.query {
        Some((i, q)) => {
            let want = &expected.iter().find(|(n, _)| *n == l.name)?.1;
            match called(db, &l.name) {
                Ok(got) if got == *want => None,
                Ok(got) => Some(format!(
                    "query {i} pinned before it returns {got:?}, not {want:?}: {q}"
                )),
                Err(e) => Some(format!("query {i} pinned before it fails: {e}: {q}")),
            }
        }
        None => rewritten(db, &[l], names),
    })
}

/// Returns where a transaction of `rewrites`, each writing rows again unchanged, changes the rows
/// of the relations `names` on `db`.
fn rewritten(db: &Database<SimRuntime>, rewrites: &[&Pinned], names: &[String]) -> Option<String> {
    rewrites.iter().find_map(|l| {
        let before = contents(db, names);
        if let Err(e) = called(db, &l.name) {
            return Some(format!("a rewrite pinned before it fails: {e}"));
        }
        let after = contents(db, names);
        (before != after).then(|| {
            format!(
                "a rewrite pinned before it changes the rows {before:?} to {after:?}: {}",
                l.text
            )
        })
    })
}

/// Returns the first law of a schema operator that walk `w` breaks, by migration.
fn laws(seed: u64, w: &Walk) -> Option<String> {
    w.steps.iter().enumerate().find_map(|(k, step)| {
        panic::catch_unwind(AssertUnwindSafe(|| {
            step_laws(seed, w, k).or_else(|| pinned_law(seed, w, k))
        }))
        .unwrap_or_else(|p| Some(format!("panics: {}", panic_message(&*p))))
        .map(|m| format!("migration {} of {:?}: {m}", k + 2, step.operators))
    })
}

/// Walks per default run of a law sweep, [`profile::deep`] sized, unless `ADTB_DIFF_CASES` says
/// otherwise. An open applying a revert takes about two seconds.
const LAW_WALKS: u64 = 2;

/// Sweep `walks` walks of `steps` migrations of one run drawing no operator of `skip`, each
/// checked by `check`.
fn sweep_walks(
    name: &str,
    walks: u64,
    steps: usize,
    skip: &[Operator],
    check: fn(u64, &Walk) -> Option<String>,
) {
    sweep_seeds(name, seeds_of(walks).collect(), steps, skip, check);
}

/// Sweep the walks of `seeds` as [`sweep_walks`] does.
fn sweep_seeds(
    name: &str,
    seeds: Vec<u64>,
    steps: usize,
    skip: &[Operator],
    check: fn(u64, &Walk) -> Option<String>,
) {
    let mut found = Vec::new();
    for &seed in &seeds {
        let w = walk_of(seed, steps, QUERIES, skip);
        if let Some(m) = check(seed, &w) {
            found.push((seed, m, w));
        }
    }
    let mut report = format!("{name}: {} of {} seeds fail\n", found.len(), seeds.len());
    for (i, (seed, message, w)) in found.iter().enumerate() {
        report.push_str(&format!("seed {seed}: {message}\n"));
        if i < SHOWN {
            report.push_str(&w.start.program());
            for (k, step) in w.steps.iter().enumerate() {
                report.push_str(&format!("-- migration {}\n{}", k + 2, step.migration));
                for t in &step.writes {
                    report.push_str(&format!("{t}\n"));
                }
            }
            report.push('\n');
        }
    }
    assert!(found.is_empty(), "{report}");
}

#[test]
fn schema_walks_keep_engine_equal_to_reference() {
    sweep_walks("schema walk", WALKS, STEPS, &STAGED, walked);
}

#[test]
fn schema_operators_keep_their_laws() {
    sweep_walks("operator laws", LAW_WALKS, STEPS, &STAGED, laws);
    let counts = TRANSLATED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let vacuous: Vec<Operator> = counts
        .iter()
        .filter(|&&(op, n)| n == 0 && TRANSLATES.contains(&op))
        .map(|&(op, _)| op)
        .collect();
    assert!(
        vacuous.is_empty(),
        "no query pinned before {vacuous:?} is translated across it in seeds {:?}",
        seeds_of(LAW_WALKS)
    );
}

#[test]
#[ignore = "draws operators the engine does not yet apply as their laws state"]
fn every_schema_walk_keeps_engine_equal_to_reference() {
    sweep_walks("every schema walk", WALKS, LONG_STEPS, &[], walked);
}

#[test]
#[ignore = "draws operators the engine does not yet apply as their laws state"]
fn every_operator_keeps_its_laws() {
    sweep_walks("every operator's laws", LAW_WALKS, LONG_STEPS, &[], laws);
}

/// The walks whose operators [`walks_draw_every_operator`] counts.
const DRAWS: u64 = 200;

#[test]
fn walks_draw_every_operator() {
    let mut drawn = Vec::new();
    for seed in 0..DRAWS {
        drawn.extend(walk(seed, 8, 0).steps.into_iter().flat_map(|s| s.operators));
    }
    let missing: Vec<&Operator> = Operator::ALL
        .iter()
        .filter(|o| !drawn.contains(o))
        .collect();
    assert!(
        missing.is_empty(),
        "no walk of seeds 0..{DRAWS} draws {missing:?}"
    );
}

/// Returns the constructors a `merge constructor` line of `migration` merges.
fn merged(migration: &str) -> Vec<String> {
    migration
        .lines()
        .filter_map(|l| l.strip_prefix("merge constructor "))
        .filter_map(|l| l.split_once(".(")?.1.split_once(')'))
        .flat_map(|(cs, _)| cs.split(" | ").map(str::to_owned).collect::<Vec<_>>())
        .collect()
}

/// The operators other than adding a check and merging constructors.
fn beside_checked_merges() -> Vec<Operator> {
    Operator::ALL
        .into_iter()
        .filter(|o| ![Operator::AddCheck, Operator::Merge].contains(o))
        .collect()
}

/// Returns the first `want` seeds of `seeds` whose walk of `steps` migrations and `queries`
/// queries, drawing no operator of `skip`, merges a constructor a check ranges over alone.
fn checked_merges(
    seeds: Range<u64>,
    want: usize,
    steps: usize,
    queries: usize,
    skip: &[Operator],
) -> Vec<u64> {
    let mut out = Vec::new();
    for seed in seeds {
        if out.len() == want {
            break;
        }
        let w = walk_of(seed, steps, queries, skip);
        let mut before = w.start.program();
        for step in &w.steps {
            let checked = merged(&step.migration).iter().any(|c| {
                let on = format!(".{c} (");
                before
                    .lines()
                    .any(|l| l.contains("check ") && l.contains(&on))
            });
            if checked {
                out.push(seed);
                break;
            }
            before = step.written.program();
        }
    }
    out
}

#[test]
fn walks_merge_a_constructor_a_check_ranges_over() {
    assert!(
        !checked_merges(0..DRAWS, 1, 8, 0, &[]).is_empty(),
        "no walk of seeds 0..{DRAWS} merges a constructor a check ranges over"
    );
}

/// The migrations of each walk of a sweep over merges carrying checks: one adding a check, then
/// one merging its constructor.
const CHECKED_STEPS: usize = 2;

/// Returns the first walks, [`profile::deep`] sized, of seeds 0..[`DRAWS`] drawing only checks
/// and merges that merge a constructor a check ranges over alone.
fn checked_merge_seeds() -> Vec<u64> {
    let want = profile::deep(1) as usize;
    let seeds = checked_merges(
        0..DRAWS,
        want,
        CHECKED_STEPS,
        QUERIES,
        &beside_checked_merges(),
    );
    assert!(
        !seeds.is_empty(),
        "a walk of seeds 0..{DRAWS} merges a checked constructor"
    );
    seeds
}

#[test]
fn merges_carrying_checks_keep_engine_equal_to_reference() {
    let skip = beside_checked_merges();
    sweep_seeds(
        "checked merge",
        checked_merge_seeds(),
        CHECKED_STEPS,
        &skip,
        walked,
    );
}

#[test]
fn merges_carrying_checks_keep_their_laws() {
    let skip = beside_checked_merges();
    sweep_seeds(
        "checked merge laws",
        checked_merge_seeds(),
        CHECKED_STEPS,
        &skip,
        laws,
    );
}
