/// Unique node identifier. u32 is sufficient (4 billion nodes) and halves memory on 64-bit.
pub type NodeId = u32;

/// Index into the interned string table.
pub type StringId = u32;

/// Repository identifier (max 65535 repos).
pub type RepoId = u16;

/// Sentinel value indicating no string.
pub const STRING_NONE: StringId = u32::MAX;

/// Sentinel value indicating no node.
pub const NODE_NONE: NodeId = u32::MAX;

/// Node kind discriminant.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NodeKind {
    Repo = 0,
    Deployable = 1,
    Module = 2,
    Symbol = 3,
    Endpoint = 4,
    Surface = 5,
    InfraConfig = 6,
    Resource = 7,
}

impl NodeKind {
    /// Total number of node kinds.
    pub const COUNT: usize = 8;

    /// Convert from u8 discriminant. Returns None for invalid values.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Repo),
            1 => Some(Self::Deployable),
            2 => Some(Self::Module),
            3 => Some(Self::Symbol),
            4 => Some(Self::Endpoint),
            5 => Some(Self::Surface),
            6 => Some(Self::InfraConfig),
            7 => Some(Self::Resource),
            _ => None,
        }
    }
}

/// Node flag bitflags.
pub const NODE_IS_ENTRY_POINT: u16 = 1 << 0;
pub const NODE_IS_PUBLIC: u16 = 1 << 1;
pub const NODE_IS_DEPRECATED: u16 = 1 << 2;
pub const NODE_IS_GENERATED: u16 = 1 << 3;
pub const NODE_IS_TEST: u16 = 1 << 4;

/// PERFORMANCE CRITICAL: Hot-path node stored in CSR arrays.
/// Fixed 32 bytes. No heap allocations. No String fields. No Vec fields.
/// All variable-length data lives in side tables referenced by u32 IDs.
#[repr(C, align(32))]
#[derive(Clone, Copy)]
pub struct Node {
    pub id: NodeId,
    pub kind: u8,
    pub sub_kind: u8,
    pub flags: u16,
    pub name: StringId,
    pub file: StringId,
    pub line: u32,
    pub parent: NodeId,
    pub repo: RepoId,
    pub _pad: [u8; 2],
}

impl Node {
    /// Create a new node with the given fields and sensible defaults for the rest.
    pub fn new(id: NodeId, kind: NodeKind, name: StringId) -> Self {
        Self {
            id,
            kind: kind as u8,
            sub_kind: 0,
            flags: 0,
            name,
            file: STRING_NONE,
            line: 0,
            parent: NODE_NONE,
            repo: 0,
            _pad: [0; 2],
        }
    }

    /// Get the NodeKind enum value.
    pub fn node_kind(&self) -> Option<NodeKind> {
        NodeKind::from_u8(self.kind)
    }
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("sub_kind", &self.sub_kind)
            .field("flags", &self.flags)
            .field("name", &self.name)
            .field("file", &self.file)
            .field("line", &self.line)
            .field("parent", &self.parent)
            .field("repo", &self.repo)
            .finish()
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.kind == other.kind
            && self.sub_kind == other.sub_kind
            && self.flags == other.flags
            && self.name == other.name
            && self.file == other.file
            && self.line == other.line
            && self.parent == other.parent
            && self.repo == other.repo
    }
}

impl Eq for Node {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_size_is_32_bytes() {
        assert_eq!(std::mem::size_of::<Node>(), 32);
    }

    #[test]
    fn node_alignment_is_32() {
        assert_eq!(std::mem::align_of::<Node>(), 32);
    }

    #[test]
    fn node_kind_roundtrip() {
        for kind_u8 in 0..=7u8 {
            let kind = NodeKind::from_u8(kind_u8).unwrap();
            assert_eq!(kind as u8, kind_u8);
        }
        assert!(NodeKind::from_u8(8).is_none());
        assert!(NodeKind::from_u8(255).is_none());
    }

    #[test]
    fn node_new_defaults() {
        let n = Node::new(42, NodeKind::Symbol, 10);
        assert_eq!(n.id, 42);
        assert_eq!(n.kind, NodeKind::Symbol as u8);
        assert_eq!(n.sub_kind, 0);
        assert_eq!(n.flags, 0);
        assert_eq!(n.name, 10);
        assert_eq!(n.file, STRING_NONE);
        assert_eq!(n.line, 0);
        assert_eq!(n.parent, NODE_NONE);
        assert_eq!(n.repo, 0);
    }

    #[test]
    fn node_flags() {
        let mut n = Node::new(0, NodeKind::Symbol, 0);
        n.flags = NODE_IS_PUBLIC | NODE_IS_ENTRY_POINT;
        assert_ne!(n.flags & NODE_IS_PUBLIC, 0);
        assert_ne!(n.flags & NODE_IS_ENTRY_POINT, 0);
        assert_eq!(n.flags & NODE_IS_DEPRECATED, 0);
    }

    #[test]
    fn node_is_copy() {
        let n = Node::new(1, NodeKind::Repo, 5);
        let n2 = n; // Copy
        assert_eq!(n.id, n2.id);
    }

    #[test]
    fn node_kind_ordering() {
        assert!(NodeKind::Repo < NodeKind::Deployable);
        assert!(NodeKind::Deployable < NodeKind::Module);
        assert!(NodeKind::Module < NodeKind::Symbol);
        assert!(NodeKind::Symbol < NodeKind::Endpoint);
    }

    #[test]
    fn two_nodes_per_cache_line() {
        // 64-byte cache line, 32-byte node = 2 nodes per cache line
        assert_eq!(64 / std::mem::size_of::<Node>(), 2);
    }
}
