//! Global cross-repo index for fast lookups.
//!
//! Stores a mapping of exposed APIs and outgoing targets per repo,
//! so cross-repo resolution can match new repos without re-scanning all repos.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// A single entry in the global index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    /// Repo index (position in config.repos)
    pub repo_id: u16,
    /// Repo name (derived from path)
    pub repo_name: String,
    /// File where this symbol/endpoint is defined
    pub file: String,
    /// Line number
    pub line: u32,
    /// Symbol name (function, method, etc.)
    pub symbol: String,
}

/// The global cross-repo index, stored as index.json.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GlobalIndex {
    /// Exposed APIs: endpoint path/name → list of providers
    /// e.g., "POST /api/orders" → [IndexEntry { repo_id: 0, ... }]
    pub exposed_apis: HashMap<String, Vec<IndexEntry>>,

    /// Outgoing targets: target address/service → list of callers
    /// e.g., "order-service:8080" → [IndexEntry { repo_id: 1, ... }]
    pub outgoing_targets: HashMap<String, Vec<IndexEntry>>,

    /// gRPC services: service name → list of servers
    pub grpc_servers: HashMap<String, Vec<IndexEntry>>,

    /// gRPC clients: service name → list of clients
    pub grpc_clients: HashMap<String, Vec<IndexEntry>>,
}

impl GlobalIndex {
    /// Load from index.json, returning default if it doesn't exist.
    pub fn load(root: &Path) -> Result<Self> {
        let path = index_path(root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Save to index.json.
    pub fn save(&self, root: &Path) -> Result<()> {
        let dir = root.join(".cx").join("graph");
        std::fs::create_dir_all(&dir)?;
        let path = index_path(root);
        let content = serde_json::to_string_pretty(self)
            .context("failed to serialize index")?;
        std::fs::write(&path, content)
            .with_context(|| format!("failed to write {}", path.display()))
    }

    /// Remove all entries for a given repo_id (before re-adding updated ones).
    pub fn remove_repo(&mut self, repo_id: u16) {
        for entries in self.exposed_apis.values_mut() {
            entries.retain(|e| e.repo_id != repo_id);
        }
        self.exposed_apis.retain(|_, v| !v.is_empty());

        for entries in self.outgoing_targets.values_mut() {
            entries.retain(|e| e.repo_id != repo_id);
        }
        self.outgoing_targets.retain(|_, v| !v.is_empty());

        for entries in self.grpc_servers.values_mut() {
            entries.retain(|e| e.repo_id != repo_id);
        }
        self.grpc_servers.retain(|_, v| !v.is_empty());

        for entries in self.grpc_clients.values_mut() {
            entries.retain(|e| e.repo_id != repo_id);
        }
        self.grpc_clients.retain(|_, v| !v.is_empty());
    }

    /// Add entries from a freshly indexed repo's graph.
    pub fn add_from_graph(
        &mut self,
        repo_id: u16,
        repo_name: &str,
        graph: &cx_core::graph::csr::CsrGraph,
    ) {
        use cx_core::graph::nodes::NodeKind;

        for node in &graph.nodes {
            let name = graph.strings.get(node.name);
            let file = if node.file != cx_core::graph::nodes::STRING_NONE {
                graph.strings.get(node.file).to_string()
            } else {
                String::new()
            };

            let entry = IndexEntry {
                repo_id,
                repo_name: repo_name.to_string(),
                file: file.clone(),
                line: node.line,
                symbol: name.to_string(),
            };

            // Endpoint nodes are exposed APIs
            if node.kind == NodeKind::Endpoint as u8 {
                self.exposed_apis
                    .entry(name.to_string())
                    .or_default()
                    .push(entry.clone());
            }

            // Surface nodes are outgoing network calls
            if node.kind == NodeKind::Surface as u8 {
                self.outgoing_targets
                    .entry(name.to_string())
                    .or_default()
                    .push(entry);
            }
        }
    }
}

fn index_path(root: &Path) -> std::path::PathBuf {
    root.join(".cx").join("graph").join("index.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".cx").join("graph")).unwrap();

        let mut index = GlobalIndex::default();
        index.exposed_apis.insert(
            "POST /api/orders".to_string(),
            vec![IndexEntry {
                repo_id: 0,
                repo_name: "order-service".to_string(),
                file: "server.go".to_string(),
                line: 10,
                symbol: "handleOrders".to_string(),
            }],
        );

        index.save(dir.path()).unwrap();
        let loaded = GlobalIndex::load(dir.path()).unwrap();
        assert_eq!(loaded.exposed_apis.len(), 1);
        assert!(loaded.exposed_apis.contains_key("POST /api/orders"));
    }

    #[test]
    fn remove_repo_cleans_entries() {
        let mut index = GlobalIndex::default();
        index.exposed_apis.insert(
            "GET /health".to_string(),
            vec![
                IndexEntry {
                    repo_id: 0,
                    repo_name: "svc-a".to_string(),
                    file: "a.go".to_string(),
                    line: 1,
                    symbol: "health".to_string(),
                },
                IndexEntry {
                    repo_id: 1,
                    repo_name: "svc-b".to_string(),
                    file: "b.go".to_string(),
                    line: 1,
                    symbol: "health".to_string(),
                },
            ],
        );

        index.remove_repo(0);
        let entries = index.exposed_apis.get("GET /health").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].repo_id, 1);
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let index = GlobalIndex::load(dir.path()).unwrap();
        assert!(index.exposed_apis.is_empty());
    }
}
