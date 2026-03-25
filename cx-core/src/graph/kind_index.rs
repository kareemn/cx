/// Index into the nodes array by NodeKind.
/// Avoids full scan when looking for specific kinds.
pub struct KindIndex {
    /// kind_ranges[k] = (start, end) indices into the nodes array for NodeKind k.
    pub kind_ranges: [(u32, u32); 8],
}
