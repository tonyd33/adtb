//! The engine over generated cases: its results against the reference interpreter's, a query
//! against its partitions by constructor and by predicate, and every planner rewrite setting
//! against the default.

use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};

use adtb::plan::{Rewrites, with_rewrites};
use adtb::{CallOptions, CreateOptions, Database, Error, Log, OpenOptions, RejectReason};
use adtb_lang::textual;
use adtb_reference::{Abort, Value};
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::Rng;
use adtb_testkit::cases::{Case, case};
use adtb_testkit::fixture::Fixture;

/// Cases per run unless `ADTB_DIFF_CASES` says otherwise.
const CASES: u64 = 12;

/// The queries of each case, over one database.
const QUERIES: usize = 16;

/// Returns the seeds of one run, from `ADTB_DIFF_SEED`, else zero.
fn seeds() -> Range<u64> {
    let var = |name: &str, default: u64| {
        std::env::var(name).map_or(default, |v| {
            v.parse()
                .unwrap_or_else(|_| panic!("{name} is not a number: {v}"))
        })
    };
    let first = var("ADTB_DIFF_SEED", 0);
    first..first + var("ADTB_DIFF_CASES", CASES)
}

/// The disagreements printed whole with their program; the rest are listed by seed.
const SHOWN: usize = 5;

/// How a query ended: its rows, ascending, or a fault, or a refusal of another kind.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Got {
    Rows(Vec<Value>),
    Faulted,
    Refused(String),
}

