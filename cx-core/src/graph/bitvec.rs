use super::nodes::NodeId;

/// PERFORMANCE CRITICAL: Bitset for visited tracking during traversal.
/// For 1M nodes, this is 128KB (fits in L2 cache).
/// A HashSet<u32> for the same would be ~8MB with pointer chasing.
pub struct BitVec {
    bits: Vec<u64>,
}

impl BitVec {
    /// Create a new BitVec that can hold `capacity` bits, all initially unset.
    pub fn new(capacity: u32) -> Self {
        let words = (capacity as usize).div_ceil(64);
        Self {
            bits: vec![0u64; words],
        }
    }

    /// Set the bit at position `id`.
    #[inline(always)]
    pub fn set(&mut self, id: NodeId) {
        let word = id as usize / 64;
        let bit = id % 64;
        self.bits[word] |= 1u64 << bit;
    }

    /// Test whether the bit at position `id` is set.
    #[inline(always)]
    pub fn test(&self, id: NodeId) -> bool {
        let word = id as usize / 64;
        let bit = id % 64;
        self.bits[word] & (1u64 << bit) != 0
    }

    /// Clear all bits without deallocating. Reuse between queries.
    #[inline]
    pub fn clear(&mut self) {
        self.bits.fill(0);
    }

    /// Return the capacity in bits.
    pub fn capacity(&self) -> u32 {
        (self.bits.len() * 64) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitvec_correctness() {
        // TEST bitvec_correctness from ARCHITECTURE.md:
        // Create BitVec for 1M nodes. Set nodes at positions 0, 1, 63, 64, 65, 999999.
        // test() returns true for set positions, false for all others.
        let mut bv = BitVec::new(1_000_000);

        let positions = [0u32, 1, 63, 64, 65, 999_999];
        for &pos in &positions {
            bv.set(pos);
        }

        for &pos in &positions {
            assert!(bv.test(pos), "bit {} should be set", pos);
        }

        // Check some positions that should NOT be set
        let unset = [2u32, 62, 66, 100, 1000, 500_000, 999_998];
        for &pos in &unset {
            assert!(!bv.test(pos), "bit {} should not be set", pos);
        }
    }

    #[test]
    fn bitvec_clear_reuse() {
        // TEST bitvec_clear_reuse from ARCHITECTURE.md:
        // Set 10K nodes. Clear. Verify all test() return false.
        let mut bv = BitVec::new(100_000);

        for i in 0..10_000u32 {
            bv.set(i);
        }

        // Verify they're set
        for i in 0..10_000u32 {
            assert!(bv.test(i));
        }

        bv.clear();

        // Verify all cleared — no stale bits
        for i in 0..10_000u32 {
            assert!(!bv.test(i), "bit {} should be cleared", i);
        }
    }

    #[test]
    fn bitvec_boundary_words() {
        // Test at word boundaries (multiples of 64)
        let mut bv = BitVec::new(256);
        bv.set(0);
        bv.set(63);
        bv.set(64);
        bv.set(127);
        bv.set(128);
        bv.set(255);

        assert!(bv.test(0));
        assert!(bv.test(63));
        assert!(bv.test(64));
        assert!(bv.test(127));
        assert!(bv.test(128));
        assert!(bv.test(255));
        assert!(!bv.test(1));
        assert!(!bv.test(62));
        assert!(!bv.test(129));
    }

    #[test]
    fn bitvec_capacity() {
        let bv = BitVec::new(100);
        // Rounds up to next multiple of 64
        assert!(bv.capacity() >= 100);
        assert_eq!(bv.capacity(), 128); // 2 words * 64

        let bv = BitVec::new(64);
        assert_eq!(bv.capacity(), 64);
    }

    #[test]
    fn bitvec_set_idempotent() {
        let mut bv = BitVec::new(100);
        bv.set(50);
        bv.set(50);
        assert!(bv.test(50));
    }
}
