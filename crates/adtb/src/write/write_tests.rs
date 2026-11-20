use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use crate::exec::{self, Val};
use crate::test_world::finished;
use crate::test_world::support::{to_ref, to_val};
use crate::test_world::write_path::{self, WritePath};
use crate::write::{self, Database, Rejection as WRejection, Rows};
use adtb_lang::kernel::{Terms, Tm, Var};
use adtb_lang::schema::Schema;
use adtb_lang::types::{KeyCtors, RelRef, Ty};
use adtb_reference::Value;
use adtb_sim::{Config, Outcome, Sim, SimRuntime};
use adtb_testkit::fixture::Fixture;
use adtb_testkit::seeds::{share, sweep};

/// One statement: its text, and its parameters as name, type and value.
struct Step {
    text: String,
    params: Vec<(&'static str, &'static str, Value)>,
}

fn with(text: &str, params: Vec<(&'static str, &'static str, Value)>) -> Step {
    Step {
        text: text.to_string(),
        params,
    }
}

/// An abort as the two sides are compared: what it is, and the rows it reports.
type Failed = (String, BTreeSet<Val>);

fn plain(s: String) -> Failed {
    (s, BTreeSet::new())
}

fn engine_error(schema: &Schema, e: &write::Error) -> Failed {
    match e {
        write::Error::Exec(exec::Error::Fault(f)) => plain(format!("Fault {}", f.name())),
        write::Error::Exec(exec::Error::StatementLimit(_)) => plain("Limit".into()),
        write::Error::Exec(exec::Error::Rejected(detail)) => plain(format!(
            "BadInput {{ source: \"\", line: 0, detail: {detail:?} }}"
        )),
        write::Error::Rejected(r) => match r {
            WRejection::KeyViolation { relation, key } => {
                let ty = Ty::Key {
                    rel: RelRef::Declared(*relation),
                    ctors: KeyCtors::All,
                };
                let printed =
                    adtb_reference::textual::print(schema, &to_ref(schema, &ty, key), &ty)
                        .expect("a key prints");
                plain(format!(
                    "KeyViolation {} {printed}",
                    schema.relation(*relation).name
                ))
            }
            WRejection::Violated { constraint, report } => {
                (format!("Violated {constraint}"), report.clone())
            }
            WRejection::KeyChanged { relation } => {
                plain(format!("KeyChanged {}", schema.relation(*relation).name))
            }
            WRejection::NotOpen { ctor, stage } => plain(format!("NotOpen {ctor} {stage:?}")),
            WRejection::UserError { message } => plain(format!("UserError {message}")),
            WRejection::BadInput {
                source,
                line,
                detail,
            } => plain(format!(
                "BadInput {{ source: {source:?}, line: {line}, detail: {detail:?} }}"
            )),
            WRejection::RowTooLarge {
                relation,
                ctor,
                size,
                limit,
            } => {
                let rel = schema.relation(*relation);
                let ctor = &schema.ty(rel.elem_def).ctors()[*ctor as usize].name;
                plain(format!("RowTooLarge {} {ctor} {size} {limit}", rel.name))
            }
            WRejection::KeyTooLarge {
                relation,
                ctor,
                index,
                size,
                limit,
            } => {
                let rel = schema.relation(*relation);
                let ctor = &schema.ty(rel.elem_def).ctors()[*ctor as usize].name;
                plain(format!(
                    "KeyTooLarge {} {ctor} {index} {size} {limit}",
                    rel.name
                ))
            }
        },
        other => plain(format!("{other:?}")),
    }
}

/// A statement ready for the engine: its normal form, and its parameters' values.
struct Prepared {
    terms: Terms,
    item: Tm,
    params: Vec<(Var, Val)>,
}

fn prepare(w: &WritePath<SimRuntime>, s: &Step) -> Prepared {
    let types: Vec<(&str, &str)> = s.params.iter().map(|(n, t, _)| (*n, *t)).collect();
    let (terms, item, typed) = w.prepare(&s.text, &types);
    let params = typed
        .iter()
        .zip(&s.params)
        .map(|((var, ty), (_, _, v))| (*var, to_val(w.schema(), ty, v)))
        .collect();
    Prepared {
        item,
        terms,
        params,
    }
}

