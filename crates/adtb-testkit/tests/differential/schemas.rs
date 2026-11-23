//! Generated schemas and statements: the engine against the reference statement by statement,
//! each write followed by the contents of every relation on both sides.

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};

use adtb::{CallOptions, Database};
use adtb_reference::Value;
use adtb_sim::SimRuntime;
use adtb_testkit::cases::Case;
use adtb_testkit::corpus;
use adtb_testkit::fixture::Fixture;
use adtb_testkit::schemas::{self, Mark, Stmt};

use super::{
    Got, Reason, engine_eval, engine_failed, on_engine, panic_message, reference,
    reference_aborted, reference_eval, seeds_of,
};

/// Cases per default run, [`adtb_testkit::profile::deep`] sized, unless `ADTB_DIFF_CASES` says
/// otherwise.
const CASES: u64 = 12;

/// The statements of each case.
const STATEMENTS: usize = 64;

/// The cases a shrink of one finding may run.
const SHRINK_TRIES: usize = 150;

/// How a statement ended on one side: committed, or another way a query ends.
#[derive(Clone, Debug)]
enum Ended {
    Committed,
    Not(Got),
}

impl Ended {
    fn agrees(&self, other: &Ended) -> bool {
        match (self, other) {
            (Ended::Committed, Ended::Committed) => true,
            (Ended::Not(a), Ended::Not(b)) => a.agrees(b, 0.0),
            _ => false,
        }
    }

    /// Returns how the statement ended, with the reason of a rejection or refusal.
    fn sig(&self) -> String {
        match self {
            Ended::Committed => "committed".into(),
            Ended::Not(Got::Rows(_)) => "rows".into(),
            Ended::Not(Got::Faulted) => "faulted".into(),
            Ended::Not(Got::Rejected(r)) => format!("{r:?}"),
            Ended::Not(Got::Refused(m)) => format!("refused: {m}"),
        }
    }
}

/// A disagreement: the statement it shows at, a signature a shrink keeps, and its text.
#[derive(Clone, Debug)]
pub(super) struct Finding {
    at: Option<usize>,
    pub(super) sig: String,
    pub(super) text: String,
}

/// The shapes the committed and refused statements of a sweep reached.
#[derive(Debug, Default)]
pub(super) struct Reached {
    cascade: usize,
    clear: usize,
    retag_action: usize,
    retag_beside_clear: usize,
    restrict: usize,
    retag: usize,
    cross_sum: usize,
    sum_key: usize,
    set_unique: usize,
    twinned: usize,
}

/// A row's constructor and value, by its key.
type ByKey = BTreeMap<Vec<Option<Value>>, (Option<u32>, Value)>;

fn by_key(got: &Got, keys: &[String]) -> ByKey {
    let Got::Rows(rows) = got else {
        return ByKey::new();
    };
    rows.iter()
        .map(|v| {
            let k = keys.iter().map(|f| v.field(f).cloned()).collect();
            (k, (v.tag(), v.clone()))
        })
        .collect()
}

/// Returns whether row `after` holds `None` in a field where `before` does not.
fn cleared(before: &Value, after: &Value) -> bool {
    let (Value::Ctor(_, bs), Value::Ctor(_, xs)) = (before, after) else {
        return false;
    };
    bs.iter()
        .zip(xs)
        .any(|((_, b), (_, x))| *x == Value::none() && *b != Value::none())
}

