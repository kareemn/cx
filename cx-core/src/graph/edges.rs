use super::nodes::NodeId;

/// Bitmask for edge kind filtering.
pub type EdgeKindMask = u16;

/// All 11 edge kinds.
pub const ALL_EDGES: EdgeKindMask = 0x07FF;
/// Service-level edges: DependsOn | Exposes | Consumes.
pub const SERVICE_EDGES: EdgeKindMask = (1 << 3) | (1 << 4) | (1 << 5);
/// Code-level edges: Calls | Imports.
pub const CODE_EDGES: EdgeKindMask = (1 << 1) | (1 << 2);

/// Edge flag bitflags.
pub const EDGE_IS_CROSS_REPO: u16 = 1 << 0;
pub const EDGE_IS_ASYNC: u16 = 1 << 1;
pub const EDGE_IS_INFERRED: u16 = 1 << 2;

/// No metadata sentinel.
pub const META_NONE: u32 = u32::MAX;

/// Edge kind discriminant. Values are sequential indices used with bitmask filtering:
/// `(1u16 << edge.kind) & mask != 0`
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EdgeKind {
    Contains = 0,
    Calls = 1,
    Imports = 2,
    DependsOn = 3,
    Exposes = 4,
    Consumes = 5,
    Configures = 6,
    Resolves = 7,
    Connects = 8,
    Publishes = 9,
    Subscribes = 10,
}

impl EdgeKind {
    /// Total number of edge kinds.
    pub const COUNT: usize = 11;

    /// Convert from u8 discriminant. Returns None for invalid values.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Contains),
            1 => Some(Self::Calls),
            2 => Some(Self::Imports),
            3 => Some(Self::DependsOn),
            4 => Some(Self::Exposes),
            5 => Some(Self::Consumes),
            6 => Some(Self::Configures),
            7 => Some(Self::Resolves),
            8 => Some(Self::Connects),
            9 => Some(Self::Publishes),
            10 => Some(Self::Subscribes),
            _ => None,
        }
    }

    /// Return the bitmask for this single edge kind.
    pub fn mask(self) -> EdgeKindMask {
        1u16 << (self as u8)
    }
}

/// PERFORMANCE CRITICAL: Hot-path edge stored in CSR edge array.
/// Fixed 16 bytes. Four edges per cache line.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct Edge {
    pub target: NodeId,
    pub kind: u8,
    pub confidence_u8: u8,
    pub flags: u16,
    pub meta_idx: u32,
    pub _pad: [u8; 4],
}

impl Edge {
    /// Create a new edge with sensible defaults.
    pub fn new(target: NodeId, kind: EdgeKind) -> Self {
        Self {
            target,
            kind: kind as u8,
            confidence_u8: 255, // full confidence by default
            flags: 0,
            meta_idx: META_NONE,
            _pad: [0; 4],
        }
    }

    /// Check if this edge matches the given bitmask filter.
    #[inline(always)]
    pub fn matches_mask(&self, mask: EdgeKindMask) -> bool {
        (1u16 << self.kind) & mask != 0
    }

    /// Get the EdgeKind enum value.
    pub fn edge_kind(&self) -> Option<EdgeKind> {
        EdgeKind::from_u8(self.kind)
    }
}

impl std::fmt::Debug for Edge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Edge")
            .field("target", &self.target)
            .field("kind", &self.kind)
            .field("confidence_u8", &self.confidence_u8)
            .field("flags", &self.flags)
            .field("meta_idx", &self.meta_idx)
            .finish()
    }
}

impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        self.target == other.target
            && self.kind == other.kind
            && self.confidence_u8 == other.confidence_u8
            && self.flags == other.flags
            && self.meta_idx == other.meta_idx
    }
}

impl Eq for Edge {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_size_is_16_bytes() {
        assert_eq!(std::mem::size_of::<Edge>(), 16);
    }

