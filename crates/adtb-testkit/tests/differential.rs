//! The engine over generated cases: its results against the reference interpreter's, a query
//! against its partitions by constructor and by predicate, and every planner rewrite setting
//! against the default. A disagreement is shrunk to a small program before it is reported.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::OnceLock;

use adtb::plan::{Rewrites, with_rewrites};
use adtb::{CallOptions, CreateOptions, Database, Error, Log, OpenOptions, RejectReason};
use adtb_lang::textual;
use adtb_reference::{Abort, Value};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::Rng;
use adtb_testkit::cases::{Case, Forms, Query, Walk, case, walk};
use adtb_testkit::corpus::{self, Script};
use adtb_testkit::fixture::Fixture;

/// Cases per run unless `ADTB_DIFF_CASES` says otherwise.
const CASES: u64 = 12;

/// The queries of each case, over one database.
const QUERIES: usize = 24;

/// Returns the seeds of one run of a drawn or corpus case.
fn seeds() -> Range<u64> {
    seeds_of(CASES)
}

/// Returns the seeds of one run, from `ADTB_DIFF_SEED`, else zero, `ADTB_DIFF_CASES` of them,
/// else `cases`.
fn seeds_of(cases: u64) -> Range<u64> {
    let var = |name: &str, default: u64| {
        std::env::var(name).map_or(default, |v| {
            v.parse()
                .unwrap_or_else(|_| panic!("{name} is not a number: {v}"))
        })
    };
    let first = var("ADTB_DIFF_SEED", 0);
    first..first + var("ADTB_DIFF_CASES", cases)
}

/// The disagreements printed whole with their program; the rest are listed by seed.
const SHOWN: usize = 3;

/// The evaluations of a law a shrink of the queries of a case may try.
const QUERY_TRIES: usize = 400;

/// The engine databases a shrink of the database of a case may open.
const DATABASE_TRIES: usize = 24;

/// How a query ended: its rows, ascending, or a fault, or a refusal of another kind.
#[derive(Clone, Debug, PartialEq)]
enum Got {
    Rows(Vec<Value>),
    Faulted,
    Refused(String),
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
        (Value::F64(x), Value::F64(y)) => x == y || (x - y).abs() <= tolerance,
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
        Err(a) => Got::Refused(format!("{a:?}")),
    }
}

fn engine_eval(db: &Database<SimRuntime>, text: &str) -> Got {
    let faulted = |e: &Error| {
        matches!(
            e,
            Error::Rejected {
                reason: RejectReason::Faulted { .. }
            }
        )
    };
    let mut results = match db.query_text(text, &[], &CallOptions::default()) {
        Ok(r) => r,
        Err(e) if faulted(&e) => return Got::Faulted,
        Err(e) => return Got::Refused(e.to_string()),
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
            Err(e) if faulted(&e) => return Got::Faulted,
            Err(e) => return Got::Refused(e.to_string()),
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
    let options = OpenOptions {
        create: Some(CreateOptions::default()),
        ..OpenOptions::default()
    };
    Database::open_in(rt, "differential.adt", &Log::new(log.to_vec()), &options).unwrap_or_else(
        |e| {
            let last = log.last().map_or("", String::as_str);
            panic!(
                "the engine does not open at migration {}: {e:?}\n{last}",
                log.len()
            )
        },
    )
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

/// Returns the disagreement `law` shows on `q`, a panic included.
fn fails(env: &Env, law: Law, q: &Query) -> Option<String> {
    match panic::catch_unwind(AssertUnwindSafe(|| law(env, q))) {
        Ok(m) => m,
        Err(p) => Some(format!("panics: {}", panic_message(&*p))),
    }
}

/// Runs `body` over the reference and the engine holding `case`, a panic reported as its
/// message.
fn with_case<T>(seed: u64, case: &Case, body: impl FnOnce(&Env) -> T) -> Result<T, String> {
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
    .map_err(|p| format!("panics: {}", panic_message(&*p)))
}

/// Returns the first query of `case` that breaks `law`, by index, and the disagreement.
fn first_failure(seed: u64, case: &Case, law: Law) -> Option<(Option<usize>, String)> {
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

/// Returns `q` shrunk greedily against `law` over the database `env` reads.
fn shrink_query(env: &Env, law: Law, q: Query, message: String) -> (Query, String) {
    let (mut q, mut message) = (q, message);
    let mut tries = 0;
    'outer: while tries < QUERY_TRIES {
        for s in q.shrinks() {
            tries += 1;
            if let Some(m) = fails(env, law, &s) {
                (q, message) = (s, m);
                continue 'outer;
            }
            if tries >= QUERY_TRIES {
                break;
            }
        }
        break;
    }
    (q, message)
}

/// Returns `case` shrunk against `law`, from its failing query `at`: that query alone, shrunk,
/// over a database shrunk, then shrunk again over that database.
fn shrink(seed: u64, case: &Case, law: Law, at: Option<usize>, message: &str) -> (Case, String) {
    let mut c = match at {
        Some(i) => case.with_queries(vec![case.queries[i].clone()]),
        None => case.clone(),
    };
    let mut message = message.to_owned();
    let queries = |c: &Case, message: &str| {
        let [q] = c.queries.as_slice() else {
            return None;
        };
        with_case(seed, c, |env| {
            shrink_query(env, law, q.clone(), message.to_owned())
        })
        .ok()
    };
    if let Some((q, m)) = queries(&c, &message) {
        (c, message) = (c.with_queries(vec![q]), m);
    }
    let mut opened = 0;
    'outer: while opened < DATABASE_TRIES {
        for s in c.smaller() {
            if panic::catch_unwind(AssertUnwindSafe(|| reference(&s))).is_err() {
                continue;
            }
            opened += 1;
            if let Some((_, m)) = first_failure(seed, &s, law) {
                (c, message) = (s, m);
                continue 'outer;
            }
            if opened >= DATABASE_TRIES {
                break;
            }
        }
        break;
    }
    if let Some((q, m)) = queries(&c, &message) {
        (c, message) = (c.with_queries(vec![q]), m);
    }
    (c, message)
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
        if let Some((at, message)) = first_failure(seed, &c, law) {
            found.push((seed, c, at, format!("{label}{message}")));
        }
    }
    let Some((seed, c, at, message)) = found.first() else {
        return;
    };
    let mut report = format!(
        "{name}: {} of {} seeds disagree\n",
        found.len(),
        seeds.end - seeds.start
    );
    let (small, why) = shrink(*seed, c, law, *at, message);
    report.push_str(&format!("seed {seed}, shrunk: {why}\n{}", small.program()));
    for q in &small.queries {
        report.push_str(&format!("query: {}\n", q.text()));
    }
    report.push('\n');
    for (i, (seed, c, _, message)) in found.iter().enumerate() {
        report.push_str(&format!("seed {seed}: {message}\n"));
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

const REWRITES: usize = 9;

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
        _ => &mut r.semi_naive,
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

/// Walks per run unless `ADTB_DIFF_CASES` says otherwise.
const WALKS: u64 = 4;

/// The migrations of each walk.
const STEPS: usize = 2;

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

#[test]
fn schema_walks_keep_engine_equal_to_reference() {
    let mut found = Vec::new();
    let seeds = seeds_of(WALKS);
    for seed in seeds.clone() {
        let w = walk(seed, STEPS, QUERIES);
        if let Some(m) = walked(seed, &w) {
            found.push((seed, m, w));
        }
    }
    let mut report = format!(
        "schema walk: {} of {} seeds disagree\n",
        found.len(),
        seeds.end - seeds.start
    );
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
