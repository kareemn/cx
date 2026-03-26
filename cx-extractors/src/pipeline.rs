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
    /// HTTP server routes extracted from code: (repo_name, routes)
    pub http_server_routes: Vec<(String, Vec<HttpServerRoute>)>,
    /// HTTP client calls extracted from code: (repo_name, calls)
    pub http_client_calls: Vec<(String, Vec<HttpClientCall>)>,
    /// Env var reads extracted from code: (repo_name, reads)
    pub env_var_reads: Vec<(String, Vec<EnvVarRead>)>,
    /// Env var definitions from Helm/k8s manifests: (repo_name, defs)
    pub helm_env_defs: Vec<(String, Vec<HelmEnvDef>)>,
    /// Docker images from Dockerfiles: (repo_name, images)
    pub docker_images: Vec<(String, Vec<DockerImage>)>,
    /// Container images from k8s manifests: (repo_name, images)
    pub k8s_container_images: Vec<(String, Vec<K8sContainerImage>)>,
    /// WebSocket server endpoints: (repo_name, endpoints)
    pub ws_servers: Vec<(String, Vec<WsServerEndpoint>)>,
    /// WebSocket client connections: (repo_name, connections)
    pub ws_clients: Vec<(String, Vec<WsClientConnection>)>,
}

/// An HTTP server route, for passing to the resolution engine.
#[derive(Debug, Clone)]
pub struct HttpServerRoute {
    pub path: String,
    pub method: String,
    pub framework: String,
    pub file: String,
    pub line: u32,
}

/// An HTTP client call, for passing to the resolution engine.
#[derive(Debug, Clone)]
pub struct HttpClientCall {
    pub path: String,
    pub method: String,
    pub base_url_env_var: Option<String>,
    pub file: String,
    pub line: u32,
}

/// An env var read from code.
#[derive(Debug, Clone)]
pub struct EnvVarRead {
    pub var_name: String,
    pub file: String,
    pub line: u32,
}

/// An env var definition from Helm/k8s YAML.
#[derive(Debug, Clone)]
pub struct HelmEnvDef {
    pub var_name: String,
    pub value: String,
    pub file: String,
    pub line: u32,
}

/// A Docker image reference from a Dockerfile.
#[derive(Debug, Clone)]
pub struct DockerImage {
    pub image_ref: String,
    pub file: String,
}

/// A container image reference from k8s manifests.
#[derive(Debug, Clone)]
pub struct K8sContainerImage {
    pub image_ref: String,
    pub file: String,
    pub line: u32,
    pub deployment_name: Option<String>,
}

/// A WebSocket server endpoint.
#[derive(Debug, Clone)]
pub struct WsServerEndpoint {
    pub path: String,
    pub file: String,
    pub line: u32,
}

/// A WebSocket client connection.
#[derive(Debug, Clone)]
pub struct WsClientConnection {
    pub url_or_path: String,
    pub file: String,
    pub line: u32,
}

/// A file to index, tagged with its repo root and repo ID.
struct RepoFile {
    path: PathBuf,
    root: PathBuf,
    repo_id: u16,
}

