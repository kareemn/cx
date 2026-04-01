//! Overlay graph for incremental cross-repo edge management.
//!
//! Stores only cross-repo edges (DependsOn, Connects from resolution) so that
//! adding a new repo only requires updating the overlay, not re-merging all
//! per-repo graphs from scratch.
//!
//! Edges reference nodes by (repo_id, file, line) triples which are stable
//! across graph rebuilds (unlike node IDs which are remapped on every merge).

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// A single cross-repo edge stored in the overlay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayEdge {
    /// Source node: repo index from config.repos.
    pub source_repo: u16,
    /// Source file path (relative to repo root, as stored in the graph).
    pub source_file: String,
    /// Source line number.
    pub source_line: u32,
    /// Source symbol name (for fallback resolution).
    pub source_symbol: String,
    /// Target node: repo index from config.repos.
    pub target_repo: u16,
    /// Target file path.
    pub target_file: String,
    /// Target line number.
    pub target_line: u32,
    /// Target symbol name (for fallback resolution).
    pub target_symbol: String,
    /// Edge kind (EdgeKind as u8).
    pub kind: u8,
    /// Confidence (0–255).
    pub confidence: u8,
    /// Edge flags (e.g., EDGE_IS_CROSS_REPO).
    pub flags: u16,
}

/// The overlay graph: a collection of cross-repo edges stored as JSON.
/// Typically <50KB even for 1000 repos.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct OverlayGraph {
    /// All cross-repo edges from resolution.
    pub edges: Vec<OverlayEdge>,
}

