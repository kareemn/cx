# CX Architecture Document

> **Current CLI:** `cx build`, `cx trace`, `cx network`, `cx diff`, `cx add`, `cx pull`, `cx fix`, `cx hook`, `cx skill`, `cx mcp`.
> **Current MCP tools:** `cx_path`, `cx_network`, `cx_diff`, `cx_explain`.
> Some sections below describe planned query primitives (cx_search, cx_depends, cx_resolve, cx_context, cx_impact) that are not yet implemented as standalone commands — their functionality is subsumed by `cx trace` and `cx network`.

## What CX Is

CX is a distributed code intelligence engine that builds a complete, type-resolved map of how services communicate — across repositories, languages, and infrastructure. It answers one question with 100% accuracy:

> **What are every single incoming API and outgoing network call in this codebase, where does each connection target come from, and how do all these services connect to each other?**

Given one or more source code repositories and their deployment configs, CX produces:

1. **Every exposed API**: HTTP endpoints, gRPC services, WebSocket endpoints, message queue topics — with exact file:line locations.
2. **Every outgoing network call**: HTTP clients, gRPC dials, database connections, Redis, Kafka, SQS, raw TCP — with a full provenance chain showing where the connection target comes from (string literal, env var, config file, function parameter).
3. **The complete service wiring**: How services connect to each other via env vars, K8s DNS, Helm values, and Docker networking — traced from source code through infrastructure config to the target service's exposed API.

CX works like git: local-first, distributed, collaborative. Each team builds their repo's graph independently. Teams add remotes to pull other repos' graphs and build the complete cross-service topology. The graph is never a static document — it's a living, compiler-derived artifact that improves with each index run and can be tuned by teams to reach 100% coverage of their codebase.

CX is designed to scale to **1000+ repositories** across an organization.

The primary interfaces are a CLI (`cx`) and an MCP server that plugs directly into Claude Code, Cursor, Gemini CLI, and any MCP-compatible AI agent. There is no web UI. There is no SaaS dashboard. CX lives in the terminal.

## Why CX Exists

### The Problem

In any distributed system with multiple repositories and services, structural knowledge — what connects to what, what depends on what, what breaks if something changes — exists only as tribal knowledge distributed across engineers' heads. When those engineers leave, the knowledge disappears.

Current tools don't solve this:
- **LSPs** break at repository boundaries and can't trace cross-service calls
- **Sourcegraph/GitHub code search** finds text matches but can't follow gRPC boundaries, resolve environment variables through Helm charts, or show the full request path across services
- **AI agents with large context windows** can read code and infer structure, but they're expensive per query, non-deterministic, potentially incomplete (they miss edges in files they skim), and can't do temporal queries across git history
- **Architecture documentation** goes stale the moment it's written

### The Insight

Every network call in every language takes an address argument — a URL, host:port, connection string. That address comes from somewhere: a string literal, an environment variable, a config file, a function parameter. By tracing that data flow from network call back to its source, and matching it against infrastructure configs (K8s manifests, Helm values), we can deterministically reconstruct the entire service topology without running anything.

The reason nobody has built this is that it requires: fast multi-language parsing, cross-file taint analysis, infrastructure config resolution, and type-resolved call graphs — across every major language and framework. AI coding agents change that equation.

### CX's Position

CX is the structural map. The AI agent is the reasoning engine. They're complementary:
- CX provides instant, deterministic, exhaustive structural queries (what connects to what, with provenance)
- The AI agent provides reasoning (why does it connect that way, how should I change it)
- The MCP interface makes them work together: the agent queries CX to understand structure, then loads only the relevant files into its context window for deep reasoning

This makes the AI agent dramatically more effective — instead of burning 50k+ tokens grepping and reading files to understand a call chain, the agent makes one `cx_path` call and gets the complete answer in milliseconds.

### CX as Lossless Context

Inspired by [Lossless Context Management](https://papers.voltropy.com/LCM) (Voltropy, 2026), CX's graph is a precomputed, hierarchical exploration summary of a codebase's network structure. LCM demonstrates that context management is the primary bottleneck for long-horizon agentic tasks — tools like CX that pre-compute compact graph representations eliminate this bottleneck.

The graph provides multiple resolution levels:
- `cx trace 'env:*'` → compact overview of all env vars (condensed summary)
- `cx trace DATABASE_URL` → specific provenance chain and paths (expanded detail)
- `cx network` → all network boundaries with provenance (full resolution)

Every edge is traceable to source code. No information is lost — the graph is lossless, deterministic, and reproducible. An agent can drill from summary to detail without re-analyzing code.

## How CX Achieves This

1. **tree-sitter** for fast structural parsing — always available, <2s per repo, extracts all symbols, calls, imports, and string literals with 100% recall.
2. **Import-aware FQN resolution** maps receiver names to import paths, constructing fully-qualified names that match the sink registry — deterministic, no external dependencies.
3. **LLM classification (optional)** sends source context to Claude Haiku for calls that FQN resolution misses — resolves both call type and target address.
4. **LSP servers** (ty, gopls, tsserver, jdtls, clangd) for type-resolved accuracy when installed.
5. **Backward taint analysis** tracing address arguments through variable assignments, function parameters, env var reads, and config file loads — cross-file, inter-procedural, depth-bounded.
6. **Infrastructure resolution** linking code-side env var reads to K8s manifest values to service DNS names to target services' exposed APIs.
7. **Distributed graph protocol** enabling teams to share graphs via remotes, with collaborative parser config refinement via `.cx/config/`.

## Classification Pipeline

After tree-sitter extracts raw call sites, each outgoing call must be classified — what kind of network call is it (gRPC, HTTP, database, etc.) and where does it connect? CX uses a three-tier pipeline, falling through from highest to lowest confidence:

### Tier 1: Import-Aware FQN Resolution

`build_fqn_candidates()` in `taint.rs` uses `raw.imports` to construct fully-qualified names for each call's receiver. `default_alias_for_lang()` derives the default alias for a given import path per language convention (e.g., Go uses the last path segment, Python uses the module name). For Go, versioned module paths with `/v8` suffixes are stripped before matching. The constructed FQN is matched against the sink registry to classify the call.

This tier is deterministic, fast, and requires no external dependencies. It handles the majority of calls in well-structured codebases where import paths are unambiguous.

### Tier 2: LLM Classification

For calls that Tier 1 cannot resolve (ambiguous receivers, aliased imports, dynamic dispatch), `upgrade_via_llm()` in `indexing.rs` sends source context to Claude Haiku via the `claude` CLI or the Anthropic API. The LLM receives the call site with surrounding context and returns both the call kind and target address.

Results are cached in `.cx/graph/llm_cache.json` keyed by `file:line:callee` with a hash of the source context. On subsequent builds, cached results are applied instantly — only new or changed calls hit the API. This makes post-commit rebuilds fast (~2s) even when the initial build took 15-70s for LLM classification.

This tier is optional. CX never requires an API key or network access to function.

### Tier 3: Heuristic Classification

`heuristic_classify_call()` in `sink_registry.rs` pattern-matches on receiver and method names (e.g., a method named `Dial` on a receiver containing `grpc` is classified as gRPC). This tier is always available and has no external dependencies, but produces the lowest confidence results.

### Confidence Enum

Each classified call carries a `Confidence` level reflecting how it was resolved:

| Level | Source | Typical Score |
|-------|--------|---------------|
| TypeConfirmed | LSP-resolved fully-qualified type | 0.95+ |
| LLMClassified | Claude Haiku classification | 0.80-0.90 |
| ImportResolved | FQN constructed from import paths | 0.70-0.85 |
| Heuristic | Pattern-matched receiver/method names | 0.40-0.65 |

Higher tiers always take precedence. If Tier 1 resolves a call, Tier 2 and 3 are skipped. If Tier 2 resolves it, Tier 3 is skipped.

## Core Design Principles

1. **Local-first.** The entire index lives on the developer's machine as a memory-mapped file. No server, no cloud, no API keys. Queries are sub-10ms.

2. **Git-native.** The fundamental unit of indexing is a commit, not a directory. Branches are graph states. Diffs between branches are graph diffs. Temporal queries ("when did this dependency appear") walk git history.

3. **Progressive resolution.** CX is honest about what it doesn't know. Every query response carries a completeness score and explicit gap information. Dangling edges (references to unindexed services) are first-class objects, not silent omissions. Results carry a confidence level from the classification pipeline: TypeConfirmed (LSP-resolved), LLMClassified (Claude Haiku), ImportResolved (FQN via import paths), or Heuristic (pattern-matched).

4. **Incremental.** File changes trigger incremental re-parsing via tree-sitter. Cross-repo updates happen in the background. Per-repo graphs are independent — adding repo 1000 is as fast as adding repo 2.

5. **Single binary, optional LSP and LLM.** CX compiles to one static Rust binary. It always works standalone with tree-sitter and import-aware FQN resolution. LSP servers (ty, gopls, tsserver, jdtls, clangd) and LLM classification (Claude Haiku) enhance accuracy but are never required. CX always works standalone.

6. **Distributed like git.** Each repo has a `.cx/` directory. Teams build their own graph. `cx add` / `cx pull` shares graphs. Custom parser config (`.cx/config/sinks.toml`) is version-controlled and shareable — enabling teams to reach 100% coverage without modifying CX source code.

7. **Network boundaries are the primary concern.** Every incoming API and outgoing network call is detected, classified, and traced to its address source. The graph is structured around network I/O boundaries, not arbitrary code structure.

8. **1000-repo scale.** Per-repo graphs with a global index. Cross-repo resolution is O(new_repo) not O(N^2). Queries load only the repos they touch via mmap.

## Performance Rules (MANDATORY)

These rules are non-negotiable. Every code path must follow them. They exist because CX's value proposition depends on queries feeling instant — sub-5ms. If any query takes more than 10ms, something is wrong.

### Rule 1: Zero Allocation in Query Hot Paths

Once the graph is loaded, query execution must not allocate heap memory. This means:
- **No `Vec::new()` during traversal.** Pre-allocate result vectors and reuse them between queries. Use `Vec::with_capacity()` with conservative estimates, then `clear()` between queries.
- **No `String` creation during traversal.** All string comparisons use `StringId` (u32 integer comparison). Strings are only resolved to `&str` at the final output stage.
- **No `HashMap` or `BTreeMap` during traversal.** Use `BitVec` (bitset) for visited tracking. Use pre-sorted arrays with binary search for lookups.
- **No `Box`, `Rc`, `Arc` in hot-path structs.** All graph data is contiguous arrays of `Copy` types.
- **No `format!()` or string formatting during traversal.** Format output only after collecting results.

### Rule 2: Cache-Line Aware Data Layout

Modern CPUs load data in 64-byte cache lines. Data structures must be designed for sequential access:
- **Node struct: exactly 32 bytes, `#[repr(C, align(32))]`.** Two nodes per cache line. Sequential node scan has zero waste.
- **Edge struct: exactly 16 bytes, `#[repr(C, align(16))]`.** Four edges per cache line. Scanning a node's edges is a sequential memory sweep.
- **All arrays are contiguous.** No linked lists. No pointer-based trees. No indirection during traversal.
- **Hot data separated from cold data.** The graph traversal only touches Node (32B), Edge (16B), and offsets (4B). Metadata (file paths, provenance chains, detailed locations) lives in separate cold arrays only accessed for display.

### Rule 3: Use Integer Operations, Not Branching

Edge filtering during traversal uses bitmask operations, not match statements or if-else chains:
```rust
// CORRECT: single integer AND — no branch prediction misses
if (1u16 << edge.kind) & edge_mask != 0 { /* include edge */ }

// WRONG: branch on each variant — causes branch prediction misses
match edge.kind {
    EdgeKind::Calls | EdgeKind::Imports => { /* include */ }
    _ => { /* skip */ }
}
```

### Rule 4: Mmap for Zero-Cost Graph Loading

The on-disk graph format is identical to the in-memory representation. Loading the graph is a single `mmap()` call — the OS maps the file into virtual memory and pages in data on demand. There is no deserialization step.
- All hot-path structs (`Node`, `Edge`) are `#[repr(C)]` with fixed layout.
- The mmap'd file can be cast directly to `&[Node]` and `&[Edge]` slices (after verifying the header/magic number).
- Cold metadata sections at the end of the file are only paged in if the user requests detail on a specific node/edge.

### Rule 5: Parallel Everything During Indexing

`cx build` is the only operation that is allowed to be slow (seconds, not milliseconds). But even it must be as fast as possible:
- **File parsing: parallel with rayon.** Each file is parsed independently. Tree-sitter parsers are per-thread (not `Send`), so use `rayon::ThreadLocal` for parser pooling.
- **String interning: concurrent with DashMap during indexing.** After all files are parsed, freeze the concurrent map into the packed `StringInterner` format.
- **Graph construction: single-threaded merge.** Parallel extractors produce `ExtractionResult` bags. The merge step sorts nodes by (kind, id), computes offsets, packs edges. This is a single O(N log N) sort followed by a linear scan. Do not try to parallelize the merge — it's fast enough and correctness matters more.
- **File I/O: use `ignore` crate (same as ripgrep) for parallel directory walking with .gitignore respect.**

### Rule 6: Minimize Syscalls

- **Read files in bulk.** Use `mmap` to read source files during indexing rather than `read_to_string()`. For small files, `read_to_string()` is fine, but for large codebases the mmap approach avoids per-file allocate-and-copy.
- **Single mmap for the graph file.** Not multiple mmaps for different sections.
- **Batch file system operations.** When watching for file changes, debounce events and process batches, not individual file changes.

### Rule 7: Benchmark Continuously

Every PR must maintain these benchmarks (use `criterion` crate):

| Operation | Target | Notes |
|-----------|--------|-------|
| Graph load (mmap) | < 50ms | For 500MB graph file |
| `cx trace` (5 hops, 100K nodes) | < 1ms | BFS with edge filter |
| `cx network` (all boundaries, 100K nodes) | < 5ms | Graph scan + taint data |
| `cx build` (100K LOC Go repo) | < 2s | Parallel tree-sitter + extractors |
| `cx build` (1M LOC multi-repo) | < 15s | Parallel across repos |
| `cx add` (pre-built remote graph) | < 1s | Copy + cross-repo resolution |

Add `#[bench]` tests for all query functions. Use `criterion` for statistical benchmarking. **Never merge a PR that regresses a benchmark by more than 10%.** Detailed per-milestone benchmarks are specified in the Implementation Milestones section below.

## Data Model

### The Repo Is Not the Unit of Analysis

A single repository can contain multiple deployable services, shared libraries, proto definitions, infrastructure configs, and CLI tools. The old "repo = service" model breaks on real-world codebases.

CX's data model reflects this reality:

### Node Types

The logical node types are: Repo, Deployable, Module, Symbol, Endpoint, Surface, InfraConfig, Resource. These represent the conceptual model (git boundary, something that runs, code package, function/class/method, exposed API, exported interface, deployment config, database/cache/queue).

However, nodes in the hot-path graph must be compact fixed-size structs, not Rust enums with variable-size payloads. The graph uses a split representation:

```rust
/// PERFORMANCE CRITICAL: This is the hot-path node stored in the CSR arrays.
/// Fixed 32 bytes. No heap allocations. No String fields. No Vec fields.
/// All variable-length data lives in side tables referenced by u32 IDs.
#[repr(C, align(32))]
#[derive(Clone, Copy)]
struct Node {
    id: NodeId,             // u32 — unique node identifier
    kind: u8,               // NodeKind discriminant (0=Repo, 1=Deployable, ..., 7=Resource)
    sub_kind: u8,           // Sub-discriminant (e.g., SymbolKind::Function, Protocol::gRPC)
    flags: u16,             // Bitflags: IS_ENTRY_POINT, IS_PUBLIC, IS_DEPRECATED, etc.
    name: StringId,         // u32 — index into interned string table
    file: StringId,         // u32 — index into interned string table (empty for non-file nodes)
    line: u32,              // source line (0 for non-source nodes)
    parent: NodeId,         // u32 — containment parent (repo for deployable, module for symbol)
    repo: RepoId,           // u16 — which repo this belongs to (max 65535 repos)
    _pad: [u8; 2],          // padding to 32-byte alignment
}

// NodeId, StringId, RepoId are newtypes over u32/u16.
// NEVER use usize for IDs — u32 is sufficient (4 billion nodes) and halves memory on 64-bit.
type NodeId = u32;
type StringId = u32;
type RepoId = u16;
```

Extended node metadata (protocol details, deploy configs, location columns, confidence) lives in a **cold side table** indexed by `NodeId`. This is only accessed when the user drills into a specific node, never during graph traversal:

```rust
/// Cold storage — accessed only for display/detail, never during traversal.
/// Stored in a separate Vec<NodeMeta> indexed by NodeId.
struct NodeMeta {
    column: u16,
    commit: CommitSha,          // [u8; 20]
    detail: NodeDetail,         // enum with the full Repo{remote, branch}, Endpoint{protocol, path}, etc.
}
```
```

### Edge Types

Same principle as nodes: the hot-path edge is a compact fixed-size struct. All variable-length metadata lives in a cold side table.

```rust
/// PERFORMANCE CRITICAL: Hot-path edge stored in CSR edge array.
/// Fixed 16 bytes. Fits two edges per cache line.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Edge {
    target: NodeId,         // u32 — destination node
    kind: u8,               // EdgeKind discriminant (see below)
    confidence_u8: u8,      // confidence * 255, quantized to save space (0=unknown, 255=certain)
    flags: u16,             // Bitflags: IS_CROSS_REPO, IS_ASYNC, IS_INFERRED, etc.
    meta_idx: u32,          // index into EdgeMeta cold table (u32::MAX = no metadata)
    _pad: [u8; 4],          // padding to 16-byte alignment
}

