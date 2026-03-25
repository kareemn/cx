use anyhow::{Context, Result};
use std::path::Path;

/// Run `cx add <path>` — index another repo and merge into the graph.
pub fn run(root: &Path, repo_path: &str) -> Result<()> {
    let repo = Path::new(repo_path)
        .canonicalize()
        .with_context(|| format!("repo path not found: {}", repo_path))?;

    eprintln!("Adding {}...", repo.display());

    // Index the new repo
    let result = cx_extractors::pipeline::index_directory(&repo)
        .context("failed to index repo")?;

    eprintln!(
        "Indexed {} files: {} symbols, {} edges",
        result.file_count, result.node_count, result.edge_count,
    );

    // For now, write a separate graph file for the added repo
    let cx_dir = root.join(".cx").join("graph");
    std::fs::create_dir_all(&cx_dir)?;

    let repo_name = repo
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_string());
    let graph_path = cx_dir.join(format!("{}.cxgraph", repo_name));

    cx_core::store::mmap::write_graph(&result.graph, &graph_path)?;

    eprintln!(
        "Added {}. Graph written to {}",
        repo_name,
        graph_path.display()
    );

    if !result.errors.is_empty() {
        eprintln!("Warnings:");
        for err in &result.errors {
            eprintln!("  {}", err);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn add_indexes_another_repo() {
        let main_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();

        // Create main repo
        fs::write(main_dir.path().join("main.go"), "package main\nfunc main() {}\n").unwrap();
        super::super::init::run(main_dir.path()).unwrap();

        // Create other repo
        fs::write(other_dir.path().join("server.go"), "package server\nfunc Serve() {}\n").unwrap();

        // Add other repo
        run(main_dir.path(), other_dir.path().to_str().unwrap()).unwrap();

        // Should have created a graph file for the other repo
        let cx_dir = main_dir.path().join(".cx").join("graph");
        let entries: Vec<_> = fs::read_dir(&cx_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(entries.len() >= 2, "should have at least 2 graph files");
    }
}
