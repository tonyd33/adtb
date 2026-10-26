use std::ops::Range;

/// Returns `seeds`, cut to its first seed under the nextest profile `quick`.
pub fn sweep(seeds: Range<u64>) -> Range<u64> {
    if std::env::var_os("NEXTEST_PROFILE").is_some_and(|p| p == "quick") {
        seeds.start..seeds.end.min(seeds.start + 1)
    } else {
        seeds
    }
}

/// Returns `count` scaled from all of `seeds` to the seeds [`sweep`] keeps of them, rounded
/// down.
pub fn share(seeds: Range<u64>, count: usize) -> usize {
    let kept = sweep(seeds.clone());
    count * (kept.end - kept.start) as usize / (seeds.end - seeds.start) as usize
}