/// Edge kinds as u8 discriminants. Use bitmask filtering in queries.
/// IMPORTANT: Assign values as powers of 2 so they can be OR'd into bitmask filters.
/// A query for "all service-level edges" uses mask 0b0011_1000 (DependsOn | Exposes | Consumes).
#[repr(u8)]
enum EdgeKind {
    Contains    = 0,   // containment hierarchy
    Calls       = 1,   // symbol→symbol
    Imports     = 2,   // module→module
    DependsOn   = 3,   // deployable→deployable (via gRPC, HTTP, queue)
    Exposes     = 4,   // deployable→endpoint
    Consumes    = 5,   // deployable→endpoint (on another service)
    Configures  = 6,   // infra_config→deployable
    Resolves    = 7,   // infra_config→env var reference
    Connects    = 8,   // deployable→resource
    Publishes   = 9,   // deployable→queue topic
    Subscribes  = 10,  // deployable→queue topic
}

// For bitmask-based edge filtering in traversals:
type EdgeKindMask = u16; // bit N set = include EdgeKind with discriminant N
const SERVICE_EDGES: EdgeKindMask = (1 << 3) | (1 << 4) | (1 << 5); // DependsOn | Exposes | Consumes
const CODE_EDGES: EdgeKindMask    = (1 << 1) | (1 << 2);             // Calls | Imports
const ALL_EDGES: EdgeKindMask     = 0x07FF;                           // all 11 kinds
```

### Edge Metadata (Cold Side Table)

```rust
/// Cold storage — only accessed when displaying edge details, never during traversal.
/// Stored in a separate Vec<EdgeMeta>, indexed by Edge::meta_idx.
struct EdgeMeta {
    source_location: Location,        // file, line, column where we found this
    provenance: SmallVec<[ProvenanceStep; 4]>, // chain of reasoning (usually ≤4 steps)
    last_validated: CommitSha,        // last commit where this edge was confirmed
}
```

### Confidence Scoring

- **0.9–1.0 (high):** Proto service match (client stub in repo A, server registration in repo B, same proto service name). Deterministic.
- **0.7–0.9 (medium):** Env var resolved through Helm chart to Kubernetes DNS to service name. Template-resolved values.
- **0.4–0.7 (low):** HTTP URL pattern matching. Dynamic string construction partially resolved. Message queue topic prefix match.
- **0.0–0.4 (speculative):** Heuristic matches. Same-org repo name similarity. Commit author cross-referencing.

High-confidence edges are stated as fact. Low-confidence edges are presented as possibilities with explicit gaps. The MCP responses separate `resolved_edges` from `possible_edges`.

### Address Provenance

The `AddressSource` enum tracks where a network call's target address originates. In addition to the core variants (string literal, env var, function parameter, config file), the following variants support service discovery and LLM-augmented classification:

```rust
/// Service discovery mechanisms (Consul, K8s DNS, etc.)
AddressSource::ServiceDiscovery {
    service_name: StringId,   // e.g., "acme-orders"
    mechanism: StringId,      // e.g., "k8s-dns", "consul", "eureka"
}
```

`LLMClassification` structs carry the LLM's response for calls classified via Tier 2: the inferred call kind, target address, and a confidence score. `apply_llm_classification()` merges these results into the call's existing metadata, upgrading its confidence from Heuristic to LLMClassified and filling in the target address when the LLM provides one.

## Graph Storage

### Performance Target

All queries must complete in under 5ms for a graph with 1 million nodes and 10 million edges. Graph loading from disk (mmap) must complete in under 50ms. `cx build` indexing must process 100,000 lines of code per second.

### Format: Compressed Sparse Row (CSR)

The graph is stored as a memory-mapped file using CSR format. This is the same format used by high-performance graph analytics libraries (e.g., GraphBLAS, Ligra). Every design choice prioritizes sequential memory access and cache-line efficiency.

```rust
/// The core graph structure. All arrays are contiguous and cache-friendly.
/// NEVER use HashMap, BTreeMap, or any pointer-chasing structure for the graph.
struct CsrGraph {
    /// Nodes sorted by (kind, id). Fixed 32 bytes each → cache-line aligned.
    nodes: Vec<Node>,       // or &[Node] when mmap'd

    /// Outgoing edges grouped by source node. Fixed 16 bytes each.
    /// All edges for node_i are in edges[offsets[i]..offsets[i+1]].
    edges: Vec<Edge>,       // or &[Edge] when mmap'd

    /// offsets[i] = index into edges[] where node i's edges begin.
    /// offsets.len() == nodes.len() + 1 (sentinel at end).
    /// MUST be u32, not usize — saves 50% memory on 64-bit, sufficient for 4B edges.
    offsets: Vec<u32>,      // or &[u32] when mmap'd

    /// Reverse edge index for upstream queries (who points TO this node).
    /// Same CSR format: rev_edges grouped by target, rev_offsets indexes into rev_edges.
    rev_edges: Vec<Edge>,
    rev_offsets: Vec<u32>,

    /// Interned string table. All symbol names, file paths, repo URLs stored once.
    /// Strings are deduplicated and packed contiguously.
    strings: StringInterner,

    /// PERFORMANCE CRITICAL: Pre-computed service-level summary graph.
    /// Contains only Deployable and Resource nodes (~10-200 nodes typically).
    /// Used for macro queries (cx_depends at service level) to avoid scanning 1M symbol nodes.
    summary: SummaryGraph,

    /// Index into nodes array by NodeKind. Avoids full scan when looking for specific kinds.
    kind_index: KindIndex,
}
```

**Memory layout on disk (mmap'd file):**

```
┌─────────────────────────────────────────┐
│ Header (64 bytes)                       │  magic, version, counts, offset table
├─────────────────────────────────────────┤
│ Nodes array    [N × 32 bytes]           │  page-aligned start
├─────────────────────────────────────────┤
│ Offsets array  [(N+1) × 4 bytes]        │  u32 offsets into edges
├─────────────────────────────────────────┤
│ Edges array    [E × 16 bytes]           │  page-aligned start
├─────────────────────────────────────────┤
│ Rev-Offsets    [(N+1) × 4 bytes]        │  reverse index offsets
├─────────────────────────────────────────┤
│ Rev-Edges      [E × 16 bytes]           │  reverse edges
├─────────────────────────────────────────┤
│ Summary nodes  [S × 32 bytes]           │  summary graph (Deployable/Resource only)
├─────────────────────────────────────────┤
│ Summary offsets [(S+1) × 4 bytes]       │  summary graph offsets
├─────────────────────────────────────────┤
│ Summary edges  [SE × 16 bytes]          │  summary graph edges
├─────────────────────────────────────────┤
│ Kind index     [8 × 8 bytes]            │  (start, end) per NodeKind
├─────────────────────────────────────────┤
│ Trigram index                           │  for cx_search fuzzy matching
├─────────────────────────────────────────┤
│ String table                            │  length-prefixed, packed, deduplicated
├─────────────────────────────────────────┤
│ Node metadata  [N × variable]           │  cold side table, only loaded on demand
├─────────────────────────────────────────┤
│ Edge metadata  [M × variable]           │  cold side table, M ≤ E (not all edges have meta)
└─────────────────────────────────────────┘
```

**File header (64 bytes, fixed):**

```rust
/// The first 64 bytes of the .cxgraph file. Validates format and locates all sections.
/// RULE: All offset fields are byte offsets from start of file. All sizes in bytes.
#[repr(C)]
struct GraphFileHeader {
    magic: [u8; 4],           // b"CX01" — file format identifier
    version: u32,             // format version (start at 1, increment on breaking changes)
    node_count: u32,          // N
    edge_count: u32,          // E
    summary_node_count: u32,  // S
    summary_edge_count: u32,  // SE
    string_table_size: u32,   // bytes
    nodes_offset: u64,        // byte offset to nodes array (page-aligned)
    edges_offset: u64,        // byte offset to edges array (page-aligned)
    strings_offset: u64,      // byte offset to string table
    checksum: u32,            // CRC32 of header fields (for corruption detection)
    _reserved: [u8; 4],       // padding to 64 bytes
}

/// LOADING SEQUENCE:
/// 1. mmap the entire file
/// 2. Read first 64 bytes as GraphFileHeader
/// 3. Verify magic == b"CX01" and version is supported
/// 4. Verify checksum
/// 5. Cast byte ranges to typed slices:
///    nodes = &mmap[header.nodes_offset..] as &[Node]  (length = node_count)
///    edges = &mmap[header.edges_offset..] as &[Edge]  (length = edge_count)
///    etc.
/// 6. Build string lookup table from packed string data (the ONE deserialization step)
/// 7. Call advise_mmap() with section pointers
/// 8. Ready for queries
```

**Key invariants:**
- `nodes`, `offsets`, `edges` sections are page-aligned so mmap can map them directly without copying.
- Node and Edge structs are `#[repr(C)]` with explicit alignment so mmap'd data can be cast directly to `&[Node]` / `&[Edge]` with zero deserialization.
- Cold metadata sections are at the end of the file so the OS only pages them in when accessed.
- The string table uses a hash map (built at load time from packed data) for O(1) intern lookups.

### Mmap Advise Hints

After mmap'ing the graph file, issue OS hints about access patterns:

```rust
/// PERFORMANCE CRITICAL: Tell the OS how we'll access the data.
/// This dramatically improves page fault behavior.
fn advise_mmap(graph_mmap: &Mmap) {
    #[cfg(unix)]
    {
        use libc::{madvise, MADV_RANDOM, MADV_WILLNEED};
        unsafe {
            // Hot sections (nodes, offsets, edges): we'll access them randomly during BFS.
            // MADV_RANDOM disables readahead which hurts random access patterns.
            madvise(nodes_ptr, nodes_len, MADV_RANDOM);
            madvise(edges_ptr, edges_len, MADV_RANDOM);
            madvise(offsets_ptr, offsets_len, MADV_RANDOM);

            // For the first query after load, prefault hot sections into memory.
            // MADV_WILLNEED triggers background page-in without blocking.
            madvise(nodes_ptr, nodes_len, MADV_WILLNEED);
            madvise(offsets_ptr, offsets_len, MADV_WILLNEED);
            // Do NOT willneed edges — they're large and we only access a subset per query.
            // Do NOT willneed cold metadata sections — only paged in on demand.
        }
    }
}
```

### Edge Sorting Within Adjacency Lists

Within each node's edge range `edges[offsets[i]..offsets[i+1]]`, edges are sorted by `kind`. This enables early termination when filtering:

```rust
/// When a query only wants SERVICE_EDGES (kind 3,4,5), and edges are sorted by kind,
/// we can binary-search to the first edge of kind 3 and stop at the first edge of kind 6.
/// This skips potentially hundreds of Calls/Imports edges per node without touching them.
///
/// RULE: During graph construction, after grouping edges by source node,
/// sort each node's edge range by edge.kind (secondary sort).
/// The primary sort is by source NodeId (required for CSR). Secondary sort is by kind.
```

### Two-Level Graph: Summary Layer

For macro queries ("what services depend on what"), scanning through millions of symbol-level nodes is wasteful. Build a **summary graph** that contains only Deployable and Resource nodes with aggregated edges:

```rust
/// PERFORMANCE CRITICAL: Service-level queries should not touch symbol-level nodes.
/// The summary graph is a small CSR (typically <1000 nodes) for fast macro queries.
struct SummaryGraph {
    /// Only Deployable and Resource nodes. Typically 10-200 nodes.
    graph: CsrGraph,
    /// Maps summary NodeId → set of detail NodeIds in the full graph.
    /// Used for drill-down: "show me the symbol-level details for this service edge."
    detail_mapping: Vec<(NodeId, NodeId)>,  // (summary_id, full_graph_start_id)
}

// cx_depends at the service level: query SummaryGraph (microseconds, <100 nodes).
// cx_impact at the symbol level: query full CsrGraph (milliseconds, 1M nodes).
// cx_path: start on SummaryGraph for macro hops, drill into full graph for intra-service detail.
```

Build the summary graph during the merge step. It's derived from the full graph by collapsing all Contains edges — every symbol inside a Deployable is collapsed into the Deployable node, and edges between symbols in different Deployables become a single DependsOn edge in the summary.

