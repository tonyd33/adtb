use adtb::runtime::Entropy;
use adtb_sim::SimRuntime;

/// Returns a draw from `0..n` of the runtime's entropy.
pub fn below(rt: &SimRuntime, n: usize) -> usize {
    (rt.next_u64() % n as u64) as usize
}
