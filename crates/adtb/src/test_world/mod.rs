//! Hold the page size, file name and options of pager tests, and the component world they build.

pub mod api;
pub mod fixture;
pub mod grants;
pub mod load;
pub mod recovery;
pub mod rows;
pub mod support;
pub mod write_path;

pub use adtb_harness::process::finished;
use adtb_sim::SimRuntime;

use crate::storage::{OpenError, Options, Pager};

pub const PAGE: usize = 4096;
pub const NAME: &str = "app.adt";

/// Returns the options of a small database file named [`NAME`] with `pool_frames` frames.
pub fn options(pool_frames: usize) -> Options {
    Options {
        page_size: PAGE as u32,
        log_block: 512,
        segment_size: 256 * 1024,
        pool_frames,
        ..Options::new(NAME)
    }
}

/// Open the pager of the file `options` name, recovering it from its log.
pub fn reopen(rt: &SimRuntime, options: &Options) -> Result<Pager<SimRuntime>, OpenError> {
    Pager::open(rt.clone(), options).map(|(pager, _)| pager)
}