### Node Kind Index

Since nodes are sorted by `(kind, id)`, store the offset range for each kind in the header:

```rust
/// PERFORMANCE CRITICAL: Avoid scanning all 1M nodes when you only want Endpoints.
/// The kind_index tells you exactly where each node kind starts and ends.
struct KindIndex {
    /// kind_ranges[k] = (start, end) indices into the nodes array for NodeKind k.
    /// Find all Endpoint nodes: nodes[kind_ranges[4].0 .. kind_ranges[4].1]
    kind_ranges: [(u32, u32); 8],  // 8 node kinds
}

// "Find all endpoints across the codebase" → scan ~500 nodes, not 1M.
// "Find all deployables" → scan ~20 nodes, not 1M.
```

### BFS Double-Buffer (Not VecDeque)

`VecDeque` has overhead from ring-buffer wrap-around logic. For BFS, use two `Vec`s that swap each level:

```rust
/// PERFORMANCE CRITICAL: Two Vec swap is faster than VecDeque for BFS.
/// No modular arithmetic. No branch for wrap-around. Pure sequential writes and reads.
struct BfsState {
    current_level: Vec<NodeId>,  // nodes at current BFS depth
    next_level: Vec<NodeId>,     // nodes discovered for next depth
    visited: BitVec,
    result: Vec<NodeId>,
}

impl BfsState {
    fn run(&mut self, graph: &CsrGraph, seeds: &[NodeId], mask: EdgeKindMask, max_depth: u32) {
        self.visited.clear();
        self.result.clear();
        self.current_level.clear();
        self.next_level.clear();

        for &seed in seeds {
            self.visited.set(seed);
            self.current_level.push(seed);
        }

        for _depth in 0..=max_depth {
            for &node in &self.current_level {
                self.result.push(node);
                let start = graph.offsets[node as usize] as usize;
                let end = graph.offsets[node as usize + 1] as usize;
                for edge in &graph.edges[start..end] {
                    if (1u16 << edge.kind) & mask == 0 { continue; }
                    if self.visited.test(edge.target) { continue; }
                    self.visited.set(edge.target);
                    self.next_level.push(edge.target);
                }
            }
            // Swap buffers — no allocation, just pointer swap
            std::mem::swap(&mut self.current_level, &mut self.next_level);
            self.next_level.clear();  // clear does NOT deallocate
            if self.current_level.is_empty() { break; }
        }
    }
}
```

### MCP Output: Direct JSON Serialization

Do not build a `serde_json::Value` tree and then serialize it. Write JSON directly to a pre-allocated buffer:

```rust
/// PERFORMANCE CRITICAL: For MCP responses, avoid the serde_json::Value intermediate.
/// serde_json::Value allocates a tree of heap objects, then serializes to string.
/// Instead, serialize directly to a BufWriter using serde's Serializer trait.
///
/// For a response with 500 nodes: serde_json::Value approach allocates ~500 objects.
/// Direct serialization: zero intermediate allocations.
fn serialize_response(result: &QueryResult, graph: &CsrGraph, buf: &mut Vec<u8>) {
    buf.clear();
    // Use serde_json::to_writer(buf, result) with #[derive(Serialize)] on QueryResult.
    // The Serialize impl resolves StringIds to &str lazily during serialization.
    // NEVER collect results into a Vec<serde_json::Value> first.
    serde_json::to_writer(buf, result).unwrap();
}
```

### String Interning

Every string (symbol name, file path, repo URL) is stored exactly once. References use `StringId` (u32).

```rust
/// PERFORMANCE CRITICAL: String interning eliminates all string allocations
/// during graph traversal and all string comparisons during queries.
struct StringInterner {
    /// Packed string data: [len:u32][bytes...][len:u32][bytes...]...
    data: Vec<u8>,          // or &[u8] when mmap'd
    /// Lookup table: string hash → StringId. Built at load time.
    /// Uses FxHashMap (faster than std HashMap for small keys).
    lookup: FxHashMap<u64, StringId>,
}

impl StringInterner {
    /// Get string content by ID — zero-copy when mmap'd.
    fn get(&self, id: StringId) -> &str { /* offset + length from packed data */ }

    /// Intern a string — returns existing ID if already present.
    fn intern(&mut self, s: &str) -> StringId { /* hash, lookup, insert if new */ }
}

// RULE: Never store String or &str in any hot-path struct.
// RULE: Never compare strings directly. Compare StringId (u32 == u32) instead.
// RULE: Never allocate strings during query execution.
```

### Traversal: Bitset-Based Visited Tracking

```rust
/// PERFORMANCE CRITICAL: Use a bitset, not a HashSet, for visited tracking.
/// For 1M nodes, a bitset is 128KB (fits in L2 cache).
/// A HashSet<u32> for the same would be ~8MB with pointer chasing.
struct BitVec {
    bits: Vec<u64>,  // 1 bit per node, packed into u64 words
}

impl BitVec {
    fn set(&mut self, id: NodeId)   { self.bits[id as usize / 64] |= 1 << (id % 64); }
    fn test(&self, id: NodeId) -> bool { self.bits[id as usize / 64] & (1 << (id % 64)) != 0 }
    fn clear(&mut self)             { self.bits.fill(0); }  // reuse between queries
}
```

### Query Execution Model

```rust
/// All queries follow this pattern:
/// 1. Start from a set of seed nodes
/// 2. Traverse edges matching a bitmask filter
/// 3. Collect results into a pre-allocated Vec
///
/// RULES:
/// - NEVER allocate during traversal. Pre-allocate result Vec with estimated capacity.
/// - NEVER use HashSet<NodeId> for visited tracking. Use BitVec.
/// - NEVER filter edges with match statements. Use bitmask: (1 << edge.kind) & mask != 0
/// - NEVER follow edges into cold metadata during traversal. Collect NodeIds first, enrich after.
/// - REUSE the BfsState between queries (clear, don't reallocate).
/// - Use BfsState double-buffer pattern (see above), NOT VecDeque.
///
/// The canonical traversal implementation is BfsState::run() defined in the
/// "BFS Double-Buffer" section above. All query functions (cx_path, cx_impact,
/// cx_depends) are built on top of it with different seed selection,
/// edge masks, and result post-processing.
```

### Parallel Indexing

```rust
/// PERFORMANCE CRITICAL: Index files in parallel during `cx build`.
/// Tree-sitter parsing is CPU-bound and embarrassingly parallel.
/// Use rayon for parallel iteration over files.
///
/// Pipeline:
/// 1. Collect all file paths using `ignore` crate (parallel, .gitignore aware)
/// 2. Parse files in parallel with rayon (each thread gets its own tree-sitter parser)
/// 3. Run extractors in parallel per-file (each produces ExtractionResult)
/// 4. Merge ExtractionResults into a single graph (single-threaded, sequential)
///
/// RULES:
/// - Each rayon thread owns its own tree-sitter Parser instance (Parser is not Send).
///   Use thread_local! or rayon::ThreadLocal for parser pooling.
/// - ExtractionResult uses arena-allocated temporary nodes/edges.
/// - The merge step builds the CSR arrays in one pass: sort nodes, compute offsets, pack edges.
/// - String interning uses a concurrent DashMap during parallel extraction,
///   then freezes into the packed StringInterner format during merge.

/// PERFORMANCE CRITICAL: Use per-thread bump allocators for ExtractionResult construction.
/// During parallel extraction, each thread produces hundreds of Node/Edge values.
/// Individual Vec pushes cause many small allocations. Instead, pre-allocate a Vec
/// with generous capacity per-thread (e.g., 10K nodes, 50K edges) and fill it.
///
/// let mut nodes = Vec::with_capacity(estimated_symbols_per_file * file_count / num_threads);
/// let mut edges = Vec::with_capacity(estimated_edges_per_file * file_count / num_threads);
///
/// After all threads complete, merge all per-thread Vecs into the final arrays.
/// This is faster than using a shared concurrent Vec (lock contention) or
/// per-file Vec allocations (thousands of small allocs).
```

### Graph Construction Pipeline

The merge step that converts parallel `ExtractionResult` bags into the final CSR graph:

```rust
/// Build the CSR graph from collected extraction results.
/// This is single-threaded but fast — dominated by sorting.
///
/// PERFORMANCE RULES:
/// - Allocate all arrays once with exact capacity. No resizing.
/// - Sort nodes in-place (unstable sort is fine — faster than stable).
/// - Build offsets in a single linear scan after sorting edges by source.
/// - Build the reverse index at the same time (sort edges by target for a second CSR).
fn build_csr(mut all_results: Vec<ExtractionResult>) -> CsrGraph {
    // 1. Flatten all nodes, count total
    let total_nodes = all_results.iter().map(|r| r.nodes.len()).sum();
    let total_edges = all_results.iter().map(|r| r.edges.len()).sum();

    let mut nodes = Vec::with_capacity(total_nodes);
    let mut edges = Vec::with_capacity(total_edges);
    for result in &mut all_results {
        nodes.append(&mut result.nodes);
        edges.append(&mut result.edges);
    }

    // 2. Assign sequential NodeIds, sort nodes by (kind, name) for locality
    nodes.sort_unstable_by_key(|n| (n.kind, n.name));
    // Remap node IDs and update edge source/target references...

    // 3. Sort edges by source NodeId, build offsets
    edges.sort_unstable_by_key(|e| e.source); // source is implicit in CSR — sort for grouping
    let mut offsets = Vec::with_capacity(nodes.len() + 1);
    // ... build offset array in single linear scan

    // 4. Build reverse index: sort copy of edges by target, build rev_offsets
    let mut rev_edges = edges.clone();
    rev_edges.sort_unstable_by_key(|e| e.target);
    // ... build rev_offsets

    // 5. Freeze string interner: convert DashMap → packed format
    // 6. Write mmap file: header + nodes + offsets + edges + rev_offsets + rev_edges + strings
}
```

### Target Size Estimates

For a large codebase (10M lines of code across 200 repos):
- ~1M nodes × 32 bytes = 32MB (nodes)
- ~10M edges × 16 bytes = 160MB (edges, forward)
- ~10M edges × 16 bytes = 160MB (edges, reverse)
- ~1M offsets × 4 bytes = 4MB (offsets)
- ~200MB string table
- **Total: ~550MB** — fits easily in RAM. OS will page in/out as needed via mmap.

For a typical microservices setup (5-10 repos, 500K lines): ~30-60MB total. Loads in <10ms.

### Git-Native Versioning

The graph is versioned against git history:

```
index/
  refs/
    main          → points to graph snapshot abc123
    feature/x     → points to delta from main + changes
    staging       → points to delta from main + changes
  objects/
    abc123.graph  → full CSR graph (the base snapshot)
    def456.delta  → delta: +3 nodes, +5 edges, -2 edges
    ghi789.delta  → delta: +1 node, +8 edges, -3 edges
```

- **Base snapshot** for a reference commit (usually `main` HEAD). Full CSR graph. Memory-mapped. Fast.
- **Delta-encoded layers** for branches. Most branches change a tiny fraction of the graph, so deltas are small.
- Querying `main` hits the base graph directly. Querying `feature/x` overlays the delta at query time.
- Periodically (or on merge to main), deltas get compacted into the base snapshot.

This is analogous to git's own object model — full snapshots plus diffs — applied to a code graph instead of file content.

## Extractor Architecture

Each discovery mechanism is a pluggable extractor:

```rust
trait Extractor: Send + Sync {
    /// File patterns this extractor cares about (e.g., "*.proto", "*.go", "values.yaml")
    fn file_patterns(&self) -> &[GlobPattern];

    /// Given a parsed file, produce nodes and edges.
    ///
    /// PERFORMANCE RULES:
    /// - This runs in parallel via rayon. Must be thread-safe (Send + Sync).
    /// - Use the provided StringInterner (DashMap-backed during indexing) for all strings.
    /// - Do NOT allocate Strings for symbol names. Intern immediately and store StringId.
    /// - Pre-allocate ExtractionResult vectors with estimated capacity.
    /// - Tree-sitter gives zero-copy access to source text — use byte slices, not String.
    fn extract(
        &self,
        file: &ParsedFile,       // contains tree-sitter Tree + source bytes (zero-copy mmap'd)
        context: &RepoContext,
        strings: &DashMap<u64, StringId>,  // concurrent string interning
    ) -> ExtractionResult;
}

struct ExtractionResult {
    nodes: Vec<Node>,
    edges: Vec<Edge>,            // fully resolved edges
    dangling: Vec<DanglingEdge>, // edges waiting for cross-repo resolution
    meta: Vec<NodeMeta>,         // cold metadata for the nodes we created
    edge_meta: Vec<EdgeMeta>,    // cold metadata for edges that have provenance
}

/// Source file pre-parsed by tree-sitter. The source bytes are borrowed (zero-copy
/// from mmap or read_to_string), and the Tree is owned by the parsing thread.
struct ParsedFile<'src> {
    tree: tree_sitter::Tree,
    source: &'src [u8],         // zero-copy reference to file content
    path: StringId,             // interned file path
    repo: RepoId,
}
```

### Source Context Preservation

The indexing pipeline preserves source bytes alongside each `RawFileExtraction` so that downstream stages (particularly LLM classification) can access original source code without re-reading files from disk.

`extract_call_context()` reads approximately 30 lines around each call site, providing enough surrounding code for the LLM to understand the call's purpose and classify it accurately. `RawDef.byte_start` and `RawDef.byte_end` fields provide exact function boundaries within the source bytes, enabling precise slicing of function bodies for context extraction.

This design avoids re-opening files during classification while keeping memory usage bounded — source bytes are only retained for files that contain unresolved calls requiring LLM analysis.

### Extractors to Build (Priority Order)

#### Phase 1: Core (Weeks 1-4)

1. **TreeSitterExtractor** — Language-generic symbol extraction using tree-sitter grammars. Functions, classes, methods, types, call sites, imports. Supports Go, TypeScript, Python initially.

2. **ProtoExtractor** — Parses `.proto` files. Extracts service definitions, RPC methods, message types. Builds a global map of `(package.ServiceName.MethodName) → file location`.

3. **GrpcClientExtractor** — Language-specific. Finds gRPC client dial calls and stub constructors:
   - Go: `pb.New{ServiceName}Client(conn)`, `grpc.Dial()`
   - Python: `{ServiceName}Stub(channel)`
   - TypeScript: similar patterns for grpc-js

4. **GrpcServerExtractor** — Language-specific. Finds gRPC server registrations:
   - Go: `pb.Register{ServiceName}Server()`
   - Python: `add_{ServiceName}Servicer_to_server()`

5. **EnvVarExtractor** — Finds environment variable reads across languages:
   - Go: `os.Getenv("X")`
   - Python: `os.environ["X"]`, `os.getenv("X")`
   - TypeScript: `process.env.X`

#### Phase 2: Infrastructure (Weeks 3-6)

6. **HelmValuesExtractor** — Parses Helm `values.yaml` and `deployment.yaml` templates. Extracts env var definitions (`name: X / value: Y` pairs). Resolves Go templates against defaults. Detects `valueFrom` configmap/secret references.

