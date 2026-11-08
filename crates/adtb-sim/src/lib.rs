//! Simulated storage, clock, randomness and scheduler, with fault injection.
#![forbid(unsafe_code)]

macro_rules! trace {
    ($trace:expr, $($line:tt)+) => {
        writeln!($trace, $($line)+).expect("a String accepts every write")
    };
}

mod check;
mod disk;
mod plan;
mod rng;
mod sim;

pub use check::{Fate, Finding, FindingKind, Ledger, Recovered, Violation, check};
pub use disk::Recording;
pub use plan::{Action, Count, Match, Numbered, OpLog, Plan, Planned, Struck, Submitted};
pub use sim::{Config, Outcome, Sim, SimIo, SimRuntime};
