//! Run a simulated process that may crash, at chosen times, and check what each crash leaves.

use std::collections::BTreeSet;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;
use std::time::Duration;

use adtb_sim::{
    Finding, FindingKind, Ledger, Outcome, Recovered, Sim, SimRuntime, Violation, check,
};
use adtb_sys::runtime::FatalIo;
use adtb_sys::runtime::Faults;

/// Keep panic-hook reports of [`FatalIo`] off stderr for the rest of the process.
pub fn silence_fatal_panics() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !info.payload().is::<FatalIo>() {
                default(info);
            }
        }));
    });
}

/// Run `body` as a process, treating a crash caused by a failed write or force as a crash.
pub fn run_process<F: FnOnce(SimRuntime)>(sim: &mut Sim, body: F) -> Outcome {
    match panic::catch_unwind(AssertUnwindSafe(|| sim.run(body))) {
        Ok(o) => o,
        Err(payload) if payload.is::<FatalIo>() => Outcome::Crashed,
        Err(payload) => panic::resume_unwind(payload),
    }
}

/// Returns how long round `round` of seed `seed` runs before its crash: `base` microseconds and
/// a spread below `span` more.
pub fn crash_after(seed: u64, round: u64, base: u64, span: u64) -> Duration {
    Duration::from_micros(base + (seed * 7919 + round * 104_729) % span)
}

/// The rounds of a crash loop and when each crashes, as [`crash_after`] takes them.
#[derive(Clone, Copy, Debug)]
pub struct Rounds {
    count: usize,
    base: u64,
    span: u64,
}

impl Rounds {
    /// Returns `count` rounds, each crashing between `base` and `base + span` microseconds in.
    pub const fn new(count: usize, base: u64, span: u64) -> Rounds {
        Rounds { count, base, span }
    }
}

/// A database reopened after a round of a crash loop.
pub trait Reopened: Recovered {
    /// Returns whether the round's faults account for `v`. The loop returns such a violation
    /// with its round instead of failing.
    fn allows(&self, _v: &Violation) -> bool {
        false
    }

    /// Settle each commit `ledger` holds in doubt by what this database shows.
    fn settle(&self, ledger: &Ledger) {
        ledger.settle(&self.committed());
    }

    /// Returns whether the loop ends after this round.
    fn last(&self) -> bool {
        false
    }
}

/// A reopened database that reports the commits it holds and each problem a test found in it.
#[derive(Clone, Debug, Default)]
pub struct Reopen {
    committed: BTreeSet<u64>,
    problems: Vec<String>,
}

impl Reopen {
    /// Returns a database holding `committed`, in which a test found `problems`.
    pub fn new(committed: BTreeSet<u64>, problems: Vec<String>) -> Reopen {
        Reopen {
            committed,
            problems,
        }
    }

    /// Returns a database holding no commits, in which a test found `problems`.
    pub fn found(problems: Vec<String>) -> Reopen {
        Reopen::new(BTreeSet::new(), problems)
    }

    pub fn problems(&self) -> &[String] {
        &self.problems
    }
}

impl Recovered for Reopen {
    fn committed(&self) -> BTreeSet<u64> {
        self.committed.clone()
    }

    /// Returns each problem as a logic finding.
    fn verify(&self) -> Vec<Finding> {
        self.problems
            .iter()
            .map(|at| Finding {
                kind: FindingKind::Logic,
                at: at.clone(),
            })
            .collect()
    }
}

impl Reopened for Reopen {}

/// One round of a crash loop, as its workload runs it.
pub struct Round<'a> {
    sim: &'a mut Sim,
    ledger: &'a Ledger,
    seed: u64,
    index: usize,
    faults: Faults,
    rounds: Rounds,
    crashed: bool,
}