7. **K8sManifestExtractor** — Parses Kubernetes YAML. Extracts service names, deployment specs, ingress rules, configmaps. Resolves Kubernetes DNS patterns (`{service}.{namespace}.svc.cluster.local`).

8. **DockerfileExtractor** — Parses Dockerfiles. Extracts base images, exposed ports, CMD/ENTRYPOINT (identifies deployable entry points).

#### Phase 3: Extended Protocols (Weeks 5-8)

9. **RestClientExtractor** — Finds HTTP client calls. Extracts URL patterns, matches against OpenAPI specs or server route registrations. Confidence varies by how dynamic the URL construction is.

10. **OpenApiExtractor** — Parses OpenAPI/Swagger specs. Extracts endpoints and schemas.

11. **MessageQueueExtractor** — Finds Kafka/NATS/SQS publish/subscribe calls. Extracts topic strings. Matches publishers to subscribers across repos.

12. **DatabaseExtractor** — Finds `sql.Open()` and ORM model definitions. Creates resource nodes. Detects shared database dependencies between services.

13. **SqlMigrationExtractor** — Parses SQL migration files for schema information.

### Auto-Detection on `cx build`

When `cx build` runs, it scans the repo and auto-detects what's inside:

**Deployable detection:** Look for `main` packages (Go), Dockerfiles, Helm chart `templates/deployment.yaml`, serverless configs (`serverless.yml`, `lambda.tf`), `Procfile` entries.

**Surface detection:** Proto files with `service` definitions, `go.mod` (importable module), `package.json` with `main`/`exports`, OpenAPI specs, GraphQL schema files.

**InfraConfig detection:** `Chart.yaml` (Helm), `*.tf` (Terraform), Kubernetes manifests, `docker-compose.yml`, CI configs referencing deployments.

**Module boundary detection:** In Go, each `main` package is a distinct binary with its own import tree — CX traces imports from each main to build separate dependency subgraphs. In Python/TypeScript, use Dockerfile CMD, package.json scripts, pyproject.toml entry points. When heuristics fail, treat the whole repo as one module and let the developer refine via config.

## Resolution Engine

The resolution engine runs after all extractors produce their results, matching dangling edges to nodes across repos.

### Cross-Repo Edge Resolution

#### Edge Type 1: gRPC / Proto (Highest Confidence)

1. **Proto surface extraction.** Parse every `.proto` file across all indexed repos. Build global map: `(package.ServiceName.MethodName) → file location`.
2. **Client stub detection.** Language-specific matchers for generated client patterns: `New{ServiceName}Client()` (Go), `{ServiceName}Stub()` (Python).
3. **Server registration detection.** `Register{ServiceName}Server()` (Go), `add_{ServiceName}Servicer_to_server()` (Python).
4. **Edge creation.** Match client in repo A to server in repo B via proto service name. Confidence: 0.95.

Proto files may live in the client repo, server repo, or a shared proto registry. CX matches by fully qualified service name, not file location. CX also detects proto mismatches: if client and server reference the same service but have different field counts, flag it.

#### Edge Type 2: Environment Variables to Helm Charts (Medium Confidence)

1. **Env var extraction from code.** Find every `os.Getenv("X")` etc.
2. **Env var definition extraction from infra.** Parse Helm `values.yaml`, `deployment.yaml`, `docker-compose.yml`, `.env` files.
3. **Match by variable name.** String match on env var names. Confidence: 0.85 for exact match.
4. **Resolve value to service.** Parse Kubernetes DNS pattern `{service-name}.{namespace}.svc.cluster.local:{port}` → match to repo deploying as that service name.

**Hard cases:**
- Helm Go templates: resolve with `values.yaml` defaults, mark as "template-resolved, confidence: medium"
- `valueFrom` configmap/secret references: flag as gap, resolve if Terraform/configmap repo is indexed
- Per-environment overrides: support `values-prod.yaml`, `values-staging.yaml` for per-environment resolution

#### Edge Type 3: REST / HTTP (Lower Confidence)

In order of decreasing confidence:
1. **OpenAPI spec exists:** Parse spec, match HTTP client calls by URL pattern. Confidence: 0.85.
2. **URL pattern discoverable:** Extract URL pattern from code, match against route registrations in target service. Confidence: 0.5-0.7.
3. **Dynamic URL construction:** Extract partial pattern, flag as partial edge. Confidence: 0.3-0.5.

#### Edge Type 4: Message Queues (Medium Confidence)

Find all publish calls, find all subscribe calls, match by topic string. When topic names are dynamic/constructed, extract known prefix and match with lower confidence. Verify publisher and subscriber point to the same broker cluster by tracing broker config env vars.

#### Edge Type 5: Database Connections (Resource Nodes)

Databases become resource nodes, not service nodes. CX infers what the database stores by analyzing ORM models, migrations, or raw SQL. If two services connect to the same `DATABASE_URL` (or a read replica), CX detects the shared data dependency — two services coupled through a database.

#### Cross-Repo Endpoint Matching

`find_cross_repo_matches()` in `network.rs` matches outbound calls to inbound endpoints across all indexed repos. Three matching strategies are applied in order:

1. **Path match.** An outbound HTTP call to `/api/v1/orders` is matched against an inbound endpoint registered at `/api/v1/orders` in another repo.
2. **gRPC service name match.** A gRPC client stub for `OrderProcessing` is matched against a `RegisterOrderProcessingServer` registration.
3. **K8s DNS URL match.** An outbound call to `acme-orders.prod.svc.cluster.local:50051` is matched against a repo whose Helm chart deploys under the service name `acme-orders`.

Noise filtering via `is_noise_path()` excludes test files, archive directories, and vendored code from matching to avoid false positives. Deduplication handles cases where multiple git remotes point to the same repository.

#### K8s Gotmpl Parsing

The resolution engine handles Helm Go template syntax in Kubernetes manifests:

- `extract_helm_env_vars()` scans YAML and `.gotmpl` files for environment variable definitions (`name`/`value` pairs in container specs).
- `replace_gotmpl_expressions()` resolves `{{ }}` template expressions against `values.yaml` defaults.
- `infer_category_from_url()` classifies resolved URLs by scheme (e.g., `postgres://` becomes a database connection, `grpc://` becomes a gRPC dial).
- `is_network_value()` detects values that look like network addresses (URLs, host:port patterns, DNS names) to separate network config from non-network config.

### Progressive Resolution

When `cx add` brings in a new repo, a resolution pass runs:
- Take all dangling edges across the graph
- Try to match them against the new repo's surfaces (proto services, route registrations, queue subscriptions)
- Report: "Added acme-orders. Resolved 3 dangling edges. 7 edges still unresolved."

The graph knits itself together incrementally. Each `cx add` gives immediate feedback.

## Query Engine

### Query Primitives

```
cx_resolve(query, kind?)
  Resolve a fuzzy name to specific symbols in the indexed codebase.
  → [{ id, name, kind, file, line, service, confidence }]

cx_path(from, direction?, ref?)
  Trace the full execution path from an entry point through all service boundaries to terminal resources.
  → { hops, boundary_crossings, terminal_resources, gaps, completeness }

cx_impact(symbol_id, max_depth?, compare_ref?)
  Show everything transitively affected by changing a symbol.
  → { affected_symbols, affected_services, affected_endpoints, affected_resources, config_dependencies }

cx_depends(target, direction, depth?, ref?)
  Get the dependency set for a service or symbol.
  → { graph: { nodes, edges }, summary }

cx_search(query, filters?)
  Semantic symbol search across the entire indexed codebase.
  → [{ id, name, kind, file, line, service, snippet }]

cx_context(service | file | repo)
  Get a structured summary — what it does, what it exposes, what it depends on.
  → { deployables, surfaces, endpoints_exposed, dependencies, resources, config, gaps }

cx_diff(ref_a, ref_b)
  Structural diff between two git refs.
  → { added_nodes, removed_nodes, added_edges, removed_edges, changed_paths, config_gaps }

cx_log(symbol_id | edge)
  Git-log but for graph edges. When did this dependency appear? Who introduced it?
  → [{ commit, author, date, change_type, description }]

cx_blame(edge)
  Who introduced this dependency?
  → { commit, author, date, pr_url? }
```

### Query Algorithms

Each query primitive has a specific algorithm. An agent implementing these MUST follow the specified approach — the algorithms are chosen for performance, not simplicity.

#### cx_path: Path Reconstruction with Parent Tracking

`cx_path` must return the actual ordered sequence of hops, not just the set of reachable nodes. This requires parent tracking during BFS.

```rust
/// CRITICAL: BfsState::run() only gives you the SET of reachable nodes.
/// cx_path needs the actual PATH. Use a parent array.
///
/// parent[node_id] = (predecessor_node_id, edge_kind) that led us here.
/// After BFS completes, reconstruct the path by walking parent[] backwards from target to seed.
///
/// PERFORMANCE: parent[] is a flat Vec<(NodeId, u8)> indexed by NodeId.
/// Pre-allocate to graph.nodes.len(). Do NOT use HashMap.

struct PathFinder {
    parent: Vec<(NodeId, u8)>,   // parent[i] = (predecessor, edge_kind). u32::MAX = no parent.
    visited: BitVec,
    queue_current: Vec<NodeId>,
    queue_next: Vec<NodeId>,
}

impl PathFinder {
    /// Find path from `from` to `to`. Returns ordered hops.
    fn find_path(&mut self, graph: &CsrGraph, from: NodeId, to: NodeId, mask: EdgeKindMask) -> Option<Vec<Hop>> {
        // BFS with parent tracking
        // When we reach `to`, walk parent[] backwards to reconstruct path
        // If `to` is NodeId::MAX (no specific target), collect ALL terminal nodes
        // (nodes with no outgoing edges matching the mask) and return paths to each
    }

    /// For cx_path --downstream with no specific target:
    /// BFS from seed, follow ALL edges matching mask, record parent[].
    /// Identify terminal nodes (no outgoing edges matching mask, or unresolved dangling edges).
    /// Return paths to each terminal, plus any gaps encountered.
    fn find_all_paths_downstream(&mut self, graph: &CsrGraph, from: NodeId, mask: EdgeKindMask) -> PathResult {
        // 1. BFS with parent tracking
        // 2. Identify terminals: nodes where offsets[i] == offsets[i+1] (no outgoing edges)
        //    OR all outgoing edges are to unindexed services (dangling)
        // 3. For each terminal, walk parent[] backwards to build the hop sequence
        // 4. Detect gaps: any dangling edge encountered during BFS becomes a gap entry
        // 5. Compute completeness: resolved_edges / (resolved_edges + dangling_edges)
    }
}

/// A single hop in a path result.
/// RULE: Hops are enriched AFTER traversal. During BFS, only collect NodeIds and EdgeKinds.
/// After collecting the path as [(NodeId, u8)], resolve to Hop structs in a single pass.
struct Hop {
    node: NodeId,
    node_name: StringId,
    node_kind: u8,
    service: StringId,       // which deployable/repo this belongs to (from node.parent chain)
    file: StringId,
    line: u32,
    edge_to_next: u8,       // EdgeKind that connects this hop to the next
    is_cross_repo: bool,     // true if this hop crosses a repo boundary
}
```

#### cx_search: Trigram Index for Fuzzy Matching

String search across 1M symbols must be fast. Linear scan with substring matching is O(N×M). Use a trigram index for O(1) candidate retrieval.

```rust
/// PERFORMANCE CRITICAL: Build a trigram index over symbol names during graph construction.
/// A trigram index maps every 3-character substring to the set of StringIds containing it.
///
/// Example: "handleAudio" → trigrams: "han", "and", "ndl", "dle", "leA", "eAu", "Aud", "udi", "dio"
/// Searching for "Audio" → trigrams: "Aud", "udi", "dio" → intersect candidate sets → rank by score.

struct TrigramIndex {
    /// Maps trigram (packed as u32: 3 bytes + padding) → list of StringIds.
    /// PERFORMANCE: Use a flat array, not HashMap.
    /// Trigram space is 256^3 = 16M possible trigrams, but only ~50K are actually used.
    /// Use a hash map from trigram → Vec<StringId> during construction,
    /// then flatten to sorted arrays for query-time intersection.
    trigrams: FxHashMap<u32, Vec<StringId>>,
}

impl TrigramIndex {
    /// Search: extract trigrams from query, intersect candidate sets, rank.
    fn search(&self, query: &str, max_results: usize) -> Vec<(StringId, f32)> {
        // 1. Extract trigrams from query (lowercase for case-insensitive)
        // 2. For each trigram, get candidate StringId set
        // 3. Intersect all sets (start with smallest set for efficiency)
        // 4. Score each candidate: number of matching trigrams / total trigrams in query
        //    Bonus for exact substring match. Bonus for match at word boundary.
        // 5. Sort by score descending, take top max_results
        // 6. Map StringIds back to NodeIds (may be multiple nodes with same name)
    }
}

/// BUILD RULE: The TrigramIndex is built during the graph construction merge step.
/// It is serialized into the mmap file after the string table.
/// At load time, it is deserialized (this is the one section that requires deserialization,
/// not zero-copy mmap cast — keep it small).
```

#### cx_resolve: Name Resolution

`cx_resolve` is `cx_search` with an additional constraint: it returns nodes, not just string matches. It also supports qualified names like "service.Symbol" or "package.Function".

```rust
/// cx_resolve("OrderProcessing.StreamingRecognize", kind=Endpoint)
///
/// Algorithm:
/// 1. Split query on "." to detect qualified names
/// 2. If qualified: search for the last component, then filter by parent chain matching earlier components
/// 3. If unqualified: trigram search on the full query string
/// 4. Filter results by `kind` parameter if specified
/// 5. Rank by: exact match > prefix match > substring match > fuzzy match
///           Within each tier: higher confidence nodes first, then alphabetical
```

#### cx_impact: Transitive Closure with Classification

```rust
/// cx_impact is a transitive closure (all reachable nodes downstream from seed).
/// Standard BFS, but with post-processing to classify results.
///
/// Algorithm:
/// 1. BFS downstream from seed node using ALL_EDGES mask
/// 2. Collect all reachable NodeIds into result set
/// 3. CLASSIFY results by node kind:
///    - affected_symbols: result nodes where kind == Symbol
///    - affected_services: unique Deployable ancestors of affected symbols
///      (walk parent chain for each affected symbol → collect unique Deployable NodeIds)
///    - affected_endpoints: result nodes where kind == Endpoint
///    - affected_resources: result nodes where kind == Resource
/// 4. CONFIG DEPENDENCIES: for each affected Deployable, look up its InfraConfig
///    (via Configures edges in reverse). List all env vars and helm values.
/// 5. VERSION SKEW DETECTION: if seed is in a shared library (Surface node),
///    and multiple Deployables import it, flag independent deploy risk.
///
/// PERFORMANCE: Classification is a single pass over the result Vec.
/// Do NOT run separate BFS queries for each classification — that's 4x the work.
```

