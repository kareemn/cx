use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;

/// Run `cx init` — index the current directory and write the graph to .cx/graph/.
///
/// Creates both the per-repo graph in repos/ and the unified base.cxgraph.
/// Also initializes the global index (index.json).
pub fn run(root: &Path, verbose: bool) -> Result<()> {
    let start = Instant::now();

    eprintln!("Indexing {}...", root.display());

    // Save current repo to config
    let canon_root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", root.display()))?;
    let mut config = crate::config::load(root).unwrap_or_default();
    crate::config::add_repo(&mut config, canon_root.clone());
    crate::config::save(root, &config)?;

    // Find the repo index in config
    let repo_idx = config
        .repos
        .iter()
        .position(|r| r.path == canon_root)
        .unwrap_or(0);

    // Index all repos from config (on init, typically just this one)
    let repos: Vec<_> = config
        .repos
        .iter()
        .enumerate()
        .map(|(i, r)| (r.path.clone(), i as u16))
        .collect();

    let result = crate::indexing::index_repos_with_resolution(&repos, verbose)?;

    let elapsed = start.elapsed();

    // Create .cx/graph/ and repos/ directories
    let cx_dir = root.join(".cx").join("graph");
    let repos_dir = cx_dir.join("repos");
    std::fs::create_dir_all(&repos_dir)
        .context("failed to create .cx/graph/repos/ directory")?;

    // Write per-repo graph
    let per_repo_name = crate::config::per_repo_filename(repo_idx, &canon_root);
    let per_repo_path = repos_dir.join(&per_repo_name);
    cx_core::store::mmap::write_graph(&result.graph, &per_repo_path)
        .with_context(|| format!("failed to write per-repo graph {}", per_repo_name))?;

    // Write unified graph (same content for single repo, but also serves as base.cxgraph)
    let graph_path = cx_dir.join("base.cxgraph");
    cx_core::store::mmap::write_graph(&result.graph, &graph_path)
        .context("failed to write graph file")?;

    // Build and write global index
    let mut index = crate::graph_index::GlobalIndex::default();
    let repo_name = crate::config::repo_name(&canon_root);
    index.add_from_graph(repo_idx as u16, &repo_name, &result.graph);
    index.save(root)?;

    // Report results
    eprintln!(
        "Indexed {} files: {} symbols, {} edges in {:.1}ms",
        result.file_count,
        result.node_count,
        result.edge_count,
        elapsed.as_secs_f64() * 1000.0,
    );

    if !result.errors.is_empty() {
        eprintln!("Warnings:");
        for err in &result.errors {
            eprintln!("  {}", err);
        }
    }

    let file_size = std::fs::metadata(&graph_path)
        .map(|m| m.len())
        .unwrap_or(0);
    eprintln!(
        "Graph written to {} ({} bytes)",
        graph_path.display(),
        file_size,
    );
    eprintln!("Per-repo graph: {}", per_repo_path.display());

    // Write network calls with provenance to network.json
    if !result.network_calls.is_empty() {
        let network_path = cx_dir.join("network.json");
        let json = serde_json::to_string_pretty(&result.network_calls)
            .context("failed to serialize network calls")?;
        std::fs::write(&network_path, json)
            .context("failed to write network.json")?;
        eprintln!(
            "Network analysis: {} call(s) written to {}",
            result.network_calls.len(),
            network_path.display(),
        );
    }

    Ok(())
}

/// Load the unified graph from .cx/graph/base.cxgraph.
/// Falls back to merging per-repo graphs if base.cxgraph doesn't exist.
pub fn load_graph(root: &Path) -> Result<cx_core::graph::csr::CsrGraph> {
    let graph_path = root.join(".cx").join("graph").join("base.cxgraph");
    if graph_path.exists() {
        return cx_core::store::mmap::load_graph(&graph_path).context("failed to load graph");
    }

    // Try layered loading (per-repo graphs + overlay)
    load_graph_layered(root)
}

/// Load graph by merging per-repo graphs with overlay cross-repo edges.
///
/// This is the layered loading path: each per-repo graph is loaded independently,
/// then merged with cross-repo edges from the overlay. This avoids needing a
/// pre-built base.cxgraph and always reflects the latest overlay state.
pub fn load_graph_layered(root: &Path) -> Result<cx_core::graph::csr::CsrGraph> {
    let repos_dir = root.join(".cx").join("graph").join("repos");
    if repos_dir.exists() {
        return crate::indexing::merge_per_repo_graphs(root);
    }

    anyhow::bail!("index not found: run `cx init` first")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn init_creates_graph_file() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        run(dir.path(), false).unwrap();

        let graph_path = dir.path().join(".cx").join("graph").join("base.cxgraph");
        assert!(graph_path.exists(), "graph file should exist");
        assert!(
            fs::metadata(&graph_path).unwrap().len() > 0,
            "graph file should not be empty"
        );
    }

    #[test]
    fn init_creates_per_repo_graph() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        run(dir.path(), false).unwrap();

        let repos_dir = dir.path().join(".cx").join("graph").join("repos");
        assert!(repos_dir.exists(), "repos/ directory should exist");

        let entries: Vec<_> = fs::read_dir(&repos_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1, "should have 1 per-repo graph file");
        assert!(
            entries[0]
                .file_name()
                .to_str()
                .unwrap()
                .ends_with(".cxgraph"),
            "per-repo file should have .cxgraph extension"
        );
    }

    #[test]
    fn init_creates_index_json() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        run(dir.path(), false).unwrap();

        let index_path = dir.path().join(".cx").join("graph").join("index.json");
        assert!(index_path.exists(), "index.json should exist");
    }

    #[test]
    fn init_creates_config() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        run(dir.path(), false).unwrap();

        let config_path = dir.path().join(".cx").join("config.toml");
        assert!(config_path.exists(), "config.toml should exist");

        let config = crate::config::load(dir.path()).unwrap();
        assert_eq!(config.repos.len(), 1);
    }

    #[test]
    fn init_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("main.go"),
            r#"package main

func hello() {}
func world() { hello() }
"#,
        )
        .unwrap();

        run(dir.path(), false).unwrap();

        let graph = load_graph(dir.path()).unwrap();
        assert!(graph.node_count() > 0, "should have nodes");

        let names: Vec<&str> = graph
            .nodes
            .iter()
            .map(|n| graph.strings.get(n.name))
            .collect();

        assert!(names.contains(&"hello"));
        assert!(names.contains(&"world"));
    }

    #[test]
    fn load_graph_missing_index() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_graph(dir.path());
        assert!(result.is_err());
    }
}
