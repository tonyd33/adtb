//! Build the component world of heap tests over the store's pager world.

pub mod fixture;
pub mod recovery;
pub mod rows;

pub use adtb_store::test_world::*;
#[cfg(test)]
pub use adtb_testkit::process::finished;
