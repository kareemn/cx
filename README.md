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
  ↑ addr = os.Getenv("X")      K8s deployment.yaml:
                            →     SERVICE_ADDR
                                  = "backend:3550"        →  backend service
                                                               ↓
                                                            pb.RegisterBackendServer()
                                                              (Go, src/server/)
```

cx traces this entire chain: **code → env var → K8s value → DNS name → target service → exposed API.**

## Quick Start

```bash
cargo install cx-cli

cd ~/code/my-service
cx build
```

cx indexes your repo with tree-sitter (instant, no dependencies). Import-aware FQN resolution classifies most network calls automatically. If `claude` CLI is on PATH, cx optionally uses LLM classification for ambiguous calls (~$0.04/repo).

```bash
cx network
```

```
Network Boundaries — my-service
  Inbound: 4 endpoints    Outbound: 7 calls

Inbound Endpoints
  POST /api/v1/orders           [import-resolved]     src/handlers/orders.go:42
  POST /api/v1/checkout         [import-resolved]     src/handlers/checkout.go:18
  GET  /healthz                 [import-resolved]     src/handlers/health.go:10
  gRPC OrderService.PlaceOrder  [import-resolved]     src/grpc/order.go:25

Outbound Calls
  grpc    backend:3550          [import-resolved]     src/clients/catalog.go:31
          ← env SERVICE_ADDR ← K8s deployment.yaml
  http    payment-svc:8080      [import-resolved]     src/clients/payment.go:44
  redis   cache:6379            [import-resolved]     src/cache/redis.go:12
          ← env REDIS_ADDR ← K8s deployment.yaml
  kafka   order-events          [heuristic]           src/events/publish.go:28
```

## CLI

```
cx build [paths...]              Build the graph for one or more repos
cx trace <target>                Trace lineage of a network call or env var
cx network                       List all network calls and exposed APIs
cx add <path_or_git_url>         Add a remote repo's pre-built graph
cx pull                          Refresh graphs from registered remotes
cx fix                           Show unresolved calls, generate sink config
cx mcp                           Start MCP server (JSON-RPC over stdio)
```

### cx build

```bash
cx build                         # index current directory
cx build ./frontend ./backend    # index multiple repos together
cx build --verbose               # show LSP + LLM classification progress
```

Indexes repos with tree-sitter, runs cross-repo resolution (gRPC, REST, K8s env, Docker image, WebSocket matching), optionally upgrades heuristic calls via LSP and LLM.

### cx trace

```bash
cx trace DATABASE_URL            # full trace with provenance (both directions)
cx trace 'env:*'                 # compact overview of all env vars
cx trace 'env:*_ADDR'            # glob — all address env vars
cx trace pgxpool.New             # trace external library call (via network.json)
cx trace writer.go:27            # trace function at a file:line
cx trace call:client.go:Dial     # trace call in a specific file
cx trace DATABASE_URL --upstream # only upstream paths
cx trace DATABASE_URL --json     # JSON output
```

Target syntax supports `env:PATTERN` (with globs), `call:file:Func`, `file:line`, symbol names, and external library callees from network.json. Fuzzy match suggests alternatives on miss.

### cx network

```bash
cx network                       # all network boundaries
cx network --kind database       # filter by kind
cx network --direction outbound  # filter by direction
cx network --json                # JSON output
cx network --local-only          # exclude remote repos
cx network --include-all         # include test/vendor/example files
```

### cx add / cx pull

```bash
cx add ../other-service                 # add local repo
cx add git@github.com:org/k8s-config    # clone + add via git
cx pull                                 # refresh all remotes
cx pull --name other-service            # refresh specific remote
```

Copies the remote repo's pre-built `.cxgraph` and `network.json` — no re-indexing. After adding, creates cross-repo Resolves edges linking matching env var names across repos.

### cx fix

```bash
cx fix                           # show what's unresolved
cx fix --check                   # detailed view with dynamic sources
cx fix --init                    # generate .cx/config/sinks.toml template
```

## Custom Sink Config

The last 5% of coverage is always repo-specific. Teams teach cx about custom network functions via `.cx/config/sinks.toml`:

```toml
[[sinks]]
fqn = "pgxpool.New"
category = "database"
addr_arg = 1
direction = "outbound"

[[sinks]]
fqn = "internal/bus.Publish"
category = "kafka_producer"
addr_arg = 0
```

Custom sinks are checked before the built-in registry (user overrides win). Short names like `pgxpool.New` match against full FQNs. Run `cx fix --init` to generate a starter template from unresolved calls.

## Classification Pipeline

```
           ┌──────────────────────────────┐
Tier 1     │  Import-Aware FQN Resolution │  Free, deterministic
           │  + Custom sinks.toml         │  User overrides win
           └──────────┬───────────────────┘
                      │ unresolved calls
           ┌──────────▼───────────────────┐
Tier 2     │  LLM Classification          │  Optional: claude CLI
           │  ~30 lines context → Haiku   │  ~$0.04/repo
           └──────────┬───────────────────┘
                      │ still unresolved
           ┌──────────▼───────────────────┐
Tier 3     │  Heuristic Fallback          │  Pattern matching
           └──────────────────────────────┘
```

Every result is tagged with its confidence level:
- `[import-resolved]` — FQN matched via import alias or custom config
- `[llm-classified]` — LLM confirmed classification and target
- `[heuristic]` — pattern-matched only

## Distributed Graph

Each repo has a `.cx/` directory (like `.git/`):

```
repo/
  .cx/
    config.toml              # repos and remotes
    graph/
      base.cxgraph           # unified graph
      network.json           # taint analysis results
      repos/
        0000-my-service.cxgraph
      index.json             # global cross-repo index
      overlay.json           # cross-repo edges
    config/
      sinks.toml             # custom network function definitions
    remotes/
      other-service.cxgraph  # pulled from that team
      k8s-config.cxgraph     # pulled from infra team
```

```bash
cx build                         # build local graph
cx add ../other-service          # copy their pre-built graph
cx add git@github.com:org/repo   # clone + add via git
cx pull                          # refresh all remotes
cx network                       # query across all connected repos
cx network --local-only          # suppress remote data
```

Remote network calls are filtered: only env vars that match local code reads are shown. Use `--include-all` for everything.

## MCP Integration

cx runs as an MCP server for AI coding agents:

```json
{
  "mcpServers": {
    "cx": { "command": "cx", "args": ["mcp"] }
  }
}
```

| Tool | What it does |
|------|-------------|
| `cx_path` | Trace execution flow across service boundaries |
| `cx_network` | All network boundaries with address provenance chains |

## Architecture

cx is built in Rust as a cargo workspace:

- **cx-core** — Graph engine: CSR storage, mmap, BFS traversal, string interning
- **cx-extractors** — tree-sitter parsing, LSP integration, taint analysis, sink registry, custom sinks
- **cx-resolution** — Cross-repo matching: gRPC, REST, K8s env, Docker image, Helm, WebSocket
- **cx-cli** — CLI commands (build, trace, network, add, pull, fix), MCP server

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design, data model, query algorithms, and performance rules.

## Performance Targets

| Operation | Target |
|-----------|--------|
| `cx trace` (5 service hops, 100K nodes) | < 1ms |
| `cx network` (all boundaries, 100K nodes) | < 5ms |
| `cx build` (100K LOC repo, no LSP) | < 2s |
| `cx build` (100K LOC repo, with LLM) | < 15s |
| `cx add` (pre-built graph) | < 1s |
| Graph load from disk (mmap) | < 50ms |

## License

MIT OR Apache-2.0
