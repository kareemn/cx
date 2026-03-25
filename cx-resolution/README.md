# cx-resolution

Cross-repo edge resolution — matches dangling gRPC client stubs to server registrations across repositories.

## Key Types

| Type | File | Description |
|------|------|-------------|
| `ResolutionInput` | `resolver.rs` | Aggregated client stubs, server registrations, and proto services from all repos |
| `ResolutionResult` | `resolver.rs` | Resolved `ProtoMatch` list + unresolved stubs |
| `ProtoMatch` | `proto_matching.rs` | A matched client-server pair with confidence score and source locations |

## How It Works

1. **Collect** — Gather `GrpcClientStub`, `GrpcServerRegistration`, and `ProtoService` from each indexed repo into a `ResolutionInput`
2. **Match** — For each client stub, find server registrations with the same service name
3. **Score** — Cross-repo matches get confidence 0.95; same-repo matches get 0.5
4. **Validate** — Check matches against proto definitions; add warnings if proto is missing
5. **Report** — Return resolved matches and list of unresolved client stubs

## Confidence Scoring

| Scenario | Confidence |
|----------|-----------|
| Cross-repo match (client in repo A, server in repo B) | 0.95 |
| Same-repo match | 0.50 |

## Modules

| Module | Status | Purpose |
|--------|--------|---------|
| `resolver.rs` | Implemented | Orchestrates resolution pass |
| `proto_matching.rs` | Implemented | Service name matching with confidence scores |
| `discovery.rs` | Stub | Future: service discovery mechanisms |
| `envvar_resolution.rs` | Stub | Future: resolve connections via environment variables |
| `k8s_dns.rs` | Stub | Future: Kubernetes DNS-based resolution |

## Dependencies

- **cx-core** — `Node`, `Edge` types
- **cx-extractors** — `GrpcClientStub`, `GrpcServerRegistration`, `ProtoService`

## Who Depends on This

- **cx-cli** — uses resolution in the `add` command for multi-repo indexing

## Example Usage

```rust
use cx_resolution::resolver::{ResolutionInput, resolve};

let input = ResolutionInput {
    client_stubs: vec![("frontend".into(), frontend_stubs)],
    server_registrations: vec![("backend".into(), backend_regs)],
    proto_services: vec![("protos".into(), proto_svcs)],
};

let result = resolve(&input);
for m in &result.proto_matches {
    println!("{}: {} -> {} (confidence: {})",
        m.service_name, m.client_repo, m.server_repo, m.confidence);
}
```
