//! Remove dead row versions and index entries.

mod dead;
mod pace;
mod run;
#[cfg(test)]
mod vacuum_tests;
#[cfg(test)]
mod vacuum_threads_tests;
#[cfg(test)]
mod verify;

#[cfg(any(test, feature = "testing"))]
pub use pace::Charged;
pub use pace::Pacer;
pub use run::{
    Error, Freed, Horizon, Phase, Progress, Report, Step, Sum, expire, insert, update, vacuum,
    vacuum_keeping,
};
#[cfg(test)]
pub use run::{judge, prune};
#[cfg(test)]
pub use verify::verify;
