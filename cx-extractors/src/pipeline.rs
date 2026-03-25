use crate::grammars::{self, Language};
use crate::grpc::{self, GrpcClientStub, GrpcScanResult, GrpcServerRegistration};
use crate::proto::{self, ProtoService};
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

/// Intermediate result after extraction and merging, before CSR construction.
/// Exposed so callers can inject additional edges (e.g., from the resolution engine)
/// before building the final graph.
pub struct MergedResult {
    pub nodes: Vec<Node>,
    pub edges: Vec<EdgeInput>,
    pub strings: StringInterner,
    pub file_count: usize,
    pub errors: Vec<String>,
    /// gRPC client stubs found per repo: (repo_name, stubs)
    pub grpc_clients: Vec<(String, Vec<GrpcClientStub>)>,
    /// gRPC server registrations found per repo: (repo_name, registrations)
    pub grpc_servers: Vec<(String, Vec<GrpcServerRegistration>)>,
    /// Proto service definitions found per repo: (repo_name, services)
    pub proto_services: Vec<(String, Vec<ProtoService>)>,
}

/// A file to index, tagged with its repo root and repo ID.
struct RepoFile {
    path: PathBuf,
    root: PathBuf,
    repo_id: u16,
}

/// Run the indexing pipeline on a single directory (convenience wrapper).
pub fn index_directory(root: &Path) -> crate::Result<IndexResult> {
    index_repos(&[(root.to_path_buf(), 0)])
}

/// Run the indexing pipeline across multiple repos, producing a single unified graph.
pub fn index_repos(repos: &[(PathBuf, u16)]) -> crate::Result<IndexResult> {
    let merged = extract_and_merge_repos(repos)?;
    Ok(build_index(merged))
}

/// Build the final CSR graph from merged extraction data.
pub fn build_index(merged: MergedResult) -> IndexResult {
    let node_count = merged.nodes.len() as u32;
    let edge_count = merged.edges.len() as u32;
    let graph = CsrGraph::build(merged.nodes, merged.edges, merged.strings);
    IndexResult {
        graph,
        file_count: merged.file_count,
        node_count,
        edge_count,
        errors: merged.errors,
    }
}

