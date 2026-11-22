//! The store's transactions, with the tests of the commit log that verify what it holds.

pub use adtb_store::mvcc::*;

#[cfg(test)]
mod clog_tests;
