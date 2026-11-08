//! Hold the page size, file name and options of pager tests.

use adtb::storage::Options;
pub use adtb_harness::process::finished;

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