/// Extract and merge all repos into a single node/edge list with gRPC/proto metadata.
///
/// This does everything except building the CSR graph, so callers can inject
/// additional edges (e.g., from the gRPC resolution engine) before calling `build_index`.
pub fn extract_and_merge_repos(repos: &[(PathBuf, u16)]) -> crate::Result<MergedResult> {
    // Step 1: Collect files from all repos (including .proto files)
    let mut all_repo_files: Vec<RepoFile> = Vec::new();
    let mut proto_files: Vec<RepoFile> = Vec::new();

    for (root, repo_id) in repos {
        let files = collect_files(root)?;
        for path in files {
            if path.extension().is_some_and(|e| e == "proto") {
                proto_files.push(RepoFile {
                    path,
                    root: root.clone(),
                    repo_id: *repo_id,
                });
            } else {
                all_repo_files.push(RepoFile {
                    path,
                    root: root.clone(),
                    repo_id: *repo_id,
                });
            }
        }
    }

    if all_repo_files.is_empty() && proto_files.is_empty() {
        return Ok(MergedResult {
            nodes: vec![],
            edges: vec![],
            strings: StringInterner::new(),
            file_count: 0,
            errors: vec![],
            grpc_clients: vec![],
            grpc_servers: vec![],
            proto_services: vec![],
        });
    }

    // Build repo_id → repo_name mapping
    let repo_names: rustc_hash::FxHashMap<u16, String> = repos
        .iter()
        .map(|(root, id)| {
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| format!("repo-{}", id));
            (*id, name)
        })
        .collect();

    // Global ID counter shared across threads
    let id_counter = AtomicU32::new(0);

    // Step 2-4: Parse and extract in parallel, also scanning for gRPC patterns
    let per_file_results: Vec<(ExtractionResult, StringInterner, Vec<String>, GrpcScanResult)> =
        all_repo_files
            .par_iter()
            .filter_map(|rf| {
                let lang = Language::from_path(&rf.path)?;
                Some((rf, lang))
            })
            .map(|(rf, lang)| {
                let mut errors = Vec::new();
                let mut strings = StringInterner::new();
                let mut result = ExtractionResult::new();
                let mut grpc_result = GrpcScanResult {
                    client_stubs: vec![],
                    server_registrations: vec![],
                };

                // Read file
                let source = match std::fs::read(&rf.path) {
                    Ok(s) => s,
                    Err(e) => {
                        errors.push(format!("{}: {}", rf.path.display(), e));
                        return (result, strings, errors, grpc_result);
                    }
                };

                // Parse with thread-local parser
                let ts_lang = lang.ts_language();
                let mut parser = tree_sitter::Parser::new();
                if parser.set_language(&ts_lang).is_err() {
                    errors.push(format!("{}: failed to set language", rf.path.display()));
                    return (result, strings, errors, grpc_result);
                }

                let tree = match parser.parse(&source, None) {
                    Some(t) => t,
                    None => {
                        errors.push(format!("{}: parse failed", rf.path.display()));
                        return (result, strings, errors, grpc_result);
                    }
                };

                // Create extractor
                let extractor = match grammars::extractor_for_language(lang) {
                    Some(e) => e,
                    None => return (result, strings, errors, grpc_result),
                };

                // Compute repo-relative path
                let path_str = rf
                    .path
                    .strip_prefix(&rf.root)
                    .unwrap_or(&rf.path)
                    .to_string_lossy();
                let path_id = strings.intern(&path_str);

                let file = ParsedFile {
                    tree,
                    source: &source,
                    path: path_id,
                    path_str: &path_str,
                    repo_id: rf.repo_id,
                };

                // Reserve IDs atomically
                let base_id = id_counter.fetch_add(10000, Ordering::Relaxed);
                let mut local_id = base_id;
                result = extractor.extract(&file, &mut strings, &mut local_id);

                // Scan for gRPC patterns on Go files (reuse the parsed tree)
                if lang == Language::Go {
                    grpc_result =
                        grpc::scan_go_grpc(&file.tree, &source, &path_str, &ts_lang);
                }

                (result, strings, errors, grpc_result)
            })
            .collect();

    // Step 2b: Extract proto files (simple line parser, no tree-sitter needed)
    let proto_id_base = id_counter.load(Ordering::Relaxed);
    let mut proto_id = proto_id_base;
    let mut proto_strings = StringInterner::new();
    let mut all_proto_nodes: Vec<Node> = Vec::new();
    let mut all_proto_services: Vec<(u16, Vec<ProtoService>)> = Vec::new();

    for pf in &proto_files {
        if let Ok(source) = std::fs::read_to_string(&pf.path) {
            let path_str = pf
                .path
                .strip_prefix(&pf.root)
                .unwrap_or(&pf.path)
                .to_string_lossy();
            let proto_result =
                proto::extract_proto(&source, &path_str, &mut proto_strings, &mut proto_id);
            // Tag proto nodes with repo_id
            for mut node in proto_result.nodes {
                node.repo = pf.repo_id;
                all_proto_nodes.push(node);
            }
            if !proto_result.services.is_empty() {
                all_proto_services.push((pf.repo_id, proto_result.services));
            }
        }
    }

    // Step 5: Merge all results
    let file_count = per_file_results.len() + proto_files.len();
    let mut all_nodes: Vec<Node> = Vec::new();
    let mut all_edges: Vec<EdgeInput> = Vec::new();
    let mut merged_strings = StringInterner::new();
    let mut all_errors: Vec<String> = Vec::new();
    let mut all_unresolved: Vec<UnresolvedCall> = Vec::new();

    // Collect gRPC data per repo
    let mut grpc_clients_by_repo: rustc_hash::FxHashMap<u16, Vec<GrpcClientStub>> =
        rustc_hash::FxHashMap::default();
    let mut grpc_servers_by_repo: rustc_hash::FxHashMap<u16, Vec<GrpcServerRegistration>> =
        rustc_hash::FxHashMap::default();

    // Remap string IDs and node IDs from per-file interners into merged
    // We need to track which repo each file result came from for gRPC data grouping
    let repo_ids: Vec<u16> = all_repo_files
        .iter()
        .filter_map(|rf| {
            Language::from_path(&rf.path)?;
            Some(rf.repo_id)
        })
        .collect();

    for (i, (result, file_strings, errors, grpc_scan)) in per_file_results.into_iter().enumerate()
    {
        all_errors.extend(errors);

        // Collect gRPC scan data grouped by repo
        let repo_id = repo_ids[i];
        if !grpc_scan.client_stubs.is_empty() {
            grpc_clients_by_repo
                .entry(repo_id)
                .or_default()
                .extend(grpc_scan.client_stubs);
        }
        if !grpc_scan.server_registrations.is_empty() {
            grpc_servers_by_repo
                .entry(repo_id)
                .or_default()
                .extend(grpc_scan.server_registrations);
        }

        let mut string_remap: rustc_hash::FxHashMap<u32, u32> =
            rustc_hash::FxHashMap::default();
        let mut id_remap: rustc_hash::FxHashMap<u32, u32> = rustc_hash::FxHashMap::default();

        // Remap nodes — assign sequential IDs
        for mut node in result.nodes {
            let old_id = node.id;
            let new_id = all_nodes.len() as u32;
            id_remap.insert(old_id, new_id);
            node.id = new_id;
            node.name = remap_string(
                node.name,
                &file_strings,
                &mut merged_strings,
                &mut string_remap,
            );
            node.file = remap_string(
                node.file,
                &file_strings,
                &mut merged_strings,
                &mut string_remap,
            );
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

    // Merge proto nodes into the main node list
    {
        let mut proto_string_remap: rustc_hash::FxHashMap<u32, u32> =
            rustc_hash::FxHashMap::default();
        for mut node in all_proto_nodes {
            node.id = all_nodes.len() as u32;
            node.name = remap_string(
                node.name,
                &proto_strings,
                &mut merged_strings,
                &mut proto_string_remap,
            );
            node.file = remap_string(
                node.file,
                &proto_strings,
                &mut merged_strings,
                &mut proto_string_remap,
            );
            all_nodes.push(node);
        }
    }

    // Step 5b: Deduplicate nodes
    {
        let mut canonical_fileless: rustc_hash::FxHashMap<(u32, u8), u32> =
            rustc_hash::FxHashMap::default();
        let mut canonical_with_file: rustc_hash::FxHashMap<(u32, u8, u32), u32> =
            rustc_hash::FxHashMap::default();
        let mut dedup_remap: rustc_hash::FxHashMap<u32, u32> = rustc_hash::FxHashMap::default();
        let mut keep = vec![true; all_nodes.len()];

        for (idx, node) in all_nodes.iter().enumerate() {
            if node.file == u32::MAX {
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
            let mut idx = 0;
            all_nodes.retain(|_| {
                let k = keep[idx];
                idx += 1;
                k
            });

            let mut id_reassign: rustc_hash::FxHashMap<u32, u32> =
                rustc_hash::FxHashMap::default();
            for (new_idx, node) in all_nodes.iter_mut().enumerate() {
                id_reassign.insert(node.id, new_idx as u32);
                node.id = new_idx as u32;
            }
            for (dup_id, canonical_id) in &dedup_remap {
                if let Some(&new_id) = id_reassign.get(canonical_id) {
                    id_reassign.insert(*dup_id, new_id);
                }
            }

            for edge in &mut all_edges {
                if let Some(&new_id) = id_reassign.get(&edge.source) {
                    edge.source = new_id;
                }
                if let Some(&new_id) = id_reassign.get(&edge.target) {
                    edge.target = new_id;
                }
            }

            for call in &mut all_unresolved {
                if let Some(&new_id) = id_reassign.get(&call.caller_id) {
                    call.caller_id = new_id;
                }
            }

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
    let mut symbol_by_name: rustc_hash::FxHashMap<u32, u32> = rustc_hash::FxHashMap::default();
    for node in &all_nodes {
        if node.kind == NodeKind::Symbol as u8 {
            symbol_by_name.entry(node.name).or_insert(node.id);
        }
    }

    for call in &all_unresolved {
        if let Some(&target_id) = symbol_by_name.get(&call.target_name) {
            if target_id != call.caller_id {
                all_edges.push(EdgeInput::new(
                    call.caller_id,
                    target_id,
                    EdgeKind::Calls,
                ));
            }
        }
    }

    // Step 5d: Cross-repo endpoint resolution (name-based)
    resolve_cross_repo_endpoints(&all_nodes, &mut all_edges);

    // Build gRPC/proto output data
    let grpc_clients: Vec<(String, Vec<GrpcClientStub>)> = grpc_clients_by_repo
        .into_iter()
        .map(|(id, stubs)| {
            let name = repo_names.get(&id).cloned().unwrap_or_default();
            (name, stubs)
        })
        .collect();

    let grpc_servers: Vec<(String, Vec<GrpcServerRegistration>)> = grpc_servers_by_repo
        .into_iter()
        .map(|(id, regs)| {
            let name = repo_names.get(&id).cloned().unwrap_or_default();
            (name, regs)
        })
        .collect();

    let proto_services_out: Vec<(String, Vec<ProtoService>)> = all_proto_services
        .into_iter()
        .map(|(id, svcs)| {
            let name = repo_names.get(&id).cloned().unwrap_or_default();
            (name, svcs)
        })
        .collect();

    Ok(MergedResult {
        nodes: all_nodes,
        edges: all_edges,
        strings: merged_strings,
        file_count,
        errors: all_errors,
        grpc_clients,
        grpc_servers,
        proto_services: proto_services_out,
    })
}

/// Resolve cross-repo connections by matching Endpoint nodes with the same name
/// across different repos. Creates Connects edges between the nodes.
fn resolve_cross_repo_endpoints(nodes: &[Node], edges: &mut Vec<EdgeInput>) {
    use rustc_hash::FxHashMap;

    let mut endpoints_by_name: FxHashMap<u32, Vec<(u32, u16)>> = FxHashMap::default();
    for node in nodes {
        if node.kind == NodeKind::Endpoint as u8 {
            endpoints_by_name
                .entry(node.name)
                .or_default()
                .push((node.id, node.repo));
        }
    }

    for eps in endpoints_by_name.values() {
        if eps.len() < 2 {
            continue;
        }
        for i in 0..eps.len() {
            for j in (i + 1)..eps.len() {
                if eps[i].1 != eps[j].1 {
                    edges.push(EdgeInput::new(eps[i].0, eps[j].0, EdgeKind::Connects));
                    edges.push(EdgeInput::new(eps[j].0, eps[i].0, EdgeKind::Connects));
                }
            }
        }
    }
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
        assert!(
            result.errors.is_empty(),
            "should have no errors: {:?}",
            result.errors
        );

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

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        fs::write(dir.path().join(".gitignore"), "vendor/\n").unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc included() {}\n",
        )
        .unwrap();
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
        assert!(
            !names.contains(&"excluded"),
            "should not find excluded (gitignored)"
        );
    }

    #[test]
    fn index_respects_gitignore_venv_and_nested() {
        let dir = tempfile::tempdir().unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        fs::write(
            dir.path().join(".gitignore"),
            "venv/\nnode_modules/\n*.generated.go\n",
        )
        .unwrap();

        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc keepMe() {}\n",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("venv/lib")).unwrap();
        fs::write(
            dir.path().join("venv/lib/setup.go"),
            "package lib\nfunc venvFunc() {}\n",
        )
        .unwrap();
        fs::create_dir(dir.path().join("node_modules")).unwrap();
        fs::write(
            dir.path().join("node_modules/pkg.go"),
            "package pkg\nfunc nodeFunc() {}\n",
        )
        .unwrap();
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
        assert!(
            !names.contains(&"generatedFunc"),
            "should ignore *.generated.go"
        );
    }

    #[test]
    fn cross_package_call_resolution() {
        let dir = tempfile::tempdir().unwrap();

        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();

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

        let main_id = result
            .graph
            .nodes
            .iter()
            .find(|n| {
                result.graph.strings.get(n.name) == "main" && n.kind == NodeKind::Symbol as u8
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

        let has_cross_call = result
            .graph
            .edges_for(
                result
                    .graph
                    .nodes
                    .iter()
                    .position(|n| n.id == main_id)
                    .unwrap() as u32,
            )
            .iter()
            .any(|e| {
                let target_node = result.graph.node(e.target);
                result.graph.strings.get(target_node.name) == "StartWSServer"
                    && e.kind == EdgeKind::Calls as u8
            });

        assert!(
            has_cross_call,
            "main should have a Calls edge to StartWSServer (cross-package)"
        );

        let start_ws_idx = result
            .graph
            .nodes
            .iter()
            .position(|n| n.id == start_ws_id)
            .unwrap() as u32;
        let has_intra_call = result.graph.edges_for(start_ws_idx).iter().any(|e| {
            let target_node = result.graph.node(e.target);
            result.graph.strings.get(target_node.name) == "loggingMiddleware"
                && e.kind == EdgeKind::Calls as u8
        });

        assert!(
            has_intra_call,
            "StartWSServer should call loggingMiddleware (intra-file)"
        );
    }

    #[test]
    fn index_produces_valid_graph() {
        let dir = tempfile::tempdir().unwrap();
        create_go_project(dir.path());

        let result = index_directory(dir.path()).unwrap();

        let graph = &result.graph;
        assert!(graph.node_count() > 0);
        assert_eq!(graph.offsets.len() as u32, graph.node_count() + 1);
        assert_eq!(graph.rev_offsets.len() as u32, graph.node_count() + 1);
    }

    #[test]
    fn real_go_repo_structure() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::create_dir_all(root.join("cmd/server")).unwrap();
        fs::write(
            root.join("cmd/server/main.go"),
            "package main\nimport \"fmt\"\nfunc main() { fmt.Println(\"server starting\") }\n",
        )
        .unwrap();

        fs::create_dir_all(root.join("cmd/migrate")).unwrap();
        fs::write(
            root.join("cmd/migrate/main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();

        fs::create_dir_all(root.join("pkg/auth")).unwrap();
        fs::write(
            root.join("pkg/auth/login.go"),
            "package auth\nfunc Login() {}\n",
        )
        .unwrap();
        fs::write(
            root.join("pkg/auth/token.go"),
            "package auth\nfunc ValidateToken() {}\n",
        )
        .unwrap();

        fs::create_dir_all(root.join("internal/db")).unwrap();
        fs::write(
            root.join("internal/db/query.go"),
            "package db\nfunc Query() {}\n",
        )
        .unwrap();

        let result = index_directory(root).unwrap();
        let graph = &result.graph;

        let deployables: Vec<&str> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Deployable as u8)
            .map(|n| graph.strings.get(n.name))
            .collect();
        assert_eq!(
            deployables.len(),
            2,
            "should have 2 deployables, got: {:?}",
            deployables
        );

        let modules: Vec<&str> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module as u8)
            .map(|n| graph.strings.get(n.name))
            .collect();
        assert!(modules.contains(&"auth"), "should have auth module");
        assert!(modules.contains(&"db"), "should have db module");
        assert!(
            !modules.contains(&"main"),
            "main should be Deployable, not Module"
        );

        let symbols: Vec<&str> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| graph.strings.get(n.name))
            .collect();
        assert!(symbols.contains(&"Login"), "should find Login");
        assert!(
            symbols.contains(&"ValidateToken"),
            "should find ValidateToken"
        );
        assert!(symbols.contains(&"Query"), "should find Query");
        assert_eq!(
            symbols.iter().filter(|&&s| s == "main").count(),
            2,
            "should have 2 main functions"
        );
    }

    #[test]
    fn extractor_parse_failure_nonfatal() {
        let dir = tempfile::tempdir().unwrap();

        for i in 0..10 {
            let content = if i == 5 {
                "package main\n\nfunc {{{ invalid syntax @@@ }}}\n".to_string()
            } else {
                format!("package main\n\nfunc func_{}() {{}}\n", i)
            };
            fs::write(dir.path().join(format!("file_{}.go", i)), content).unwrap();
        }

        let result = index_directory(dir.path()).unwrap();

        let func_names: Vec<&str> = result
            .graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| result.graph.strings.get(n.name))
            .collect();

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
    }

    #[test]
    fn index_repos_multi_repo_unified_graph() {
        let repo_a = tempfile::tempdir().unwrap();
        let repo_b = tempfile::tempdir().unwrap();

        fs::write(
            repo_a.path().join("server.go"),
            "package main\nfunc ServeHTTP() {}\nfunc handleAuth() {}\n",
        )
        .unwrap();

        fs::write(
            repo_b.path().join("client.go"),
            "package main\nfunc CallServer() {}\nfunc retry() {}\n",
        )
        .unwrap();

        let repos = vec![
            (repo_a.path().to_path_buf(), 0u16),
            (repo_b.path().to_path_buf(), 1u16),
        ];
        let result = index_repos(&repos).unwrap();

        let names: Vec<&str> = result
            .graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8)
            .map(|n| result.graph.strings.get(n.name))
            .collect();

        assert!(names.contains(&"ServeHTTP"), "should have repo A symbols");
        assert!(names.contains(&"handleAuth"), "should have repo A symbols");
        assert!(names.contains(&"CallServer"), "should have repo B symbols");
        assert!(names.contains(&"retry"), "should have repo B symbols");

        let repo_a_nodes: Vec<_> = result.graph.nodes.iter().filter(|n| n.repo == 0).collect();
        let repo_b_nodes: Vec<_> = result.graph.nodes.iter().filter(|n| n.repo == 1).collect();
        assert!(!repo_a_nodes.is_empty(), "should have repo 0 nodes");
        assert!(!repo_b_nodes.is_empty(), "should have repo 1 nodes");
    }

    #[test]
    fn extract_collects_grpc_data() {
        let repo = tempfile::tempdir().unwrap();

        fs::write(
            repo.path().join("server.go"),
            r#"package main

import pb "example.com/proto/order"

func main() {
    s := grpc.NewServer()
    pb.RegisterOrderProcessingServer(s, &handler{})
    s.Serve(lis)
}
"#,
        )
        .unwrap();

        fs::write(
            repo.path().join("client.go"),
            r#"package main

import pb "example.com/proto/order"

func callService() {
    conn, _ := grpc.Dial("localhost:50051")
    client := pb.NewOrderProcessingClient(conn)
    _ = client
}
"#,
        )
        .unwrap();

        let merged = extract_and_merge_repos(&[(repo.path().to_path_buf(), 0)]).unwrap();

        let all_servers: Vec<&str> = merged
            .grpc_servers
            .iter()
            .flat_map(|(_, regs)| regs.iter().map(|r| r.service_name.as_str()))
            .collect();
        let all_clients: Vec<&str> = merged
            .grpc_clients
            .iter()
            .flat_map(|(_, stubs)| stubs.iter().map(|s| s.service_name.as_str()))
            .collect();

        assert!(
            all_servers.contains(&"OrderProcessing"),
            "should detect RegisterOrderProcessingServer, got: {:?}",
            all_servers
        );
        assert!(
            all_clients.contains(&"OrderProcessing"),
            "should detect NewOrderProcessingClient, got: {:?}",
            all_clients
        );
    }

    #[test]
    fn extract_collects_proto_services() {
        let repo = tempfile::tempdir().unwrap();

        fs::write(
            repo.path().join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();

        fs::create_dir(repo.path().join("proto")).unwrap();
        fs::write(
            repo.path().join("proto/service.proto"),
            r#"syntax = "proto3";
package myapp;
service Auth {
  rpc Login (LoginRequest) returns (LoginResponse);
}
"#,
        )
        .unwrap();

        let merged = extract_and_merge_repos(&[(repo.path().to_path_buf(), 0)]).unwrap();

        let all_services: Vec<&str> = merged
            .proto_services
            .iter()
            .flat_map(|(_, svcs)| svcs.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(
            all_services.contains(&"Auth"),
            "should extract proto service, got: {:?}",
            all_services
        );

        // Proto nodes should also appear in the node list
        let node_names: Vec<&str> = merged
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Surface as u8 || n.kind == NodeKind::Endpoint as u8)
            .map(|n| merged.strings.get(n.name))
            .collect();
        assert!(
            node_names.iter().any(|n| n.contains("Auth")),
            "proto Surface/Endpoint nodes should be in the graph, got: {:?}",
            node_names
        );
    }
}
