# cx-languages

**Status: Superseded — this crate is empty and may be removed.**

Language support was originally planned for this crate, but is fully implemented in
**cx-extractors** instead (see `cx-extractors/src/grammars.rs` and `cx-extractors/queries/`).

## What's Here

Empty module stubs: `go.rs`, `typescript.rs`, `python.rs`. No public types or logic.

## Where Language Support Lives

The `cx-extractors` crate provides:
- `Language` enum with `from_extension()` and `ts_language()`
- `extractor_for_language()` to get a configured `UniversalExtractor`
- `.scm` query files for Go, Python, TypeScript, C, and C++

## Dependencies

- `cx-core`, `tree-sitter`, `thiserror` (unused)
