use crate::grammars::{self, Language};
use crate::universal::{ExtractionResult, ParsedFile, UnresolvedCall};
use cx_core::graph::csr::{CsrGraph, EdgeInput};
use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::{Node, NodeKind};
use cx_core::graph::string_interner::StringInterner;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Result of running the full indexing pipeline.
pub struct IndexResult {
    pub graph: CsrGraph,
    pub file_count: usize,
    pub node_count: u32,
    pub edge_count: u32,
    pub errors: Vec<String>,
}

/// Run the indexing pipeline on a directory.
///
/// Pipeline:
/// 1. Walk directory with `ignore` crate (parallel, .gitignore aware)
/// 2. Filter to supported file extensions
/// 3. Parse files in parallel with rayon (per-thread tree-sitter parsers)
/// 4. Run UniversalExtractor on each parsed file
/// 5. Merge all ExtractionResults
/// 6. Build CsrGraph
pub fn index_directory(root: &Path) -> crate::Result<IndexResult> {
    // Step 1: Collect file paths using ignore crate
    let files = collect_files(root)?;

    if files.is_empty() {
        let strings = StringInterner::new();
        let graph = CsrGraph::build(vec![], vec![], strings);
        return Ok(IndexResult {
            graph,
            file_count: 0,
            node_count: 0,
            edge_count: 0,
            errors: vec![],
        });
    }

    // Global ID counter shared across threads
    let id_counter = AtomicU32::new(0);

    // Step 2-4: Parse and extract in parallel
    let per_file_results: Vec<(ExtractionResult, StringInterner, Vec<String>)> = files
        .par_iter()
        .filter_map(|path| {
            let lang = Language::from_path(path)?;
            Some((path, lang))
        })
        .map(|(path, lang)| {
            let mut errors = Vec::new();
            let mut strings = StringInterner::new();
            let mut result = ExtractionResult::new();

            // Read file
            let source = match std::fs::read(path) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(format!("{}: {}", path.display(), e));
                    return (result, strings, errors);
                }
            };

            // Parse with thread-local parser
            let ts_lang = lang.ts_language();
            let mut parser = tree_sitter::Parser::new();
            if parser.set_language(&ts_lang).is_err() {
                errors.push(format!("{}: failed to set language", path.display()));
                return (result, strings, errors);
            }

            let tree = match parser.parse(&source, None) {
                Some(t) => t,
                None => {
                    errors.push(format!("{}: parse failed", path.display()));
                    return (result, strings, errors);
                }
            };

            // Create extractor
            let extractor = match grammars::extractor_for_language(lang) {
                Some(e) => e,
                None => return (result, strings, errors),
            };

            // Compute repo-relative path
            let path_str = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy();
            let path_id = strings.intern(&path_str);

            let file = ParsedFile {
                tree,
                source: &source,
                path: path_id,
                path_str: &path_str,
                repo_id: 0,
            };

            // Reserve IDs atomically (estimate: we'll adjust after extraction)
            let base_id = id_counter.fetch_add(10000, Ordering::Relaxed);
            let mut local_id = base_id;
            result = extractor.extract(&file, &mut strings, &mut local_id);

            // Update the actual count used
            let used = local_id - base_id;
            if used < 10000 {
                // We over-allocated, but that's fine — IDs just need to be unique
            }

            (result, strings, errors)
        })
        .collect();

    // Step 5: Merge all results
    let file_count = per_file_results.len();
    let mut all_nodes: Vec<Node> = Vec::new();
    let mut all_edges: Vec<EdgeInput> = Vec::new();
    let mut merged_strings = StringInterner::new();
    let mut all_errors: Vec<String> = Vec::new();
    let mut all_unresolved: Vec<UnresolvedCall> = Vec::new();

    // Remap string IDs and node IDs from per-file interners into merged
    for (result, file_strings, errors) in per_file_results {
        all_errors.extend(errors);

        let mut string_remap: rustc_hash::FxHashMap<u32, u32> = rustc_hash::FxHashMap::default();
        let mut id_remap: rustc_hash::FxHashMap<u32, u32> = rustc_hash::FxHashMap::default();

        // Remap nodes — assign sequential IDs
        for mut node in result.nodes {
            let old_id = node.id;
            let new_id = all_nodes.len() as u32;
            id_remap.insert(old_id, new_id);
            node.id = new_id;
            node.name = remap_string(node.name, &file_strings, &mut merged_strings, &mut string_remap);
            node.file = remap_string(node.file, &file_strings, &mut merged_strings, &mut string_remap);
            // Remap parent if present
            if node.parent != u32::MAX {
                if let Some(&new_parent) = id_remap.get(&node.parent) {
                    node.parent = new_parent;
                }
            }
            all_nodes.push(node);
        }

        // Remap edge source/target IDs
        for mut edge in result.edges {
            if let (Some(&new_src), Some(&new_tgt)) =
                (id_remap.get(&edge.source), id_remap.get(&edge.target))
            {
                edge.source = new_src;
                edge.target = new_tgt;
                all_edges.push(edge);
            }
        }

        // Remap unresolved calls
        for call in result.unresolved_calls {
            if let Some(&new_caller) = id_remap.get(&call.caller_id) {
                let new_target_name = remap_string(
                    call.target_name,
                    &file_strings,
                    &mut merged_strings,
                    &mut string_remap,
                );
                all_unresolved.push(UnresolvedCall {
                    caller_id: new_caller,
                    target_name: new_target_name,
                });
            }
        }
    }

    // Step 5b: Deduplicate nodes with same (name, kind) that have no file
    // (e.g., external Module nodes like "context", "fmt" created per-import)
    // Also dedup Endpoint/Resource nodes with same name and file
    {
        // Map (name, kind) → canonical node ID for fileless nodes
        // Map (name, kind, file) → canonical node ID for nodes with files
        let mut canonical_fileless: rustc_hash::FxHashMap<(u32, u8), u32> =
            rustc_hash::FxHashMap::default();
        let mut canonical_with_file: rustc_hash::FxHashMap<(u32, u8, u32), u32> =
            rustc_hash::FxHashMap::default();
        // old_id → canonical_id for duplicates
        let mut dedup_remap: rustc_hash::FxHashMap<u32, u32> = rustc_hash::FxHashMap::default();
        let mut keep = vec![true; all_nodes.len()];

        for (idx, node) in all_nodes.iter().enumerate() {
            if node.file == u32::MAX {
                // External (fileless) node — dedup by (name, kind)
                let key = (node.name, node.kind);
                match canonical_fileless.entry(key) {
                    std::collections::hash_map::Entry::Occupied(e) => {
                        dedup_remap.insert(node.id, *e.get());
                        keep[idx] = false;
                    }
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(node.id);
                    }
                }
            } else if node.kind == NodeKind::Endpoint as u8
                || node.kind == NodeKind::Resource as u8
            {
                // Endpoint/Resource nodes — dedup by (name, kind, file)
                let key = (node.name, node.kind, node.file);
                match canonical_with_file.entry(key) {
                    std::collections::hash_map::Entry::Occupied(e) => {
                        dedup_remap.insert(node.id, *e.get());
                        keep[idx] = false;
                    }
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(node.id);
                    }
                }
            }
        }

        if !dedup_remap.is_empty() {
            // Remove duplicate nodes
            let mut idx = 0;
            all_nodes.retain(|_| {
                let k = keep[idx];
                idx += 1;
                k
            });

            // Reassign sequential IDs and build old→new remap
            let mut id_reassign: rustc_hash::FxHashMap<u32, u32> =
                rustc_hash::FxHashMap::default();
            for (new_idx, node) in all_nodes.iter_mut().enumerate() {
                id_reassign.insert(node.id, new_idx as u32);
                node.id = new_idx as u32;
            }
            // Also map removed duplicates through: old_dup_id → canonical_id → new_sequential_id
            for (dup_id, canonical_id) in &dedup_remap {
                if let Some(&new_id) = id_reassign.get(canonical_id) {
                    id_reassign.insert(*dup_id, new_id);
                }
            }

            // Remap edge targets/sources
            for edge in &mut all_edges {
                if let Some(&new_id) = id_reassign.get(&edge.source) {
                    edge.source = new_id;
                }
                if let Some(&new_id) = id_reassign.get(&edge.target) {
                    edge.target = new_id;
                }
            }

            // Remap unresolved call caller IDs
            for call in &mut all_unresolved {
                if let Some(&new_id) = id_reassign.get(&call.caller_id) {
                    call.caller_id = new_id;
                }
            }

            // Remap parent references
            for node in &mut all_nodes {
                if node.parent != u32::MAX {
                    if let Some(&new_id) = id_reassign.get(&node.parent) {
                        node.parent = new_id;
                    }
                }
            }
        }
    }

    // Step 5c: Cross-file call resolution
    // Build a map from symbol name → node ID for all Symbol nodes
    let mut symbol_by_name: rustc_hash::FxHashMap<u32, u32> = rustc_hash::FxHashMap::default();
    for node in &all_nodes {
        if node.kind == NodeKind::Symbol as u8 {
            // If multiple symbols share a name, keep the first (could be improved with package context)
            symbol_by_name.entry(node.name).or_insert(node.id);
        }
    }

    for call in &all_unresolved {
        if let Some(&target_id) = symbol_by_name.get(&call.target_name) {
            if target_id != call.caller_id {
                all_edges.push(EdgeInput::new(call.caller_id, target_id, EdgeKind::Calls));
            }
        }
    }

    // Step 6: Build CsrGraph
    let node_count = all_nodes.len() as u32;
    let edge_count = all_edges.len() as u32;
    let graph = CsrGraph::build(all_nodes, all_edges, merged_strings);

    Ok(IndexResult {
        graph,
        file_count,
        node_count,
        edge_count,
        errors: all_errors,
    })
}

