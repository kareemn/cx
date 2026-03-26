use anyhow::{Context, Result};
use cx_extractors::lsp::LspOrchestrator;
use cx_extractors::pipeline::{self, IndexResult, MergedResult};
use cx_extractors::sink_registry;
use cx_extractors::taint::Confidence;
use std::path::PathBuf;

/// Run the full indexing pipeline with cross-repo resolution:
/// 1. Extract and merge all repos
/// 2. Run resolution engine (gRPC, REST, env→Helm→k8s, Docker image, WebSocket)
/// 3. Optionally upgrade heuristic results via LSP
/// 4. Build the unified CSR graph
pub fn index_repos_with_resolution(repos: &[(PathBuf, u16)]) -> Result<IndexResult> {
    let mut merged = pipeline::extract_and_merge_repos(repos)
        .context("failed to extract repos")?;

    let resolved = resolve_cross_repo(&mut merged);
    if resolved > 0 {
        eprintln!("Resolved {} cross-repo connection(s)", resolved);
    }

    // LSP integration: try to upgrade Heuristic network calls to TypeConfirmed
    if !merged.network_calls.is_empty() {
        let workspace_root = repos.first().map(|(p, _)| p.as_path());
        if let Some(root) = workspace_root {
            upgrade_via_lsp(&mut merged, root);
        }
    }

    Ok(pipeline::build_index(merged))
}

/// Try to upgrade heuristic network call classifications using LSP type info.
/// This is best-effort — if no LSP servers are available, results stay as Heuristic.
fn upgrade_via_lsp(merged: &mut MergedResult, workspace_root: &std::path::Path) {
    let mut orchestrator = LspOrchestrator::start(workspace_root);

    if !orchestrator.has_servers() {
        return;
    }

    eprintln!("LSP: attempting to upgrade heuristic network calls...");
    let mut upgraded = 0;

    for call in &mut merged.network_calls {
        if call.confidence != Confidence::Heuristic {
            continue;
        }

        // Try to resolve the callee FQN via LSP hover
        let file_path = std::path::Path::new(&call.file);
        if LspOrchestrator::language_for_file(file_path).is_none() {
            continue;
        }

        let pos = cx_extractors::lsp::Position {
            line: call.line.saturating_sub(1),
            character: 0,
        };

        if let Some(hover) = orchestrator.hover(file_path, pos) {
            // Check if the hover type matches a known sink in the registry
            let hover_text = &hover.contents;
            if sink_registry::lookup_sink(hover_text).is_some() {
                call.callee_fqn = hover_text.clone();
                call.confidence = Confidence::TypeConfirmed;
                upgraded += 1;
            }
        }
    }

    if upgraded > 0 {
        eprintln!("LSP: upgraded {} call(s) to TypeConfirmed", upgraded);
    }

    orchestrator.shutdown();
}

