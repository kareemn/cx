use anyhow::{Context, Result};
use cx_extractors::pipeline::{self, IndexResult, MergedResult};
use std::path::PathBuf;

/// Run the full indexing pipeline with gRPC resolution:
/// 1. Extract and merge all repos
/// 2. Run gRPC resolution engine to create cross-repo DependsOn edges
/// 3. Build the unified CSR graph
pub fn index_repos_with_resolution(repos: &[(PathBuf, u16)]) -> Result<IndexResult> {
    let mut merged = pipeline::extract_and_merge_repos(repos)
        .context("failed to extract repos")?;

    // Run gRPC resolution across repos
    let resolved = resolve_grpc(&mut merged);
    if resolved > 0 {
        eprintln!("Resolved {} cross-repo gRPC connection(s)", resolved);
    }

    Ok(pipeline::build_index(merged))
}

/// Run the gRPC resolution engine on merged extraction data.
/// Finds matching client stubs and server registrations across repos,
/// then creates DependsOn edges between the calling function and the
/// server registration function in the graph.
///
/// Returns the number of resolved connections.
fn resolve_grpc(merged: &mut MergedResult) -> usize {
    use cx_resolution::resolver::{ResolutionInput, self};

    let input = ResolutionInput {
        client_stubs: merged.grpc_clients.clone(),
        server_registrations: merged.grpc_servers.clone(),
        proto_services: merged.proto_services.clone(),
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

    // Convert ProtoMatch results to graph edges.
    // For each match, find the node closest to the client call site and
    // the node closest to the server registration site, then create a
    // DependsOn edge between them.
    let mut edges_added = 0;
    for m in &result.proto_matches {
        // Find the function node at or near the client call site
        let client_node = find_node_at(
            &merged.nodes,
            &merged.strings,
            &m.client_file,
            m.client_line,
        );
        // Find the function node at or near the server registration site
        let server_node = find_node_at(
            &merged.nodes,
            &merged.strings,
            &m.server_file,
            m.server_line,
        );

        if let (Some(client_id), Some(server_id)) = (client_node, server_node) {
            if client_id != server_id {
                let confidence = (m.confidence * 255.0) as u8;
                let mut edge = cx_core::graph::csr::EdgeInput::new(
                    client_id,
                    server_id,
                    cx_core::graph::edges::EdgeKind::DependsOn,
                );
                edge.confidence_u8 = confidence;
                edge.flags = cx_core::graph::edges::EDGE_IS_CROSS_REPO;
                merged.edges.push(edge);
                edges_added += 1;

                eprintln!(
                    "  gRPC: {} ({}) → {} ({}) [{}]",
                    m.client_file, m.client_repo, m.server_file, m.server_repo, m.service_name
                );
            }
        }
    }

    edges_added
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
