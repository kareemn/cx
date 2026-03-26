# CX - Distributed Code Intelligence for Network Boundary Analysis

## Project Overview
Read ARCHITECTURE.md for full design. This file contains working instructions.

cx's primary goal: find every incoming API and outgoing network call in a codebase,
trace where each connection target comes from, and map how services connect across
repos, languages, and infrastructure. Designed for 1000+ repo scale.

## Tech Stack
- Rust workspace (cargo)
- Key crates: tree-sitter, memmap2, rayon, rustc-hash, dashmap, smallvec, clap, serde, serde_json, thiserror, anyhow, libc, ignore, gix, criterion
- LSP integration: ty (Python), gopls (Go), tsserver (TS/JS), jdtls (Java), clangd (C/C++)
- tree-sitter grammars: Go, Python, TypeScript, C, C++, Java (tree-sitter-java)
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
10. Network boundary analysis is the primary use case. Every change must preserve
    or improve coverage of: exposed APIs, outgoing network calls, address provenance
    tracing, and cross-service resolution.
11. LSP integration is optional — cx must always work without LSP servers installed.
    Use tree-sitter + import heuristics as fallback. Mark results as
    "type-confirmed" vs "heuristic" in output.
12. The sink registry (known network functions) must be exhaustive. When adding a
    new language or framework, add ALL its network functions to the registry.
13. cx is designed for 1000+ repos. Never re-index all repos when adding one.
    Per-repo graphs are independent. Cross-repo resolution uses a global index.
14. Custom sink/taxonomy configs in .cx/config/ are the mechanism for teams to
    reach 100% coverage. cx should never require source code changes to handle
    repo-specific patterns.

