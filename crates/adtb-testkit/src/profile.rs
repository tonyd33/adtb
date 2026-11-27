//! Sweep sizes by nextest profile: the default run, `quick` for iteration and `deep` for the
//! long sweeps.

use std::ops::Range;

/// How many times its default count a sweep runs under the nextest profile `deep`.
pub const DEEP: u64 = 8;

/// Returns whether the nextest profile `profile` runs.
pub fn is(profile: &str) -> bool {
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

/// Returns whether every case of a sweep runs, which the nextest profile `deep` asks.
pub fn every_case() -> bool {
    is("deep")
}

/// Returns the seeds from `seeds.start`, as many as [`count`] makes of its length.
pub fn sweep(seeds: Range<u64>) -> Range<u64> {
    seeds.start..seeds.start + count(seeds.end - seeds.start)
}

/// Returns `count` scaled from all of `seeds` to the seeds [`sweep`] keeps of them, rounded
/// down, and `count` itself when [`sweep`] keeps more than `seeds`.
pub fn share(seeds: Range<u64>, count: usize) -> usize {
    let all = seeds.end - seeds.start;
    let kept = sweep(seeds.clone()).end - seeds.start;
    count * kept.min(all) as usize / all as usize
}

/// Returns `most` of `items` in order, all of them when they are fewer, evenly spread: both ends
/// when `most` is at least two, the first alone when it is one, and none when it is zero.
pub fn spread<T: Copy>(items: &[T], most: u64) -> Vec<T> {
    let len = items.len() as u64;
    if len <= most || most < 2 {
        return items.iter().copied().take(most as usize).collect();
    }
    (0..most)
        .map(|i| items[(i * (len - 1) / (most - 1)) as usize])
        .collect()
}

/// Fail unless a sweep reached the case `what` at least `floor` times.
#[track_caller]
pub fn assert_reached(what: &str, count: usize, floor: usize) {
    assert!(
        count >= floor,
        "{what}: reached {count} times, under the floor of {floor}"
    );
}

#[cfg(test)]
mod tests {
    use super::spread;

    #[test]
    fn a_spread_keeps_order_both_ends_and_at_most_its_count_over_every_small_input() {
        for len in 0..12usize {
            let items: Vec<usize> = (0..len).collect();
            for most in 0..14u64 {
                let got = spread(&items, most);
                let want = len.min(most as usize);
                assert_eq!(want, got.len(), "{len} items, most {most}: {got:?}");
                assert!(
                    got.windows(2).all(|w| w[0] < w[1]),
                    "{len}, {most}: {got:?}"
                );
                if want >= 1 {
                    assert_eq!(Some(&0), got.first(), "{len}, {most}: {got:?}");
                }
                if want >= 2 {
                    assert_eq!(Some(&(len - 1)), got.last(), "{len}, {most}: {got:?}");
                }
            }
        }
    }
}