/// Remap a StringId from a per-file interner to the merged interner.
fn remap_string(
    old_id: u32,
    old_interner: &StringInterner,
    new_interner: &mut StringInterner,
    cache: &mut rustc_hash::FxHashMap<u32, u32>,
) -> u32 {
    if old_id == u32::MAX {
        return u32::MAX;
    }
    if let Some(&new_id) = cache.get(&old_id) {
        return new_id;
    }
    let s = old_interner.get(old_id);
    let new_id = new_interner.intern(s);
    cache.insert(old_id, new_id);
    new_id
}

/// Collect all files in a directory, respecting .gitignore.
fn collect_files(root: &Path) -> crate::Result<Vec<PathBuf>> {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    let mut files = Vec::new();
    for entry in walker {
        match entry {
            Ok(e) if e.file_type().is_some_and(|ft| ft.is_file()) => {
                files.push(e.into_path());
            }
            Err(e) => {
                // Non-fatal: log and continue
                eprintln!("walk error: {}", e);
            }
            _ => {}
        }
    }
    Ok(files)
}

/// Error type for the pipeline.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("index error: {0}")]
    Index(String),
}

// Re-export Result type for the pipeline
pub type Result<T> = std::result::Result<T, PipelineError>;

#[cfg(test)]
mod tests {
    use super::*;
    use cx_core::graph::nodes::NodeKind;
    use std::fs;

