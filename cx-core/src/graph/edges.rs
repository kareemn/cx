use super::nodes::NodeId;

/// Bitmask for edge kind filtering.
pub type EdgeKindMask = u16;

/// All 11 edge kinds.
pub const ALL_EDGES: EdgeKindMask = 0x07FF;
/// Service-level edges: DependsOn | Exposes | Consumes.
pub const SERVICE_EDGES: EdgeKindMask = (1 << 3) | (1 << 4) | (1 << 5);
/// Code-level edges: Calls | Imports.
pub const CODE_EDGES: EdgeKindMask = (1 << 1) | (1 << 2);

/// Edge kind discriminant.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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

/// PERFORMANCE CRITICAL: Hot-path edge stored in CSR edge array.
/// Fixed 16 bytes.
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
