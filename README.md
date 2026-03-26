# cx — Distributed Code Intelligence for Network Boundary Analysis

## Mission

cx is the **structural map** for distributed systems. It answers one question with 100% accuracy:

> **"What are every single incoming API and outgoing network call in this codebase, where does each connection target come from, and how do all these services connect to each other across repos, languages, and infrastructure?"**

cx works like git: local-first, distributed, collaborative. Each team builds their repo's graph independently. Teams add remotes to pull other repos' graphs and build the complete cross-service topology. The graph is never a static document — it's a living, compiler-derived artifact that improves with each index run and can be tuned by teams to reach 100% coverage.

cx is designed to scale to **1000+ repos** across an organization.

## The Full Resolution Chain

```
Source Code                    Infrastructure                Cross-Service
───────────                    ──────────────                ─────────────
grpc.Dial(addr)
  ↑ addr = mustMapEnv("X")     K8s frontend.yaml:
  ↑ os.Getenv("X")         →     PRODUCT_CATALOG_SERVICE_ADDR
                                  = "productcatalog:3550"  →  productcatalog service
                                                               ↓
                                                            pb.RegisterProductCatalogServer()
                                                              (Go, src/productcatalogservice/)
```

cx traces this entire chain: **code → env var → K8s value → DNS name → target service → exposed API.**

## cx as Lossless Context for AI Agents

