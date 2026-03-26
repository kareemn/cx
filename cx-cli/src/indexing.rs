use anyhow::{Context, Result};
use cx_extractors::pipeline::{self, IndexResult, MergedResult};
use std::path::PathBuf;

/// Run the full indexing pipeline with cross-repo resolution:
/// 1. Extract and merge all repos
/// 2. Run resolution engine (gRPC, REST, env→Helm→k8s, Docker image, WebSocket)
/// 3. Build the unified CSR graph
pub fn index_repos_with_resolution(repos: &[(PathBuf, u16)]) -> Result<IndexResult> {
    let mut merged = pipeline::extract_and_merge_repos(repos)
        .context("failed to extract repos")?;

    let resolved = resolve_cross_repo(&mut merged);
    if resolved > 0 {
        eprintln!("Resolved {} cross-repo connection(s)", resolved);
    }

    Ok(pipeline::build_index(merged))
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
        k8s_env_bindings: vec![],
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
}
