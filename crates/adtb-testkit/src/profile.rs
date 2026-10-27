//! Sweep sizes by nextest profile: the default run, `quick` for iteration and `deep` for the
//! long sweeps.

use std::ops::Range;

/// How many times its default count a sweep runs under the nextest profile `deep`.
pub const DEEP: u64 = 8;

fn is(profile: &str) -> bool {
    std::env::var("NEXTEST_PROFILE").is_ok_and(|p| p == profile)
}

/// Returns `n`, times [`DEEP`] under the nextest profile `deep`.
pub fn deep(n: u64) -> u64 {
    if is("deep") { n * DEEP } else { n }
}

/// Returns `n` as [`deep`] sizes it, cut to at most one under the nextest profile `quick`.
pub fn count(n: u64) -> u64 {
    if is("quick") { n.min(1) } else { deep(n) }
}

/// Returns the seeds from `seeds.start`, as many as [`count`] makes of its length.
pub fn sweep(seeds: Range<u64>) -> Range<u64> {
    seeds.start..seeds.start + count(seeds.end - seeds.start)
}
