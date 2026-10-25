#![forbid(unsafe_code)]

mod check;
mod disk;
mod rng;
mod sim;

pub use check::{Fate, Finding, FindingKind, Ledger, Recovered, Violation, check};
pub use disk::Faults;
pub use sim::{Config, Outcome, Sim, SimIo, SimRuntime};