*Inspired by the [Lossless Context Management](https://papers.voltropy.com/LCM) paper (Voltropy, 2026).*

The LCM paper demonstrates that LLM agents are bottlenecked by context management — efficiently presenting relevant information without losing detail. cx solves this for codebases:

- **The cx graph IS the precomputed exploration summary.** Instead of an agent spending 50k tokens grepping and reading files to understand a call chain, it queries `cx_path` and gets the complete, type-resolved answer in milliseconds. This is what LCM calls an "exploration summary" — a compact, lossless, type-aware representation.

- **Hierarchical views mirror LCM's DAG.** cx provides multiple resolution levels:
  - `cx context` → top-level service topology (LCM's "condensed summary")
  - `cx path --from A --to B` → specific call chain (LCM's "expanded detail")
  - `cx network` → all network boundaries with provenance (full resolution)
  - An agent can drill down from summary to detail without re-analyzing code.

- **Deterministic retrievability.** Every edge in the cx graph is traceable to specific source locations. An agent can always `lcm_expand` from a graph edge to the actual code. No information is lost — hence "lossless."

- **Operator-level recursion for multi-repo analysis.** When analyzing 1000 repos, the pattern is `llm_map` over repos: each repo is analyzed independently and in parallel, with cx handling the structural analysis and the agent handling reasoning. The cross-repo graph assembly is a deterministic merge, not an LLM task.

## Quick Start

```bash
cargo install cx-engine

cd ~/code/my-service
cx init
```

cx indexes your repo with tree-sitter (instant, no dependencies). If `gopls` / `ty` / `tsserver` are installed, cx uses them for type-resolved accuracy.

```bash
# What network calls does this service make?
cx network

# Trace a specific call chain across services
cx path --from placeOrderHandler --to ProductCatalogService

# What services depend on this one?
cx depends my-service --upstream

# Full service context
cx context

# Add another repo to build the cross-service graph
cx add ~/code/other-service

# Add a K8s config repo to resolve env vars to service DNS
cx add ~/code/k8s-manifests
```

## Distributed Graph (like git)

Each repo has a `.cx/` directory (like `.git/`). Teams maintain their own graph and connect to others:

```
repo/
  .cx/
    config.toml              # this repo's cx config
    graph/
      self.cxgraph           # this repo's graph (symbols, calls, network boundaries)
      self.network.json      # taint analysis results (provenance chains)
    config/
      sinks.toml             # custom network function definitions for this repo
      taxonomy.toml          # custom package classifications for this repo
    remotes/
      other-service.cxgraph  # pulled from that team's graph
      infra-k8s.cxgraph      # pulled from the infra team's graph
```

```bash
# Add a remote repo's graph
cx remote add payment-service https://github.com/org/payment-service
cx remote pull

# Share your graph for others to consume
cx remote push

# Incrementally re-index only repos with git changes
cx refresh

# Query across all connected repos
cx network --all-repos
```

### Getting from 95% to 100%

The last 5% of coverage is always repo-specific: custom wrappers, proprietary frameworks, unusual patterns. Instead of modifying cx source code, teams add to `.cx/config/`:

```toml
# .cx/config/sinks.toml — teach cx about your internal frameworks
[[sinks]]
fqn = "internal/httpclient.Do"
category = "http_client"
addr_arg = 0

[[sinks]]
fqn = "internal/bus.Publish"
category = "kafka_producer"
addr_arg = 0
```

```toml
# .cx/config/taxonomy.toml — classify internal packages
[[packages]]
names = ["internal/rpc"]
role = "grpc"
```

These configs are committed to the repo and shared via git. When another team does `cx remote pull`, they get both the graph AND the config that produced it — a **collaborative refinement loop** where each team improves accuracy for their codebase and the improvements propagate to anyone who depends on that repo's graph.

## MCP Integration

cx runs as an MCP server for AI coding agents:

```json
{
  "mcpServers": {
    "cx": { "command": "cx", "args": ["mcp", "--workspace", "."] }
  }
}
```

| Tool | What it does |
|------|-------------|
| `cx_path` | Trace request flow across service boundaries |
| `cx_network` | All network boundaries with address provenance chains |
| `cx_depends` | Upstream and downstream dependency graph |
| `cx_context` | Structural summary — endpoints, dependencies, resources |
| `cx_search` | Fuzzy symbol search across all indexed repos |

## Questions You Can Answer at Scale

With 1000 repos indexed, these are **sub-second graph traversals**:

### Cloud & vendor dependency
- **"Which services depend on AWS?"** — filter outgoing connections by `aws-sdk` / `s3` / `sqs` / `dynamodb` packages
- **"What's our blast radius if us-east-1 goes down?"** — find all services with AWS endpoints in that region

### Migration planning
- **"What would it take to replace SQS with Kafka?"** — find every SQS producer/consumer, list all services, show topic topology
- **"Which services need to migrate from Redis to Valkey?"** — find all `resource:redis` connections, group by service

### Security & compliance
- **"Which services make outbound calls to external domains?"** — filter connections by non-internal domains
- **"Do any services connect to databases without TLS?"** — check connection strings for `sslmode=disable`

### Incident response
- **"ProductCatalogService is down — what's affected?"** — upstream BFS: all transitive dependents
- **"Someone rotated the Redis credentials — which services need restarting?"** — find all services with `resource:redis` connections from the same env var / K8s secret

### Architecture understanding
- **"Show me every service that can reach the production database"** — BFS from database, follow all incoming edges
- **"Which service is the most depended-upon?"** — count incoming DependsOn edges per Deployable, sort descending
- **"I need to add a field to the Order proto — what services are affected?"** — find all gRPC clients/servers for Order

### Cost optimization
- **"Which services connect to expensive APIs (OpenAI, Stripe)?"** — filter by outgoing HTTP to known API domains
- **"Which services provision their own Redis vs using a shared instance?"** — group Redis connections by connection string source

## How It Works

### Phase 1: Fast Parse (tree-sitter)
Extracts all symbols, calls, imports, and string literals in <2s per repo. 100% recall — captures everything, no framework-specific queries needed.

### Phase 2: Type Resolution (LSP, optional)
When language-specific LSP servers are installed, cx resolves every call target to its fully qualified name. This turns `client.connect(addr)` from "some method call" into `"redis.Client.connect"` — enabling exact classification.

| Language | LSP Server | Without LSP | With LSP |
|----------|-----------|------------|----------|
| Go | gopls | ~75% | ~98% |
| Python | ty (Rust, 10-100x faster) | ~60% | ~90% |
| TypeScript/JS | tsserver | ~65% | ~95% |
| Java | jdtls / tree-sitter-java | ~70% | ~93% |
| Kotlin | kotlin-lsp | ~65% | ~88% |
| C/C++ | clangd | ~55% | ~85% |
| Rust | rust-analyzer | ~80% | ~98% |

### Phase 3: Network Sink Detection
A registry of ~150 known network functions (exact FQN match, not regex). Covers HTTP, gRPC, WebSocket, Kafka, Redis, databases, SQS, S3, raw TCP — across all supported languages.

### Phase 4: Backward Taint Analysis
For each detected network call, traces the address argument backward to its origin. cx classifies each source it finds:

- **Literal** — `"http://service:8080"` (known at parse time)
- **EnvVar** — `os.Getenv("SERVICE_ADDR")` (linked to K8s value in Phase 5)
- **ConfigKey** — `viper.Get("db.host")` (linked to config file)
- **Parameter** — `func connect(addr string)` (recurse into callers)
- **FieldAccess** — `config.ServiceAddr` (find all assignment sites)
- **Concat** — `base + "/api/v1"` (resolve each part)
- **Flag** — `flag.String("addr", "localhost:8080", "...")` (with default)
- **Dynamic** — computed at runtime (marked as unresolvable)

Cross-file, inter-procedural, depth-bounded at 10 levels with cycle detection.

### Phase 5: Infrastructure Resolution
Links code-side env var reads to K8s manifest values to service DNS names. Parses Dockerfiles (EXPOSE, ENTRYPOINT), Helm charts (Go templates with defaults), and K8s Deployment specs.

### Phase 6: Cross-Service Assembly
Matches outgoing connection targets to other services' exposed APIs. Works within one repo or across 1000+ repos via the distributed graph protocol.

## Architecture

cx is built in Rust as a cargo workspace:

- **cx-core** — Graph engine: CSR storage, mmap, BFS traversal, string interning, trigram search
- **cx-extractors** — tree-sitter parsing, LSP integration, taint analysis, sink registry
- **cx-resolution** — Cross-repo edge matching, K8s env→service DNS, proto→service, Helm values
- **cx-cli** — CLI commands, MCP server

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design, data model, query algorithms, and performance rules.

## Scaling to 1000 Repos

cx never re-indexes all repos when adding one. Per-repo graphs are independent:

```
.cx/graph/
  repos/
    repo-0000.cxgraph      # per-repo graph, independent, mmap'd on demand
    repo-0001.cxgraph
    ...
  index.cxindex             # global cross-repo index (~10MB in memory)
  overlay.cxgraph           # cross-repo edges only (~5MB in memory)
```

**Adding repo #1000 is as fast as adding repo #2:**

1. Index the new repo only (tree-sitter + optional LSP)
2. Write its per-repo graph and taint analysis results
3. Update the global index with new exposed APIs and outgoing targets
4. Re-resolve cross-repo edges for the new repo only — O(new_repo × log(index))
5. Update the overlay graph

**Queries only load repos they touch.** A BFS traversing 5 services mmaps 5 × 200KB = 1MB, not the full 300MB on disk. `cx refresh` checks git HEAD hashes and only re-indexes repos with changes.

| Component | Per repo | × 1000 repos | Strategy |
|-----------|----------|--------------|----------|
| Per-repo graph | ~200KB | 200MB | mmap, loaded on demand |
| Network analysis | ~50KB | 50MB | JSON, loaded on demand |
| Global index | — | ~10MB | Always in memory |
| Overlay graph | — | ~5MB | Always in memory |

## Performance Targets

| Operation | Target |
|-----------|--------|
| `cx_path` (5 service hops, 100K nodes) | < 1ms |
| `cx_network` (all boundaries, 100K nodes) | < 5ms |
| `cx_search` (fuzzy, 1M symbols) | < 10ms |
| `cx init` (100K LOC repo, no LSP) | < 2s |
| `cx init` (100K LOC repo, with LSP) | < 5s |
| `cx add` (1 repo to 1000-repo graph) | < 3s |
| Graph load from disk (mmap) | < 50ms |

## License

MIT OR Apache-2.0
