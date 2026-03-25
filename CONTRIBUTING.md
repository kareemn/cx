# Contributing to CX

## Getting Started

```bash
# Build
cargo build

# Test
cargo test

# Lint (zero warnings policy)
cargo clippy

# Benchmarks
cargo bench
```

## Adding Language Support

> **Prerequisite:** The `UniversalExtractor` in `cx-extractors` must be implemented first. Once it lands, adding a language is a self-contained task with no coordination needed.

### Steps

1. Add `tree-sitter-[lang]` to `cx-extractors/Cargo.toml`
2. Create `cx-extractors/queries/[lang]-symbols.scm`
3. Register the language in `cx-extractors/src/grammars.rs`

### Query file captures

Your `.scm` query file must extract these captures:

| Capture | Purpose |
|---------|---------|
| `@func.name` + `@func.def` | Function declarations |
| `@method.name` + `@method.def` | Method declarations |
| `@type.name` + `@type.def` | Type/class declarations |
| `@call.name` + `@call.site` | Function/method call sites |
| `@call.receiver` | Qualifier on method calls (e.g., `obj` in `obj.method()`) |
| `@import.path` + `@import.def` | Import statements |

### Figuring out the query

Run `tree-sitter parse sample.ext` on a sample file to see the concrete syntax tree and node types. Or read the grammar's `node-types.json` in the `tree-sitter-[lang]` repo.

Use `cx-extractors/queries/go-symbols.scm` as a reference once it exists.

### Testing

Create `cx-extractors/tests/[lang]_symbols.rs` with a realistic sample source file. Verify correct symbol count, call edges, and import edges.

```bash
cargo test -p cx-extractors
```

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design, data model, query algorithms, and performance rules. Read it before making changes to core data structures or query paths.