impl OverlayGraph {
    /// Load from overlay.json, returning empty overlay if it doesn't exist.
    pub fn load(root: &Path) -> Result<Self> {
        let path = overlay_path(root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Save to overlay.json.
    #[allow(dead_code)] // Used when cx remote is re-added
    pub fn save(&self, root: &Path) -> Result<()> {
        let dir = root.join(".cx").join("graph");
        std::fs::create_dir_all(&dir)?;
        let path = overlay_path(root);
        let content = serde_json::to_string_pretty(self)
            .context("failed to serialize overlay")?;
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write {}", path.display()))
    }

    /// Remove all edges involving a given repo (before re-adding updated ones).
    #[allow(dead_code)] // Used when cx remote is re-added
    pub fn remove_repo(&mut self, repo_id: u16) {
        self.edges
            .retain(|e| e.source_repo != repo_id && e.target_repo != repo_id);
    }

    /// Add edges from cross-repo resolution of a new repo against the global index.
    ///
    /// For each outgoing target/client in the new repo, finds matching APIs/servers
    /// in other repos via the index. For each exposed API/server in the new repo,
    /// finds matching clients/targets in other repos.
    #[allow(dead_code)] // Used when cx remote is re-added
    pub fn resolve_repo_against_index(
        &mut self,
        repo_id: u16,
        index: &crate::graph_index::GlobalIndex,
    ) {
        use cx_core::graph::edges::{EdgeKind, EDGE_IS_CROSS_REPO};

        // gRPC: new repo's clients → other repos' servers
        for (service, clients) in &index.grpc_clients {
            for client in clients {
                if client.repo_id != repo_id {
                    continue;
                }
                if let Some(servers) = index.grpc_servers.get(service) {
                    for server in servers {
                        if server.repo_id == repo_id {
                            continue;
                        }
                        self.edges.push(OverlayEdge {
                            source_repo: client.repo_id,
                            source_file: client.file.clone(),
                            source_line: client.line,
                            source_symbol: client.symbol.clone(),
                            target_repo: server.repo_id,
                            target_file: server.file.clone(),
                            target_line: server.line,
                            target_symbol: server.symbol.clone(),
                            kind: EdgeKind::DependsOn as u8,
                            confidence: 230,
                            flags: EDGE_IS_CROSS_REPO,
                        });
                    }
                }
            }
        }

        // gRPC: other repos' clients → new repo's servers
        for (service, servers) in &index.grpc_servers {
            for server in servers {
                if server.repo_id != repo_id {
                    continue;
                }
                if let Some(clients) = index.grpc_clients.get(service) {
                    for client in clients {
                        if client.repo_id == repo_id {
                            continue;
                        }
                        self.edges.push(OverlayEdge {
                            source_repo: client.repo_id,
                            source_file: client.file.clone(),
                            source_line: client.line,
                            source_symbol: client.symbol.clone(),
                            target_repo: server.repo_id,
                            target_file: server.file.clone(),
                            target_line: server.line,
                            target_symbol: server.symbol.clone(),
                            kind: EdgeKind::DependsOn as u8,
                            confidence: 230,
                            flags: EDGE_IS_CROSS_REPO,
                        });
                    }
                }
            }
        }

        // REST/HTTP: new repo's outgoing targets → other repos' exposed APIs
        for (target, callers) in &index.outgoing_targets {
            for caller in callers {
                if caller.repo_id != repo_id {
                    continue;
                }
                // Try to match target against exposed APIs
                // Match by endpoint name (e.g., "POST /api/orders")
                if let Some(providers) = index.exposed_apis.get(target) {
                    for provider in providers {
                        if provider.repo_id == repo_id {
                            continue;
                        }
                        self.edges.push(OverlayEdge {
                            source_repo: caller.repo_id,
                            source_file: caller.file.clone(),
                            source_line: caller.line,
                            source_symbol: caller.symbol.clone(),
                            target_repo: provider.repo_id,
                            target_file: provider.file.clone(),
                            target_line: provider.line,
                            target_symbol: provider.symbol.clone(),
                            kind: EdgeKind::DependsOn as u8,
                            confidence: 200,
                            flags: EDGE_IS_CROSS_REPO,
                        });
                    }
                }
            }
        }

        // REST/HTTP: other repos' outgoing targets → new repo's exposed APIs
        for (endpoint, providers) in &index.exposed_apis {
            for provider in providers {
                if provider.repo_id != repo_id {
                    continue;
                }
                if let Some(callers) = index.outgoing_targets.get(endpoint) {
                    for caller in callers {
                        if caller.repo_id == repo_id {
                            continue;
                        }
                        self.edges.push(OverlayEdge {
                            source_repo: caller.repo_id,
                            source_file: caller.file.clone(),
                            source_line: caller.line,
                            source_symbol: caller.symbol.clone(),
                            target_repo: provider.repo_id,
                            target_file: provider.file.clone(),
                            target_line: provider.line,
                            target_symbol: provider.symbol.clone(),
                            kind: EdgeKind::DependsOn as u8,
                            confidence: 200,
                            flags: EDGE_IS_CROSS_REPO,
                        });
                    }
                }
            }
        }

        // Dedup edges
        self.edges.dedup();
    }

    /// Resolve overlay edges to EdgeInputs against a merged graph.
    ///
    /// For each overlay edge, finds the source and target nodes in the graph
    /// by matching (repo_id, file, line). Returns EdgeInputs ready for CsrGraph::merge().
    pub fn to_edge_inputs(
        &self,
        graphs: &[cx_core::graph::csr::CsrGraph],
    ) -> Vec<cx_core::graph::csr::EdgeInput> {
        use cx_core::graph::csr::EdgeInput;
        use cx_core::graph::edges::EdgeKind;

        // Build a mapping: (repo_idx_in_graphs, local_node_id) → global_node_id
        // We need to know the node_offset per graph (same as CsrGraph::merge logic)
        let mut node_offsets = Vec::with_capacity(graphs.len());
        let mut offset: u32 = 0;
        for graph in graphs {
            node_offsets.push(offset);
            offset += graph.nodes.len() as u32;
        }

        let mut result = Vec::with_capacity(self.edges.len());

        for edge in &self.edges {
            let source_global = find_node_in_graphs(
                graphs,
                &node_offsets,
                edge.source_repo,
                &edge.source_file,
                edge.source_line,
                &edge.source_symbol,
            );
            let target_global = find_node_in_graphs(
                graphs,
                &node_offsets,
                edge.target_repo,
                &edge.target_file,
                edge.target_line,
                &edge.target_symbol,
            );

            if let (Some(src), Some(tgt)) = (source_global, target_global) {
                if src != tgt {
                    let kind = EdgeKind::from_u8(edge.kind).unwrap_or(EdgeKind::DependsOn);
                    let mut ei = EdgeInput::new(src, tgt, kind);
                    ei.confidence_u8 = edge.confidence;
                    ei.flags = edge.flags;
                    result.push(ei);
                }
            }
        }

        result
    }
}

/// Find a node in the per-repo graphs by repo_id, file, line, and symbol name.
/// Returns the global node ID (with graph offset applied).
fn find_node_in_graphs(
    graphs: &[cx_core::graph::csr::CsrGraph],
    node_offsets: &[u32],
    repo_id: u16,
    file: &str,
    line: u32,
    symbol: &str,
) -> Option<u32> {
    use cx_core::graph::nodes::{NodeKind, STRING_NONE};

    for (graph_idx, graph) in graphs.iter().enumerate() {
        let offset = node_offsets[graph_idx];

        // Check if this graph contains nodes from the target repo
        let has_repo = graph.nodes.iter().any(|n| n.repo == repo_id);
        if !has_repo {
            continue;
        }

        // Find best matching node: prefer file:line match, fall back to symbol name
        let mut best: Option<(u32, u32)> = None; // (global_id, distance)

        for node in &graph.nodes {
            if node.repo != repo_id {
                continue;
            }
            if node.kind != NodeKind::Symbol as u8
                && node.kind != NodeKind::Endpoint as u8
                && node.kind != NodeKind::Surface as u8
            {
                continue;
            }

            // Try file:line match
            if node.file != STRING_NONE {
                let node_file = graph.strings.get(node.file);
                if node_file == file && node.line <= line {
                    let dist = line - node.line;
                    if best.is_none() || dist < best.unwrap().1 {
                        best = Some((node.id + offset, dist));
                    }
                }
            }
        }

        if best.is_some() {
            return best.map(|(id, _)| id);
        }

        // Fallback: match by symbol name
        if !symbol.is_empty() {
            for node in &graph.nodes {
                if node.repo != repo_id {
                    continue;
                }
                let name = graph.strings.get(node.name);
                if name == symbol {
                    return Some(node.id + offset);
                }
            }
        }
    }

    None
}

fn overlay_path(root: &Path) -> std::path::PathBuf {
    root.join(".cx").join("graph").join("overlay.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".cx").join("graph")).unwrap();

        let mut overlay = OverlayGraph::default();
        overlay.edges.push(OverlayEdge {
            source_repo: 0,
            source_file: "client.go".to_string(),
            source_line: 10,
            source_symbol: "CallService".to_string(),
            target_repo: 1,
            target_file: "server.go".to_string(),
            target_line: 5,
            target_symbol: "Serve".to_string(),
            kind: cx_core::graph::edges::EdgeKind::DependsOn as u8,
            confidence: 230,
            flags: cx_core::graph::edges::EDGE_IS_CROSS_REPO,
        });

        overlay.save(dir.path()).unwrap();
        let loaded = OverlayGraph::load(dir.path()).unwrap();
        assert_eq!(loaded.edges.len(), 1);
        assert_eq!(loaded.edges[0], overlay.edges[0]);
    }

    #[test]
    fn overlay_load_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let overlay = OverlayGraph::load(dir.path()).unwrap();
        assert!(overlay.edges.is_empty());
    }

    #[test]
    fn overlay_remove_repo() {
        let mut overlay = OverlayGraph::default();
        overlay.edges.push(OverlayEdge {
            source_repo: 0,
            source_file: "a.go".to_string(),
            source_line: 1,
            source_symbol: "A".to_string(),
            target_repo: 1,
            target_file: "b.go".to_string(),
            target_line: 1,
            target_symbol: "B".to_string(),
            kind: 3,
            confidence: 255,
            flags: 1,
        });
        overlay.edges.push(OverlayEdge {
            source_repo: 1,
            source_file: "b.go".to_string(),
            source_line: 1,
            source_symbol: "B".to_string(),
            target_repo: 2,
            target_file: "c.go".to_string(),
            target_line: 1,
            target_symbol: "C".to_string(),
            kind: 3,
            confidence: 255,
            flags: 1,
        });
        overlay.edges.push(OverlayEdge {
            source_repo: 0,
            source_file: "a.go".to_string(),
            source_line: 1,
            source_symbol: "A".to_string(),
            target_repo: 2,
            target_file: "c.go".to_string(),
            target_line: 1,
            target_symbol: "C".to_string(),
            kind: 3,
            confidence: 255,
            flags: 1,
        });

        // Remove repo 1 — should remove edges involving repo 1
        overlay.remove_repo(1);
        assert_eq!(overlay.edges.len(), 1);
        assert_eq!(overlay.edges[0].source_repo, 0);
        assert_eq!(overlay.edges[0].target_repo, 2);
    }

    #[test]
    fn overlay_resolve_grpc_against_index() {
        let mut index = crate::graph_index::GlobalIndex::default();

        // Repo 0 has a gRPC server for "OrderService"
        index.grpc_servers.insert(
            "OrderService".to_string(),
            vec![crate::graph_index::IndexEntry {
                repo_id: 0,
                repo_name: "order-svc".to_string(),
                file: "server.go".to_string(),
                line: 10,
                symbol: "RegisterOrderServer".to_string(),
            }],
        );

        // Repo 1 has a gRPC client for "OrderService"
        index.grpc_clients.insert(
            "OrderService".to_string(),
            vec![crate::graph_index::IndexEntry {
                repo_id: 1,
                repo_name: "frontend".to_string(),
                file: "client.go".to_string(),
                line: 20,
                symbol: "NewOrderClient".to_string(),
            }],
        );

        let mut overlay = OverlayGraph::default();

        // Resolve repo 1 (the client) against the index
        overlay.resolve_repo_against_index(1, &index);

        // Should find the client→server edge
        assert_eq!(overlay.edges.len(), 1);
        let edge = &overlay.edges[0];
        assert_eq!(edge.source_repo, 1);
        assert_eq!(edge.source_file, "client.go");
        assert_eq!(edge.target_repo, 0);
        assert_eq!(edge.target_file, "server.go");
        assert_eq!(edge.kind, cx_core::graph::edges::EdgeKind::DependsOn as u8);
        assert_ne!(edge.flags & cx_core::graph::edges::EDGE_IS_CROSS_REPO, 0);
    }

    #[test]
    fn overlay_resolve_rest_against_index() {
        let mut index = crate::graph_index::GlobalIndex::default();

        // Repo 0 exposes "POST /api/orders"
        index.exposed_apis.insert(
            "POST /api/orders".to_string(),
            vec![crate::graph_index::IndexEntry {
                repo_id: 0,
                repo_name: "order-svc".to_string(),
                file: "handler.go".to_string(),
                line: 15,
                symbol: "handleOrders".to_string(),
            }],
        );

        // Repo 1 calls "POST /api/orders"
        index.outgoing_targets.insert(
            "POST /api/orders".to_string(),
            vec![crate::graph_index::IndexEntry {
                repo_id: 1,
                repo_name: "frontend".to_string(),
                file: "api.go".to_string(),
                line: 30,
                symbol: "callOrders".to_string(),
            }],
        );

        let mut overlay = OverlayGraph::default();
        overlay.resolve_repo_against_index(1, &index);

        assert_eq!(overlay.edges.len(), 1);
        let edge = &overlay.edges[0];
        assert_eq!(edge.source_repo, 1);
        assert_eq!(edge.target_repo, 0);
    }

    #[test]
    fn overlay_to_edge_inputs() {
        use cx_core::graph::csr::CsrGraph;
        use cx_core::graph::edges::EdgeKind;
        use cx_core::graph::nodes::{Node, NodeKind};
        use cx_core::graph::string_interner::StringInterner;

        // Build two per-repo graphs
        let mut strings1 = StringInterner::new();
        let file1 = strings1.intern("client.go");
        let sym1 = strings1.intern("CallService");
        let mut n1 = Node::new(0, NodeKind::Symbol, sym1);
        n1.file = file1;
        n1.line = 10;
        n1.repo = 0;
        let graph1 = CsrGraph::build(vec![n1], vec![], strings1);

        let mut strings2 = StringInterner::new();
        let file2 = strings2.intern("server.go");
        let sym2 = strings2.intern("Serve");
        let mut n2 = Node::new(0, NodeKind::Symbol, sym2);
        n2.file = file2;
        n2.line = 5;
        n2.repo = 1;
        let graph2 = CsrGraph::build(vec![n2], vec![], strings2);

        let graphs = vec![graph1, graph2];

        let mut overlay = OverlayGraph::default();
        overlay.edges.push(OverlayEdge {
            source_repo: 0,
            source_file: "client.go".to_string(),
            source_line: 10,
            source_symbol: "CallService".to_string(),
            target_repo: 1,
            target_file: "server.go".to_string(),
            target_line: 5,
            target_symbol: "Serve".to_string(),
            kind: EdgeKind::DependsOn as u8,
            confidence: 230,
            flags: cx_core::graph::edges::EDGE_IS_CROSS_REPO,
        });

        let edge_inputs = overlay.to_edge_inputs(&graphs);
        assert_eq!(edge_inputs.len(), 1);
        // Source is from graph1 (offset 0), target from graph2 (offset 1)
        assert_ne!(edge_inputs[0].source, edge_inputs[0].target);
    }
}