impl std::fmt::Debug for Round<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Round")
            .field("seed", &self.seed)
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl Round<'_> {
    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// Returns the client's record of commits, which outlives every crash of the loop.
    pub fn ledger(&self) -> &Ledger {
        self.ledger
    }

    /// Run `body` as a process under the loop's faults, crashing at this round's time. Returns
    /// whether it crashed.
    pub fn crash(&mut self, body: impl FnOnce(SimRuntime)) -> bool {
        self.sim.set_faults(self.faults);
        let after = crash_after(
            self.seed,
            self.index as u64,
            self.rounds.base,
            self.rounds.span,
        );
        let at = self.sim.now() + after;
        self.sim.crash_at(at);
        let crashed = run_process(self.sim, body) == Outcome::Crashed;
        self.crashed |= crashed;
        crashed
    }

    /// Run `setup` and then `body` with what it returns as a process under the loop's faults,
    /// crashing at this round's time counted from when `setup` returns. Returns whether it
    /// crashed.
    pub fn crash_after_setup<T>(
        &mut self,
        setup: impl FnOnce(&SimRuntime) -> T,
        body: impl FnOnce(SimRuntime, T),
    ) -> bool {
        self.sim.set_faults(self.faults);
        let after = crash_after(
            self.seed,
            self.index as u64,
            self.rounds.base,
            self.rounds.span,
        );
        let crashed = run_process(self.sim, |rt| {
            let set = setup(&rt);
            rt.crash_after(after);
            body(rt, set);
        }) == Outcome::Crashed;
        self.crashed |= crashed;
        crashed
    }

    /// Returns the simulation, for a round that ends its run another way than [`Round::crash`].
    pub fn sim(&mut self) -> &mut Sim {
        self.sim
    }
}

/// What one round of a crash loop left, once checked.
#[derive(Debug)]
pub struct Checked<T> {
    crashed: bool,
    allowed: Vec<Violation>,
    reopened: T,
}

impl<T> Checked<T> {
    /// Returns whether the round's workload crashed.
    pub fn crashed(&self) -> bool {
        self.crashed
    }

    /// Returns the violations the reopened database allowed.
    pub fn allowed(&self) -> &[Violation] {
        &self.allowed
    }

    pub fn reopened(&self) -> &T {
        &self.reopened
    }
}

/// What a crash loop left: each round's result and the ledger every round recorded in.
#[derive(Debug)]
pub struct Crashes<T> {
    rounds: Vec<Checked<T>>,
    ledger: Ledger,
}

impl<T> Crashes<T> {
    /// Returns each round's result, in order.
    pub fn rounds(&self) -> &[Checked<T>] {
        &self.rounds
    }

    /// Returns the ledger every round recorded in.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Returns each violation a round's reopened database allowed, with the round's index.
    pub fn allowed(&self) -> impl Iterator<Item = (usize, &Violation)> {
        self.rounds
            .iter()
            .enumerate()
            .flat_map(|(i, r)| r.allowed.iter().map(move |v| (i, v)))
    }

    /// Returns how many rounds crashed.
    pub fn crashed(&self) -> usize {
        self.rounds.iter().filter(|r| r.crashed).count()
    }
}

