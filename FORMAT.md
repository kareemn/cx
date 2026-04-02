# .cxgraph Binary Format (v2)

## Overview

`.cxgraph` is the binary format for cx graph files. It is designed for memory-mapped (mmap) access with zero deserialization cost. All sections are page-aligned (4KB) for direct mmap.

## File Layout

```
Offset      Size        Content
─────────────────────────────────────────
0x000       64 bytes    Header
4KB         N * 32B     Nodes array
            N * 4B      Forward offsets (CSR)
4KB-aligned M * 16B     Edges array (forward)
            N * 4B      Reverse offsets (CSR)
            M * 16B     Edges array (reverse)
4KB-aligned variable    String table
```

## Header (64 bytes)

```c
struct GraphFileHeader {
    u8   magic[4];              // "CX01"
    u32  version;               // 2
    u32  node_count;
    u32  edge_count;
    u32  summary_node_count;    // reserved (0)
    u32  summary_edge_count;    // reserved (0)
    u32  string_table_size;     // bytes
    u8   _pad0[4];
    u64  nodes_offset;          // byte offset to nodes array
    u64  edges_offset;          // byte offset to forward edges array
    u64  strings_offset;        // byte offset to string table
    u32  checksum;              // CRC32 of header bytes [0..56)
    u8   _reserved[4];
};
```

All multi-byte integers are little-endian (native x86/ARM).

## Node (32 bytes, `#[repr(C, align(32))]`)

Two nodes per 64-byte cache line.

```c
struct Node {
    u32  id;          // unique node ID
    u8   kind;        // NodeKind enum (see below)
    u8   sub_kind;    // reserved
    u16  flags;       // node flags
    u32  name;        // StringId — index into string table
    u32  file;        // StringId — source file path (0xFFFFFFFF = none)
    u32  line;        // source line number
    u32  parent;      // parent node ID (0xFFFFFFFF = none)
    u16  repo;        // repository ID (0-65535)
    u8   _pad[6];
};
```

### NodeKind

| Value | Name | Description |
|-------|------|-------------|
| 0 | Repo | Repository root |
| 1 | Deployable | Service, container, binary |
| 2 | Module | Package, module (Go package, Python module) |
| 3 | Symbol | Function, method, variable |
| 4 | Endpoint | HTTP route, gRPC service |
| 5 | Surface | Exposed module interface |
| 6 | InfraConfig | Dockerfile, K8s manifest |
| 7 | Resource | Env var, connection target (redis, kafka topic) |

## Edge (16 bytes, `#[repr(C, align(16))]`)

Four edges per 64-byte cache line. Stored in CSR (Compressed Sparse Row) format with separate forward and reverse arrays.

```c
struct Edge {
    u32  target;          // target node ID
    u8   kind;            // EdgeKind enum (see below)
    u8   confidence_u8;   // 0-255 (0.0-1.0 mapped)
    u16  flags;           // edge flags (bit 0 = cross-repo, bit 1 = async, bit 2 = inferred)
    u32  meta_idx;        // metadata index (0xFFFFFFFF = none)
    u8   _pad[4];
};
```

### EdgeKind

| Value | Name | Description |
|-------|------|-------------|
| 0 | Contains | Module contains symbol |
| 1 | Calls | Function calls function |
| 2 | Imports | File imports module |
| 3 | DependsOn | Cross-repo dependency |
| 4 | Exposes | Service exposes endpoint |
| 5 | Consumes | Service consumes resource |
| 6 | Configures | Function reads env var / config |
| 7 | Resolves | Env var resolves to connection target |
| 8 | Connects | Function makes network connection |
| 9 | Publishes | Publishes to message queue |
| 10 | Subscribes | Subscribes to message queue |

### Edge Flags

| Bit | Name | Description |
|-----|------|-------------|
| 0 | CROSS_REPO | Edge spans repository boundary |
| 1 | ASYNC | Asynchronous connection (message queue, webhook) |
| 2 | INFERRED | Edge was inferred, not directly observed |

### CSR Format

Forward edges for node `i` are at `edges[offsets[i]..offsets[i+1]]`.
Reverse edges for node `i` are at `rev_edges[rev_offsets[i]..rev_offsets[i+1]]`.

Edge filtering uses bitmask: `(1u16 << edge.kind) & mask != 0`.

## String Table

Concatenated null-terminated UTF-8 strings. `StringId` values are byte offsets into this table. `StringId = 0xFFFFFFFF` means "no string."

## Reading a .cxgraph File

```rust
// 1. Open and mmap the file
let data = mmap(path);

// 2. Validate header
let header: &GraphFileHeader = cast(&data[0..64]);
assert!(header.magic == b"CX01");
assert!(header.version == 2);
assert!(header.checksum == crc32(&data[0..56]));

// 3. Cast sections (zero-copy)
let nodes: &[Node] = cast_slice(&data[header.nodes_offset..], header.node_count);
let edges: &[Edge] = cast_slice(&data[header.edges_offset..], header.edge_count);
let strings: &[u8] = &data[header.strings_offset..][..header.string_table_size];

// 4. Resolve a string
fn get_string(strings: &[u8], id: u32) -> &str {
    let start = id as usize;
    let end = strings[start..].iter().position(|&b| b == 0).unwrap() + start;
    std::str::from_utf8(&strings[start..end]).unwrap()
}
```

## Companion Files

| File | Format | Content |
|------|--------|---------|
| `network.json` | JSON | Taint analysis results (ResolvedNetworkCall array) |
| `index.json` | JSON | Global cross-repo index (APIs, gRPC services, targets) |
| `overlay.json` | JSON | Cross-repo edges (stable references by file:line:symbol) |
| `network.baseline.json` | JSON | Saved baseline for `cx diff` |

## Version History

| Version | Changes |
|---------|---------|
| 2 | Current. Page-aligned sections, CRC32 checksum, reverse edge arrays. |
| 1 | Initial format (deprecated). |
