# cx-core

Graph engine, storage, and query foundation for the CX code intelligence engine.

## Key Modules

### `graph/` — Core data structures

| Type | File | Description |
|------|------|-------------|
| `Node` | `nodes.rs` | 32-byte `#[repr(C)]` graph node — id, kind, name, file, line, parent, repo |
| `Edge` | `edges.rs` | 16-byte `#[repr(C)]` graph edge — target, kind, confidence, flags |
| `NodeKind` | `nodes.rs` | Enum: Repo, Deployable, Module, Symbol, Endpoint, Surface, InfraConfig, Resource |
| `EdgeKind` | `edges.rs` | Enum: Contains, Calls, Imports, DependsOn, Exposes, Consumes, Configures, Resolves, Connects, Publishes, Subscribes |
| `CsrGraph` | `csr.rs` | Compressed Sparse Row graph with forward/reverse edges and string interner |
| `StringInterner` | `string_interner.rs` | Intern strings to `StringId` (u32) — zero-allocation lookups during queries |
| `BitVec` | `bitvec.rs` | Compact bitset for visited tracking (128KB for 1M nodes) |
| `KindIndex` | `kind_index.rs` | O(1) range lookup for nodes of a given kind |
| `SummaryGraph` | `summary.rs` | Pre-computed service-level view (Deployable + Resource nodes only) |

### `store/` — Persistence

| Type | File | Description |
|------|------|-------------|
| `GraphFileHeader` | `mmap.rs` | 64-byte file header with magic, version, counts, offsets, checksum |
| `write_graph()` | `mmap.rs` | Serialize graph to disk in mmap-compatible format |
| `load_graph()` | `mmap.rs` | Memory-map graph file — zero deserialization cost |

### `query/` — Traversal and search

| Type | File | Description |
|------|------|-------------|
| `BfsState` | `bfs.rs` | Double-buffer BFS with bitmask edge filtering and BitVec visited set |
| `PathFinder` | `path.rs` | BFS-based shortest path and downstream path enumeration |
| `depends()` | `depends.rs` | Transitive dependency closure (upstream or downstream) |
| `TrigramIndex` | `trigram.rs` | Trigram-based fuzzy symbol search |

### `error.rs`

`CxError` — unified error type for all cx crates.

## Type Aliases

- `NodeId` = `u32`
- `StringId` = `u32`
- `RepoId` = `u16`
- `EdgeKindMask` = `u16`

## Dependencies

No dependencies on other cx crates. All other crates depend on cx-core.

## Example Usage

```rust
use cx_core::graph::{Node, Edge, CsrGraph, StringInterner, NodeKind, EdgeKind};
use cx_core::query::bfs::{BfsState, Direction};
use cx_core::store::mmap;

// Build a graph
let mut strings = StringInterner::new();
let name = strings.intern("my_function");
let nodes = vec![/* ... */];
let edges = vec![/* ... */];
let graph = CsrGraph::build(nodes, edges, strings)?;

// Run BFS from node 0, following Calls edges downstream
let mask = 1u16 << (EdgeKind::Calls as u16);
let mut bfs = BfsState::new(graph.node_count());
bfs.run(&graph, 0, mask, 10, Direction::Downstream);

// Persist and reload
mmap::write_graph(&graph, None, &path)?;
let loaded = mmap::load_graph(&path)?;
```