#### cx_diff: Set Operations on Graph Snapshots

```rust
/// cx_diff compares two graph states (branch A vs branch B).
/// This is NOT a BFS — it's a set difference operation on node and edge IDs.
///
/// Algorithm:
/// 1. Load graph state for ref_a and ref_b
///    (base snapshot + delta overlay for each ref)
/// 2. Node diff: symmetric difference of node sets
///    - added_nodes: in ref_b but not ref_a
///    - removed_nodes: in ref_a but not ref_b
/// 3. Edge diff: symmetric difference of edge sets
///    (edges compared by (source, target, kind) tuple, NOT by EdgeMeta)
///    - added_edges: in ref_b but not ref_a
///    - removed_edges: in ref_a but not ref_b
/// 4. Changed paths: for each added/removed DependsOn edge,
///    re-run cx_path on both refs to show before/after
///    (ONLY for service-level edges, not symbol-level — use SummaryGraph)
/// 5. Config gap detection: for each new env var read (added dangling edge of type Resolves),
///    check if corresponding InfraConfig exists. If not, flag as config gap.
///
/// PERFORMANCE: The expensive part is step 4 (re-running cx_path).
/// Limit to at most 20 changed service-level edges. If more, summarize without paths.
///
/// For set difference: sort both node/edge arrays by ID, then merge-scan in O(N+M).
/// Do NOT use HashSet — the sorted merge is faster for large arrays and cache-friendly.
```

#### cx_depends: Filtered Transitive Closure

```rust
/// cx_depends(target, direction=upstream, depth=3)
///
/// Algorithm:
/// 1. If direction=downstream: standard BFS from target using SERVICE_EDGES mask on full graph.
///    OR: use SummaryGraph for service-level queries (much faster, <100 nodes).
/// 2. If direction=upstream: BFS from target on REVERSE edge index using SERVICE_EDGES mask.
/// 3. Depth limit applied in BFS.
/// 4. Return result as a subgraph: the set of nodes and edges traversed.
///
/// DECISION: Use SummaryGraph when target is a Deployable and depth > 1.
/// Use full graph when target is a Symbol (need intra-service call chain).
```

### Gap Handling

Every query response carries completeness information:

```json
{
  "hops": [...],
  "gaps": [
    {
      "at": "frontend → media-service",
      "reason": "service 'media-service' not in index",
      "hint": "references proto media.service.v1, likely repo: github.com/acme-corp/media-service"
    }
  ],
  "completeness": 0.75
}
```

The agent sees the gap and can make an intelligent decision — ask the developer to index the missing repo, or fall back to grepping within it.

## Interface Layer

### 1. MCP Server (Highest Priority)

Exposes query primitives as MCP tools. The binary starts, loads the index from disk in milliseconds, and serves queries over JSON-RPC via stdio.

**Performance model:** The MCP server holds the graph mmap'd for the lifetime of the process. A `QueryContext` struct is created once and reused across all queries — it owns the pre-allocated `BitVec` (visited set), result `Vec`, and `VecDeque` (BFS queue). Every query clears and reuses these, never reallocates.

```rust
/// Created once when MCP server starts. Reused across all queries.
/// NEVER create a new QueryContext per query.
struct QueryContext {
    bfs: BfsState,                // double-buffer BFS state (see Graph Storage section)
    summary_bfs: BfsState,        // separate BFS state for summary graph queries
    output_buf: Vec<u8>,          // reused JSON serialization buffer
}
```

MCP config:
```json
{
  "mcpServers": {
    "cx": { "command": "cx", "args": ["mcp"] }
  }
}
```

Current MCP tools:

| Tool | What it does |
|------|-------------|
| `cx_path` | Trace execution flow across service boundaries |
| `cx_network` | All network boundaries with address provenance chains |
| `cx_diff` | Compare current network boundaries against saved baseline |
| `cx_explain` | Explain why a connection exists — provenance chain with code locations |

### 2. CLI (Second Priority)

Direct terminal queries for developers:
```bash
cx build [paths...]              # index one or more repos
cx build --model-only            # skip static analysis, LLM classifies everything
cx trace <target>                # trace lineage (env var, function, call site)
cx trace 'env:*'                 # compact overview of all env vars
cx network                       # all network boundaries with provenance
cx network --local-only          # skip remote data
cx diff --save                   # save baseline for future diffs
cx diff                          # compare current vs baseline
cx diff --branch main            # compare current vs another branch
cx add <path-or-git-url>         # add remote repo's pre-built graph
cx pull                          # refresh remotes
cx fix                           # show unresolved calls
cx fix --init                    # generate .cx/config/sinks.toml template
cx hook --install                # install post-commit hook (auto-updates graph)
cx hook --remove                 # remove it
cx skill                         # install Claude Code skill
cx mcp                           # start MCP server
```

`cx network` prints a summary header with counts of inbound endpoints, outbound calls, and cross-repo matches, followed by the detailed listing. Remote network calls are filtered to only show those matching local env var reads. The `--local-only` flag restricts output to the current repo's calls. The `--include-all` flag shows all results including test/vendor files and unmatched remote data.

### 3. CI Integration (Third Priority)

GitHub Actions step that runs `cx diff main $PR_BRANCH --format github-comment`:
```
## cx structural impact

This PR adds a new dependency: acme-gateway → acme-cache (gRPC)

Changed request paths:
  /ws/translate: added cache lookup before TTS call (+1 hop)

New config requirements:
  ⚠ CACHE_SERVICE_ADDR — not found in helm charts. Add before deploying.
```

### 4. LSP Extension (Future)

Hover over a function in the editor and see cross-service dependency chain. Third priority because the MCP and CLI interfaces serve the AI agent use case which is the primary differentiator.

## Setup and Onboarding

### First-Run Experience

```bash
cd ~/code/my-service
cx build
```

What happens:

1. **Instant local index** (sub-second). Tree-sitter parses all source files. Symbols, call edges, proto definitions, env var reads extracted.
2. **Auto-discovery** (background, seconds). CX reads the git remote, searches the same GitHub org for repos that match dangling proto service references. Prompts the developer:
   ```
   Discovered external dependencies:
     → gRPC: OrderProcessing service (client at orders/router.go:134)
     → gRPC: NotificationService (client at notify/router.go:167)

   Searching GitHub for matching repos...
     ✓ Found acme-orders — implements OrderProcessing
     ✓ Found acme-notifications — implements NotificationService

   Index these repos? [Y/n]
   ```
3. **Cross-repo resolution** (seconds). Shallow-clone discovered repos into `~/.cx/repos/`, index them, resolve edges.
4. **MCP config** (automatic). Writes `.cx/mcp.json` for Claude Code auto-discovery.

### On-Disk Directory Structure

```
# Per-workspace (inside the repo where `cx build` was run)
.cx/
├── config.toml              # workspace config (repos, remotes)
├── config/
│   └── sinks.toml           # custom network function definitions
├── graph/
│   ├── base.cxgraph         # unified graph (all repos + remotes merged)
│   ├── network.json         # taint analysis results (provenance chains)
│   ├── index.json           # global cross-repo index
│   ├── overlay.json         # cross-repo edges
│   └── repos/
│       ├── 0000-my-service.cxgraph
│       └── 0001-other-repo.cxgraph
└── remotes/
    ├── other-service.cxgraph      # pulled from other team
    ├── other-service.network.json
    ├── k8s-config.cxgraph         # pulled from infra team
    └── clones/                    # git-cloned remotes
        └── k8s-config/

# Global (shared across all workspaces)
~/.cx/
├── repos/                   # shallow-cloned remote repos for cross-repo indexing
│   ├── acme-orders/
│   ├── acme-notifications/
│   └── ...
└── cache/                   # downloaded proto files, resolved configs
```

**File extension:** `.cxgraph` for the main graph file. `.delta` for branch deltas. These are binary files (the mmap format described in Graph Storage).

### Auto-Discovery Strategies

**Same GitHub org:** Search for repos containing proto files that define the missing service. Proto service names are unique within an org.

**Go module graph:** Follow `go.mod` imports to shared proto packages, then find which repos import the same proto as server implementations.

**Convention scanning for infra repos:** Search same org for repos named `*-helm`, `*-charts`, `*-deploy`, `*-infra`. Check if they contain Helm charts referencing the current service.

For v1: auto-discover service repos via proto matching (high confidence). Prompt manually for infra repos:
```
To resolve environment variables and deployment config:
  cx add --role infra <path-or-url>
Or skip — cx will show env vars as unresolved.
```

### Progressive Disclosure

| Stage | Time | Value |
|-------|------|-------|
| Single repo, zero config | 30 seconds | `cx network`, `cx trace` — faster than grep |
| Auto-discovered services | 2 minutes | Cross-service queries: `cx trace`, `cx impact` |
| Infrastructure resolution | 5 minutes | Env vars resolve, deployment topology appears |
| Team adoption | 30 minutes | `.cx/config.toml` committed, team shares the graph |
| CI integration | 1 hour | Structural impact comments on every PR |

### Team Configuration

```toml
# .cx/config.toml — committed to the repo, shared with the team
[workspace]
name = "acme-platform"

[[repos]]
url = "github.com/acme-corp/acme-orders"
role = "service"

[[repos]]
url = "github.com/acme-corp/acme-notifications"
role = "service"

[[repos]]
url = "github.com/acme-corp/acme-analytics"
role = "service"

[[repos]]
url = "github.com/acme-corp/acme-helm-charts"
role = "infrastructure"
path_filter = "charts/acme-*"

[discovery]
github_org = "acme-corp"
auto_discover = true
```

New engineer clones, runs `cx build`, has the full topology in under a minute.

## Error Handling Strategy

CX uses `panic = "abort"` in release builds for smaller binaries and zero unwinding overhead. This means panics are fatal. All error handling must use `Result<T, E>`, never `unwrap()` or `expect()` in library code.

```rust
/// CX error type. Use thiserror for ergonomic error definitions.
/// RULE: Every public function in cx-core, cx-extractors, cx-resolution returns Result<T, CxError>.
/// RULE: NEVER use .unwrap() or .expect() in library code. Only allowed in tests and CLI main().
/// RULE: NEVER use panic!() for recoverable errors.
#[derive(Debug, thiserror::Error)]
enum CxError {
    #[error("graph file corrupted: {0}")]
    CorruptGraph(String),       // bad magic, checksum mismatch, truncated file

    #[error("graph file version {found} not supported (expected {expected})")]
    VersionMismatch { found: u32, expected: u32 },

    #[error("index not found: run 'cx build' first")]
    NoIndex,

    #[error("repo not found: {0}")]
    RepoNotFound(String),

    #[error("symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("parse error in {file}: {message}")]
    ParseError { file: String, message: String },

    #[error("git error: {0}")]
    Git(#[from] gix::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),
}
```

**CLI error display:** Use `anyhow` in the CLI binary for ergonomic error chain display. Library crates (`cx-core`, `cx-extractors`) use typed `CxError`. The CLI wraps library errors in `anyhow::Result`.

**Extractor errors are non-fatal.** If one file fails to parse, log a warning and continue. A single broken file should never prevent indexing the rest of the codebase. The graph is built from whatever succeeded. Errors are collected and reported in `cx status`.

**Mmap validation:** On graph load, verify the header magic, version, checksum, and that all section offsets are within file bounds. If any check fails, return `CxError::CorruptGraph` and suggest re-indexing.

## MCP Protocol Implementation

The MCP server communicates over stdio using JSON-RPC 2.0 framed with Content-Length headers (same as LSP).

```
Content-Length: 73\r\n
\r\n
{"jsonrpc":"2.0","method":"tools/call","params":{"name":"cx_path",...},"id":1}
```

**Implementation rules:**

```rust
/// MCP server lifecycle:
/// 1. Read from stdin in a loop. Parse Content-Length header, then read N bytes of JSON.
/// 2. Deserialize JSON-RPC request. Dispatch to appropriate query function.
/// 3. Execute query (synchronous — queries complete in microseconds, no need for async).
/// 4. Serialize response to pre-allocated buffer (output_buf in QueryContext).
/// 5. Write Content-Length header + response to stdout.
///
/// RULES:
/// - Use BufReader wrapping stdin and BufWriter wrapping stdout.
///   Raw stdin/stdout do a syscall per read/write call.
/// - Deserialize requests with serde_json::from_slice (zero-copy on the input buffer).
/// - The server process is long-lived (lifetime of the IDE/agent session).
///   The graph mmap and QueryContext persist across all requests.
/// - Handle "initialize" and "tools/list" requests per MCP spec.
/// - On graph file change (detected via mtime check or file watcher),
///   re-mmap the graph file. This is transparent to the client.
///
/// STDIN READING (avoid a common footgun):
/// Do NOT use BufRead::read_line() — JSON-RPC uses Content-Length framing, not newlines.
/// Read the header line-by-line to get Content-Length, then read exactly N bytes for the body.

/// MCP tool definitions (returned in tools/list response):
const MCP_TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "cx_path",
        description: "Trace the full execution path from an entry point through all \
                      service boundaries to terminal resources. Returns ordered hops \
                      with service names, file locations, and protocol information. \
                      Also reports gaps where services are not indexed.",
        input_schema: r#"{
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "Endpoint or symbol name to trace from" },
                "direction": { "type": "string", "enum": ["downstream", "upstream"], "default": "downstream" },
                "ref": { "type": "string", "description": "Git ref to query against (default: current HEAD)" },
                "max_depth": { "type": "integer", "default": 20 }
            },
            "required": ["from"]
        }"#,
    },
    // ... cx_impact, cx_depends, cx_context, cx_search, cx_resolve, cx_diff, cx_blame, cx_log
];
```

## Incremental Update Pipeline

When a developer edits files, the graph must update without full re-indexing. Here is the precise data flow:

```
File saved → notify event (debounce 100ms) → identify changed files
    → parallel tree-sitter re-parse (ONLY changed files)
    → run extractors on new parse trees
    → compute delta: diff old ExtractionResult vs new ExtractionResult for each file
    → apply delta to in-memory graph (or write delta file to disk)
    → if MCP server is running, it detects updated graph and re-mmaps
```

**Implementation details:**