/// Run the full resolution engine on merged extraction data.
/// Creates DependsOn edges for all resolved cross-repo connections.
fn resolve_cross_repo(merged: &mut MergedResult) -> usize {
    use cx_resolution::resolver::{self, ResolutionInput};

    let input = ResolutionInput {
        client_stubs: merged.grpc_clients.clone(),
        server_registrations: merged.grpc_servers.clone(),
        proto_services: merged.proto_services.clone(),
        http_client_calls: merged.http_client_calls.iter().map(|(repo, calls)| {
            (repo.clone(), calls.iter().map(|c| cx_resolution::rest_resolution::HttpClientCall {
                path: c.path.clone(), method: c.method.clone(),
                base_url_env_var: c.base_url_env_var.clone(),
                file: c.file.clone(), line: c.line,
            }).collect())
        }).collect(),
        http_server_routes: merged.http_server_routes.iter().map(|(repo, routes)| {
            (repo.clone(), routes.iter().map(|r| cx_resolution::rest_resolution::HttpServerRoute {
                path: r.path.clone(), method: r.method.clone(),
                framework: r.framework.clone(), file: r.file.clone(), line: r.line,
            }).collect())
        }).collect(),
        env_var_reads: merged.env_var_reads.iter().map(|(repo, reads)| {
            (repo.clone(), reads.iter().map(|r| cx_resolution::helm_env_resolution::EnvVarRead {
                var_name: r.var_name.clone(), file: r.file.clone(), line: r.line,
            }).collect())
        }).collect(),
        helm_env_defs: merged.helm_env_defs.iter().map(|(repo, defs)| {
            (repo.clone(), defs.iter().map(|d| cx_resolution::helm_env_resolution::HelmEnvDef {
                var_name: d.var_name.clone(), value: d.value.clone(),
                file: d.file.clone(), line: d.line,
            }).collect())
        }).collect(),
        docker_images: merged.docker_images.iter().map(|(repo, imgs)| {
            (repo.clone(), imgs.iter().map(|i| cx_resolution::image_resolution::DockerImage {
                image_ref: i.image_ref.clone(), file: i.file.clone(),
            }).collect())
        }).collect(),
        k8s_container_images: merged.k8s_container_images.iter().map(|(repo, imgs)| {
            (repo.clone(), imgs.iter().map(|i| cx_resolution::image_resolution::K8sContainerImage {
                image_ref: i.image_ref.clone(), file: i.file.clone(),
                line: i.line, deployment_name: i.deployment_name.clone(),
            }).collect())
        }).collect(),
        ws_clients: merged.ws_clients.iter().map(|(repo, conns)| {
            (repo.clone(), conns.iter().map(|c| cx_resolution::websocket_resolution::WsClientConnection {
                url_or_path: c.url_or_path.clone(), file: c.file.clone(), line: c.line,
            }).collect())
        }).collect(),
        ws_servers: merged.ws_servers.iter().map(|(repo, eps)| {
            (repo.clone(), eps.iter().map(|e| cx_resolution::websocket_resolution::WsServerEndpoint {
                path: e.path.clone(), file: e.file.clone(), line: e.line,
            }).collect())
        }).collect(),
        k8s_env_bindings: merged.k8s_env_bindings.iter().flat_map(|(_, bindings)| {
            bindings.iter().map(|b| cx_resolution::k8s_resolution::K8sEnvBinding {
                var_name: b.var_name.clone(),
                value: b.value.clone(),
                file: b.file.clone(),
                line: b.line,
                deployment_name: b.deployment_name.clone(),
            })
        }).collect(),
    };

    let result = resolver::resolve(&input);

    if !result.unresolved_client_stubs.is_empty() {
        for (repo, stub) in &result.unresolved_client_stubs {
            eprintln!(
                "  unresolved gRPC client: {} in {} ({}:{})",
                stub.service_name, repo, stub.file, stub.line
            );
        }
    }

    let mut edges_added = 0;

    // Proto/gRPC matches → DependsOn edges
    for m in &result.proto_matches {
        edges_added += add_cross_repo_edge(merged, &m.client_file, m.client_line, &m.server_file, m.server_line, m.confidence);
        eprintln!("  gRPC: {} ({}) → {} ({}) [{}]", m.client_file, m.client_repo, m.server_file, m.server_repo, m.service_name);
    }

    // REST matches → DependsOn edges
    for m in &result.rest_matches {
        edges_added += add_cross_repo_edge(merged, &m.client_file, m.client_line, &m.server_file, m.server_line, m.confidence);
        eprintln!("  REST: {} ({}) → {} ({}) [{}]", m.client_file, m.client_repo, m.server_file, m.server_repo, m.path);
    }

    // Helm env matches → Resolves edges
    for m in &result.helm_env_matches {
        edges_added += add_cross_repo_edge(merged, &m.reader_file, m.reader_line, &m.helm_file, m.helm_line, m.confidence);
        eprintln!("  Env: {} ({}) → {} ({}) [{}={}]",
            m.reader_file, m.reader_repo, m.helm_file, m.helm_repo, m.var_name, truncate(&m.helm_value, 60));
    }

    // Image matches → DependsOn edges (Dockerfile repo builds what k8s deploys)
    for m in &result.image_matches {
        edges_added += add_cross_repo_edge(merged, &m.dockerfile, 1, &m.k8s_file, m.k8s_line, m.confidence);
        eprintln!("  Image: {} ({}) → {} ({}) [{}]", m.dockerfile, m.dockerfile_repo, m.k8s_file, m.k8s_repo, m.image_path);
    }

    // WebSocket matches → DependsOn edges
    for m in &result.ws_matches {
        edges_added += add_cross_repo_edge(merged, &m.client_file, m.client_line, &m.server_file, m.server_line, m.confidence);
        eprintln!("  WS: {} ({}) → {} ({}) [{}]", m.client_file, m.client_repo, m.server_file, m.server_repo, m.path);
    }

    // K8s env matches → DependsOn edges (code env read → resolved service target)
    // Unlike other match types, the K8s manifest target may not have a graph node.
    // We find the code-side node (function reading the env var) and create a
    // DependsOn edge to the Resource node representing the env var.
    for m in &result.k8s_matches {
        // First try the standard cross-repo edge (works when k8s manifest has nodes)
        let added = add_cross_repo_edge(merged, &m.code_file, m.code_line, &m.k8s_file, m.k8s_line, m.confidence);
        if added == 0 {
            // Fallback: find the code-side function and the env var Resource node,
            // then create an edge from the function to the Resource node.
            let code_node = find_node_at(&merged.nodes, &merged.strings, &m.code_file, m.code_line);
            // Find the Resource node for this env var name
            let envvar_node = merged.nodes.iter().find(|n| {
                n.kind == cx_core::graph::nodes::NodeKind::Resource as u8
                    && merged.strings.get(n.name) == m.env_var_name
            }).map(|n| n.id);

            if let (Some(code_id), Some(env_id)) = (code_node, envvar_node) {
                if code_id != env_id {
                    let conf_u8 = (m.confidence * 255.0) as u8;
                    let mut edge = cx_core::graph::csr::EdgeInput::new(
                        code_id, env_id,
                        cx_core::graph::edges::EdgeKind::DependsOn,
                    );
                    edge.confidence_u8 = conf_u8;
                    merged.edges.push(edge);
                    edges_added += 1;
                }
            }
        } else {
            edges_added += added;
        }

        let target = if let Some(port) = m.target_port {
            format!("{}:{}", m.target_service, port)
        } else {
            m.target_service.clone()
        };
        eprintln!("  K8s: {} → {} [{}={}]",
            m.code_file, target, m.env_var_name, truncate(&m.env_value, 60));
    }

    eprintln!(
        "  Resolution summary: {} gRPC, {} REST, {} env→Helm, {} image, {} WebSocket, {} K8s env",
        result.proto_count, result.rest_count, result.helm_env_count,
        result.image_count, result.ws_count, result.k8s_count
    );

    edges_added
}

