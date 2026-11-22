//! Hold the page size, file name and options of pager tests, and the component world they build.

#[cfg(test)]
pub use adtb_testkit::process::finished;

use crate::runtime::Runtime;
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
pub fn reopen<R: Runtime>(rt: &R, options: &Options) -> Result<Pager<R>, OpenError> {
    Pager::open(rt.clone(), options).map(|(pager, _)| pager)
}
