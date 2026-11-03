//! A small seeded generator (SplitMix64), so the mock needs no `rand`.

/// Deterministic pseudo-random numbers from a seed.
#[derive(Debug, Clone)]
pub(crate) struct Rng(u64);

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `lo..=hi`.
    pub(crate) fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next_u64() % (hi - lo + 1)
    }

    /// True `percent` times in a hundred.
    pub(crate) fn chance(&mut self, percent: u64) -> bool {
        self.next_u64() % 100 < percent
    }

    /// One element of a non-empty slice, or `None` if it is empty.
    pub(crate) fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            return None;
        }
        let index = (self.next_u64() % items.len() as u64) as usize;
        items.get(index)
    }

    /// An index chosen with the given weights.
    pub(crate) fn weighted(&mut self, weights: &[u64]) -> usize {
        let total: u64 = weights.iter().sum();
        let mut roll = self.next_u64() % total.max(1);
        for (index, weight) in weights.iter().enumerate() {
            if roll < *weight {
                return index;
            }
            roll -= weight;
        }
        0
    }
}
