# cx

A distributed code intelligence engine that maps every network boundary in your distributed system — across repos, languages, and infrastructure.

CX finds every incoming API and outgoing network call in your codebase, traces where each connection target comes from (string literal, env var, config file, function parameter), and builds the complete cross-service topology. It works across Go, Python, TypeScript, Java, C/C++, and Rust — with Kubernetes, Helm, and Docker config resolution.

Designed for organizations with **1000+ microservices**. Works like git: local-first, distributed, collaborative.

## The Problem

Every distributed system has the same problem: nobody knows how it's actually wired together. The engineer who understood the full topology left last year. The architecture diagram is from 2022.

When you need to answer "which services depend on AWS SQS?" or "what's the blast radius if the payment service goes down?" — you spend hours grepping across repos and asking around.

AI agents have it worse. When Claude Code needs to trace a cross-service call chain, it spends 50k+ tokens reading files. With CX connected via MCP, the agent makes one `cx_path` call and gets the complete, type-resolved answer in milliseconds.

## What CX Tells You

```
Network Call: grpc.Dial(addr)
  Location:  src/frontend/rpc.go:30
  Kind:      gRPC client
  Target:    CURRENCY_SERVICE_ADDR (env var)
             → "currencyservice:7000" (k8s manifest)
             → CurrencyService (src/currencyservice/server.js)
  Provenance:
    main() → mustMapEnv() → os.Getenv("CURRENCY_SERVICE_ADDR")
```

CX traces the complete chain: **code → env var → K8s manifest → DNS name → target service → exposed API.** Across languages. Across repos.

## Quick Start

```bash
cargo install cx-engine

cd ~/code/my-service
cx init
```

CX indexes your repo with tree-sitter (instant, no dependencies). If `gopls` / `ty` / `tsserver` are installed, CX uses them for type-resolved accuracy.

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

Each team builds their repo's graph. Connect them to see the full topology:

```bash
# Add a remote repo's graph
cx remote add payment-service https://github.com/org/payment-service
cx remote pull

# Share your graph for others to consume
cx remote push

# Query across all connected repos
cx network --all-repos
```

Teams tune accuracy for their codebase via `.cx/config/` — not static docs, but compiler config that makes CX smarter:

```toml
# .cx/config/sinks.toml — teach CX about your internal frameworks
[[sinks]]
fqn = "internal/httpclient.Do"
category = "http_client"
addr_arg = 0
```

## MCP Integration

CX runs as an MCP server for AI coding agents:

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

- **"Which services depend on AWS?"** — filter outgoing connections by `aws-sdk` / `s3` / `sqs` packages
- **"What's our blast radius if us-east-1 goes down?"** — find all services with AWS endpoints in that region
- **"What would it take to replace SQS with Kafka?"** — find every SQS producer/consumer, list all services
- **"Which services make outbound calls to external domains?"** — filter connections by non-internal domains
- **"ProductCatalogService is down — what's affected?"** — upstream BFS: all transitive dependents
- **"Show me every service that can reach the production database"** — BFS from database, follow all incoming edges
- **"I need to add a field to the Order proto — what services are affected?"** — find all gRPC clients/servers for Order
- **"Which services connect to expensive APIs (OpenAI, Stripe)?"** — filter by outgoing HTTP to known API domains

## How It Works

### Phase 1: Fast Parse (tree-sitter)
Extracts all symbols, calls, imports, and string literals in <2s per repo. 100% recall — captures everything, no framework-specific queries needed.

### Phase 2: Type Resolution (LSP, optional)
When language-specific LSP servers are installed, CX resolves every call target to its fully qualified name. This turns `client.connect(addr)` from "some method call" into `"redis.Client.connect"` — enabling exact classification.

| Language | LSP Server | Without LSP | With LSP |
|----------|-----------|------------|----------|
| Go | gopls | ~75% | ~98% |
| Python | ty (Rust, 10-100x faster) | ~60% | ~90% |
| TypeScript/JS | tsserver | ~65% | ~95% |
| Java | jdtls / tree-sitter-java | ~70% | ~93% |
| C/C++ | clangd | ~55% | ~85% |
| Rust | rust-analyzer | ~80% | ~98% |

### Phase 3: Network Sink Detection
A registry of ~150 known network functions (exact FQN match, not regex). Covers HTTP, gRPC, WebSocket, Kafka, Redis, databases, SQS, S3, raw TCP — across all supported languages.

### Phase 4: Backward Taint Analysis
For each detected network call, traces the address argument backward through variable assignments, function parameters, env var reads, and config file loads. Cross-file, inter-procedural, depth-bounded.

### Phase 5: Infrastructure Resolution
Links code-side env var reads to K8s manifest values to service DNS names. Parses Dockerfiles (EXPOSE, ENTRYPOINT), Helm charts (Go templates with defaults), and K8s Deployment specs.

### Phase 6: Cross-Service Assembly
Matches outgoing connection targets to other services' exposed APIs. Works within one repo or across 1000+ repos via the distributed graph protocol.

## CX as Lossless Context

Inspired by [Lossless Context Management](https://papers.voltropy.com/LCM), CX's graph is a precomputed, hierarchical exploration summary. Instead of an agent ingesting raw source code, it queries CX for a compact, lossless, type-resolved representation. Every graph edge is traceable to source code — no information is lost.

## Architecture

CX is built in Rust as a cargo workspace:

- **cx-core** — Graph engine: CSR storage, mmap, BFS traversal, string interning, trigram search
- **cx-extractors** — tree-sitter parsing, LSP integration, taint analysis, sink registry
- **cx-resolution** — Cross-repo edge matching, K8s env→service DNS, proto→service, Helm values
- **cx-cli** — CLI commands, MCP server

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design, data model, query algorithms, and performance rules.

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
