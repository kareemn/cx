use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;

/// Run `cx refresh` — re-index repos whose git HEAD has changed since last index.
///
/// For each tracked repo:
/// 1. Compare current git HEAD against stored hash in config.toml
/// 2. If changed, re-index that repo only and write updated per-repo graph
/// 3. Update the global index entries for changed repos
/// 4. Merge all per-repo graphs into base.cxgraph
pub fn run(root: &Path) -> Result<()> {
    let start = Instant::now();

    let mut config = crate::config::load(root)
        .context("failed to load config — run `cx init` first")?;

    if config.repos.is_empty() {
        eprintln!("No repos tracked. Run `cx init` first.");
        return Ok(());
    }

    let repos_dir = root.join(".cx").join("graph").join("repos");
    std::fs::create_dir_all(&repos_dir)?;

    let mut changed_count = 0;
    let mut index = crate::graph_index::GlobalIndex::load(root).unwrap_or_default();

    for (repo_idx, entry) in config.repos.iter_mut().enumerate() {
        let current_hash = crate::config::git_head_hash(&entry.path);
        let needs_update = match (&entry.git_hash, &current_hash) {
            (Some(old), Some(new)) => old != new,
            (None, Some(_)) => true,
            // If we can't get current hash, check if per-repo graph exists
            _ => {
                let per_repo_name = crate::config::per_repo_filename(repo_idx, &entry.path);
                !repos_dir.join(&per_repo_name).exists()
            }
        };

        if !needs_update {
            eprintln!(
                "  {} — unchanged",
                crate::config::repo_name(&entry.path),
            );
            continue;
        }

        eprintln!(
            "  {} — re-indexing...",
            crate::config::repo_name(&entry.path),
        );

        let result = crate::indexing::index_single_repo(&entry.path, repo_idx as u16)?;

        // Write per-repo graph
        let per_repo_name = crate::config::per_repo_filename(repo_idx, &entry.path);
        let per_repo_path = repos_dir.join(&per_repo_name);
        cx_core::store::mmap::write_graph(&result.graph, &per_repo_path)
            .with_context(|| format!("failed to write {}", per_repo_name))?;

        // Update index for this repo
        index.remove_repo(repo_idx as u16);
        let repo_name = crate::config::repo_name(&entry.path);
        index.add_from_graph(repo_idx as u16, &repo_name, &result.graph);

        // Update git hash
        entry.git_hash = current_hash;

        eprintln!(
            "    {} files, {} symbols, {} edges",
            result.file_count, result.node_count, result.edge_count,
        );

        changed_count += 1;
    }

    if changed_count == 0 {
        eprintln!("All repos up to date.");
        return Ok(());
    }

    // Save updated config with new hashes
    crate::config::save(root, &config)?;

    // Save updated index
    index.save(root)?;

    // Rebuild overlay: re-resolve cross-repo edges for all repos against updated index
    let mut overlay = crate::overlay::OverlayGraph::load(root).unwrap_or_default();
    for (repo_idx, _entry) in config.repos.iter().enumerate() {
        overlay.remove_repo(repo_idx as u16);
        overlay.resolve_repo_against_index(repo_idx as u16, &index);
    }
    overlay.save(root)?;
    if !overlay.edges.is_empty() {
        eprintln!("  Overlay: {} cross-repo edge(s)", overlay.edges.len());
    }

    // Merge all per-repo graphs + overlay into base.cxgraph
    eprintln!("Merging {} repo graphs...", config.repos.len());
    let merged = crate::indexing::merge_per_repo_graphs(root)?;
    let graph_path = root.join(".cx").join("graph").join("base.cxgraph");
    cx_core::store::mmap::write_graph(&merged, &graph_path)?;

    let elapsed = start.elapsed();

    eprintln!(
        "Refreshed {} repo{} in {:.1}ms — {} symbols, {} edges",
        changed_count,
        if changed_count == 1 { "" } else { "s" },
        elapsed.as_secs_f64() * 1000.0,
        merged.node_count(),
        merged.edge_count(),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn refresh_detects_no_changes() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        super::super::init::run(dir.path()).unwrap();

        // Refresh should succeed with no changes
        run(dir.path()).unwrap();

        // Graph should still be loadable
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        assert!(graph.node_count() > 0);
    }

    #[test]
    fn refresh_reindexes_when_hash_missing() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        super::super::init::run(dir.path()).unwrap();

        // Clear git hash to force re-index
        let mut config = crate::config::load(dir.path()).unwrap();
        config.repos[0].git_hash = None;
        crate::config::save(dir.path(), &config).unwrap();

        // Delete per-repo graph to force re-index
        let repos_dir = dir.path().join(".cx").join("graph").join("repos");
        if repos_dir.exists() {
            for entry in fs::read_dir(&repos_dir).unwrap() {
                let entry = entry.unwrap();
                fs::remove_file(entry.path()).unwrap();
            }
        }

        run(dir.path()).unwrap();

        // Should have re-created the graph
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        assert!(graph.node_count() > 0);
    }
}
