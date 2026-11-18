//! The worker threads a search runs its plans and cases on.

use std::any::Any;
use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use super::case::{self, Outcome, Plan, Point};
use super::generate;

/// The plans and cases one test runs at once. The damage and deep profiles give each damage
/// search test as many nextest slots.
pub const WIDTH: usize = 4;

/// A job's panic and the job, named.
pub struct Panicked {
    job: String,
    payload: Box<dyn Any + Send>,
}

impl Panicked {
    /// Panic with the job's message, naming the job.
    pub fn raise(self) -> ! {
        let message = match self.payload.downcast::<String>() {
            Ok(text) => *text,
            Err(payload) => payload.downcast::<&str>().map_or_else(
                |_| "a panic with no message".to_owned(),
                |text| (*text).to_owned(),
            ),
        };
        panic!("{}: {message}", self.job)
    }
}

/// A job's result, or its panic.
pub type Ran<R> = Result<R, Panicked>;

/// The outcomes of cases, each or its panic.
pub type Outcomes = Vec<Ran<Option<Outcome>>>;

/// Returns `f` of each of `jobs`, in order, running up to [`WIDTH`] of them at once. Raises the
/// panic of the first job that panics, named by `name`.
pub fn map<J: Sync, R: Send>(
    jobs: &[J],
    name: impl Fn(&J) -> String + Sync,
    f: impl Fn(&J) -> R + Sync,
) -> Vec<R> {
    try_map(jobs, name, f)
        .into_iter()
        .map(|r| r.unwrap_or_else(|p| p.raise()))
        .collect()
}

/// Returns `f` of each of `jobs`, in order, or the job's panic named by `name`, running up to
/// [`WIDTH`] of them at once.
pub fn try_map<J: Sync, R: Send>(
    jobs: &[J],
    name: impl Fn(&J) -> String + Sync,
    f: impl Fn(&J) -> R + Sync,
) -> Vec<Ran<R>> {
    let next = AtomicUsize::new(0);
    let mut out: Vec<Option<Ran<R>>> = jobs.iter().map(|_| None).collect();
    thread::scope(|s| {
        let workers: Vec<_> = (0..WIDTH.min(jobs.len()))
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(job) = jobs.get(i) else {
                            return done;
                        };
                        let ran = panic::catch_unwind(AssertUnwindSafe(|| f(job)));
                        done.push((
                            i,
                            ran.map_err(|payload| Panicked {
                                job: name(job),
                                payload,
                            }),
                        ));
                    }
                })
            })
            .collect();
        for worker in workers {
            match worker.join() {
                Ok(done) => {
                    for (i, r) in done {
                        out[i] = Some(r);
                    }
                }
                Err(payload) => panic::resume_unwind(payload),
            }
        }
    });
    out.into_iter()
        .map(|r| r.expect("a worker ran every job it took"))
        .collect()
}

/// One case of a plan, and for a pair its second point.
pub type Case = (Point, Option<Point>);

/// The plans a test made, by seed, for its later runs of the same seeds with the same setting of
/// a vacuum beside each write.
pub type Kept = BTreeMap<u64, Arc<Plan>>;

/// The seeds of a run of sequences still to plan, and the plans kept from earlier runs.
pub struct Seeds<'a> {
    seeds: &'a [u64],
    concurrent: bool,
    kept: Option<&'a mut Kept>,
}

enum Job<'p> {
    Plan(u64),
    Case(&'p Plan, Case),
}

enum Made {
    Plan(Box<Plan>),
    Case(Option<Outcome>),
}

impl<'a> Seeds<'a> {
    /// Returns the seeds `seeds`, planned with a vacuum beside each write when `concurrent`,
    /// taking and keeping plans in `kept`.
    pub fn new(seeds: &'a [u64], concurrent: bool, kept: Option<&'a mut Kept>) -> Self {
        Self {
            seeds,
            concurrent,
            kept,
        }
    }