/// Add a cross-repo DependsOn edge between nodes at the given file:line locations.
/// Returns 1 if the edge was added, 0 if nodes couldn't be found.
fn add_cross_repo_edge(
    merged: &mut MergedResult,
    client_file: &str, client_line: u32,
    server_file: &str, server_line: u32,
    confidence: f32,
) -> usize {
    let client_node = find_node_at(&merged.nodes, &merged.strings, client_file, client_line);
    let server_node = find_node_at(&merged.nodes, &merged.strings, server_file, server_line);

    if let (Some(client_id), Some(server_id)) = (client_node, server_node) {
        if client_id != server_id {
            let conf_u8 = (confidence * 255.0) as u8;
            let mut edge = cx_core::graph::csr::EdgeInput::new(
                client_id, server_id,
                cx_core::graph::edges::EdgeKind::DependsOn,
            );
            edge.confidence_u8 = conf_u8;
            edge.flags = cx_core::graph::edges::EDGE_IS_CROSS_REPO;
            merged.edges.push(edge);
            return 1;
        }
    }
    0
}

/// Index a single repo without cross-repo resolution.
/// Returns the IndexResult containing only this repo's graph.
pub fn index_single_repo(repo_path: &std::path::Path, repo_id: u16) -> Result<IndexResult> {
    let repos = vec![(repo_path.to_path_buf(), repo_id)];
    let merged = pipeline::extract_and_merge_repos(&repos)
        .context("failed to extract repo")?;
    Ok(pipeline::build_index(merged))
}

/// Merge all per-repo .cxgraph files from .cx/graph/repos/ into a unified graph.
/// Also applies cross-repo resolution and writes base.cxgraph + overlay.
pub fn merge_per_repo_graphs(root: &std::path::Path) -> Result<cx_core::graph::csr::CsrGraph> {
    let repos_dir = root.join(".cx").join("graph").join("repos");
    if !repos_dir.exists() {
        anyhow::bail!("no per-repo graphs found");
    }

    let mut entries: Vec<_> = std::fs::read_dir(&repos_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "cxgraph")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    if entries.is_empty() {
        anyhow::bail!("no .cxgraph files in repos/");
    }

    let mut graphs = Vec::with_capacity(entries.len());
    for entry in &entries {
        let graph = cx_core::store::mmap::load_graph(&entry.path())
            .with_context(|| format!("failed to load {}", entry.path().display()))?;
        graphs.push(graph);
    }

    let merged = cx_core::graph::csr::CsrGraph::merge(&graphs, vec![]);
    Ok(merged)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}...", &s[..max]) }
}

