use anyhow::{bail, Result};
use std::path::Path;

const SKILL_CONTENT: &str = r#"---
description: Query service topology, network boundaries, and connection provenance using cx — a structural index for distributed systems.
---

# cx — Service topology, derived from code

cx builds a persistent, queryable graph of every incoming API, outgoing network call, and the provenance chain connecting them across repos, languages, and infrastructure. Use cx when you need to understand how services connect — it answers in milliseconds what would take thousands of tokens of grepping and file reading.

## When to use cx

- Understanding what a service connects to (databases, APIs, queues, caches)
- Tracing where a connection target comes from (env var → K8s manifest → DNS → target service)
- Finding all exposed endpoints (HTTP routes, gRPC services, WebSocket handlers)
- Investigating how an env var flows through code and infrastructure
- Understanding cross-service dependencies before making changes

## Commands

### cx network — list all network boundaries

```bash
cx network                       # all network boundaries
cx network --kind database       # filter: http, grpc, database, redis, kafka, websocket, sqs, s3, tcp
cx network --direction outbound  # filter: inbound or outbound
cx network --json                # machine-readable output
cx network --local-only          # exclude remote repos
```

Start here to get a complete picture of a service's network surface.

### cx trace — trace lineage of a network call or env var

```bash
cx trace DATABASE_URL            # full provenance trace (both directions)
cx trace 'env:*'                 # compact overview of all env vars
cx trace 'env:*_ADDR'            # glob — all address env vars
cx trace pgxpool.New             # trace an external library call
cx trace writer.go:27            # trace function at a file:line
cx trace call:client.go:Dial     # trace a call site in a specific file
cx trace DATABASE_URL --upstream # only upstream paths (who feeds this?)
cx trace DATABASE_URL --json     # JSON output
```

Target syntax: `env:PATTERN` (with globs), `call:file:Func`, `file:line`, symbol names. Fuzzy match suggests alternatives on miss.

### cx fix — show what's unresolved

```bash
cx fix                           # summary of unresolved calls
cx fix --check                   # detailed view with dynamic sources
cx fix --init                    # generate .cx/config/sinks.toml template
```

Use after `cx network` to understand gaps in coverage and improve the graph.

### cx diff — compare graph across states

```bash
cx diff --save                   # save current state as baseline
cx diff                          # compare current vs baseline
cx diff --branch main            # compare current vs another branch
cx diff --json                   # machine-readable output
```

### cx build — rebuild the graph

```bash
cx build                         # index current directory
cx build --verbose               # show classification progress
```

Only needed if source code has changed since the last build.

## Interpreting output

### Confidence levels

Each network call is tagged with how it was classified:
- `[import-resolved]` — deterministic, matched via import path and FQN (highest confidence)
- `[llm-classified]` — model-confirmed classification and target
- `[heuristic]` — pattern-matched only (lowest confidence, may need review)

### Address provenance

Outbound calls show where the connection target comes from:
```
grpc    backend:3550          [import-resolved]     src/clients/catalog.go:31
        ← env SERVICE_ADDR ← K8s deployment.yaml
```

The `←` chain traces the address backward: code reads env var, env var is set in K8s manifest, value resolves to a service DNS name.

### Graph structure

Nodes: Symbol (functions), Resource (env vars, connection targets), Endpoint (HTTP routes, gRPC services), Module (packages), Deployable (services)

Edges: Calls, Configures (reads env var), Connects (network call), Resolves (env var → target), Contains, Imports, DependsOn (cross-repo)

## Typical workflow

1. `cx network` — get the full picture of network boundaries
2. `cx trace <interesting_target>` — drill into a specific connection's provenance
3. `cx fix` — check if anything is unresolved
4. `cx diff --branch main` — see what changed in a PR
"#;

pub fn run(global: bool) -> Result<()> {
    let target_dir = if global {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"));
        match home {
            Ok(h) => Path::new(&h).join(".claude").join("skills").join("cx"),
            Err(_) => bail!("could not determine home directory"),
        }
    } else {
        Path::new(".claude").join("skills").join("cx")
    };

    std::fs::create_dir_all(&target_dir)
        .map_err(|e| anyhow::anyhow!("failed to create {}: {}", target_dir.display(), e))?;

    let target_file = target_dir.join("SKILL.md");
    std::fs::write(&target_file, SKILL_CONTENT)
        .map_err(|e| anyhow::anyhow!("failed to write {}: {}", target_file.display(), e))?;

    eprintln!("Wrote {}", target_file.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_content_starts_with_frontmatter() {
        assert!(SKILL_CONTENT.starts_with("---\n"));
        assert!(SKILL_CONTENT.contains("description:"));
    }
}
