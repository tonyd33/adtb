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

pub use index::{Index, SumKey, tag_prefix, tag_type};
pub use node::FORMAT;
pub use tree::{BTree, Cursor, Entries, Error, prefix_end};