    #[test]
    fn edge_alignment_is_16() {
        assert_eq!(std::mem::align_of::<Edge>(), 16);
    }

    #[test]
    fn four_edges_per_cache_line() {
        assert_eq!(64 / std::mem::size_of::<Edge>(), 4);
    }

    #[test]
    fn edge_kind_roundtrip() {
        for kind_u8 in 0..=10u8 {
            let kind = EdgeKind::from_u8(kind_u8).unwrap();
            assert_eq!(kind as u8, kind_u8);
        }
        assert!(EdgeKind::from_u8(11).is_none());
        assert!(EdgeKind::from_u8(255).is_none());
    }

    #[test]
    fn bitmask_filtering() {
        let calls_edge = Edge::new(1, EdgeKind::Calls);
        let depends_edge = Edge::new(2, EdgeKind::DependsOn);
        let contains_edge = Edge::new(3, EdgeKind::Contains);

        // CODE_EDGES includes Calls and Imports
        assert!(calls_edge.matches_mask(CODE_EDGES));
        assert!(!depends_edge.matches_mask(CODE_EDGES));
        assert!(!contains_edge.matches_mask(CODE_EDGES));

        // SERVICE_EDGES includes DependsOn, Exposes, Consumes
        assert!(!calls_edge.matches_mask(SERVICE_EDGES));
        assert!(depends_edge.matches_mask(SERVICE_EDGES));
        assert!(!contains_edge.matches_mask(SERVICE_EDGES));

        // ALL_EDGES includes everything
        assert!(calls_edge.matches_mask(ALL_EDGES));
        assert!(depends_edge.matches_mask(ALL_EDGES));
        assert!(contains_edge.matches_mask(ALL_EDGES));
    }

    #[test]
    fn edge_kind_mask_values() {
        // Verify bitmask constants match the spec
        assert_eq!(SERVICE_EDGES, (1 << 3) | (1 << 4) | (1 << 5));
        assert_eq!(CODE_EDGES, (1 << 1) | (1 << 2));
        assert_eq!(ALL_EDGES, 0x07FF);

        // Verify individual kind masks
        assert_eq!(EdgeKind::Contains.mask(), 1 << 0);
        assert_eq!(EdgeKind::Calls.mask(), 1 << 1);
        assert_eq!(EdgeKind::Subscribes.mask(), 1 << 10);
    }

    #[test]
    fn edge_new_defaults() {
        let e = Edge::new(42, EdgeKind::Calls);
        assert_eq!(e.target, 42);
        assert_eq!(e.kind, EdgeKind::Calls as u8);
        assert_eq!(e.confidence_u8, 255);
        assert_eq!(e.flags, 0);
        assert_eq!(e.meta_idx, META_NONE);
    }

    #[test]
    fn edge_flags() {
        let mut e = Edge::new(0, EdgeKind::DependsOn);
        e.flags = EDGE_IS_CROSS_REPO | EDGE_IS_ASYNC;
        assert_ne!(e.flags & EDGE_IS_CROSS_REPO, 0);
        assert_ne!(e.flags & EDGE_IS_ASYNC, 0);
        assert_eq!(e.flags & EDGE_IS_INFERRED, 0);
    }

    #[test]
    fn edge_is_copy() {
        let e = Edge::new(1, EdgeKind::Calls);
        let e2 = e; // Copy
        assert_eq!(e.target, e2.target);
    }

    #[test]
    fn all_edges_mask_covers_all_kinds() {
        for kind_u8 in 0..EdgeKind::COUNT as u8 {
            let edge = Edge {
                target: 0,
                kind: kind_u8,
                confidence_u8: 255,
                flags: 0,
                meta_idx: META_NONE,
                _pad: [0; 4],
            };
            assert!(
                edge.matches_mask(ALL_EDGES),
                "EdgeKind {} not covered by ALL_EDGES",
                kind_u8
            );
        }
    }
}