    fn create_go_project(dir: &Path) {
        fs::write(
            dir.join("main.go"),
            r#"package main

import "fmt"

func main() {
    fmt.Println("hello")
    helper()
}

func helper() {
    fmt.Println("helping")
}
"#,
        )
        .unwrap();

        fs::write(
            dir.join("server.go"),
            r#"package main

type Server struct {
    port int
}

func (s *Server) Start() {}
func (s *Server) Stop() {}

func newServer() *Server {
    return &Server{port: 8080}
}
"#,
        )
        .unwrap();
    }

    #[test]
    fn index_go_project() {
        let dir = tempfile::tempdir().unwrap();
        create_go_project(dir.path());

        let result = index_directory(dir.path()).unwrap();

        assert_eq!(result.file_count, 2, "should process 2 Go files");
        assert!(result.node_count > 0, "should find symbols");
        assert!(result.errors.is_empty(), "should have no errors: {:?}", result.errors);

        // Check specific symbols exist
        let names: Vec<&str> = result
            .graph
            .nodes
            .iter()
            .map(|n| result.graph.strings.get(n.name))
            .collect();

        assert!(names.contains(&"main"), "should find main func");
        assert!(names.contains(&"helper"), "should find helper func");
        assert!(names.contains(&"Server"), "should find Server type");
        assert!(names.contains(&"Start"), "should find Start method");
        assert!(names.contains(&"Stop"), "should find Stop method");
        assert!(names.contains(&"newServer"), "should find newServer func");
    }

