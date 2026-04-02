use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;

/// Run `cx build [paths...]` — index one or more repos and write the unified graph.
///
/// If no paths are given, indexes the current directory (like old `cx init`).
/// If multiple paths are given, indexes all of them with cross-repo resolution.
pub fn run(root: &Path, paths: &[String], verbose: bool, model_only: bool) -> Result<()> {
    let start = Instant::now();

    // Determine which paths to index
    let repo_paths: Vec<std::path::PathBuf> = if paths.is_empty() {
        vec![root.to_path_buf()]
    } else {
        paths
            .iter()
            .map(|p| {
                let path = Path::new(p);
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    root.join(p)
                }
            })
            .collect()
    };

    // Canonicalize all paths and register in config
    let mut config = crate::config::load(root).unwrap_or_default();
    let mut canon_paths = Vec::with_capacity(repo_paths.len());

    for repo_path in &repo_paths {
        let canon = repo_path
            .canonicalize()
            .with_context(|| format!("path not found: {}", repo_path.display()))?;
        crate::config::add_repo(&mut config, canon.clone());
        canon_paths.push(canon);
    }
    crate::config::save(root, &config)?;

    // Build (path, repo_id) pairs for all configured repos
    let repos: Vec<_> = config
        .repos
        .iter()
        .enumerate()
        .map(|(i, r)| (r.path.clone(), i as u16))
        .collect();

    eprintln!(
        "Building graph for {} repo(s)...",
        repos.len()
    );
    for (path, id) in &repos {
        eprintln!("  [{}] {}", id, path.display());
    }

    // Load custom sink definitions from .cx/config/sinks.toml
    let custom_sinks = cx_extractors::custom_sinks::CustomSinkConfig::load(root);
    if !custom_sinks.is_empty() {
        eprintln!(
            "Custom sinks: {} sink(s), {} endpoint(s) from .cx/config/sinks.toml",
            custom_sinks.sinks.len(),
            custom_sinks.endpoints.len(),
        );
    }

    if model_only {
        eprintln!("Mode: model-only (skipping static classification, all calls go to LLM)");
    }

    let result = crate::indexing::index_repos_with_resolution(&repos, verbose, &custom_sinks, model_only)?;

    let elapsed = start.elapsed();

    // Create .cx/graph/ and repos/ directories
    let cx_dir = root.join(".cx").join("graph");
    let repos_dir = cx_dir.join("repos");
    std::fs::create_dir_all(&repos_dir)
        .context("failed to create .cx/graph/repos/ directory")?;

    // Write per-repo graphs for each path we just indexed
    for canon in &canon_paths {
        let repo_idx = config
            .repos
            .iter()
            .position(|r| r.path == *canon)
            .unwrap_or(0);
        let per_repo_name = crate::config::per_repo_filename(repo_idx, canon);
        let per_repo_path = repos_dir.join(&per_repo_name);
        cx_core::store::mmap::write_graph(&result.graph, &per_repo_path)
            .with_context(|| format!("failed to write per-repo graph {}", per_repo_name))?;
    }

    // Write unified base.cxgraph
    let graph_path = cx_dir.join("base.cxgraph");
    cx_core::store::mmap::write_graph(&result.graph, &graph_path)
        .context("failed to write graph file")?;

    // Build and write global index
    let mut index = crate::graph_index::GlobalIndex::default();
    for canon in &canon_paths {
        let repo_idx = config
            .repos
            .iter()
            .position(|r| r.path == *canon)
            .unwrap_or(0);
        let repo_name = crate::config::repo_name(canon);
        index.add_from_graph(repo_idx as u16, &repo_name, &result.graph);
    }
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

    // Write network calls with provenance to network.json.
    // Filter out Unknown calls — these are permissive filter candidates that the
    // LLM didn't classify as network (either said not_network or wasn't processed).
    if !result.network_calls.is_empty() {
        let classified: Vec<_> = result.network_calls.iter()
            .filter(|c| c.net_kind != cx_extractors::sink_registry::NetworkCategory::Unknown)
            .collect();
        let dropped = result.network_calls.len() - classified.len();
        let network_path = cx_dir.join("network.json");
        let json = serde_json::to_string_pretty(&classified)
            .context("failed to serialize network calls")?;
        std::fs::write(&network_path, json)
            .context("failed to write network.json")?;
        eprintln!(
            "Network analysis: {} call(s) written to {} ({} candidates filtered by LLM)",
            classified.len(),
            network_path.display(),
            dropped,
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn build_creates_graph() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        run(dir.path(), &[], false, false).unwrap();

        let graph_path = dir.path().join(".cx").join("graph").join("base.cxgraph");
        assert!(graph_path.exists());
        assert!(fs::metadata(&graph_path).unwrap().len() > 0);
    }

    #[test]
    fn build_with_explicit_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc hello() {}\n",
        )
        .unwrap();

        run(dir.path(), &[".".to_string()], false, false).unwrap();

        let graph_path = dir.path().join(".cx").join("graph").join("base.cxgraph");
        assert!(graph_path.exists());
    }

    #[test]
    fn build_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\n\nfunc hello() {}\nfunc world() { hello() }\n",
        )
        .unwrap();

        run(dir.path(), &[], false, false).unwrap();

        let graph = crate::indexing::load_graph(dir.path()).unwrap();
        assert!(graph.node_count() > 0);

        let names: Vec<&str> = graph
            .nodes
            .iter()
            .map(|n| graph.strings.get(n.name))
            .collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"world"));
    }
}
