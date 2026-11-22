//! Run a process of a simulation that must finish.

use adtb_sim::{Config, Outcome, Sim, SimRuntime};

/// Run `body` as one process of `sim` and return what it returns.
#[inline]
pub fn run<T>(sim: &mut Sim, body: impl FnOnce(SimRuntime) -> T) -> T {
    ran(sim, Box::new(body), "the run")
}

/// Run `body` as the one process of a new simulation seeded `seed` and return what it returns.
#[inline]
pub fn once<T>(seed: u64, body: impl FnOnce(SimRuntime) -> T) -> T {
    let mut sim = Sim::new(seed, Config::default());
    ran(&mut sim, Box::new(body), &format!("the run of seed {seed}"))
}

/// Run `body` as a process of `sim` that must finish.
#[inline]
pub fn finished(sim: &mut Sim, body: impl FnOnce(SimRuntime)) {
    ran(sim, Box::new(body), "the run");
}

fn ran<T>(sim: &mut Sim, body: Box<dyn FnOnce(SimRuntime) -> T + '_>, what: &str) -> T {
    let mut got = None;
    let outcome = sim.run(|rt| got = Some(body(rt)));
    assert_eq!(Outcome::Finished, outcome, "{what} finishes");
    got.expect("the run returns")
}