impl Got {
    fn agrees(&self, other: &Got) -> bool {
        match (self, other) {
            (Got::Rows(a), Got::Rows(b)) => a == b,
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

/// Runs `f` on a fresh simulated engine database holding the case, loaded.
fn on_engine<T>(seed: u64, case: &Case, f: impl FnOnce(&Database<SimRuntime>) -> T) -> T {
    let mut out = None;
    let outcome = Sim::new(seed, Config::default()).run(|rt| {
        let options = OpenOptions {
            create: Some(CreateOptions::default()),
            ..OpenOptions::default()
        };
        let log = Log::new([case.migration.clone()]);
        let db = Database::open_in(rt, "differential.adt", &log, &options)
            .unwrap_or_else(|e| panic!("the engine does not open the schema: {e:?}"));
        for w in &case.load {
            if let Err(e) = db.query_text(w, &[], &CallOptions::default()) {
                panic!("the engine refuses `{w}`: {e}");
            }
        }
        out = Some(f(&db));
        if let Err(e) = db.close() {
            panic!("the engine does not close: {e}");
        }
    });
    match (outcome, out) {
        (Outcome::Finished, Some(v)) => v,
        (o, _) => panic!("the simulated run ends {o:?}"),
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default()
}

/// Runs `check` over every seed and fails with each disagreement it returns, or panic, by seed.
fn sweep(law: &str, check: impl Fn(u64, &Case) -> Option<String>) {
    let mut found = Vec::new();
    let seeds = seeds();
    for seed in seeds.clone() {
        let c = case(seed, QUERIES);
        let ran = panic::catch_unwind(AssertUnwindSafe(|| check(seed, &c)));
        let message = match ran {
            Ok(None) => continue,
            Ok(Some(m)) => m,
            Err(p) => format!("panics: {}", panic_message(&*p)),
        };
        found.push((seed, message, c.program()));
    }
    if found.is_empty() {
        return;
    }
    let mut report = format!(
        "{law}: {} of {} seeds disagree\n",
        found.len(),
        seeds.end - seeds.start
    );
    for (i, (seed, message, program)) in found.iter().enumerate() {
        report.push_str(&format!("seed {seed}: {message}\n"));
        if i < SHOWN {
            report.push_str(&format!("{program}\n\n"));
        }
    }
    panic!("{report}");
}

#[test]
fn generated_cases_check_and_load() {
    sweep("generated cases", |_, c| {
        let fx = reference(c);
        let mut texts = Vec::new();
        for q in &c.queries {
            texts.push(q.text());
            texts.extend(q.by_ctor().unwrap_or_default());
            if let Some(split) = q.by_predicate() {
                texts.push(split.whole);
                texts.extend(split.parts);
            }
            let counts = q.counts();
            texts.push(counts.whole);
            texts.extend(counts.parts);
        }
        texts
            .into_iter()
            .find_map(|t| match reference_eval(&fx, &t) {
                Got::Refused(why) => Some(format!("the reference refuses `{t}`: {why}")),
                _ => None,
            })
    });
}

#[test]
fn engine_equals_reference() {
    sweep("differential", |seed, c| {
        let fx = reference(c);
        let texts: Vec<String> = c
            .queries
            .iter()
            .flat_map(|q| [q.text(), q.counts().whole])
            .collect();
        on_engine(seed, c, |db| {
            texts.iter().find_map(|t| {
                let (want, got) = (reference_eval(&fx, t), engine_eval(db, t));
                (!got.agrees(&want))
                    .then(|| format!("`{t}`: the engine gives {got:?}, the reference {want:?}"))
            })
        })
    });
}

#[test]
fn sum_is_union_of_constructors() {
    sweep("constructor partition", |seed, c| {
        let laws: Vec<(String, Vec<String>)> = c
            .queries
            .iter()
            .filter_map(|q| Some((q.text(), q.by_ctor()?)))
            .collect();
        on_engine(seed, c, |db| {
            laws.iter().find_map(|(text, parts)| {
                let whole = engine_eval(db, text);
                let union = Got::union(parts.iter().map(|p| engine_eval(db, p)).collect());
                (!whole.agrees(&union)).then(|| {
                    format!("`{text}` gives {whole:?}, the union of {parts:?} gives {union:?}")
                })
            })
        })
    });
}

#[test]
fn predicate_partitions_bindings() {
    sweep("predicate partition", |seed, c| {
        on_engine(seed, c, |db| {
            c.queries.iter().find_map(|q| {
                let split = q.by_predicate()?;
                let whole = engine_eval(db, &split.whole);
                let union = Got::union(split.parts.iter().map(|p| engine_eval(db, p)).collect());
                if !whole.agrees(&union) {
                    return Some(format!(
                        "`{}` gives {whole:?}, the union of {:?} gives {union:?}",
                        split.whole, split.parts
                    ));
                }
                let counts = q.counts();
                let total = engine_eval(db, &counts.whole);
                let sum = Got::sum(counts.parts.iter().map(|p| engine_eval(db, p)).collect());
                (!total.agrees(&sum)).then(|| {
                    format!(
                        "`{}` gives {total:?}, the sum of {:?} gives {sum:?}",
                        counts.whole, counts.parts
                    )
                })
            })
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

/// Returns every rewrite off, each turned off alone, each turned on alone, and four subsets
/// drawn from `seed`.
fn settings(seed: u64) -> Vec<Rewrites> {
    let mut out = vec![Rewrites::NONE];
    for i in 0..REWRITES {
        let mut off = Rewrites::ALL;
        *flag(&mut off, i) = false;
        let mut on = Rewrites::NONE;
        *flag(&mut on, i) = true;
        out.extend([off, on]);
    }
    let mut rng = Rng::new(seed);
    for _ in 0..4 {
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
    sweep("plan toggles", |seed, c| {
        let texts: Vec<String> = c
            .queries
            .iter()
            .flat_map(|q| [q.text(), q.counts().whole])
            .collect();
        on_engine(seed, c, |db| {
            for t in &texts {
                let want = engine_eval(db, t);
                for r in settings(seed) {
                    let got = with_rewrites(r, || engine_eval(db, t));
                    if !got.agrees(&want) {
                        return Some(format!(
                            "`{t}` gives {got:?} under {r:?}, {want:?} under every rewrite"
                        ));
                    }
                }
            }
            None
        })
    });
}
