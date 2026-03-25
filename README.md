# cx

A local-first code intelligence engine that maps how distributed systems are wired together — across repos, languages, and infrastructure.

CX builds a queryable structural graph of your codebase. It traces gRPC calls through service boundaries, resolves environment variables through Helm charts to Kubernetes services, detects shared database dependencies, and tells you the exact blast radius of any change. Queries complete in under 5ms.

The primary interface is an MCP server for AI coding agents (Claude Code, Cursor, Gemini CLI) and a CLI for developers. No cloud. No SaaS. Single binary.

## Why

Every distributed system has the same problem: nobody knows how it's actually wired together. The engineer who understood the full topology left last year. The architecture diagram is from 2022. When you need to answer "what breaks if I change this?" you spend hours grepping across repos and asking around.

CX answers that question in milliseconds by building a graph from the source of truth — your code, your proto files, your Helm charts, your Kubernetes manifests.

AI coding agents have the same problem, worse. When Claude Code needs to understand a cross-service call chain, it spends 15-20 tool calls grepping and reading files, burning 50k+ tokens. With CX connected via MCP, the agent makes one `cx_path` call and gets the complete answer instantly.

## Quick Start

```bash
cargo install cx-engine

cd ~/code/my-service
cx init
```

CX indexes your repo, auto-discovers connected services in the same GitHub org, and builds the graph. First query is ready in seconds.

```bash
# What does this service look like?
cx context

# Trace the full request path from an endpoint
cx path --from "WS /ws/translate" --downstream

# What breaks if I change this function?
cx impact orders/router.go:ProcessOrder

# What services does this depend on?
cx depends my-service --downstream

# What changed structurally on this branch?
cx diff main feature/new-cache

# Add a related repo to expand the graph
cx add ~/code/other-service
cx add --role infra ~/code/helm-charts
```

## MCP Integration

CX runs as an MCP server that any compatible AI agent can use. Add to your Claude Code config:

```json
{
  "mcpServers": {
    "cx": {
      "command": "cx",
      "args": ["mcp", "--workspace", "."]
    }
  }
}
```

The agent gets these tools:

| Tool | What it does |
|------|-------------|
| `cx_path` | Trace request flow across service boundaries |
| `cx_impact` | Blast radius of a change — all affected services, endpoints, configs |
| `cx_depends` | Upstream and downstream dependency graph |
| `cx_context` | Structural summary of a service — endpoints, dependencies, resources |
| `cx_search` | Fuzzy symbol search across all indexed repos |
| `cx_resolve` | Resolve a name to specific symbols (supports qualified names) |
| `cx_diff` | Structural diff between two git refs |
| `cx_blame` | Who introduced a specific dependency |

## What CX Understands

**Cross-service connections:** gRPC client/server matching via proto files. REST endpoint matching via OpenAPI specs and URL patterns. Async dependencies via Kafka/NATS/SQS topic matching.

**Infrastructure wiring:** Environment variables traced from code (`os.Getenv("X")`) through Helm chart definitions to Kubernetes DNS resolution to the actual service. Missing config detected automatically.

**Shared resources:** Two services connecting to the same database. Library code imported across repos with independent deploy cycles (version skew risk).

**Git history:** When did a dependency appear? Who introduced it? What's the structural diff between branches? Does this PR add a new service dependency or create a config gap?

## How It Works

CX uses [tree-sitter](https://tree-sitter.github.io/tree-sitter/) for fast, incremental parsing across languages and stores the dependency graph in a compressed sparse row (CSR) format that's memory-mapped from disk. Queries are graph traversals with bitmask-based edge filtering — no heap allocation, no pointer chasing, sub-millisecond on million-node graphs.

The graph is git-native: snapshots are keyed by commit, branches are delta-encoded overlays, and temporal queries walk history to find when dependencies appeared or changed.

Adding language support requires only a tree-sitter query file (~50 lines of S-expressions) — no Rust code per language.

## Language Support

| Language | Symbols | gRPC | Env Vars |
|----------|---------|------|----------|
| Go | 🔜 | 🔜 | 🔜 |
| TypeScript | 🔜 | 🔜 | 🔜 |
| Python | 🔜 | 🔜 | 🔜 |
| Java | planned | planned | planned |
| Rust | planned | planned | planned |
| C/C++ | planned | planned | planned |

## Infrastructure Support

| Format | Status |
|--------|--------|
| Protocol Buffers (.proto) | ✅ |
| Helm charts | 🔜 |
| Kubernetes manifests | 🔜 |
| Dockerfiles | 🔜 |
| OpenAPI / Swagger | planned |
| Terraform | planned |
| docker-compose | planned |

## Architecture

CX is built in Rust as a cargo workspace:

- **cx-core** — Graph engine: CSR storage, mmap, BFS traversal, string interning, trigram search
- **cx-extractors** — Tree-sitter parsing pipeline and structural extractors
- **cx-resolution** — Cross-repo edge matching (proto→service, env var→Helm→k8s DNS)
- **cx-cli** — CLI commands and MCP server

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design, data model, query algorithms, and performance rules.

## Performance Targets

| Operation | Target |
|-----------|--------|
| `cx_path` (5 service hops, 100K nodes) | < 1ms |
| `cx_impact` (depth 5, 100K nodes) | < 5ms |
| `cx_search` (fuzzy, 1M symbols) | < 10ms |
| `cx init` (100K LOC repo) | < 2s |
| Graph load from disk (mmap) | < 50ms |

## Contributing

CX is early. The graph engine is solid, the extractor pipeline is in progress. The easiest way to contribute is adding language support — each language is a tree-sitter query file with no Rust code required. See [ARCHITECTURE.md](ARCHITECTURE.md) for the capture naming conventions.

## License

MIT OR Apache-2.0
