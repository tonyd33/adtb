use std::ops::Range;

pub use adtb_testkit::profile::sweep;

/// Returns `count` scaled from all of `seeds` to the seeds [`sweep`] keeps of them, rounded
/// down, and `count` itself when [`sweep`] keeps more than `seeds`.
pub fn share(seeds: Range<u64>, count: usize) -> usize {
    let all = seeds.end - seeds.start;
    let kept = sweep(seeds.clone()).end - seeds.start;
    count * kept.min(all) as usize / all as usize
}