/// Run `workload` for each of `rounds`, reopening after each through `recovered` with no faults
/// and checking the reopened database against the ledger. Returns each round's result and the
/// ledger.
///
/// Preconditions:
/// - `sim` was seeded `seed` and holds what the workload opens.
///
/// Postconditions:
/// - Every round ends in a run of `recovered` that finishes, then [`check`] of its result against
///   the loop's ledger, which holds every commit the workloads recorded. The loop panics, naming
///   the seed and round, at a run that does not finish or at a violation the result does not
///   allow.
/// - After a passing check, the result settles the commits left in doubt.
/// - The loop ends after the last round or the first round whose result is the last.
pub fn crash_loop<T: Reopened>(
    sim: &mut Sim,
    seed: u64,
    faults: Faults,
    rounds: Rounds,
    mut workload: impl FnMut(&mut Round<'_>),
    mut recovered: impl FnMut(SimRuntime, &Ledger) -> T,
) -> Crashes<T> {
    silence_fatal_panics();
    let quiet = Faults {
        atomic_unit: faults.atomic_unit,
        ..Faults::none(faults.sector)
    };
    let ledger = Ledger::new();
    let mut out = Vec::new();
    for index in 0..rounds.count {
        let mut round = Round {
            sim,
            ledger: &ledger,
            seed,
            index,
            faults,
            rounds,
            crashed: false,
        };
        workload(&mut round);
        let crashed = round.crashed;
        sim.set_faults(quiet);
        let mut reopened = None;
        let outcome = sim.run(|rt| reopened = Some(recovered(rt, &ledger)));
        assert_eq!(
            Outcome::Finished,
            outcome,
            "seed {seed} round {index}: the reopen"
        );
        let reopened = reopened.expect("a finished run returns");
        let (allowed, violations): (Vec<Violation>, Vec<Violation>) = check(&ledger, &reopened)
            .into_iter()
            .partition(|v| reopened.allows(v));
        assert_eq!(
            Vec::<Violation>::new(),
            violations,
            "seed {seed} round {index}"
        );
        reopened.settle(&ledger);
        let last = reopened.last();
        out.push(Checked {
            crashed,
            allowed,
            reopened,
        });
        if last {
            break;
        }
    }
    Crashes {
        rounds: out,
        ledger,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::panic;

    use adtb_sim::{Config, Finding};

    use super::*;

    const ROUNDS: Rounds = Rounds::new(3, 100, 1000);

    /// A database that reports `committed` and no finding.
    #[derive(Debug)]
    struct Shows(BTreeSet<u64>);

    impl Recovered for Shows {
        fn committed(&self) -> BTreeSet<u64> {
            self.0.clone()
        }

        fn verify(&self) -> Vec<Finding> {
            Vec::new()
        }
    }

    impl Reopened for Shows {}

    fn message(payload: &(dyn std::any::Any + Send)) -> &str {
        payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .expect("a formatted panic")
    }

    #[test]
    fn a_commit_settled_in_one_round_and_missing_in_the_next_fails_that_round() {
        let run = || {
            let mut sim = Sim::new(5, Config::default());
            let mut shown = vec![BTreeSet::from([1]), BTreeSet::new(), BTreeSet::new()].into_iter();
            crash_loop(
                &mut sim,
                5,
                Faults::none(512),
                ROUNDS,
                |r| {
                    if r.index() == 0 {
                        r.ledger().submit(1);
                    }
                },
                |_, _| Shows(shown.next().expect("a round's database")),
            )
        };
        let failed = panic::catch_unwind(run).expect_err("the loop fails");
        assert_eq!(
            "assertion `left == right` failed: seed 5 round 1\n  left: []\n right: [Lost(1)]",
            message(&*failed)
        );
    }

    #[test]
    fn every_round_reopens_and_returns_what_its_database_allowed() {
        let mut sim = Sim::new(6, Config::default());
        let mut reopens = 0;
        let rounds = crash_loop(
            &mut sim,
            6,
            Faults::none(512),
            ROUNDS,
            |r| {
                let id = r.index() as u64;
                r.ledger().submit(id);
                r.ledger().acknowledge(id);
            },
            |_, ledger| {
                reopens += 1;
                assert_eq!(Some(adtb_sim::Fate::Acknowledged), ledger.fate(0));
                Lossy
            },
        );
        assert_eq!(3, reopens);
        let allowed: Vec<(usize, &Violation)> = rounds.allowed().collect();
        assert_eq!(
            vec![
                (0, &Violation::Lost(0)),
                (1, &Violation::Lost(0)),
                (1, &Violation::Lost(1)),
                (2, &Violation::Lost(0)),
                (2, &Violation::Lost(1)),
                (2, &Violation::Lost(2)),
            ],
            allowed
        );
    }

    /// A database that keeps no commit, and whose faults account for losing one.
    #[derive(Debug)]
    struct Lossy;

    impl Recovered for Lossy {
        fn committed(&self) -> BTreeSet<u64> {
            BTreeSet::new()
        }

        fn verify(&self) -> Vec<Finding> {
            Vec::new()
        }
    }

    impl Reopened for Lossy {
        fn allows(&self, v: &Violation) -> bool {
            matches!(v, Violation::Lost(_))
        }
    }
}
