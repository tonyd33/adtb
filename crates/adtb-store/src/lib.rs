//! The engine's store: keys, the file, its pages and log, transactions and B-link trees.
#![forbid(unsafe_code)]

pub mod btree;
pub mod faults;
pub mod key;
pub mod mvcc;
pub mod ranks;
pub mod steps;
pub mod storage;
#[cfg(any(test, feature = "testing"))]
pub mod test_world;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

use std::any::Any;

pub use adtb_sys::runtime;

/// Returns the text of a `String` or `&str` panic payload.
pub fn panic_text(payload: &(dyn Any + Send)) -> Option<String> {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
}