impl Reached {
    /// Count the shapes statement `s` of `case` reached, ending `ended` on the reference, from
    /// `around`, the contents of each relation before and after it.
    fn count(&mut self, case: &schemas::Case, s: &Stmt, ended: &Ended, around: [&[Got]; 2]) {
        let [before, after] = around;
        match ended {
            Ended::Not(Got::Rejected(Reason::Violated { constraint })) => {
                self.set_unique += usize::from(constraint.contains(") in ("));
                self.twinned += usize::from(case.twinned(constraint));
                let delete = s.text().starts_with("delete");
                self.restrict += usize::from(delete && case.restricts(constraint));
                return;
            }
            Ended::Committed => {}
            Ended::Not(_) => return,
        }
        for m in s.marks() {
            match m {
                Mark::CrossSum => self.cross_sum += 1,
                Mark::SumKey => self.sum_key += 1,
                Mark::Retag => {}
            }
        }
        let deletes = !s.deletes().is_empty();
        let rels = case.relations();
        for (i, ((name, keys), (b, a))) in rels.iter().zip(before.iter().zip(after)).enumerate() {
            let (b, a) = (by_key(b, keys), by_key(a, keys));
            for (k, (tag, row)) in &b {
                match a.get(k) {
                    None if deletes && !s.deletes().contains(name) => self.cascade += 1,
                    Some((t, _)) if t != tag && deletes => {
                        self.retag_action += 1;
                        if let (Some(from), Some(to)) = (tag, t) {
                            self.retag_beside_clear +=
                                usize::from(case.shares_clear(i, *from, *to));
                        }
                    }
                    Some((t, _)) if t != tag && s.marks().contains(&Mark::Retag) => self.retag += 1,
                    Some((_, now)) if deletes && cleared(row, now) => self.clear += 1,
                    _ => {}
                }
            }
        }
    }

    fn floors(&self, scale: impl Fn(usize) -> usize) {
        let rows = [
            ("a cascade deleting a referrer", self.cascade),
            ("a clear setting a field to None", self.clear),
            ("a retag action moving a referrer", self.retag_action),
            (
                "a retag action moving a row whose clear field the target shares by name",
                self.retag_beside_clear,
            ),
            ("a restrict refusing a delete", self.restrict),
            ("an update to another constructor", self.retag),
            ("an insert of a reference across sums", self.cross_sum),
            ("an insert keyed by a value-level sum", self.sum_key),
            (
                "a unique over a constructor set refusing a write",
                self.set_unique,
            ),
            ("a write violating two constraints at once", self.twinned),
        ];
        for (what, n) in rows {
            adtb_testkit::profile::assert_reached(what, n, scale(1));
        }
    }
}

fn reference_ran(fx: &mut Fixture, text: &str) -> Ended {
    match fx.run(text) {
        Ok(_) => Ended::Committed,
        Err(a) => Ended::Not(reference_aborted(a)),
    }
}

fn engine_ran(db: &Database<SimRuntime>, text: &str) -> Ended {
    match db.query_text(text, &[], &CallOptions::default()) {
        Ok(_) => Ended::Committed,
        Err(e) => Ended::Not(engine_failed(e)),
    }
}

/// Returns `f()`, or its panic as the reference's refusal.
fn guarded(f: impl FnOnce() -> Got) -> Got {
    panic::catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|p| Got::Refused(format!("the reference panics: {}", panic_message(&*p))))
}

fn differs(at: usize, text: &str, want: &Ended, got: &Ended) -> Finding {
    Finding {
        at: Some(at),
        sig: format!("{} / {}", want.sig(), got.sig()),
        text: format!("`{text}`: the engine gives {got:?}, the reference {want:?}"),
    }
}

/// Run the statements of `case` on the reference `fx` and the engine `db`, each write followed by
/// the contents of every relation. Returns the first disagreement.
pub(super) fn statements(
    fx: &mut Fixture,
    db: &Database<SimRuntime>,
    case: &schemas::Case,
    reached: &mut Reached,
) -> Option<Finding> {
    let rels = case.relations();
    let mut before: Vec<Got> = rels.iter().map(|_| Got::Rows(Vec::new())).collect();
    for (at, s) in case.stmts().iter().enumerate() {
        if !s.is_write() {
            let want = Ended::Not(guarded(|| reference_eval(fx, s.text())));
            let got = Ended::Not(engine_eval(db, s.text()));
            if !got.agrees(&want) {
                return Some(differs(at, s.text(), &want, &got));
            }
            continue;
        }
        let want = match panic::catch_unwind(AssertUnwindSafe(|| reference_ran(fx, s.text()))) {
            Ok(w) => w,
            Err(p) => Ended::Not(Got::Refused(format!(
                "the reference panics: {}",
                panic_message(&*p)
            ))),
        };
        let got = engine_ran(db, s.text());
        if !got.agrees(&want) {
            return Some(differs(at, s.text(), &want, &got));
        }
        let mut after = Vec::new();
        for (name, _) in &rels {
            let q = format!("from x in {name} select x");
            let (w, g) = (guarded(|| reference_eval(fx, &q)), engine_eval(db, &q));
            if !g.agrees(&w, 0.0) {
                return Some(Finding {
                    at: Some(at),
                    sig: format!("contents of {name}"),
                    text: format!(
                        "after `{}`, `{q}`: the engine gives {g:?}, the reference {w:?}",
                        s.text()
                    ),
                });
            }
            after.push(w);
        }
        reached.count(case, s, &want, [&before, &after]);
        before = after;
    }
    None
}

