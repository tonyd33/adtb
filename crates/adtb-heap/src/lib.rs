//! Row versions in heaps, their history and vacuum, and structural verification.
#![forbid(unsafe_code)]

mod btree;
mod consumed;
pub mod heap;
pub mod history;
mod mvcc;
mod storage;
#[cfg(any(test, feature = "testing"))]
pub mod test_world;
pub mod vacuum;
pub mod verify;

#[cfg(test)]
use adtb_store::testing;
use adtb_store::{faults, key, ranks, steps};
use adtb_sys::runtime;