    #[test]
    fn index_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let result = index_directory(dir.path()).unwrap();
        assert_eq!(result.file_count, 0);
        assert_eq!(result.node_count, 0);
        assert_eq!(result.edge_count, 0);
    }

    #[test]
    fn index_ignores_non_go_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("README.md"), "# Hello").unwrap();
        fs::write(dir.path().join("data.json"), "{}").unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        let result = index_directory(dir.path()).unwrap();

        // Should only process main.go, not README.md or data.json
        // file_count includes only files that had a matching language
        assert!(result.node_count > 0, "should find Go symbols");

        let names: Vec<&str> = result
            .graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8)
            .map(|n| result.graph.strings.get(n.name))
            .collect();

        assert!(names.contains(&"hello"));
    }

    #[test]
    fn index_respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();

        // Init git repo so .gitignore is respected
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Create .gitignore
        fs::write(dir.path().join(".gitignore"), "vendor/\n").unwrap();

        // Create a file that should be indexed
        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc included() {}\n",
        )
        .unwrap();

        // Create a file in vendor/ that should be ignored
        fs::create_dir(dir.path().join("vendor")).unwrap();
        fs::write(
            dir.path().join("vendor/dep.go"),
            "package dep\nfunc excluded() {}\n",
        )
        .unwrap();

        let result = index_directory(dir.path()).unwrap();

        let names: Vec<&str> = result
            .graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8)
            .map(|n| result.graph.strings.get(n.name))
            .collect();

        assert!(names.contains(&"included"), "should find included");
        assert!(!names.contains(&"excluded"), "should not find excluded (gitignored)");
    }

    #[test]
    fn index_respects_gitignore_venv_and_nested() {
        let dir = tempfile::tempdir().unwrap();

        // Init git repo so .gitignore is respected
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Create .gitignore with multiple patterns
        fs::write(
            dir.path().join(".gitignore"),
            "venv/\nnode_modules/\n*.generated.go\n",
        )
        .unwrap();

        // Indexed: normal Go file
        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc keepMe() {}\n",
        )
        .unwrap();

        // Ignored: venv/ directory (Python virtualenv)
        fs::create_dir_all(dir.path().join("venv/lib")).unwrap();
        fs::write(
            dir.path().join("venv/lib/setup.go"),
            "package lib\nfunc venvFunc() {}\n",
        )
        .unwrap();

        // Ignored: node_modules/
        fs::create_dir(dir.path().join("node_modules")).unwrap();
        fs::write(
            dir.path().join("node_modules/pkg.go"),
            "package pkg\nfunc nodeFunc() {}\n",
        )
        .unwrap();

        // Ignored: wildcard pattern *.generated.go
        fs::write(
            dir.path().join("api.generated.go"),
            "package main\nfunc generatedFunc() {}\n",
        )
        .unwrap();

        let result = index_directory(dir.path()).unwrap();

        let names: Vec<&str> = result
            .graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8)
            .map(|n| result.graph.strings.get(n.name))
            .collect();

        assert!(names.contains(&"keepMe"), "should index normal file");
        assert!(!names.contains(&"venvFunc"), "should ignore venv/");
        assert!(!names.contains(&"nodeFunc"), "should ignore node_modules/");
        assert!(!names.contains(&"generatedFunc"), "should ignore *.generated.go");
    }

    #[test]
    fn cross_package_call_resolution() {
        let dir = tempfile::tempdir().unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Package "transport" with StartWSServer
        fs::create_dir(dir.path().join("transport")).unwrap();
        fs::write(
            dir.path().join("transport/ws.go"),
            r#"package transport

func StartWSServer() {
    loggingMiddleware()
}

func loggingMiddleware() {}
"#,
        )
        .unwrap();

        // Package "main" calls transport.StartWSServer()
        fs::write(
            dir.path().join("main.go"),
            r#"package main

import "myapp/transport"

func main() {
    transport.StartWSServer()
}
"#,
        )
        .unwrap();

        let result = index_directory(dir.path()).unwrap();

        // Find node IDs
        let main_id = result
            .graph
            .nodes
            .iter()
            .find(|n| {
                result.graph.strings.get(n.name) == "main"
                    && n.kind == NodeKind::Symbol as u8
            })
            .map(|n| n.id)
            .expect("main function should exist");

        let start_ws_id = result
            .graph
            .nodes
            .iter()
            .find(|n| {
                result.graph.strings.get(n.name) == "StartWSServer"
                    && n.kind == NodeKind::Symbol as u8
            })
            .map(|n| n.id)
            .expect("StartWSServer should exist");

        // There should be a Calls edge from main → StartWSServer (cross-package)
        let has_cross_call = result.graph.edges_for(
            result.graph.nodes.iter().position(|n| n.id == main_id).unwrap() as u32
        ).iter().any(|e| {
            let target_node = result.graph.node(e.target);
            result.graph.strings.get(target_node.name) == "StartWSServer"
                && e.kind == EdgeKind::Calls as u8
        });

        assert!(has_cross_call, "main should have a Calls edge to StartWSServer (cross-package)");

        // Intra-file call should still work: StartWSServer → loggingMiddleware
        let start_ws_idx = result.graph.nodes.iter().position(|n| n.id == start_ws_id).unwrap() as u32;
        let has_intra_call = result.graph.edges_for(start_ws_idx).iter().any(|e| {
            let target_node = result.graph.node(e.target);
            result.graph.strings.get(target_node.name) == "loggingMiddleware"
                && e.kind == EdgeKind::Calls as u8
        });

        assert!(has_intra_call, "StartWSServer should call loggingMiddleware (intra-file)");
    }

    #[test]
    fn index_produces_valid_graph() {
        let dir = tempfile::tempdir().unwrap();
        create_go_project(dir.path());

        let result = index_directory(dir.path()).unwrap();

        // Graph should be queryable
        let graph = &result.graph;
        assert!(graph.node_count() > 0);
        assert_eq!(graph.offsets.len() as u32, graph.node_count() + 1);
        assert_eq!(graph.rev_offsets.len() as u32, graph.node_count() + 1);
    }

    #[test]
    fn real_go_repo_structure() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // cmd/server/main.go
        fs::create_dir_all(root.join("cmd/server")).unwrap();
        fs::write(root.join("cmd/server/main.go"), r#"package main

import "fmt"

func main() {
    fmt.Println("server starting")
}
"#).unwrap();

        // cmd/migrate/main.go
        fs::create_dir_all(root.join("cmd/migrate")).unwrap();
        fs::write(root.join("cmd/migrate/main.go"), r#"package main

func main() {}
"#).unwrap();

        // pkg/auth/login.go
        fs::create_dir_all(root.join("pkg/auth")).unwrap();
        fs::write(root.join("pkg/auth/login.go"), r#"package auth

func Login() {}
"#).unwrap();

        // pkg/auth/token.go
        fs::write(root.join("pkg/auth/token.go"), r#"package auth

func ValidateToken() {}
"#).unwrap();

        // internal/db/query.go
        fs::create_dir_all(root.join("internal/db")).unwrap();
        fs::write(root.join("internal/db/query.go"), r#"package db

func Query() {}
"#).unwrap();

        let result = index_directory(root).unwrap();
        let graph = &result.graph;

        // 2 Deployable nodes (from the two package main files)
        let deployables: Vec<&str> = graph.nodes.iter()
            .filter(|n| n.kind == NodeKind::Deployable as u8)
            .map(|n| graph.strings.get(n.name))
            .collect();
        assert_eq!(deployables.len(), 2, "should have 2 deployables, got: {:?}", deployables);

        // 3 distinct Module nodes (auth x2 deduplicated to auth, db, but auth appears in two files)
        // Actually each file produces its own Module node, so auth appears twice, db once = 3 Module nodes
        let modules: Vec<&str> = graph.nodes.iter()
            .filter(|n| n.kind == NodeKind::Module as u8)
            .map(|n| graph.strings.get(n.name))
            .collect();
        assert!(modules.contains(&"auth"), "should have auth module");
        assert!(modules.contains(&"db"), "should have db module");
        assert!(!modules.contains(&"main"), "main should be Deployable, not Module");

        // Symbol count: main(2) + Login + ValidateToken + Query = 5 functions
        let symbols: Vec<&str> = graph.nodes.iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| graph.strings.get(n.name))
            .collect();
        assert!(symbols.contains(&"Login"), "should find Login");
        assert!(symbols.contains(&"ValidateToken"), "should find ValidateToken");
        assert!(symbols.contains(&"Query"), "should find Query");
        assert_eq!(symbols.iter().filter(|&&s| s == "main").count(), 2, "should have 2 main functions");
    }

    #[test]
    fn extractor_parse_failure_nonfatal() {
        // TEST extractor_parse_failure_nonfatal from ARCHITECTURE.md:
        // Repo with 10 Go files. File 5 has syntax errors (invalid Go).
        // Other 9 files indexed successfully. Graph contains symbols from good files.
        let dir = tempfile::tempdir().unwrap();

        for i in 0..10 {
            let content = if i == 5 {
                // Invalid Go — tree-sitter will still produce a tree (it's error-tolerant)
                // but extraction should produce fewer/no useful symbols
                "package main\n\nfunc {{{ invalid syntax @@@ }}}\n".to_string()
            } else {
                format!("package main\n\nfunc func_{}() {{}}\n", i)
            };
            fs::write(dir.path().join(format!("file_{}.go", i)), content).unwrap();
        }

        let result = index_directory(dir.path()).unwrap();

        // Should have symbols from the 9 good files
        let func_names: Vec<&str> = result
            .graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| result.graph.strings.get(n.name))
            .collect();

        // Should find at least 9 functions (func_0 through func_9, excluding func_5)
        for i in 0..10 {
            if i == 5 {
                continue;
            }
            let name = format!("func_{}", i);
            assert!(
                func_names.contains(&name.as_str()),
                "should find {} from good files, got: {:?}",
                name,
                func_names.len()
            );
        }

        // Pipeline should not have fatal errors
        // (parse failures are non-fatal per ARCHITECTURE.md)
    }
}
