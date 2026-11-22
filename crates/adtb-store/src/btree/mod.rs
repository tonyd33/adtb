//! Hold the B-link trees and the union indexes over them.

pub mod build;
mod index;
mod leaf;
mod node;
mod tree;
mod verify;

#[cfg(test)]
mod btree_threads_tests;
#[cfg(test)]
mod tests;

pub use index::{Index, check_key};
#[cfg(test)]
pub use node::FORMAT;
#[cfg(any(test, feature = "testing"))]
pub use tree::before_next_stray_refusal;
pub use tree::{BTree, Error, prefix_end};
#[cfg(any(test, feature = "testing"))]
pub use tree::{Cursor, Entries};
