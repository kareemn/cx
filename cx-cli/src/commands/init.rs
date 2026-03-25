use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;

/// Run `cx init` — index the current directory and write the graph to .cx/graph/.
pub fn run(root: &Path) -> Result<()> {
    let start = Instant::now();

    eprintln!("Indexing {}...", root.display());

    // Save current repo to config
    let canon_root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", root.display()))?;
    let mut config = crate::config::load(root).unwrap_or_default();
    crate::config::add_repo(&mut config, canon_root);
    crate::config::save(root, &config)?;

    // Index all repos from config (on init, typically just this one)
    let repos: Vec<_> = config
        .repos
        .iter()
        .enumerate()
        .map(|(i, r)| (r.path.clone(), i as u16))
        .collect();

    let result = cx_extractors::pipeline::index_repos(&repos)
        .context("failed to index repos")?;

    let elapsed = start.elapsed();

    // Create .cx/graph/ directory
    let cx_dir = root.join(".cx").join("graph");
    std::fs::create_dir_all(&cx_dir)
        .context("failed to create .cx/graph/ directory")?;

    // Write unified graph to disk
    let graph_path = cx_dir.join("base.cxgraph");
    cx_core::store::mmap::write_graph(&result.graph, &graph_path)
        .context("failed to write graph file")?;

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

    Ok(())
}

/// Load the unified graph from .cx/graph/base.cxgraph.
pub fn load_graph(root: &Path) -> Result<cx_core::graph::csr::CsrGraph> {
    let graph_path = root.join(".cx").join("graph").join("base.cxgraph");
    if !graph_path.exists() {
        anyhow::bail!("index not found: run `cx init` first");
    }
    cx_core::store::mmap::load_graph(&graph_path).context("failed to load graph")
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

        run(dir.path()).unwrap();

        let graph_path = dir.path().join(".cx").join("graph").join("base.cxgraph");
        assert!(graph_path.exists(), "graph file should exist");
        assert!(
            fs::metadata(&graph_path).unwrap().len() > 0,
            "graph file should not be empty"
        );
    }

    #[test]
    fn init_creates_config() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        run(dir.path()).unwrap();

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

        run(dir.path()).unwrap();

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