    /// Run `cases` and, while fewer than [`WIDTH`] plans are `held`, plan the next seeds until
    /// twice as many are. Returns the new plans in seed order and each case's outcome in order,
    /// each or its panic.
    pub fn run(&mut self, held: usize, cases: &[(&Plan, Case)]) -> (Vec<Ran<Arc<Plan>>>, Outcomes) {
        let more = if held < WIDTH { 2 * WIDTH - held } else { 0 };
        let (now, later) = self.seeds.split_at(more.min(self.seeds.len()));
        self.seeds = later;
        let kept = self.kept.as_deref();
        let fresh: Vec<u64> = now
            .iter()
            .copied()
            .filter(|s| kept.is_none_or(|k| !k.contains_key(s)))
            .collect();
        let mut jobs: Vec<Job<'_>> = fresh.iter().map(|s| Job::Plan(*s)).collect();
        jobs.extend(cases.iter().map(|(plan, c)| Job::Case(plan, *c)));
        let concurrent = self.concurrent;
        let name = |job: &Job<'_>| match job {
            Job::Plan(seed) => format!("the plan of seed {seed}"),
            Job::Case(plan, (first, None)) => {
                format!("seed {} at {} {:?}", plan.seed, first.at, first.injection)
            }
            Job::Case(plan, (first, Some(second))) => format!(
                "seed {} at {} {:?} then {} {:?}",
                plan.seed, first.at, first.injection, second.at, second.injection
            ),
        };
        let made = try_map(&jobs, name, |job| match job {
            Job::Plan(seed) => Made::Plan(Box::new(case::plan(
                *seed,
                generate::sequence(*seed),
                concurrent,
            ))),
            Job::Case(plan, (first, None)) => Made::Case(case::run(plan, *first)),
            Job::Case(plan, (first, Some(second))) => {
                Made::Case(case::run_pair(plan, *first, *second))
            }
        });
        let mut made = made.into_iter();
        let mut planned: BTreeMap<u64, Ran<Arc<Plan>>> = fresh
            .iter()
            .zip(made.by_ref())
            .map(|(seed, m)| {
                let plan = m.map(|m| match m {
                    Made::Plan(p) => Arc::new(*p),
                    Made::Case(_) => unreachable!("a plan job makes a plan"),
                });
                (*seed, plan)
            })
            .collect();
        let outcomes = made
            .map(|m| {
                m.map(|m| match m {
                    Made::Case(o) => o,
                    Made::Plan(_) => unreachable!("a case job makes an outcome"),
                })
            })
            .collect();
        let plans = now
            .iter()
            .map(|seed| match planned.remove(seed) {
                Some(plan) => plan,
                None => Ok(self.kept.as_deref().expect("an unplanned seed is kept")[seed].clone()),
            })
            .collect::<Vec<_>>();
        if let Some(kept) = self.kept.as_deref_mut() {
            for plan in plans.iter().flatten() {
                kept.insert(plan.seed, plan.clone());
            }
        }
        (plans, outcomes)
    }
}

/// Returns, for each profile of the nextest `config`, the slots its override of every damage
/// search test requires.
fn damage_slots(config: &str) -> Vec<(String, usize)> {
    let mut slots = Vec::new();
    let (mut profile, mut damage) = (None, false);
    for line in config.lines() {
        if let Some(header) = line.strip_prefix("[[profile.") {
            profile = header.strip_suffix(".overrides]]");
            damage = false;
        } else if line.starts_with('[') {
            (profile, damage) = (None, false);
        } else if line == r#"filter = "binary(=storage) and test(/^damage_search::/)""# {
            damage = true;
        } else if let (Some(profile), true, Some(n)) =
            (profile, damage, line.strip_prefix("threads-required = "))
        {
            slots.push((profile.to_owned(), n.parse().expect("a slot count")));
        }
    }
    slots
}

#[test]
fn a_damage_search_test_holds_a_slot_for_each_job_it_runs() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.config/nextest.toml");
    let config = std::fs::read_to_string(path).expect("the nextest config reads");
    assert_eq!(
        vec![("damage".to_owned(), WIDTH), ("deep".to_owned(), WIDTH)],
        damage_slots(&config)
    );
}

#[test]
fn a_map_raises_the_panic_of_the_first_job_that_panics_named() {
    let jobs: Vec<usize> = (0..4 * WIDTH).collect();
    let raised = panic::catch_unwind(|| {
        map(
            &jobs,
            |j| format!("job {j}"),
            |&j| assert!(j % 5 != 3, "job {j} fails"),
        )
    })
    .expect_err("jobs 3, 8 and 13 panic");
    assert_eq!(
        Some("job 3: job 3 fails"),
        raised.downcast_ref::<String>().map(String::as_str)
    );
}
