/// Unique node identifier.
pub type NodeId = u32;

/// Index into the interned string table.
pub type StringId = u32;

/// Repository identifier.
pub type RepoId = u16;

/// Node kind discriminant.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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

/// PERFORMANCE CRITICAL: Hot-path node stored in CSR arrays.
/// Fixed 32 bytes. No heap allocations.
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