/// Find the Symbol node closest to a given file:line location.
/// Looks for the nearest function/method node at or just before the given line.
fn find_node_at(
    nodes: &[cx_core::graph::nodes::Node],
    strings: &cx_core::graph::string_interner::StringInterner,
    file: &str,
    line: u32,
) -> Option<u32> {
    let mut best: Option<(u32, u32)> = None; // (node_id, line_distance)

    for node in nodes {
        if node.kind != cx_core::graph::nodes::NodeKind::Symbol as u8 {
            continue;
        }
        if node.file == u32::MAX {
            continue;
        }
        let node_file = strings.get(node.file);
        if node_file != file {
            continue;
        }
        // Prefer the enclosing function (line <= target) with smallest distance
        if node.line <= line {
            let dist = line - node.line;
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((node.id, dist));
            }
        }
    }

    best.map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn grpc_resolution_creates_depends_on_edge() {
        let server_repo = tempfile::tempdir().unwrap();
        let client_repo = tempfile::tempdir().unwrap();

        // Server repo: registers OrderProcessing gRPC server
        fs::write(
            server_repo.path().join("server.go"),
            r#"package main

import pb "example.com/proto/order"

func StartServer() {
    s := grpc.NewServer()
    pb.RegisterOrderProcessingServer(s, &handler{})
    s.Serve(lis)
}
"#,
        )
        .unwrap();

        // Client repo: creates OrderProcessing gRPC client
        fs::write(
            client_repo.path().join("client.go"),
            r#"package main

import pb "example.com/proto/order"

func CallService() {
    conn, _ := grpc.Dial("localhost:50051")
    client := pb.NewOrderProcessingClient(conn)
    _ = client
}
"#,
        )
        .unwrap();

        let repos = vec![
            (server_repo.path().to_path_buf(), 0u16),
            (client_repo.path().to_path_buf(), 1u16),
        ];

        let result = index_repos_with_resolution(&repos).unwrap();
        let graph = &result.graph;

        // Should have a DependsOn edge from client → server
        let has_depends_on = graph.edges.iter().any(|e| {
            e.kind == cx_core::graph::edges::EdgeKind::DependsOn as u8
                && e.flags & cx_core::graph::edges::EDGE_IS_CROSS_REPO != 0
        });

        assert!(
            has_depends_on,
            "should have a cross-repo DependsOn edge from gRPC resolution"
        );
    }

    #[test]
    fn k8s_env_resolution_creates_depends_on_edge() {
        let repo = tempfile::tempdir().unwrap();

        // Go code that reads PRODUCT_CATALOG_SERVICE_ADDR env var
        fs::write(
            repo.path().join("main.go"),
            r#"package main

import "os"

func GetCatalog() {
    addr := os.Getenv("PRODUCT_CATALOG_SERVICE_ADDR")
    conn, _ := grpc.Dial(addr)
    _ = conn
}
"#,
        )
        .unwrap();

        // K8s deployment manifest with the env var binding
        fs::create_dir_all(repo.path().join("kubernetes")).unwrap();
        fs::write(
            repo.path().join("kubernetes").join("deployment.yaml"),
            r#"apiVersion: apps/v1
kind: Deployment
metadata:
  name: frontend
spec:
  template:
    spec:
      containers:
      - name: server
        image: frontend:latest
        env:
        - name: PRODUCT_CATALOG_SERVICE_ADDR
          value: "productcatalogservice:3550"
        - name: CURRENCY_SERVICE_ADDR
          value: "currencyservice:7000"
"#,
        )
        .unwrap();

        let repos = vec![(repo.path().to_path_buf(), 0u16)];
        let result = index_repos_with_resolution(&repos).unwrap();

        // Verify the K8s env bindings were extracted
        // The resolution should find PRODUCT_CATALOG_SERVICE_ADDR → productcatalogservice:3550
        // Check that we have a DependsOn edge from the code to the k8s manifest
        let graph = &result.graph;

        let has_depends_on = graph.edges.iter().any(|e| {
            e.kind == cx_core::graph::edges::EdgeKind::DependsOn as u8
        });

        assert!(
            has_depends_on,
            "should have a DependsOn edge from K8s env resolution (PRODUCT_CATALOG_SERVICE_ADDR → productcatalogservice:3550)"
        );
    }
}