```rust
/// RULE: Never re-index the entire repo on a single file change.
/// Only re-parse the changed file(s) and compute a delta.

struct IncrementalUpdater {
    /// Previous extraction results per file, keyed by file path StringId.
    /// Used to compute what nodes/edges were added/removed.
    prev_results: FxHashMap<StringId, ExtractionResult>,
    /// File watcher
    watcher: notify::RecommendedWatcher,
    /// Debounce timer — batch file events within 100ms window
    debounce_ms: u64,
}

impl IncrementalUpdater {
    fn handle_file_changes(&mut self, changed_files: &[PathBuf], graph: &mut CsrGraph) {
        // 1. For each changed file:
        //    a. Re-parse with tree-sitter (incremental parse if possible)
        //    b. Run extractors → new ExtractionResult
        //    c. Diff against prev_results[file]:
        //       - added_nodes: in new but not old
        //       - removed_nodes: in old but not new
        //       - added_edges: in new but not old
        //       - removed_edges: in old but not new
        //    d. Store new result in prev_results[file]

        // 2. Aggregate all per-file deltas into a single GraphDelta

        // 3. Apply delta:
        //    - FOR IN-MEMORY UPDATES: apply delta directly to the CsrGraph.
        //      This requires rebuilding the CSR arrays (sort + compact).
        //      ONLY do this if delta is large (>1000 changes).
        //      For small deltas (<1000 changes), store as an overlay and
        //      apply at query time (delta overlay is cheap for small deltas).
        //    - FOR ON-DISK UPDATES: write delta file alongside the base .cxgraph.
        //      The MCP server detects the delta file and loads it as an overlay.

        // PERFORMANCE: The expensive operation is CSR rebuild after delta.
        // Heuristic: if delta affects <1% of nodes, use overlay. If >1%, rebuild.
    }
}

/// DEBOUNCE: File watchers fire multiple events for a single save
/// (write, truncate, chmod, etc). Collect events for 100ms, deduplicate by path,
/// then process the batch. Use crossbeam-channel with recv_timeout for debouncing.
///
/// GIT OPERATIONS: A `git checkout` or `git pull` may change hundreds of files at once.
/// The debounce window catches these. If >50% of files change, do a full re-index
/// instead of incremental — it's faster than computing hundreds of per-file deltas.
```

## Rust Project Structure

```
cx/
├── Cargo.toml                    # workspace root
├── cx-cli/                       # CLI binary and MCP server
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs               # CLI entry point, argument parsing
│       ├── commands/              # init, add, path, impact, depends, diff, etc.
│       └── mcp/                  # MCP server implementation (JSON-RPC over stdio)
│           ├── mod.rs
│           └── serialize.rs      # Direct JSON serialization (no serde_json::Value intermediate)
├── cx-core/                      # Core library: graph, storage, queries
│   ├── Cargo.toml
│   ├── benches/
│   │   ├── graph_traversal.rs    # Benchmark: BFS/DFS at various scales (1K, 10K, 100K, 1M nodes)
│   │   ├── graph_loading.rs      # Benchmark: mmap load time for various graph sizes
│   │   ├── query_path.rs         # Benchmark: cx_path at various hop depths
│   │   ├── query_impact.rs       # Benchmark: cx_impact at various depths
│   │   └── string_intern.rs      # Benchmark: interning throughput and lookup speed
│   └── src/
│       ├── graph/
│       │   ├── mod.rs
│       │   ├── nodes.rs          # Node types and NodeKind enum
│       │   ├── edges.rs          # Edge types and EdgeKind enum
│       │   ├── csr.rs            # CSR storage format
│       │   ├── summary.rs        # Two-level summary graph (Deployable/Resource only)
│       │   ├── kind_index.rs     # Node kind offset ranges for fast kind-filtered lookups
│       │   └── bitvec.rs         # Bitset for visited tracking
│       ├── store/
│       │   ├── mod.rs
│       │   ├── snapshot.rs       # Base graph snapshots
│       │   ├── delta.rs          # Delta-encoded branch layers
│       │   ├── mmap.rs           # Memory-mapped file access, header validation, section casting
│       │   ├── writer.rs         # Graph file writer (header + page-aligned sections)
│       │   └── incremental.rs    # Incremental update pipeline (debounce, per-file delta, overlay)
│       ├── query/
│       │   ├── mod.rs
│       │   ├── bfs.rs            # BfsState double-buffer traversal engine
│       │   ├── path.rs           # PathFinder: trace request flows (cx trace)
│       │   ├── depends.rs        # Dependency sets (upstream/downstream)
│       │   └── trigram.rs        # Trigram index for fuzzy matching
│       ├── git/
│       │   ├── mod.rs
│       │   ├── refs.rs           # Branch/tag/commit resolution
│       │   └── history.rs        # Walking git history for temporal queries
│       └── config.rs             # .cx/config.toml parsing
├── cx-extractors/                # Pluggable extractors
│   ├── Cargo.toml
│   └── src/
│       ├── mod.rs                # Extractor trait definition
│       ├── treesitter.rs         # Generic tree-sitter symbol extraction
│       ├── proto.rs              # Proto file parser
│       ├── grpc_client.rs        # gRPC client stub detection (per-language)
│       ├── grpc_server.rs        # gRPC server registration detection (per-language)
│       ├── envvar.rs             # Environment variable read detection
│       ├── helm.rs               # Helm values/templates parser
│       ├── k8s.rs                # Kubernetes manifest parser
│       ├── dockerfile.rs         # Dockerfile parser
│       ├── rest.rs               # REST/HTTP client detection
│       ├── openapi.rs            # OpenAPI spec parser
│       ├── messagequeue.rs       # Kafka/NATS/SQS publish/subscribe detection
│       └── database.rs           # Database connection and ORM detection
├── cx-resolution/                # Cross-repo edge resolution engine
│   ├── Cargo.toml
│   └── src/
│       ├── mod.rs
│       ├── resolver.rs           # Core resolution algorithm
│       ├── proto_matching.rs     # Match proto clients to servers across repos
│       ├── envvar_resolution.rs  # Resolve env vars through Helm/k8s to services
│       ├── k8s_dns.rs            # Parse Kubernetes DNS patterns
│       └── discovery.rs          # GitHub API auto-discovery of related repos
└── cx-languages/                 # Language-specific tree-sitter configurations
    ├── Cargo.toml
    └── src/
        ├── mod.rs
        ├── go.rs                 # Go-specific patterns (main detection, grpc patterns)
        ├── typescript.rs         # TypeScript-specific patterns
        └── python.rs             # Python-specific patterns
```

### Key Dependencies

```toml
[workspace.dependencies]
# Parsing
tree-sitter = "0.24"
tree-sitter-go = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-python = "0.23"

# CLI & serialization
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
thiserror = "2"           # typed errors for library crates
anyhow = "1"              # ergonomic error chains for CLI binary
crossbeam-channel = "0.5" # for file watcher debounce

# PERFORMANCE-CRITICAL: memory-mapped I/O for zero-copy graph access
memmap2 = "0.9"

# PERFORMANCE-CRITICAL: madvise hints for mmap access patterns (unix only)
libc = "0.2"

# PERFORMANCE-CRITICAL: fast hash map for string interning (2-3x faster than std HashMap)
rustc-hash = "2"          # provides FxHashMap / FxHashSet

# PERFORMANCE-CRITICAL: parallel file indexing
rayon = "1.10"

# PERFORMANCE-CRITICAL: small-vec optimization for edge metadata (avoids heap alloc for ≤4 items)
smallvec = "1.13"

# PERFORMANCE-CRITICAL: concurrent hash map for parallel string interning during indexing
dashmap = "6"

# Filesystem & git
notify = "7"              # file system watcher
gix = "0.68"              # pure-Rust git implementation
ignore = "0.4"            # PERFORMANCE-CRITICAL: parallel directory walking with .gitignore (same as ripgrep)
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
glob = "0.3"

[workspace.dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }  # statistical benchmarking

[profile.release]
opt-level = 3
lto = "fat"               # link-time optimization across all crates — slower build, faster binary
codegen-units = 1          # single codegen unit — allows maximum optimization
strip = true               # strip debug symbols from release binary
panic = "abort"            # abort on panic — smaller binary, no unwinding overhead
target-cpu = "native"      # optimize for the build machine's CPU (distributable builds should remove this)
```

## Implementation Milestones

Each milestone has a clear definition of done: a set of commands that must work, test cases that must pass, and performance benchmarks that must be met. A milestone is not complete until ALL of its tests and benchmarks pass.

### Milestone 1: Core Graph Engine

**What to build:** The CSR graph data structure, string interner, memory-mapped storage, and BFS/DFS traversal engine. No parsing, no extractors — just the graph engine itself with synthetic test data.

**Deliverables:**
- `cx-core` crate with `CsrGraph`, `Node`, `Edge`, `StringInterner`, `BitVec`, `QueryContext`
- Graph builder: takes a `Vec<Node>` and `Vec<(NodeId, NodeId, EdgeKind)>`, produces a `CsrGraph`
- Mmap serialization: write `CsrGraph` to disk, load it back with zero-copy mmap
- `traverse_bfs` and `traverse_dfs` functions with edge-kind bitmask filtering
- Reverse edge index for upstream queries

**Test cases:**

```
TEST graph_roundtrip:
  Build a graph with 100 nodes and 500 edges programmatically.
  Write to disk. Mmap load. Verify every node and edge matches.
  PASS: byte-for-byte identical graph after roundtrip.

TEST graph_traversal_downstream:
  Build graph: A→B→C→D, A→E→F, B→G
  traverse_bfs(seed=A, direction=downstream, mask=ALL_EDGES)
  PASS: result contains {A,B,C,D,E,F,G} in BFS order.

TEST graph_traversal_upstream:
  Same graph. traverse_bfs(seed=D, direction=upstream, mask=ALL_EDGES)
  PASS: result contains {D,C,B,A}.

TEST graph_edge_filtering:
  Build graph with mixed edge kinds: A→B (Calls), A→C (DependsOn), B→D (Calls)
  traverse_bfs(seed=A, mask=CODE_EDGES)  // only Calls
  PASS: result contains {A,B,D}. C is excluded.

TEST graph_cross_repo_filtering:
  Build graph with nodes in repo_1 and repo_2.
  Query: "all nodes reachable from A that cross a repo boundary"
  PASS: result includes only paths where edge.flags & IS_CROSS_REPO.

TEST string_interner_dedup:
  Intern "foo" twice. Get same StringId both times.
  Intern 100K unique strings. Verify all retrievable by ID.
  PASS: no duplicate IDs, all strings round-trip correctly.

TEST string_interner_roundtrip:
  Intern 100K strings. Serialize to packed format. Reload.
  PASS: all 100K strings retrievable by original ID.

TEST bitvec_correctness:
  Create BitVec for 1M nodes. Set nodes at positions 0, 1, 63, 64, 65, 999999.
  PASS: test() returns true for set positions, false for all others.

TEST bitvec_clear_reuse:
  Set 10K nodes. Clear. Verify all test() return false.
  PASS: no stale bits after clear.

TEST edge_sorting_within_node:
  Node A has 10 edges: 3 Calls, 2 Imports, 3 DependsOn, 2 Exposes.
  Build graph. Verify edges in A's range are sorted by kind.
  PASS: edges appear in order: Calls, Calls, Calls, Imports, Imports, DependsOn, ...

TEST kind_index:
  Build graph: 50 Symbol nodes, 10 Endpoint nodes, 5 Deployable nodes.
  Use KindIndex to find all Endpoints.
  PASS: kind_ranges returns exactly the 10 Endpoint nodes.
  PASS: zero scanning of Symbol or Deployable nodes.

TEST summary_graph_construction:
  Full graph: 3 Deployables (A, B, C) each with 100 symbols.
  Symbols in A call symbols in B (gRPC). Symbols in B call symbols in C (gRPC).
  Build summary graph.
  PASS: summary has 3 nodes (A, B, C) and 2 edges (A→B, B→C).
  PASS: summary edges aggregate all symbol-level gRPC calls into single DependsOn.

TEST summary_graph_query:
  cx_depends on summary graph: A downstream.
  PASS: returns {B, C} using only summary graph (not full graph).

TEST bfs_double_buffer:
  Build graph: A→B→C→D→E (linear chain).
  Run BfsState::run(seed=A, max_depth=3).
  PASS: result contains {A, B, C, D}. E excluded (depth 4).
  PASS: no heap allocations during run (verified by pre-allocating with sufficient capacity).

TEST mmap_advise_no_crash:
  Mmap a graph file. Call advise_mmap(). Run queries.
  PASS: no segfaults, no errors. (Correctness test — madvise is a hint, not a guarantee.)

TEST file_header_validation:
  Create a valid .cxgraph file. Verify it loads.
  Corrupt the magic bytes. Verify CxError::CorruptGraph returned.
  Set version to 999. Verify CxError::VersionMismatch returned.
  Truncate the file mid-nodes-section. Verify CxError::CorruptGraph returned.
  PASS: all corruption cases detected, none cause panics or segfaults.

TEST file_header_checksum:
  Create valid .cxgraph. Flip one byte in the nodes section. Reload.
  PASS: CRC32 checksum in header detects the corruption.

TEST trigram_index_build:
  Build trigram index over 10K symbol names.
  PASS: every 3-character substring maps to the correct StringIds.

TEST trigram_search_exact:
  Index contains "handleAudioStream", "handleVideoStream", "processAudio".
  Search for "handleAudio".
  PASS: "handleAudioStream" ranked first (exact substring match).
  PASS: "processAudio" ranked lower (partial match on "Audio").
  PASS: "handleVideoStream" ranked lowest (matches "handle" but not "Audio").

TEST trigram_search_case_insensitive:
  Index contains "StreamingRecognize".
  Search for "streamingrecognize" (lowercase).
  PASS: "StreamingRecognize" found.

TEST trigram_search_no_results:
  Search for "xyzzyplugh" (no matches).
  PASS: empty result set, no errors.
```

**Performance benchmarks (criterion):**

```
BENCH graph_load_mmap:
  Generate graph: 100K nodes, 1M edges. Write to disk. Mmap load.
  TARGET: < 10ms for load (excluding OS page-in; measure wall clock of mmap + header verify).

BENCH graph_load_mmap_large:
  Generate graph: 1M nodes, 10M edges (~350MB file). Write to disk. Mmap load.
  TARGET: < 50ms.

BENCH bfs_100k_nodes:
  Random graph: 100K nodes, 1M edges. BFS from random seed, depth unlimited.
  TARGET: < 1ms (median over 1000 runs).

BENCH bfs_1m_nodes:
  Random graph: 1M nodes, 10M edges. BFS from random seed, depth 5.
  TARGET: < 5ms (median).

BENCH bfs_filtered:
  Random graph: 1M nodes, 10M edges (mixed edge kinds). BFS with SERVICE_EDGES mask.
  TARGET: < 5ms (median). Verify bitmask filtering has <5% overhead vs unfiltered.

BENCH string_intern_throughput:
  Intern 1M unique strings (average 30 chars each).
  TARGET: > 2M interns/second.

BENCH string_lookup_throughput:
  Lookup 1M StringIds in packed interner.
  TARGET: > 10M lookups/second.

BENCH bitvec_set_test:
  Set and test 1M nodes in sequence.
  TARGET: > 100M operations/second.

BENCH bfs_double_buffer_vs_vecdeque:
  Same graph, same query. Compare BfsState (double-buffer) vs VecDeque-based BFS.
  TARGET: double-buffer is at least 20% faster.

BENCH summary_graph_query:
  Summary graph: 200 Deployable nodes, 500 edges. BFS depth 3.
  TARGET: < 10μs (microseconds). This must be near-instant.

BENCH kind_index_lookup:
  Graph: 1M nodes. Find all Endpoints (expect ~5K).
  TARGET: < 1μs for the index lookup (just reading two u32 values).

BENCH edge_sorted_filter_vs_unsorted:
  Node with 200 edges. Filter for SERVICE_EDGES only (expect ~30 edges).
  Compare sorted (binary search to range) vs unsorted (linear scan with bitmask).
  TARGET: sorted is at least 30% faster when <20% of edges match filter.

BENCH mmap_cold_start:
  Mmap graph file (100MB). Run first query (pages faulted in by OS).
  TARGET: < 200ms for cold start (dominated by OS page-in, not our code).
  NOTE: subsequent queries on the same graph are < 5ms because pages are warm.
```

