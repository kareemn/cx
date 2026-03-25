use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;

/// Run `cx add <path>` — add a repo and rebuild the unified graph.
///
/// Flow:
/// 1. Add path to .cx/config.toml under [[repos]]
/// 2. Re-read all repo paths from config
/// 3. Re-index ALL repos in parallel
/// 4. Build single unified CsrGraph
/// 5. Write to .cx/graph/base.cxgraph
pub fn run(root: &Path, repo_path: &str) -> Result<()> {
    let repo = Path::new(repo_path)
        .canonicalize()
        .with_context(|| format!("repo path not found: {}", repo_path))?;

    // Load config and add the new repo
    let mut config = crate::config::load(root).unwrap_or_default();
    let added = crate::config::add_repo(&mut config, repo.clone());
    if !added {
        eprintln!("Repo already tracked: {}", repo.display());
    } else {
        eprintln!("Adding {}...", repo.display());
    }
    crate::config::save(root, &config)?;

    let start = Instant::now();

    // Re-index ALL repos from config
    let repos: Vec<_> = config
        .repos
        .iter()
        .enumerate()
        .map(|(i, r)| (r.path.clone(), i as u16))
        .collect();

    eprintln!(
        "Indexing {} repo{}...",
        repos.len(),
        if repos.len() == 1 { "" } else { "s" }
    );

    let result = crate::indexing::index_repos_with_resolution(&repos)?;

    let elapsed = start.elapsed();

    // Write unified graph
    let cx_dir = root.join(".cx").join("graph");
    std::fs::create_dir_all(&cx_dir)?;
    let graph_path = cx_dir.join("base.cxgraph");
    cx_core::store::mmap::write_graph(&result.graph, &graph_path)?;

    eprintln!(
        "Indexed {} files: {} symbols, {} edges in {:.1}ms",
        result.file_count,
        result.node_count,
        result.edge_count,
        elapsed.as_secs_f64() * 1000.0,
    );

    eprintln!(
        "Unified graph written to {} ({} repos)",
        graph_path.display(),
        repos.len(),
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
    use cx_core::graph::nodes::NodeKind;
    use std::fs;

    #[test]
    fn add_creates_unified_graph() {
        let main_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();

        // Create main repo
        fs::write(
            main_dir.path().join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();
        super::super::init::run(main_dir.path()).unwrap();

        // Create other repo
        fs::write(
            other_dir.path().join("server.go"),
            "package server\nfunc Serve() {}\n",
        )
        .unwrap();

        // Add other repo
        run(main_dir.path(), other_dir.path().to_str().unwrap()).unwrap();

        // Should have ONE graph file (base.cxgraph), not separate per-repo files
        let cx_dir = main_dir.path().join(".cx").join("graph");
        let entries: Vec<_> = fs::read_dir(&cx_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "cxgraph")
            })
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "should have exactly 1 graph file, got: {:?}",
            entries.iter().map(|e| e.file_name()).collect::<Vec<_>>()
        );
        assert_eq!(
            entries[0].file_name().to_str().unwrap(),
            "base.cxgraph"
        );

        // Load the unified graph and verify it has nodes from both repos
        let graph = super::super::init::load_graph(main_dir.path()).unwrap();
        let names: Vec<&str> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8)
            .map(|n| graph.strings.get(n.name))
            .collect();

        assert!(names.contains(&"main"), "should have main from repo 1");
        assert!(names.contains(&"Serve"), "should have Serve from repo 2");
    }

    #[test]
    fn add_updates_config() {
        let main_dir = tempfile::tempdir().unwrap();
        let repo_a = tempfile::tempdir().unwrap();
        let repo_b = tempfile::tempdir().unwrap();

        fs::write(
            main_dir.path().join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();
        fs::write(
            repo_a.path().join("a.go"),
            "package a\nfunc A() {}\n",
        )
        .unwrap();
        fs::write(
            repo_b.path().join("b.go"),
            "package b\nfunc B() {}\n",
        )
        .unwrap();

        super::super::init::run(main_dir.path()).unwrap();
        run(main_dir.path(), repo_a.path().to_str().unwrap()).unwrap();
        run(main_dir.path(), repo_b.path().to_str().unwrap()).unwrap();

        // Config should have 3 repos
        let config = crate::config::load(main_dir.path()).unwrap();
        assert_eq!(config.repos.len(), 3, "should track 3 repos");

        // Graph should have symbols from all 3
        let graph = super::super::init::load_graph(main_dir.path()).unwrap();
        let names: Vec<&str> = graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8)
            .map(|n| graph.strings.get(n.name))
            .collect();

        assert!(names.contains(&"main"));
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));
    }

    #[test]
    fn add_same_repo_twice_is_idempotent() {
        let main_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();

        fs::write(
            main_dir.path().join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();
        fs::write(
            other_dir.path().join("lib.go"),
            "package lib\nfunc Lib() {}\n",
        )
        .unwrap();

        super::super::init::run(main_dir.path()).unwrap();
        run(main_dir.path(), other_dir.path().to_str().unwrap()).unwrap();
        run(main_dir.path(), other_dir.path().to_str().unwrap()).unwrap();

        let config = crate::config::load(main_dir.path()).unwrap();
        assert_eq!(config.repos.len(), 2, "should not duplicate repo");
    }

    #[test]
    fn add_assigns_distinct_repo_ids() {
        let main_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();

        fs::write(
            main_dir.path().join("main.go"),
            "package main\nfunc MainFunc() {}\n",
        )
        .unwrap();
        fs::write(
            other_dir.path().join("other.go"),
            "package other\nfunc OtherFunc() {}\n",
        )
        .unwrap();

        super::super::init::run(main_dir.path()).unwrap();
        run(main_dir.path(), other_dir.path().to_str().unwrap()).unwrap();

        let graph = super::super::init::load_graph(main_dir.path()).unwrap();

        // Find repo IDs for specific symbols
        let main_node = graph
            .nodes
            .iter()
            .find(|n| {
                n.kind == NodeKind::Symbol as u8
                    && graph.strings.get(n.name) == "MainFunc"
            })
            .expect("MainFunc should exist");

        let other_node = graph
            .nodes
            .iter()
            .find(|n| {
                n.kind == NodeKind::Symbol as u8
                    && graph.strings.get(n.name) == "OtherFunc"
            })
            .expect("OtherFunc should exist");

        assert_ne!(
            main_node.repo, other_node.repo,
            "nodes from different repos should have different repo IDs"
        );
    }
}
