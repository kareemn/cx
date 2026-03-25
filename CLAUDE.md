# CX - Code Intelligence Engine

## Project Overview
Read ARCHITECTURE.md for full design. This file contains working instructions.

## Tech Stack
- Rust workspace (cargo)
- Key crates: tree-sitter, memmap2, rayon, rustc-hash, dashmap, smallvec, clap, serde, serde_json, thiserror, anyhow, libc, ignore, gix, criterion
- Release profile: opt-level=3, lto="fat", codegen-units=1, panic="abort"

## Build Commands
- `cargo build` — build all crates
- `cargo test` — run all tests
- `cargo bench` — run criterion benchmarks
- `cargo clippy` — lint (zero warnings policy)

## Architecture Reference
All data structures, algorithms, and performance rules are in ARCHITECTURE.md.
Read it before implementing any module. Follow it exactly.

## Critical Rules
1. ALL hot-path structs are #[repr(C)] with fixed size. Node=32 bytes, Edge=16 bytes.
2. ZERO heap allocation in query paths. Pre-allocate and reuse.
3. BitVec for visited sets, NOT HashSet.
4. BFS uses double-buffer (two Vecs that swap), NOT VecDeque.
5. String interning via StringId (u32) everywhere. No String in graph structs.
6. Bitmask edge filtering: `(1u16 << edge.kind) & mask != 0`
7. Every public function returns Result<T, CxError>. No unwrap() in library code.
8. Tests go in the same file as the code (#[cfg(test)] mod tests).
9. Benchmarks use criterion in cx-core/benches/.

## Implementation Order
Build bottom-up. Each step must compile and pass tests before moving on:
1. cx-core/src/graph/nodes.rs + edges.rs — Node, Edge, StringId, NodeId types
2. cx-core/src/graph/bitvec.rs — BitVec
3. cx-core/src/graph/csr.rs — CsrGraph struct, builder, traversal
4. cx-core/src/store/mmap.rs — GraphFileHeader, write to disk, mmap load
5. cx-core/src/query/bfs.rs — BfsState double-buffer traversal
6. cx-core/src/graph/summary.rs — SummaryGraph construction
7. cx-core/src/graph/kind_index.rs — KindIndex
8. Benchmarks for all of the above
9. Then move to Milestone 2 (extractors)