### Milestone 2: Tree-Sitter Indexing Pipeline

**What to build:** Parallel file parsing with tree-sitter, the Extractor trait, and the Go language extractor. `cx build` indexes a single Go repo and builds the CSR graph.

**Deliverables:**
- `cx-extractors` crate with `Extractor` trait
- `TreeSitterExtractor` for Go: extracts functions, methods, types, constants, call sites, imports
- Parallel indexing pipeline: `ignore` crate for directory walking → rayon parallel parse → merge → CSR build
- Deployable auto-detection: find `main` packages in Go, associate with Dockerfiles
- `cx-cli` crate: `cx build` command
- `cx network` command: prints detected deployables, endpoints, symbols, modules

**Test cases:**

```
TEST parse_go_functions:
  Input: Go file with 5 functions, 2 methods, 1 interface.
  Run TreeSitterExtractor.
  PASS: ExtractionResult contains 5 Function nodes, 2 Method nodes, 1 Type node.
  PASS: all have correct name (StringId), file, and line number.

TEST parse_go_calls:
  Input: Go file where funcA calls funcB and funcC.
  PASS: ExtractionResult contains 2 Calls edges: funcA→funcB, funcA→funcC.

TEST parse_go_imports:
  Input: Go file importing "fmt", "github.com/acme-corp/acme-platform/pkg/orders".
  PASS: ExtractionResult contains 2 Imports edges from this module to the imported packages.

TEST detect_go_main:
  Input: Go repo with cmd/server/main.go and cmd/migrate/main.go.
  PASS: 2 Deployable nodes detected with correct entry_point paths.

TEST detect_grpc_server_registration:
  Input: Go file containing pb.RegisterTranslationServiceServer(s, &handler{}).
  PASS: 1 Endpoint node: kind=gRPC, name="TranslationService", with Exposes edge from deployable.

TEST detect_websocket_handler:
  Input: Go file with http.HandleFunc("/ws/translate", wsHandler).
  PASS: 1 Endpoint node: kind=WebSocket, path="/ws/translate".

TEST detect_env_var_reads:
  Input: Go file with os.Getenv("ORDER_SERVICE_ADDR") and os.Getenv("REDIS_URL").
  PASS: 2 dangling edges with env var names as unresolved references.

TEST cx_init_empty_dir:
  Run `cx build` in an empty directory.
  PASS: exits with error message "no source files found", non-zero exit code.

TEST cx_init_go_repo:
  Run `cx build` in a Go repo with 10 files, 50 functions, 1 main package.
  Run `cx network`.
  PASS: output lists 1 deployable, correct function count, correct module structure.

TEST parallel_correctness:
  Index a repo with 100 Go files using 1 thread and N threads.
  PASS: identical graph (same nodes, same edges, same StringIds) regardless of thread count.
```

**Performance benchmarks:**

```
BENCH index_small_repo:
  Go repo: 50 files, 5K LOC.
  TARGET: `cx build` completes in < 500ms (including graph build and disk write).

BENCH index_medium_repo:
  Go repo: 500 files, 50K LOC.
  TARGET: `cx build` completes in < 2s.

BENCH index_large_repo:
  Go repo: 5000 files, 500K LOC.
  TARGET: `cx build` completes in < 10s.

BENCH parse_throughput:
  Parse Go files with tree-sitter in parallel.
  TARGET: > 100K LOC/second on a 4-core machine.

BENCH extractor_throughput:
  Run Go extractor on parsed tree-sitter output.
  TARGET: > 200K LOC/second (extractors should be faster than parsing).

BENCH cx_context_latency:
  Index a 50K LOC repo. Run `cx network`.
  TARGET: < 5ms from command invocation to output (graph is already mmap'd).

BENCH cx_search_latency:
  Index a 50K LOC repo with ~5K symbols. Run `cx search "handleAudio"`.
  TARGET: < 10ms to return ranked results.

BENCH trigram_index_build:
  Build trigram index over 100K symbol names.
  TARGET: < 500ms.

BENCH trigram_search:
  Trigram index over 100K names. Search for a 10-character query.
  TARGET: < 2ms to return top 20 results.

BENCH index_scaling:
  Same 50K LOC Go repo. Run with RAYON_NUM_THREADS=1, 2, 4, 8, 16.
  Report throughput (LOC/sec) at each thread count.
  This tells you the actual parallelism efficiency and where it plateaus.
```

### Milestone 3: Cross-Repo Resolution and Path Tracing

**What to build:** Proto extractor, gRPC client/server extractors, cross-repo resolution engine, `cx add`, `cx trace`, `cx trace`, and the MCP server.

**Deliverables:**
- `ProtoExtractor`: parses `.proto` files, extracts service definitions, RPC methods, message types
- `GrpcClientExtractor` for Go: detects `pb.New{Service}Client()` calls
- `GrpcServerExtractor` for Go: detects `pb.Register{Service}Server()` calls
- `cx-resolution` crate: matches dangling proto edges across repos
- Multi-repo graph: single CSR graph spanning multiple repos
- `cx add <path>` command
- `cx trace` command with `--from` and `--downstream`/`--upstream`
- `cx trace` command
- `cx-cli/mcp`: MCP server exposing cx_path, cx_network, cx_diff, cx_explain over JSON-RPC stdio

**Test cases:**

```
TEST proto_extraction:
  Input: .proto file defining service OrderProcessing with 3 RPC methods.
  PASS: 1 Surface node (proto package), 3 Endpoint nodes (RPC methods).

TEST grpc_client_detection_go:
  Input: Go file with `conn, _ := grpc.Dial(addr, opts...)` and
         `client := pb.NewOrderProcessingClient(conn)` and
         `client.StreamingRecognize(ctx)`.
  PASS: dangling edge to OrderProcessing.StreamingRecognize.

TEST grpc_server_detection_go:
  Input: Go file with `pb.RegisterOrderProcessingServer(s, &handler{})`.
  PASS: Exposes edge from deployable to OrderProcessing endpoint.

TEST cross_repo_proto_resolution:
  Repo A: Go code calling NewOrderProcessingClient().
  Repo B: Go code calling RegisterOrderProcessingServer(). Same proto.
  cx build on A, cx add B.
  PASS: DependsOn edge from A's deployable to B's deployable, confidence ≥ 0.9.
  PASS: edge provenance chain: [client_stub, proto_match, server_registration].

TEST cross_repo_proto_mismatch:
  Repo A's proto has 6 fields in StreamingRecognizeRequest.
  Repo B's proto has 7 fields.
  PASS: edge still created but with a warning in metadata: "proto field count mismatch".

TEST cx_trace_downstream:
  3 repos: service-a → service-b → (terminal).
  service-a has WS endpoint /ws/stream and gRPC client to service-b.
  service-b has gRPC server registration.
  cx trace handleWebSocket --downstream
  PASS: output shows: handleWebSocket → grpc → service-b.StreamingRecognize.
  PASS: completeness = 1.0 (no gaps).

TEST cx_trace_with_gaps:
  2 repos: service-a has gRPC client to service-b AND service-c.
  Only service-b is indexed, service-c is not.
  cx trace handleWebSocket --downstream
  PASS: path to service-b is fully resolved.
  PASS: path to service-c shows dangling edge.

TEST cx_trace_upstream:
  3 repos: A depends on B, B depends on C.
  cx trace A --upstream
  PASS: returns empty (nothing depends on A).
  cx trace C --upstream
  PASS: returns B and A (transitively).

TEST cx_trace_downstream_deps:
  cx trace A --downstream
  PASS: returns B and C (transitively).

TEST mcp_server_tool_listing:
  Start MCP server. Send initialize request.
  PASS: tool list includes cx_resolve, cx_path, cx_depends, cx_context, cx_search.
  PASS: each tool has input_schema with correct parameter types.

TEST mcp_cx_path_call:
  Start MCP server with indexed repos.
  Send JSON-RPC call to cx_path with valid endpoint.
  PASS: returns valid JSON with hops, boundary_crossings, gaps, completeness.
  PASS: response time < 50ms including JSON serialization.

TEST mcp_cx_context_call:
  Send JSON-RPC call to cx_context with valid service name.
  PASS: returns deployables, endpoints, dependencies, resources, gaps.

TEST mcp_content_length_framing:
  Send a valid JSON-RPC request with correct Content-Length header.
  PASS: parsed correctly.
  Send a request with wrong Content-Length (too short). 
  PASS: server reads the partial body, returns JSON-RPC error, does NOT crash.
  Send two requests back-to-back without waiting for response.
  PASS: both processed and responded to in order.

TEST mcp_invalid_json:
  Send syntactically invalid JSON with correct Content-Length.
  PASS: server returns JSON-RPC parse error (-32700), continues accepting requests.

TEST mcp_unknown_tool:
  Send tools/call with name "nonexistent_tool".
  PASS: server returns JSON-RPC method not found error, continues running.

TEST mcp_query_after_error:
  Send an invalid request (triggers error response), then send a valid cx_path request.
  PASS: valid request succeeds — errors don't corrupt server state.

TEST extractor_parse_failure_nonfatal:
  Repo with 10 Go files. File 5 has syntax errors (invalid Go).
  cx build.
  PASS: other 9 files indexed successfully.
  PASS: cx status reports 1 parse error with file name and reason.
  PASS: graph contains symbols from the 9 good files.
```

**Performance benchmarks:**

```
BENCH proto_parse_throughput:
  Parse 100 .proto files (average 200 lines each).
  TARGET: < 200ms total.

BENCH resolution_pass:
  5 repos, 500 dangling edges to resolve.
  TARGET: resolution pass completes in < 100ms.

BENCH cx_path_5_hops:
  Graph with 100K nodes across 5 repos. Path query spanning 5 service boundaries.
  TARGET: < 2ms (excluding JSON serialization).

BENCH cx_path_10_hops:
  Graph with 1M nodes across 20 repos. Path query spanning 10 service boundaries.
  TARGET: < 10ms.

BENCH cx_depends_depth3:
  Graph with 100K nodes. Transitive dependency query, depth 3.
  TARGET: < 2ms.

BENCH mcp_roundtrip:
  Full MCP JSON-RPC roundtrip: parse request → execute query → serialize response.
  TARGET: < 10ms for cx_path on 100K node graph.

BENCH multi_repo_index:
  5 Go repos, 200K total LOC. cx build + cx add for all 5.
  TARGET: < 10s total including resolution pass.
```

### Milestone 4: Infrastructure Resolution and Impact Analysis

**What to build:** Env var extractor, Helm values extractor, Kubernetes manifest extractor, `cx impact`, `cx_context` enriched with config info, and GitHub org auto-discovery.

**Deliverables:**
- `HelmValuesExtractor`: parses `values.yaml` and deployment templates
- `K8sManifestExtractor`: parses Kubernetes YAML, extracts service names, configmaps
- Env var resolution engine: matches `os.Getenv("X")` in code to Helm env definitions
- Kubernetes DNS resolver: parses `{service}.{namespace}.svc.cluster.local` patterns
- `cx impact` command and `cx_impact` MCP tool
- `cx_context` now includes resolved config values and infrastructure info
- GitHub org auto-discovery: search org for repos matching dangling proto references
- `cx add --role infra` for infrastructure repos

**Test cases:**

```
TEST helm_values_extraction:
  Input: Helm values.yaml with 5 env var definitions (3 static values, 2 Go templates).
  PASS: 5 config nodes extracted.
  PASS: 3 have resolved values, 2 marked as template-resolved with lower confidence.

TEST helm_template_resolution:
  Input: values.yaml with `value: "{{ .Values.orders.host }}:{{ .Values.orders.port }}"`.
  defaults in values.yaml: orders.host = "acme-orders.prod.svc.cluster.local", orders.port = "50051".
  PASS: resolves to "acme-orders.prod.svc.cluster.local:50051".

TEST helm_valuefrom_configmap:
  Input: deployment.yaml with `valueFrom: configMapKeyRef: name: service-endpoints`.
  PASS: dangling reference flagged: "ORDER_SERVICE_ADDR populated from configmap, not resolved."

TEST k8s_dns_resolution:
  Value: "acme-orders.prod.svc.cluster.local:50051"
  Indexed repo deploys as service name "acme-orders" (from its Helm chart metadata).
  PASS: Resolves edge from env var → k8s DNS → service name → indexed repo.

TEST envvar_to_service_resolution:
  gateway code: os.Getenv("ORDER_SERVICE_ADDR")
  Helm chart: ORDER_SERVICE_ADDR = "acme-orders.prod.svc.cluster.local:50051"
  acme-orders repo indexed.
  PASS: Full resolution chain: code → env var → helm value → k8s DNS → repo.
  PASS: Resolves edge with confidence ≥ 0.7.
  PASS: provenance chain has 4 steps.

TEST envvar_missing_from_helm:
  Code reads os.Getenv("NEW_FEATURE_FLAG").
  Helm charts have no definition for NEW_FEATURE_FLAG.
  PASS: gap reported: "NEW_FEATURE_FLAG read in code but not defined in any indexed infrastructure config."

TEST cx_impact_symbol:
  Graph: funcA in service-1 calls gRPC to service-2.funcB. funcB calls funcC. funcC writes to postgres.
  cx impact funcA
  PASS: affected_symbols includes funcA, funcB, funcC.
  PASS: affected_services includes service-1, service-2.
  PASS: affected_resources includes postgres.

TEST cx_impact_with_config:
  Changing a proto field used by service-1 and service-2.
  Helm charts configure SERVICE_ADDR env vars for both.
  cx impact proto_field
  PASS: affected_services lists both services.
  PASS: config_dependencies lists the Helm values that would need updating.

TEST cx_impact_library_cross_repo:
  pkg/codec in repo A is imported by repo B.
  cx impact codec.Decode
  PASS: affected spans both repo A (direct callers) and repo B (cross-repo importers).
  PASS: warns about independent deploy risk (version skew possible).

TEST auto_discovery_github:
  Mock GitHub API. Repo has dangling proto edge to "OrderProcessing" service.
  Same org has repo "acme-orders" containing a proto defining OrderProcessing.
  PASS: auto-discovery finds acme-orders and suggests indexing it.
  PASS: does NOT index without user confirmation (unless --yes flag).
```

**Performance benchmarks:**