/// Detect test files across all supported languages.
/// Returns true for files that are test/spec fixtures and should not
/// contribute to production graph edges (Endpoint, DependsOn, etc.).
fn is_test_file(path: &str) -> bool {
    let file_name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Go: *_test.go
    if file_name.ends_with("_test.go") {
        return true;
    }
    // Rust: tests/ directory or #[cfg(test)] (directory-level only)
    if path.contains("/tests/") || file_name.ends_with("_test.rs") {
        return true;
    }
    // TypeScript/JavaScript: *.test.ts, *.test.js, *.spec.ts, *.spec.js, *.test.tsx, *.test.jsx
    for ext in &[".test.ts", ".test.tsx", ".test.js", ".test.jsx",
                 ".spec.ts", ".spec.tsx", ".spec.js", ".spec.jsx",
                 ".test-d.ts"] {
        if file_name.ends_with(ext) {
            return true;
        }
    }
    // Python: test_*.py, *_test.py
    if file_name.ends_with(".py") {
        let stem = file_name.strip_suffix(".py").unwrap_or("");
        if stem.starts_with("test_") || stem.ends_with("_test") {
            return true;
        }
    }
    // Java: *Test.java, *Tests.java, *IT.java
    if file_name.ends_with("Test.java") || file_name.ends_with("Tests.java")
        || file_name.ends_with("IT.java")
    {
        return true;
    }
    // C/C++: *_test.cc, *_test.cpp, test_*.c
    for ext in &["_test.cc", "_test.cpp", "_test.c"] {
        if file_name.ends_with(ext) {
            return true;
        }
    }
    // Directory-level patterns: test/, tests/, __tests__/, spec/
    let path_lower = path.to_lowercase();
    if path_lower.contains("/test/")
        || path_lower.contains("/__tests__/")
        || path_lower.contains("/spec/")
        || path_lower.contains("/testdata/")
        || path_lower.contains("/fixtures/")
    {
        return true;
    }
    false
}

