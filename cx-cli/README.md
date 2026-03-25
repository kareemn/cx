# cx-cli

CLI binary and MCP server for CX. This is what users install — `cargo install cx-engine` gives the `cx` binary.

## Commands

| Command | Description |
|---------|-------------|
| `cx init` | Index current directory, write graph to `.cx/graph/index.cxgraph` |
| `cx add <path>` | Index an additional repo and write a separate `.cxgraph` file |
| `cx context` | Show structured JSON summary of service structure |
| `cx search <query>` | Fuzzy symbol search using trigram index (top 20 results) |
| `cx inspect <symbol>` | Show a symbol's outgoing/incoming edges |
| `cx edges [--kind K] [--limit N]` | List edge summary by kind with optional filtering |
| `cx path <from> [--max-depth N]` | Trace downstream execution paths from a symbol |
| `cx depends <symbol> [--upstream] [--max-depth N]` | Transitive dependency closure |
| `cx mcp` | Start MCP server over stdio (JSON-RPC 2.0 with Content-Length framing) |

## MCP Server

The MCP server exposes CX queries as tools for AI agents (Claude Code, Cursor, etc.).

### Tools

| Tool | Parameters | Description |
|------|-----------|-------------|
| `cx_path` | `from`, `direction`, `max_depth` | Trace execution paths |
| `cx_depends` | `target`, `direction`, `depth` | Transitive dependencies |
| `cx_context` | `service` (optional) | Service structure summary |
| `cx_search` | `query` | Fuzzy symbol search |
| `cx_resolve` | `query`, `kind` (optional) | Resolve qualified names to symbols |

### Claude Code Configuration

Add to your MCP settings:

```json
{
  "mcpServers": {
    "cx": {
      "command": "cx",
      "args": ["mcp"],
      "cwd": "/path/to/your/repo"
    }
  }
}
```

## Key Functions

| Function | File | Description |
|----------|------|-------------|
| `init::run()` | `commands/init.rs` | Index directory and write graph |
| `init::load_graph()` | `commands/init.rs` | Load graph from `.cx/graph/index.cxgraph` |
| `search::search_graph()` | `commands/search.rs` | Programmatic search API |
| `inspect::inspect_symbol()` | `commands/inspect.rs` | Formatted symbol edge report |
| `context::build_context()` | `commands/context.rs` | Build context JSON value |
| `mcp::run()` | `mcp/mod.rs` | MCP server main loop |

## Dependencies

- **cx-core** — graph, query, and store modules
- **cx-extractors** — `index_directory()` for indexing
- **clap** — CLI argument parsing
- **serde_json** — JSON output and MCP protocol

## Who Depends on This

Nothing — this is the top-level binary crate.
