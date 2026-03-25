/// PERFORMANCE CRITICAL: Bitset for visited tracking during traversal.
/// For 1M nodes, this is 128KB (fits in L2 cache).
pub struct BitVec {
    pub bits: Vec<u64>,
}
