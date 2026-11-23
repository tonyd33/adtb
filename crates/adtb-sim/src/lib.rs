//! Simulated storage, clock, randomness and scheduler, with fault injection.
#![forbid(unsafe_code)]

macro_rules! trace {
    ($trace:expr, $($line:tt)+) => {
        writeln!($trace, $($line)+).expect("a trace accepts every write")
    };
}

/// The lines a simulation writes, kept unless tracing is off.
#[derive(Debug)]
struct Trace(Option<String>);

impl std::fmt::Write for Trace {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if let Some(text) = &mut self.0 {
            text.push_str(s);
        }
        Ok(())
    }
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
