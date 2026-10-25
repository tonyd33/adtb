/// SplitMix64: a seeded, portable generator whose output depends only on the seed.
#[derive(Clone, Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// Returns an independent generator, advancing `self` by one draw.
    pub(crate) fn split(&mut self) -> Rng {
        Rng(self.next_u64())
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Returns a uniform draw from `0..n`. Panics if `n` is zero.
    pub(crate) fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0);
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    /// Returns a uniform draw from `lo..=hi`.
    pub(crate) fn between(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    pub(crate) fn chance(&mut self, p: f64) -> bool {
        if p <= 0.0 {
            return false;
        }
        ((self.next_u64() >> 11) as f64) * (1.0 / (1u64 << 53) as f64) < p
    }

    pub(crate) fn coin(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}