/// Detect generated protobuf/gRPC files that contain stubs for ALL services.
/// These should not be scanned for gRPC patterns because they produce
/// server registrations and client stubs for every service in the proto package,
/// creating massive false-positive matches.
fn is_generated_proto_file(path: &str) -> bool {
    let file_name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    // Python: *_pb2_grpc.py, *_pb2.py
    if file_name.ends_with("_pb2_grpc.py") || file_name.ends_with("_pb2.py") {
        return true;
    }
    // Go: *.pb.go, *_grpc.pb.go
    if file_name.ends_with(".pb.go") {
        return true;
    }
    // TypeScript/JS: *_grpc_pb.js, *_pb.js, *_grpc_pb.d.ts
    if file_name.ends_with("_grpc_pb.js")
        || file_name.ends_with("_pb.js")
        || file_name.ends_with("_grpc_pb.d.ts")
    {
        return true;
    }
    // Java: *Grpc.java (generated by protoc-gen-grpc-java)
    if file_name.ends_with("Grpc.java") {
        return true;
    }
    false
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
    let mut infra_files: Vec<RepoFile> = Vec::new();

    for (root, repo_id) in repos {
        let files = collect_files(root)?;
        for path in files {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if ext == "proto" {
                proto_files.push(RepoFile {
                    path,
                    root: root.clone(),
                    repo_id: *repo_id,
                });
            } else if file_name.starts_with("Dockerfile")
                || ext == "yaml"
                || ext == "yml"
                || file_name.ends_with(".yaml.gotmpl")
                || file_name.ends_with(".yml.gotmpl")
            {
                // Keep in both lists — source files also get tree-sitter extraction
                infra_files.push(RepoFile {
                    path: path.clone(),
                    root: root.clone(),
                    repo_id: *repo_id,
                });
                if Language::from_path(&path).is_some() {
                    all_repo_files.push(RepoFile {
                        path,
                        root: root.clone(),
                        repo_id: *repo_id,
                    });
                }
            } else {
                all_repo_files.push(RepoFile {
                    path,
                    root: root.clone(),
                    repo_id: *repo_id,
                });
            }
        }
    }

    if all_repo_files.is_empty() && proto_files.is_empty() && infra_files.is_empty() {
        return Ok(MergedResult {
            nodes: vec![],
            edges: vec![],
            strings: StringInterner::new(),
            file_count: 0,
            errors: vec![],
            grpc_clients: vec![],
            grpc_servers: vec![],
            proto_services: vec![],
            http_server_routes: vec![],
            http_client_calls: vec![],
            env_var_reads: vec![],
            helm_env_defs: vec![],
            docker_images: vec![],
            k8s_container_images: vec![],
            ws_servers: vec![],
            ws_clients: vec![],
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

                // Mark all nodes from test files with NODE_IS_TEST flag
                if is_test_file(&path_str) {
                    for node in &mut result.nodes {
                        node.flags |= cx_core::graph::nodes::NODE_IS_TEST;
                    }
                }

                // Scan for gRPC patterns — skip test files and generated proto stubs
                if !is_test_file(&path_str) && !is_generated_proto_file(&path_str) {
                    grpc_result = match lang {
                        Language::Go => grpc::scan_go_grpc(&file.tree, &source, &path_str, &ts_lang),
                        Language::Python => grpc::scan_python_grpc(&source, &path_str),
                        Language::TypeScript => grpc::scan_js_grpc(&source, &path_str),
                        _ => grpc_result,
                    };
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

        // Dedup key for Deployable/Module: (name, kind, repo) — one per package per repo
        let mut canonical_by_name_kind_repo: rustc_hash::FxHashMap<(u32, u8, u16), u32> =
            rustc_hash::FxHashMap::default();

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
            } else if node.kind == NodeKind::Deployable as u8
                || node.kind == NodeKind::Module as u8
            {
                // Deduplicate by (name, kind, repo) — one Deployable/Module per package per repo
                let key = (node.name, node.kind, node.repo);
                match canonical_by_name_kind_repo.entry(key) {
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
    resolve_cross_repo_endpoints(&all_nodes, &mut all_edges, &merged_strings);

    // Step 6: Scan extracted nodes to build resolution data
    let (http_servers_by_repo, http_clients_by_repo, ws_servers_by_repo, ws_clients_by_repo, envvar_reads_by_repo) =
        scan_nodes_for_resolution(&all_nodes, &all_edges, &merged_strings);

    // Step 7: Parse infrastructure files (Dockerfiles, Helm/k8s YAML)
    let (docker_by_repo, k8s_images_by_repo, helm_env_by_repo) =
        parse_infra_files(&infra_files);

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

    // Build HTTP/WS/env resolution output keyed by repo name
    let http_server_routes = keyed_by_repo_name(&repo_names, http_servers_by_repo);
    let http_client_calls = keyed_by_repo_name(&repo_names, http_clients_by_repo);
    let ws_servers = keyed_by_repo_name(&repo_names, ws_servers_by_repo);
    let ws_clients = keyed_by_repo_name(&repo_names, ws_clients_by_repo);
    let env_var_reads = keyed_by_repo_name(&repo_names, envvar_reads_by_repo);
    let helm_env_defs = keyed_by_repo_name(&repo_names, helm_env_by_repo);
    let docker_images = keyed_by_repo_name(&repo_names, docker_by_repo);
    let k8s_container_images = keyed_by_repo_name(&repo_names, k8s_images_by_repo);

    Ok(MergedResult {
        nodes: all_nodes,
        edges: all_edges,
        strings: merged_strings,
        file_count,
        errors: all_errors,
        grpc_clients,
        grpc_servers,
        proto_services: proto_services_out,
        http_server_routes,
        http_client_calls,
        env_var_reads,
        helm_env_defs,
        docker_images,
        k8s_container_images,
        ws_servers,
        ws_clients,
    })
}

/// Resolve cross-repo connections by matching Endpoint nodes with the same name
/// across different repos. Creates Connects edges between the nodes.
/// Skips generic endpoint names that would create too many false-positive connections.
fn resolve_cross_repo_endpoints(
    nodes: &[Node],
    edges: &mut Vec<EdgeInput>,
    strings: &StringInterner,
) {
    use rustc_hash::FxHashMap;

    // Generic names that match too broadly across repos
    const SKIP_NAMES: &[&str] = &[
        "websocket", "/", "/health", "/healthz", "health", "index",
    ];

    let mut endpoints_by_name: FxHashMap<u32, Vec<(u32, u16)>> = FxHashMap::default();
    for node in nodes {
        if node.kind == NodeKind::Endpoint as u8 {
            let name = strings.get(node.name);
            // Skip generic names and very short paths
            if name.len() <= 1 || SKIP_NAMES.contains(&name) {
                continue;
            }
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

/// Convert a repo_id-keyed map into a repo_name-keyed Vec.
fn keyed_by_repo_name<T>(
    repo_names: &rustc_hash::FxHashMap<u16, String>,
    by_repo: rustc_hash::FxHashMap<u16, Vec<T>>,
) -> Vec<(String, Vec<T>)> {
    by_repo
        .into_iter()
        .map(|(id, items)| {
            let name = repo_names.get(&id).cloned().unwrap_or_default();
            (name, items)
        })
        .collect()
}

/// Scan extracted nodes and edges to build resolution input data.
///
/// Identifies HTTP server routes vs client calls by looking at edge types:
/// - Endpoint with incoming Exposes edge → server route
/// - Endpoint with incoming Connects edge → client call
/// - sub_kind=0 → HTTP, sub_kind=1 → WebSocket
/// - Resource nodes with names matching env var patterns → env var reads
#[allow(clippy::type_complexity)]
fn scan_nodes_for_resolution(
    nodes: &[Node],
    edges: &[EdgeInput],
    strings: &StringInterner,
) -> (
    rustc_hash::FxHashMap<u16, Vec<HttpServerRoute>>,
    rustc_hash::FxHashMap<u16, Vec<HttpClientCall>>,
    rustc_hash::FxHashMap<u16, Vec<WsServerEndpoint>>,
    rustc_hash::FxHashMap<u16, Vec<WsClientConnection>>,
    rustc_hash::FxHashMap<u16, Vec<EnvVarRead>>,
) {
    let mut http_servers: rustc_hash::FxHashMap<u16, Vec<HttpServerRoute>> =
        rustc_hash::FxHashMap::default();
    let mut http_clients: rustc_hash::FxHashMap<u16, Vec<HttpClientCall>> =
        rustc_hash::FxHashMap::default();
    let mut ws_servers: rustc_hash::FxHashMap<u16, Vec<WsServerEndpoint>> =
        rustc_hash::FxHashMap::default();
    let mut ws_clients: rustc_hash::FxHashMap<u16, Vec<WsClientConnection>> =
        rustc_hash::FxHashMap::default();
    let mut envvar_reads: rustc_hash::FxHashMap<u16, Vec<EnvVarRead>> =
        rustc_hash::FxHashMap::default();

    // Build a set of node IDs that are targets of Exposes edges
    let mut exposes_targets: rustc_hash::FxHashSet<u32> = rustc_hash::FxHashSet::default();
    let mut connects_targets: rustc_hash::FxHashSet<u32> = rustc_hash::FxHashSet::default();
    let mut configures_targets: rustc_hash::FxHashSet<u32> = rustc_hash::FxHashSet::default();

    for edge in edges {
        match edge.kind {
            EdgeKind::Exposes => { exposes_targets.insert(edge.target); }
            EdgeKind::Connects => { connects_targets.insert(edge.target); }
            EdgeKind::Configures => { configures_targets.insert(edge.target); }
            _ => {}
        }
    }

    for node in nodes {
        // Skip test nodes — they should not contribute to resolution
        if node.flags & cx_core::graph::nodes::NODE_IS_TEST != 0 {
            continue;
        }

        let name = strings.get(node.name);
        let file = if node.file != u32::MAX { strings.get(node.file) } else { "" };

        if node.kind == NodeKind::Endpoint as u8 {
            let is_server = exposes_targets.contains(&node.id);
            let is_client = connects_targets.contains(&node.id);
            let is_ws = node.sub_kind == 1;

            if is_ws {
                if is_server {
                    ws_servers.entry(node.repo).or_default().push(WsServerEndpoint {
                        path: name.to_string(),
                        file: file.to_string(),
                        line: node.line,
                    });
                }
                if is_client || !is_server {
                    ws_clients.entry(node.repo).or_default().push(WsClientConnection {
                        url_or_path: name.to_string(),
                        file: file.to_string(),
                        line: node.line,
                    });
                }
            } else {
                // HTTP (sub_kind=0)
                if is_server {
                    http_servers.entry(node.repo).or_default().push(HttpServerRoute {
                        path: name.to_string(),
                        method: String::new(),
                        framework: String::new(),
                        file: file.to_string(),
                        line: node.line,
                    });
                }
                if is_client || !is_server {
                    http_clients.entry(node.repo).or_default().push(HttpClientCall {
                        path: name.to_string(),
                        method: String::new(),
                        base_url_env_var: None,
                        file: file.to_string(),
                        line: node.line,
                    });
                }
            }
        }

        // Env var reads: Resource nodes targeted by Configures edges
        // whose names look like env var names (uppercase with underscores)
        if node.kind == NodeKind::Resource as u8
            && configures_targets.contains(&node.id)
            && looks_like_env_var(name)
        {
            envvar_reads.entry(node.repo).or_default().push(EnvVarRead {
                var_name: name.to_string(),
                file: file.to_string(),
                line: node.line,
            });
        }
    }

    (http_servers, http_clients, ws_servers, ws_clients, envvar_reads)
}

/// Check if a string looks like an environment variable name.
fn looks_like_env_var(name: &str) -> bool {
    if name.len() < 2 {
        return false;
    }
    // Must be mostly uppercase letters and underscores
    let alpha_count = name.chars().filter(|c| c.is_ascii_alphabetic()).count();
    if alpha_count == 0 {
        return false;
    }
    let upper_count = name.chars().filter(|c| c.is_ascii_uppercase()).count();
    // At least 60% uppercase of alphabetic chars
    upper_count * 100 / alpha_count >= 60
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Parse infrastructure files (Dockerfiles, Helm/k8s YAML) to extract
/// Docker image references, env var definitions, and k8s container images.
#[allow(clippy::type_complexity)]
fn parse_infra_files(
    infra_files: &[RepoFile],
) -> (
    rustc_hash::FxHashMap<u16, Vec<DockerImage>>,
    rustc_hash::FxHashMap<u16, Vec<K8sContainerImage>>,
    rustc_hash::FxHashMap<u16, Vec<HelmEnvDef>>,
) {
    let mut docker_images: rustc_hash::FxHashMap<u16, Vec<DockerImage>> =
        rustc_hash::FxHashMap::default();
    let mut k8s_images: rustc_hash::FxHashMap<u16, Vec<K8sContainerImage>> =
        rustc_hash::FxHashMap::default();
    let mut helm_envs: rustc_hash::FxHashMap<u16, Vec<HelmEnvDef>> =
        rustc_hash::FxHashMap::default();

    for rf in infra_files {
        let Ok(content) = std::fs::read_to_string(&rf.path) else { continue };
        let rel_path = rf
            .path
            .strip_prefix(&rf.root)
            .unwrap_or(&rf.path)
            .to_string_lossy()
            .to_string();

        let file_name = rf.path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if file_name.starts_with("Dockerfile") {
            // Parse Dockerfile: extract FROM image references
            for line in content.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("FROM ") {
                    // FROM image:tag AS alias
                    let image_ref = rest.split_whitespace().next().unwrap_or("").to_string();
                    if !image_ref.is_empty() && image_ref != "scratch" {
                        docker_images.entry(rf.repo_id).or_default().push(DockerImage {
                            image_ref,
                            file: rel_path.clone(),
                        });
                    }
                }
            }
        } else {
            // YAML: parse env var definitions and container image references
            parse_yaml_for_resolution(&content, &rel_path, rf.repo_id, &mut k8s_images, &mut helm_envs);
        }
    }

    (docker_images, k8s_images, helm_envs)
}

/// Resolve a Go template value to its most useful form.
/// - `{{ .Values.foo | default "http://bar" }}` → `http://bar`
/// - `{{ .Values.foo }}` → `.Values.foo` (keep the reference)
/// - `http://plain-value` → `http://plain-value`
/// - Mixed: `http://{{ .Values.host }}:8080/path` → `http://{{ .Values.host }}:8080/path` (keep as-is)
fn resolve_gotmpl_value(raw: &str) -> String {
    let s = raw.trim();

    // Not a template — return as-is
    if !s.contains("{{") {
        return s.to_string();
    }

    // Extract `default "value"` from template expressions
    // Pattern: {{ ... | default "value" }}  or  {{ ... | default `value` }}
    if let Some(default_idx) = s.find("default ") {
        let after_default = &s[default_idx + 8..];
        let after_default = after_default.trim();
        // Find the quoted default value
        if let Some(start_quote) = after_default.find(['"', '\'', '`'])
        {
            let quote_char = after_default.as_bytes()[start_quote] as char;
            let rest = &after_default[start_quote + 1..];
            if let Some(end_quote) = rest.find(quote_char) {
                return rest[..end_quote].to_string();
            }
        }
        // Unquoted default value (up to space, }, or end)
        let unquoted = after_default
            .split([' ', '}'])
            .next()
            .unwrap_or("");
        if !unquoted.is_empty() {
            return unquoted.to_string();
        }
    }

    // Pure template without default: extract the .Values reference
    if s.starts_with("{{") && s.ends_with("}}") {
        let inner = s.trim_start_matches('{').trim_end_matches('}').trim();
        let inner = inner.split('|').next().unwrap_or(inner).trim();
        if !inner.is_empty() {
            return inner.to_string();
        }
    }

    // Mixed template + literal (e.g., "http://{{ .Values.host }}:8080/path")
    s.to_string()
}

/// Simple line-based YAML parser to extract env var definitions and container images.
/// Not a full YAML parser — handles the common patterns in k8s manifests and Helm charts.
fn parse_yaml_for_resolution(
    content: &str,
    file: &str,
    repo_id: u16,
    k8s_images: &mut rustc_hash::FxHashMap<u16, Vec<K8sContainerImage>>,
    helm_envs: &mut rustc_hash::FxHashMap<u16, Vec<HelmEnvDef>>,
) {
    let lines: Vec<&str> = content.lines().collect();
    let mut pending_env_name: Option<String> = None;
    let mut pending_env_line: u32 = 0;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = (i + 1) as u32;

        // Container image references: "image: gcr.io/org/app:tag"
        if let Some(rest) = trimmed.strip_prefix("image:") {
            let image_ref = rest.trim().trim_matches('"').trim_matches('\'').to_string();
            if !image_ref.is_empty() && !image_ref.starts_with("{{") {
                k8s_images.entry(repo_id).or_default().push(K8sContainerImage {
                    image_ref,
                    file: file.to_string(),
                    line: line_num,
                    deployment_name: None,
                });
            }
        }

        // Env var definitions: look for "name: VAR_NAME" followed by "value: ..."
        if let Some(rest) = trimmed.strip_prefix("name:") {
            let name = rest.trim().trim_matches('"').trim_matches('\'');
            if looks_like_env_var(name) {
                pending_env_name = Some(name.to_string());
                pending_env_line = line_num;
            }
        } else if let Some(rest) = trimmed.strip_prefix("value:") {
            if let Some(var_name) = pending_env_name.take() {
                let raw = rest.trim().trim_matches('"').trim_matches('\'');
                // Handle Go template expressions: extract default value or the template itself
                let value = resolve_gotmpl_value(raw);
                if !value.is_empty() {
                    helm_envs.entry(repo_id).or_default().push(HelmEnvDef {
                        var_name,
                        value,
                        file: file.to_string(),
                        line: pending_env_line,
                    });
                }
            }
        } else if !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && !trimmed.starts_with("value")
            && !trimmed.starts_with("valueFrom")
        {
            // Reset pending env name if we hit a non-value line
            if pending_env_name.is_some() && !trimmed.starts_with('-') {
                pending_env_name = None;
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
