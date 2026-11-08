//! Hold the B-link trees and the union indexes over them.

pub mod build;
mod index;
mod leaf;
mod node;
mod tree;
mod verify;

#[cfg(test)]
mod btree_tests;
#[cfg(test)]
mod btree_threads_tests;
#[cfg(test)]
mod tests;

pub use index::Index;
#[cfg(test)]
pub use index::{SumKey, tag_prefix};
#[cfg(test)]
pub use node::FORMAT;
pub use tree::{BTree, Error, prefix_end};
#[cfg(test)]
pub use tree::{Cursor, Entries};
