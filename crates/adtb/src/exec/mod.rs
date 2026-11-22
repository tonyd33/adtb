//! The executor, with the tests of it that open a database.

pub use adtb_exec::exec::*;

#[cfg(test)]
mod exec_tests;
#[cfg(test)]
mod host_fns_tests;