```
BENCH helm_parse_throughput:
  Parse 50 Helm values.yaml and deployment.yaml files.
  TARGET: < 500ms total.

BENCH envvar_resolution:
  Resolve 200 env vars against Helm chart definitions and k8s DNS patterns.
  TARGET: < 50ms.

BENCH cx_impact_depth5:
  Graph: 100K nodes, 1M edges. Impact query from a symbol, depth 5.
  TARGET: < 5ms.

BENCH cx_impact_depth5_large:
  Graph: 1M nodes, 10M edges. Impact query, depth 5.
  TARGET: < 20ms.

BENCH cx_context_with_config:
  Full cx_context output including resolved env vars and infrastructure.
  TARGET: < 10ms (graph query) + < 5ms (JSON serialization).
```

### Milestone 5: Git-Native Versioning and Diffs

**What to build:** Commit-based graph snapshots, delta encoding for branches, `cx diff`, `cx log`, `cx blame`, file watcher for incremental updates, CI output format.

**Deliverables:**
- Graph snapshots keyed by `(RepoId, CommitSha)`
- Delta encoding: `GraphDelta` struct representing added/removed nodes and edges
- Delta overlay: query engine applies delta on top of base snapshot at query time
- `cx diff <ref-a> <ref-b>` command and MCP tool
- `cx log <symbol>` command: walk git history to find when an edge appeared/disappeared
- `cx blame <edge>` command: find the commit that introduced an edge
- File watcher: `notify` crate watches workspace, triggers incremental re-index on file change
- CI output: `cx diff --format github-comment` produces markdown for PR comments

**Test cases:**

```
TEST delta_encoding:
  Base graph: 100 nodes, 500 edges.
  Delta: +3 nodes, +5 edges, -2 edges.
  Apply delta to base. Verify resulting graph has 103 nodes, 503 edges.
  PASS: all original nodes present. New nodes present. Removed edges absent.

TEST delta_query_overlay:
  Base graph on main: A→B→C.
  Branch delta: adds edge B→D (new node D, new edge).
  Query cx_path on main: A reaches {B, C}.
  Query cx_path on branch: A reaches {B, C, D}.
  PASS: branch query includes D without modifying the base graph.

TEST cx_diff_added_dependency:
  main: service-A depends on service-B.
  feature/x: service-A depends on service-B AND service-C (new dependency).
  cx diff main feature/x
  PASS: output includes "added edge: service-A → service-C (gRPC)".
  PASS: output includes "added node: service-C endpoint".

TEST cx_diff_removed_dependency:
  main: service-A depends on service-B and service-C.
  feature/x: service-A depends on service-B only (removed service-C).
  cx diff main feature/x
  PASS: output includes "removed edge: service-A → service-C".

TEST cx_diff_config_gap:
  feature/x adds os.Getenv("NEW_CACHE_ADDR") in code.
  Helm charts on feature/x do NOT define NEW_CACHE_ADDR.
  cx diff main feature/x
  PASS: output includes config warning: "NEW_CACHE_ADDR read in code but not in helm charts."

TEST cx_diff_changed_path:
  main: /ws/translate → audio_router → asr (direct).
  feature/x: /ws/translate → audio_router → cache → asr (added cache hop).
  cx diff main feature/x
  PASS: changed_paths shows the before/after for /ws/translate route.

TEST cx_log_edge:
  Service-A → Service-B edge was introduced in commit abc123.
  cx log "service-A → service-B"
  PASS: returns commit abc123, author, date, PR URL (if extractable from commit message).

TEST cx_blame_edge:
  cx blame "service-A → service-B"
  PASS: returns the commit that first introduced this edge.

TEST incremental_update:
  Index repo. Modify one file (add a new function). File watcher triggers re-index.
  PASS: new function appears in graph within 1 second of file save.
  PASS: no full re-index — only the changed file is re-parsed.
  PASS: graph node count increased by exactly the number of new symbols.

TEST ci_output_format:
  cx diff main feature/x --format github-comment
  PASS: output is valid markdown.
  PASS: includes "## cx structural impact" header.
  PASS: lists added/removed dependencies, changed paths, config warnings.
```

**Performance benchmarks:**

```
BENCH delta_apply:
  Base graph: 1M nodes. Delta: +1000 nodes, +5000 edges, -500 edges.
  Apply delta and query.
  TARGET: delta overlay adds < 1ms to query time.

BENCH cx_diff:
  Two graph snapshots, each 100K nodes. ~500 node/edge differences.
  TARGET: < 20ms for full structural diff.

BENCH cx_diff_large:
  Two graph snapshots, each 1M nodes. ~5000 differences.
  TARGET: < 100ms.

BENCH incremental_reindex:
  Indexed repo with 500 files. Change 1 file (add 5 functions).
  TARGET: incremental update completes in < 200ms (re-parse + delta + merge).

BENCH incremental_reindex_batch:
  Change 10 files simultaneously (simulating a git checkout).
  TARGET: < 1s for batch incremental update.

BENCH cx_log_walk:
  Walk 100 commits of history checking for edge presence.
  TARGET: < 500ms (this is the slowest acceptable query — git history walking is inherently I/O bound).
```

### Milestone 6: Multi-Language and Extended Protocols

**What to build:** TypeScript and Python tree-sitter extractors, REST/HTTP extractor, message queue extractor, database extractor, OpenAPI extractor.

**Deliverables:**
- TypeScript `TreeSitterExtractor`: functions, classes, methods, imports, call sites
- Python `TreeSitterExtractor`: functions, classes, methods, imports, call sites
- `RestClientExtractor`: detects `fetch()`, `axios`, `http.Post()`, etc. across languages
- `OpenApiExtractor`: parses OpenAPI/Swagger specs
- `MessageQueueExtractor`: detects Kafka/NATS/SQS publish/subscribe patterns
- `DatabaseExtractor`: detects `sql.Open()`, ORM model definitions, connection strings

**Test cases:**

```
TEST parse_typescript_functions:
  Input: TypeScript file with 5 functions, 2 classes, 3 arrow functions.
  PASS: correct symbol nodes with accurate line numbers.

TEST parse_typescript_imports:
  Input: TypeScript file with import statements (named, default, dynamic).
  PASS: Import edges for each imported module.

TEST parse_python_functions:
  Input: Python file with 5 functions, 2 classes, decorators.
  PASS: correct symbol nodes. Decorators captured as metadata.

TEST parse_python_imports:
  Input: Python file with `import x`, `from x import y`, `from x import *`.
  PASS: Import edges for each.

TEST rest_client_detection_go:
  Input: Go file with `http.Post(os.Getenv("WEBHOOK_URL") + "/api/v1/events", ...)`.
  PASS: dangling edge with partial URL pattern "/api/v1/events" and env var reference.

TEST rest_client_detection_typescript:
  Input: TypeScript with `await fetch(\`\${process.env.API_URL}/users\`)`.
  PASS: dangling edge with partial URL pattern "/users" and env var reference.

TEST openapi_extraction:
  Input: OpenAPI 3.0 spec with 10 endpoints.
  PASS: 10 Endpoint nodes with correct HTTP methods and paths.

TEST openapi_cross_reference:
  Repo A has REST client calling /api/v1/users.
  Repo B has OpenAPI spec defining /api/v1/users.
  PASS: edge resolved between A and B with confidence ≥ 0.8.

TEST kafka_publish_detect:
  Input: Go file with `producer.Publish("translation.session.started", event)`.
  PASS: Publishes edge with topic "translation.session.started".

TEST kafka_subscribe_detect:
  Input: Go file with `consumer.Subscribe("translation.session.started", handler)`.
  PASS: Subscribes edge with topic "translation.session.started".

TEST kafka_cross_repo_resolution:
  Repo A publishes to "translation.session.started".
  Repo B subscribes to "translation.session.started".
  PASS: async DependsOn edge from A to B via message queue. Confidence ≥ 0.85.

TEST database_connection_detect:
  Input: Go file with `sql.Open("postgres", os.Getenv("DATABASE_URL"))`.
  PASS: Connects edge to a Resource node of kind Postgres.

TEST shared_database_detection:
  Repo A and Repo B both connect to DATABASE_URL.
  Helm charts resolve both to the same Postgres instance.
  PASS: both repos have Connects edges to the same Resource node.
  PASS: cx_context reports "shared database dependency" warning.

TEST mixed_language_graph:
  Repo A (Go) calls gRPC to Repo B (Python). Proto shared.
  PASS: cross-repo edge resolves correctly across languages.
  PASS: cx_path works seamlessly through the Go→Python boundary.
```

**Performance benchmarks:**

```
BENCH parse_typescript_throughput:
  500 TypeScript files, 50K LOC total.
  TARGET: < 3s for full index.

BENCH parse_python_throughput:
  500 Python files, 50K LOC total.
  TARGET: < 3s for full index.

BENCH mixed_language_index:
  3 repos: 1 Go (50K LOC), 1 TypeScript (30K LOC), 1 Python (20K LOC).
  TARGET: < 8s for full index including cross-repo resolution.

BENCH openapi_parse:
  10 OpenAPI specs, average 500 lines each.
  TARGET: < 300ms.

BENCH full_system_index:
  10 repos, 3 languages, 500K total LOC, Helm charts, proto files.
  TARGET: < 30s for full index with all extractors and resolution.
```

## Why not existing tools?

### Why not LOOM (loomai.io)?

Web-based SaaS with 3D visualization. Currently JS/TS/Python only. No cross-repo resolution, no Helm/proto/env var tracing, no git-native versioning, no MCP server, no CLI. Visualization-first approach vs. CX's query-first approach.

### Why not Sourcegraph?

Strong text search and basic cross-reference, but doesn't understand service topology — can't trace through gRPC boundaries, resolve env vars, or show infrastructure dependencies. Server-based (requires deployment). No MCP integration.

### Why not GitHub Code Search?

Good text search with basic symbol navigation within a single repo. No cross-repo structural understanding. No service topology awareness.

### Why not IDE LSPs?

Break at repository boundaries. Can't trace cross-service calls. No infrastructure awareness. No git-native temporal queries.

### What CX does differently

CX is designed to:
1. Build a unified graph across repos, languages, and infrastructure
2. Resolve cross-service edges through proto matching and env var tracing
3. Be git-native with branch diffing and temporal queries
4. Provide an MCP interface for direct AI agent integration
5. Run as a local binary with sub-10ms query latency
6. Be honest about gaps with explicit completeness scoring

## Performance Anti-Patterns (DO NOT DO THESE)

These are common patterns that Claude Code or any Rust developer might reach for. They are all wrong for this project. This list exists to prevent performance regressions.

**DO NOT use `HashMap<NodeId, T>` for anything in the query path.** Use arrays indexed by `NodeId` directly, or `BitVec` for sets. HashMap has per-lookup overhead from hashing and pointer chasing that destroys cache performance.

**DO NOT use `serde_json::Value` for internal data.** JSON serialization/deserialization only happens at the MCP boundary (input/output). All internal representations use typed Rust structs.

**DO NOT use `String` in `Node` or `Edge` structs.** Use `StringId` (u32). Strings are only resolved to `&str` when producing output for the user.

**DO NOT use `Vec<Edge>` per node** (adjacency list). Use CSR format. An adjacency list of 1M nodes with individual `Vec`s means 1M heap allocations and terrible cache behavior.

**DO NOT use `Rc`, `Arc`, `Box` in the graph.** The graph is a flat array of `Copy` types. There is no shared ownership. There is no heap indirection.

**DO NOT use `async` for query execution.** Queries complete in microseconds. The overhead of async task scheduling (even with tokio) is larger than the query itself. Use synchronous code for all query paths. Only use async for I/O: GitHub API calls, file watching, MCP server I/O.

**DO NOT use `petgraph` or any generic graph library.** They use adjacency lists internally and can't match CSR performance. Build the graph data structure from scratch — it's ~200 lines of code.

**DO NOT parse files with regex.** Use tree-sitter for source code (zero-copy, incremental, correct). Use dedicated parsers for structured formats (proto, YAML, TOML). Regex is both slower and less correct for structured parsing.

**DO NOT use `std::fs::read_to_string()` in a loop for batch file reading during indexing.** Use the `ignore` crate for parallel directory walking and `memmap2` for large files. For small files (<64KB), `read_to_string` is fine.

**DO NOT sort the entire graph on every incremental update.** Incremental updates should produce a delta that gets merged during the next compaction, not trigger a full rebuild. During query time, overlay the delta on the base graph.

**DO NOT use `println!` for user-facing output in performance-sensitive paths.** Use `std::io::BufWriter` wrapping stdout. Each `println!` flushes and does a syscall.

**DO NOT use `VecDeque` for BFS traversal.** Use the double-buffer `BfsState` pattern (two `Vec`s that swap per level). VecDeque has ring-buffer wrap-around overhead and worse cache behavior.

**DO NOT use `serde_json::Value` as an intermediate representation.** Serialize query results directly to a `Vec<u8>` buffer using `serde_json::to_writer`. Building a `Value` tree allocates hundreds of heap objects for a typical response.

**DO NOT use `clone()` on graph data during queries.** All graph data is `Copy` (fixed-size structs) or borrowed (`&[Node]`, `&[Edge]`). If you find yourself cloning, the data model is wrong.

**DO NOT scan all nodes to find nodes of a specific kind.** Use the `KindIndex` (see Graph Storage) which stores the offset range for each NodeKind. Finding all Endpoints is a slice operation, not a linear scan.

**DO NOT use `BTreeMap` for ordered output.** If results need to be sorted (e.g., by confidence), collect into a `Vec` and `sort_unstable_by_key`. BTreeMap allocates per-insert and has poor cache behavior.

## Key Risks and Mitigations

### Risk: Extractor accuracy

If the graph has wrong edges or missing connections, developers lose trust immediately. The bar is high because the value prop is "deterministic and complete, unlike the LLM guessing."

**Mitigation:** Start with gRPC/proto extraction (highest confidence). Test against real-world repos with known dependency graphs. Aggressive confidence scoring. Better to show a gap than a wrong answer.

### Risk: Setup friction

If onboarding takes more than 5 minutes, developers won't bother. "Just paste files into Claude" has zero setup cost.

**Mitigation:** Progressive disclosure. `cx build` gives value in 30 seconds (single-repo search). Auto-discovery handles cross-repo in the background. Infrastructure repo is one manual step with clear prompting.

### Risk: Context windows get cheaper and larger

If AI agents can cheaply load entire codebases into context, the structural query use case weakens.

**Mitigation:** CX and large context windows are complements, not substitutes. CX tells the agent *which* files to load (retrieval). The agent reasons over those files (inference). CX queries are also free, instant, deterministic, and exhaustive — properties that LLM inference can never guarantee. Temporal queries (branch diffs, history) are impossible with context stuffing.

### Risk: Monorepo complexity

Real repos contain multiple services, libraries, and infra configs. The "repo = service" assumption breaks.

**Mitigation:** Already addressed in the data model. Repos contain deployables, modules, surfaces, and infra configs. Auto-detection identifies boundaries. Manual refinement via `.cx/config.toml` when heuristics fail.