/// Returns the rows of `rows`, over `from`, of each relation outside the engine's modules, keyed
/// by the relation of `to` of the same module and name.
fn rekeyed(rows: &Rows, to: &Schema, from: &Schema) -> Rows {
    rows.iter()
        .filter(|((r, _), _)| !from.module(from.relation(*r).module).engine)
        .map(|((r, c), vs)| {
            (
                (adtb_testkit::load::relation_of(to, from, *r), *c),
                vs.clone(),
            )
        })
        .collect()
}

const SHOP: &str = "type Counter = { id: Key counters, n: I64 }
relation counters : Counter key id generate sequence
type Label = { id: Key tags, name: Text }
relation tags : Label key id generate sequence unique (name)";

fn shop() -> Fixture {
    let mut f = Fixture::new(SHOP);
    f.rows("counters", "[Counter { id: counters.key(1), n: 0 }]");
    f
}

const INC: &str =
    "update c in counters where key c == counters.key(1) set Counter { c with n: c.n + 1 }";
const TAG: &str = "transact let x = insert tags { name: t } return key x";
const UPSERT: &str = "insert tags { name: t } unless conflict on (name) else update old set old";
const DROP: &str = "delete g in tags where g.name == t";
const FAILING: &str =
    "(update c in counters where key c == counters.key(1) set Counter { c with n: c.n + 1 },
  insert tags { name: \"twice\" },
  insert tags { name: \"twice\" })";

/// A prepared statement over parameter `t`, if it has one.
struct Prep {
    p: Prepared,
    t: Option<Var>,
}

/// Prepare `text`, a statement over parameter `t` of type `Text` when `over_t`.
fn prep(w: &WritePath<SimRuntime>, text: &str, over_t: bool) -> Prep {
    let params = if over_t {
        vec![("t", "Text", Value::text(""))]
    } else {
        Vec::new()
    };
    let p = prepare(w, &with(text, params));
    let t = p.params.first().map(|(v, _)| *v);
    Prep { p, t }
}

/// One finished transaction of a client: what it ran and its value when it committed, or how
/// it failed.
#[derive(Clone, Debug)]
struct Done {
    op: usize,
    result: Result<Val, String>,
}

fn run_txn(
    db: &Database<SimRuntime>,
    schema: &Schema,
    s: &Prep,
    name: &str,
) -> Result<Val, String> {
    run_txn_with(db, schema, s, name, None, &mut |_| {})
}

/// Run `s` over `name` in a transaction, then `failing`, a statement that must fail and leave
/// the transaction open, then commit. `seen` gets `s`'s result before the commit.
fn run_txn_with(
    db: &Database<SimRuntime>,
    schema: &Schema,
    s: &Prep,
    name: &str,
    failing: Option<&Prep>,
    seen: &mut dyn FnMut(&Val),
) -> Result<Val, String> {
    let fail = |tx, e: &write::Error| {
        db.abort(tx).expect("an abort");
        Err(format!("{:?}", engine_error(schema, e)))
    };
    let tx = db.begin().map_err(|e| format!("{e:?}"))?;
    let params: Vec<(Var, Val)> = s.t.map(|v| (v, Val::text(name))).into_iter().collect();
    let result = match db.statement(&tx, &s.p.terms, s.p.item, &params, 0) {
        Ok(o) => o.result,
        Err(e) => return fail(tx, &e),
    };
    seen(&result);
    if let Some(f) = failing {
        match db.statement(&tx, &f.p.terms, f.p.item, &[], 0) {
            Ok(_) => panic!("a failing statement succeeded"),
            Err(e) if e.class().ends() => return fail(tx, &e),
            Err(e) => {
                let e = engine_error(schema, &e);
                assert_eq!(
                    e.0, "Violated app::tags.unique(name)",
                    "the failing statement failed by {e:?}"
                );
            }
        }
    }
    match db.commit(tx) {
        Ok(_) => Ok(result),
        Err(e) => Err(format!("{:?}", engine_error(schema, &e))),
    }
}

fn is_expected_failure(e: &str) -> bool {
    e.contains("Serialization") || e.contains("\"Violated app::tags.unique(name)\"")
}

#[test]
fn concurrent_writers_keep_keys_and_uniques_and_lose_no_update() {
    const SEEDS: Range<u64> = 0..4;
    let f = shop();
    let mut commits = 0;
    for seed in sweep(SEEDS) {
        let mut sim = write_path::sim(seed, Config::default(), &f);
        let done: Arc<std::sync::Mutex<Vec<Done>>> = Arc::default();
        let mut finals = None;
        let result = sim.run(|rt| {
            use crate::runtime::{Entropy, Threads};
            let w = WritePath::loaded(&rt, &f, None);
            let preps = Arc::new([
                prep(&w, INC, false),
                prep(&w, TAG, true),
                prep(&w, UPSERT, true),
                prep(&w, DROP, true),
            ]);
            let failing = Arc::new(prep(&w, FAILING, false));
            let w = Arc::new(w);
            let clients: Vec<_> = (0..4)
                .map(|c| {
                    let (w, rt2, preps, done, failing) = (
                        w.clone(),
                        rt.clone(),
                        preps.clone(),
                        done.clone(),
                        failing.clone(),
                    );
                    rt.spawn(&format!("client {c}"), move || {
                        for _ in 0..6 {
                            let op = (rt2.next_u64() % 4) as usize;
                            let name = ["a", "b", "c"][(rt2.next_u64() % 3) as usize].to_string();
                            let also = (rt2.next_u64() % 2 == 0).then_some(failing.as_ref());
                            let result = run_txn_with(
                                w.db(),
                                w.schema(),
                                &preps[op],
                                &name,
                                also,
                                &mut |_| {},
                            );
                            done.lock().unwrap().push(Done { op, result });
                        }
                    })
                })
                .collect();
            for c in clients {
                rt.join(c);
            }
            let tx = w.db().begin().expect("a transaction");
            finals = Some(rekeyed(
                &w.db().rows(&tx).expect("the rows"),
                &f.schema,
                w.schema(),
            ));
            w.db().abort(tx).expect("an abort");
        });
        assert_eq!(Outcome::Finished, result, "seed {seed}");
        let rows = finals.expect("the run finished");
        let done = done.lock().unwrap().clone();
        for d in &done {
            if let Err(e) = &d.result {
                assert!(is_expected_failure(e), "seed {seed}: {d:?}");
            }
        }
        let counters = f.rel("counters");
        let incs = done
            .iter()
            .filter(|d| d.op == 0 && d.result.is_ok())
            .count() as i64;
        let n: Vec<Val> = rows[&(counters, 0)].iter().map(|r| field(r, 1)).collect();
        assert_eq!(
            vec![Val::Int(incs)],
            n,
            "seed {seed}: every committed increment counts"
        );
        let tags = f.rel("tags");
        let names: Vec<Val> = rows
            .get(&(tags, 0))
            .into_iter()
            .flatten()
            .map(|r| field(r, 1))
            .collect();
        let distinct: BTreeSet<&Val> = names.iter().collect();
        assert_eq!(
            distinct.len(),
            names.len(),
            "seed {seed}: tag names are unique: {names:?}"
        );
        let keys: Vec<&Val> = done
            .iter()
            .filter(|d| d.op == 1)
            .filter_map(|d| d.result.as_ref().ok())
            .collect();
        let distinct_keys: BTreeSet<&&Val> = keys.iter().collect();
        assert_eq!(
            distinct_keys.len(),
            keys.len(),
            "seed {seed}: generated keys are distinct"
        );
        commits += done.iter().filter(|d| d.result.is_ok()).count();
    }
    assert!(
        commits > share(SEEDS, 20),
        "only {commits} transactions committed"
    );
}

#[test]
fn committed_writes_survive_a_crash_and_sequences_never_reissue() {
    use crate::runtime::Threads;
    let f = Fixture::new(SHOP);
    type Slot<T> = Arc<std::sync::Mutex<T>>;
    let load = |rt: &SimRuntime,
                created: &Slot<bool>,
                acked: &Slot<BTreeMap<String, Val>>,
                submitted: &Slot<BTreeSet<String>>,
                drawn: &Slot<BTreeSet<Val>>,
                started: &Slot<Option<u64>>| {
        let w = WritePath::of(rt, &f, None);
        *created.lock().unwrap() = true;
        let tag = Arc::new(prep(&w, TAG, true));
        let w = Arc::new(w);
        started.lock().unwrap().replace({
            use crate::runtime::Clock;
            rt.now().as_nanos()
        });
        let clients: Vec<_> = (0..3)
            .map(|c| {
                let (w, tag, acked, submitted, drawn) = (
                    w.clone(),
                    tag.clone(),
                    acked.clone(),
                    submitted.clone(),
                    drawn.clone(),
                );
                rt.spawn(&format!("client {c}"), move || {
                    for i in 0..40 {
                        let name = format!("c{c}-{i}");
                        submitted.lock().unwrap().insert(name.clone());
                        let mut seen = |k: &Val| {
                            drawn.lock().unwrap().insert(k.clone());
                        };
                        match run_txn_with(w.db(), w.schema(), &tag, &name, None, &mut seen) {
                            Ok(k) => {
                                acked.lock().unwrap().insert(name, k);
                            }
                            Err(_) => {
                                submitted.lock().unwrap().remove(&name);
                            }
                        }
                    }
                })
            })
            .collect();
        for c in clients {
            rt.join(c);
        }
    };
    let (mut crashed, mut lost_draws) = (0, 0);
    for seed in 0..4 {
        let mut dry = Sim::new(seed, Config::default());
        let started: Slot<Option<u64>> = Arc::default();
        dry.run(|rt| {
            load(
                &rt,
                &Arc::default(),
                &Arc::default(),
                &Arc::default(),
                &Arc::default(),
                &started,
            );
        });
        let begun = started.lock().unwrap().expect("the clients started");
        let span = dry.now().as_nanos() - begun;
        let mut sim = Sim::new(seed, Config::default());
        let acked: Arc<std::sync::Mutex<BTreeMap<String, Val>>> = Arc::default();
        let submitted: Arc<std::sync::Mutex<BTreeSet<String>>> = Arc::default();
        let created: Slot<bool> = Arc::default();
        let drawn: Slot<BTreeSet<Val>> = Arc::default();
        let at = begun + span * (1 + seed) / 6;
        sim.crash_at(crate::runtime::Instant::from_nanos(at));
        let first = sim.run(|rt| load(&rt, &created, &acked, &submitted, &drawn, &Arc::default()));
        if first == Outcome::Crashed && !acked.lock().unwrap().is_empty() {
            crashed += 1;
        }
        if !*created.lock().unwrap() {
            assert!(
                acked.lock().unwrap().is_empty(),
                "seed {seed}: commits were acknowledged before the file was created"
            );
            continue;
        }
        let mut after = None;
        let result = sim.run(|rt| {
            let w = WritePath::reopen(&rt, None)
                .unwrap_or_else(|| panic!("seed {seed}: the database does not reopen"));
            let tag = prep(&w, TAG, true);
            let tx = w.db().begin().expect("a transaction");
            let rows = rekeyed(&w.db().rows(&tx).expect("the rows"), &f.schema, w.schema());
            w.db().abort(tx).expect("an abort");
            let next =
                run_txn(w.db(), w.schema(), &tag, "after").expect("an insert after recovery");
            after = Some((rows, next));
        });
        assert_eq!(Outcome::Finished, result, "seed {seed}");
        let (rows, next) = after.unwrap_or_else(|| panic!("seed {seed}: recovery read no rows"));
        let held: BTreeMap<String, Val> = rows
            .get(&(f.rel("tags"), 0))
            .into_iter()
            .flatten()
            .map(|r| {
                let Val::Text(n) = field(r, 1) else {
                    panic!("a name")
                };
                (n.to_string(), field(r, 0))
            })
            .collect();
        for (name, k) in acked.lock().unwrap().iter() {
            assert_eq!(
                Some(k),
                held.get(name),
                "seed {seed}: acknowledged {name} survives"
            );
        }
        let submitted = submitted.lock().unwrap();
        for name in held.keys() {
            assert!(
                submitted.contains(name),
                "seed {seed}: {name} was never submitted"
            );
        }
        assert!(
            !held.values().any(|k| *k == next),
            "seed {seed}: the key {next:?} drawn after recovery was held"
        );
        let drawn = drawn.lock().unwrap();
        let in_flight = submitted.len() > acked.lock().unwrap().len();
        lost_draws +=
            usize::from(in_flight && drawn.iter().any(|k| !held.values().any(|h| h == k)));
        let last = drawn.last().cloned();
        assert!(
            last.is_none_or(|d| next > d),
            "seed {seed}: the key {next:?} drawn after recovery was drawn before the crash"
        );
    }
    eprintln!("{crashed} runs crashed after a commit, {lost_draws} lost a key drawn in flight");
    assert!(crashed > 0, "no run crashed after acknowledging a commit");
    assert!(lost_draws > 0, "no crash lost a key drawn in flight");
}

/// Commit a transaction of `db` that updated a counter, after `before` runs. Returns how the
/// commit ended and how many more abort records went unrecorded.
fn commit_updated(
    w: &WritePath<SimRuntime>,
    before: impl FnOnce(&Database<SimRuntime>),
) -> (Result<crate::storage::Position, write::Error>, usize) {
    let s = prep(w, INC, false);
    let db = w.db();
    let tx = db.begin().expect("a transaction");
    db.statement(&tx, &s.p.terms, s.p.item, &[], 0)
        .expect("the update");
    before(db);
    let unrecorded = db.txns().unrecorded_aborts();
    let committed = db.commit(tx);
    (committed, db.txns().unrecorded_aborts() - unrecorded)
}

#[test]
fn a_commit_refused_by_a_mark_returns_its_refusal_once_its_abort_is_recorded() {
    use crate::storage::Quarantine;
    let f = shop();
    let mut sim = write_path::sim(0, Config::default(), &f);
    finished(&mut sim, |rt| {
        let w = WritePath::loaded(&rt, &f, None);
        let got = commit_updated(&w, |db| {
            db.pager()
                .plant_mark(Quarantine::Database, db)
                .expect("the mark");
        });
        let refused = write::Error::Quarantined {
            objects: BTreeMap::from([(Quarantine::Database, None)]),
            by: write::By::Admitted,
        };
        assert_eq!((Err(refused), 0), got);
    });
}

#[test]
fn a_failed_commit_whose_abort_record_fails_returns_the_records_failure_and_counts_it() {
    use crate::storage::{Damage, PageError};
    let f = shop();
    let mut sim = write_path::sim(0, Config::default(), &f);
    finished(&mut sim, |rt| {
        let w = WritePath::loaded(&rt, &f, None);
        let mut status = None;
        let got = commit_updated(&w, |db| {
            let pages = db.txns().commit_log().status_pages().clone();
            let pid = *pages.values().next_back().expect("a status page");
            db.pager().plant_damage(pid, Damage::Checksum);
            status = Some(pid);
        });
        let damage = PageError {
            pid: status.expect("a status page"),
            damage: Damage::Checksum,
        };
        let failed = write::Error::Transactions(crate::mvcc::Error::Corrupt(damage));
        assert_eq!((Err(failed), 1), got);
    });
}

fn field(r: &Val, i: usize) -> Val {
    r.fields().expect("a row has fields")[i].clone()
}
