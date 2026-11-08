//! Draw random numbers from a simulated runtime.

use adtb_sim::SimRuntime;
use adtb_sys::runtime::Entropy;

/// Returns a draw from `0..n` of the runtime's entropy.
pub fn below(rt: &SimRuntime, n: usize) -> usize {
    (rt.next_u64() % n as u64) as usize
}
