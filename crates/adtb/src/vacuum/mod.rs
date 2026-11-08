//! Remove dead row versions and index entries.

mod dead;
mod pace;
mod run;
#[cfg(test)]
mod vacuum_tests;
#[cfg(test)]
mod vacuum_threads_tests;
mod verify;

pub use dead::DeadSet;
pub use pace::{Charged, Pacer};
pub use run::{
    Error, Freed, Horizon, Phase, Progress, Report, Step, Sum, expire, insert, judge, judge_within,
    prune, update, vacuum, vacuum_keeping,
};
pub use verify::verify;