/// Run `case` on a fresh reference and engine. Returns the first disagreement, a refusal of
/// the schema or a panic.
fn run(seed: u64, case: &schemas::Case, reached: &mut Reached) -> Option<Finding> {
    let decls = case.decls();
    let migration = match corpus::migration(&decls) {
        Ok(m) => m,
        Err(why) => {
            return Some(Finding {
                at: None,
                sig: "the checker refuses the schema".into(),
                text: why,
            });
        }
    };
    let c = Case::of_text(
        format!("module app\n{decls}"),
        migration,
        Vec::new(),
        Vec::new(),
    );
    let ran = panic::catch_unwind(AssertUnwindSafe(|| {
        let mut fx = match panic::catch_unwind(AssertUnwindSafe(|| reference(&c))) {
            Ok(fx) => fx,
            Err(p) => {
                return Some(Finding {
                    at: None,
                    sig: "the reference refuses the schema".into(),
                    text: panic_message(&*p),
                });
            }
        };
        on_engine(seed, &c, |db| statements(&mut fx, db, case, reached))
    }));
    ran.unwrap_or_else(|p| {
        Some(Finding {
            at: None,
            sig: format!(
                "panics: {}",
                panic_message(&*p).lines().next().unwrap_or_default()
            ),
            text: panic_message(&*p),
        })
    })
}

/// Returns `case` shrunk against its finding `f`: its statements up to the one `f` shows at,
/// then smaller cases while one fails with the same signature.
fn shrink(seed: u64, case: &schemas::Case, f: &Finding) -> (schemas::Case, Finding) {
    let cut = |c: &schemas::Case, f: &Finding| f.at.map_or_else(|| c.clone(), |at| c.first(at + 1));
    let (mut c, mut f) = (cut(case, f), f.clone());
    let mut tries = 0;
    'outer: while tries < SHRINK_TRIES {
        for s in c.smaller() {
            tries += 1;
            if let Some(g) = run(seed, &s, &mut Reached::default()).filter(|g| g.sig == f.sig) {
                (c, f) = (cut(&s, &g), g);
                continue 'outer;
            }
            if tries >= SHRINK_TRIES {
                break;
            }
        }
        break;
    }
    (c, f)
}

#[test]
fn generated_schemas_keep_engine_equal_to_reference() {
    let seeds = seeds_of(CASES);
    let mut reached = Reached::default();
    let idle = adtb::testing::idle_deltas_checked();
    let mut found = Vec::new();
    for seed in seeds.clone() {
        let c = schemas::case(seed, STATEMENTS);
        if let Some(f) = run(seed, &c, &mut reached) {
            found.push((seed, c, f));
        }
    }
    if let Some((seed, c, f)) = found.first() {
        let (small, why) = shrink(*seed, c, f);
        let mut report = format!(
            "{} of {} seeds disagree\nseed {seed}, shrunk: {}: {}\n{}\n",
            found.len(),
            seeds.end - seeds.start,
            why.sig,
            why.text,
            small.program()
        );
        for (seed, _, f) in &found {
            report.push_str(&format!("seed {seed}: {}: {}\n", f.sig, f.text));
        }
        panic!("{report}");
    }
    let cases = (seeds.end - seeds.start).min(CASES) as usize;
    reached.floors(|n| n * cases / CASES as usize);
    adtb_testkit::profile::assert_reached(
        "an incremental check whose triggering change is empty",
        (adtb::testing::idle_deltas_checked() - idle) as usize,
        cases.min(1),
    );
}
