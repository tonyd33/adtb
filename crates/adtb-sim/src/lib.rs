#![forbid(unsafe_code)]

macro_rules! trace {
    ($trace:expr, $($line:tt)+) => {
        writeln!($trace, $($line)+).expect("a String accepts every write")
    };
}

mod check;
mod disk;
#[cfg(all(debug_assertions, not(test)))]
mod instances;
mod rng;
mod sim;

pub use check::{Fate, Finding, FindingKind, Ledger, Recovered, Violation, check};
pub use disk::{Faults, Recording};
pub use sim::{Config, Outcome, Sim, SimIo, SimRuntime};
