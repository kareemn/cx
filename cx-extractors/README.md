# cx-extractors

Parsing pipeline that extracts structural graphs from source code using tree-sitter and `.scm` query files.

## Key Types

| Type | File | Description |
|------|------|-------------|
| `UniversalExtractor` | `universal.rs` | Runs a tree-sitter query against a parsed file, produces nodes and edges |
| `ExtractionResult` | `universal.rs` | Bag of `Vec<Node>` and `Vec<EdgeInput>` from one file |
| `ParsedFile` | `universal.rs` | Pre-parsed source file with tree, source bytes, path, and repo id |
| `RepoContext` | `universal.rs` | Repository identity (repo_id, root path) |
| `Language` | `grammars.rs` | Enum: Go, TypeScript, Python, C, Cpp |
| `IndexResult` | `pipeline.rs` | Full indexing result: `CsrGraph` + file/node/edge counts + errors |
| `ProtoService` | `proto.rs` | Parsed proto service with package, name, FQN, methods |
| `GrpcClientStub` | `grpc.rs` | Detected `pb.New*Client()` call site |
| `GrpcServerRegistration` | `grpc.rs` | Detected `pb.Register*Server()` call site |

## Supported Languages

| Language | Extensions | Query File |
|----------|-----------|------------|
| Go | `.go` | `queries/go-symbols.scm` |
| Python | `.py` | `queries/python-symbols.scm` |
| TypeScript | `.ts`, `.tsx`, `.js`, `.jsx` | `queries/typescript-symbols.scm` |
| C | `.c`, `.h` | `queries/c-symbols.scm` |
| C++ | `.cpp`, `.cc`, `.cxx`, `.hpp` | `queries/cpp-symbols.scm` |

## Connection Patterns (Capture Names)

Queries use standardized capture names that `UniversalExtractor` maps to graph structures:

| Capture | Produces |
|---------|----------|
| `@func.name` + `@func.def` | `Symbol` node (sub_kind=0), `Contains` edge from parent |
| `@type.name` + `@type.def` | `Symbol` node (sub_kind=1), `Contains` edge from parent |
| `@pkg.name` + `@pkg.def` | `Module` or `Deployable` node |
| `@call.name` + `@call.site` | `Calls` edge (caller resolved by byte offset containment) |
| `@import.path` + `@import.def` | Synthetic `Module` node + `Imports` edge |

Additional patterns: `@grpc.client.constructor`, `@grpc.server.register` (Go gRPC detection).

## Adding a New Language

1. Write a `.scm` query file in `queries/` using the capture names above
2. Add a variant to `Language` enum in `grammars.rs`
3. Add the `tree-sitter-{lang}` dependency to `Cargo.toml`
4. Register the language in `Language::ts_language()` and `from_extension()`

## Adding a New Connection Pattern

1. Add new `@capture.name` pairs to the relevant `.scm` query file
2. Add capture index fields to `UniversalExtractor`
3. Handle the new captures in `extract()` to produce appropriate nodes/edges

## Dependencies

- **cx-core** — `Node`, `Edge`, `EdgeInput`, `CsrGraph`, `StringInterner`, `NodeKind`, `EdgeKind`
- **tree-sitter** + per-language grammars — parsing
- **rayon** — parallel file processing
- **ignore** — `.gitignore`-aware directory walking

## Who Depends on This

- **cx-cli** — calls `index_directory()` from `init` and `add` commands
- **cx-resolution** — uses `GrpcClientStub`, `GrpcServerRegistration`, `ProtoService`

## Example Usage

```rust
use cx_extractors::pipeline::index_directory;

let result = index_directory(Path::new("./my-repo"))?;
println!("{} files, {} nodes, {} edges", result.file_count, result.node_count, result.edge_count);
// result.graph is a CsrGraph ready for queries
```
