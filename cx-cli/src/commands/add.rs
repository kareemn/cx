use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;

/// Run `cx add <path>` — add a repo using incremental per-repo graph storage.
///
/// Flow:
/// 1. Add path to .cx/config.toml under [[repos]]
/// 2. Index ONLY the new repo → write to repos/NNNN-reponame.cxgraph
/// 3. Update global index (index.json) with new repo's APIs and targets
/// 4. Merge all per-repo graphs into base.cxgraph
/// 5. Run cross-repo resolution for new repo against index
///
/// Existing repos are NOT re-indexed.
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

    // Find repo index
    let repo_idx = config
        .repos
        .iter()
        .position(|r| r.path == repo)
        .unwrap();

    let start = Instant::now();

    // Ensure repos/ directory exists
    let cx_dir = root.join(".cx").join("graph");
    let repos_dir = cx_dir.join("repos");
    std::fs::create_dir_all(&repos_dir)?;

    // Check if per-repo graph already exists (re-add of same repo)
    let per_repo_name = crate::config::per_repo_filename(repo_idx, &repo);
    let per_repo_path = repos_dir.join(&per_repo_name);

    if added || !per_repo_path.exists() {
        // Index ONLY the new repo
        eprintln!("Indexing {} (repo {})...", repo.display(), repo_idx);
        let result = crate::indexing::index_single_repo(&repo, repo_idx as u16)?;

        // Write per-repo graph
        cx_core::store::mmap::write_graph(&result.graph, &per_repo_path)
            .with_context(|| format!("failed to write per-repo graph {}", per_repo_name))?;

        eprintln!(
            "  {} files: {} symbols, {} edges",
            result.file_count, result.node_count, result.edge_count,
        );

        if !result.errors.is_empty() {
            eprintln!("  Warnings:");
            for err in &result.errors {
                eprintln!("    {}", err);
            }
        }

        // Update global index
        let mut index = crate::graph_index::GlobalIndex::load(root).unwrap_or_default();
        index.remove_repo(repo_idx as u16);
        let repo_name = crate::config::repo_name(&repo);
        index.add_from_graph(repo_idx as u16, &repo_name, &result.graph);
        index.save(root)?;

        // Update overlay: resolve new repo's cross-repo edges against the index
        let mut overlay = crate::overlay::OverlayGraph::load(root).unwrap_or_default();
        overlay.remove_repo(repo_idx as u16);
        overlay.resolve_repo_against_index(repo_idx as u16, &index);
        overlay.save(root)?;

        if !overlay.edges.is_empty() {
            eprintln!("  Overlay: {} cross-repo edge(s)", overlay.edges.len());
        }
    } else {
        eprintln!("Per-repo graph already exists, skipping extraction");
    }

    // Merge all per-repo graphs + overlay into unified base.cxgraph
    eprintln!("Merging {} repo graphs...", config.repos.len());
    let merged = crate::indexing::merge_per_repo_graphs(root)?;
    let graph_path = cx_dir.join("base.cxgraph");
    cx_core::store::mmap::write_graph(&merged, &graph_path)?;

    let elapsed = start.elapsed();

    eprintln!(
        "Unified graph: {} symbols, {} edges in {:.1}ms ({} repos)",
        merged.node_count(),
        merged.edge_count(),
        elapsed.as_secs_f64() * 1000.0,
        config.repos.len(),
    );
    eprintln!("Graph written to {}", graph_path.display());

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

        // Should have base.cxgraph
        let graph_path = main_dir.path().join(".cx").join("graph").join("base.cxgraph");
        assert!(graph_path.exists(), "base.cxgraph should exist");

        // Should have per-repo graphs in repos/
        let repos_dir = main_dir.path().join(".cx").join("graph").join("repos");
        let entries: Vec<_> = fs::read_dir(&repos_dir)
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
            2,
            "should have 2 per-repo graph files, got: {:?}",
            entries.iter().map(|e| e.file_name()).collect::<Vec<_>>()
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

    #[test]
    fn add_creates_index_json() {
        let main_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();

        fs::write(
            main_dir.path().join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();
        fs::write(
            other_dir.path().join("server.go"),
            "package server\nfunc Serve() {}\n",
        )
        .unwrap();

        super::super::init::run(main_dir.path()).unwrap();
        run(main_dir.path(), other_dir.path().to_str().unwrap()).unwrap();

        let index_path = main_dir.path().join(".cx").join("graph").join("index.json");
        assert!(index_path.exists(), "index.json should exist after add");
    }
}
