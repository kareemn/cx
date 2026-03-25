use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;

/// Run `cx init` — index the current directory and write the graph to .cx/graph/.
pub fn run(root: &Path) -> Result<()> {
    let start = Instant::now();

    eprintln!("Indexing {}...", root.display());

    let result = cx_extractors::pipeline::index_directory(root)
        .context("failed to index directory")?;

    let elapsed = start.elapsed();

    // Create .cx/graph/ directory
    let cx_dir = root.join(".cx").join("graph");
    std::fs::create_dir_all(&cx_dir)
        .context("failed to create .cx/graph/ directory")?;

    // Write graph to disk
    let graph_path = cx_dir.join("index.cxgraph");
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

/// Load the graph from .cx/graph/index.cxgraph.
pub fn load_graph(root: &Path) -> Result<cx_core::graph::csr::CsrGraph> {
    let graph_path = root.join(".cx").join("graph").join("index.cxgraph");
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

        let graph_path = dir.path().join(".cx").join("graph").join("index.cxgraph");
        assert!(graph_path.exists(), "graph file should exist");
        assert!(
            fs::metadata(&graph_path).unwrap().len() > 0,
            "graph file should not be empty"
        );
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
