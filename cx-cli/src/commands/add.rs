use anyhow::{bail, Context, Result};
use std::path::Path;

/// Run `cx add <path_or_url>` — register a remote repo and import its pre-built graph.
///
/// Accepts a local path or git URL. Does NOT re-index the remote repo —
/// it must already have been built with `cx build`. Copies the .cxgraph
/// and network.json into the local workspace for cross-repo queries.
pub fn run(root: &Path, remote_path: &str) -> Result<()> {
    let (resolved_path, name) = resolve_remote(root, remote_path)?;

    // Validate the remote has a built graph
    let remote_graph = resolved_path.join(".cx").join("graph").join("base.cxgraph");
    if !remote_graph.exists() {
        bail!(
            "{} has no graph (no .cx/graph/base.cxgraph). Run `cx build` there first.",
            resolved_path.display()
        );
    }

    // Register in config
    let mut config = crate::config::load(root).unwrap_or_default();
    let added = crate::config::add_remote(&mut config, name.clone(), resolved_path.clone());
    if !added {
        eprintln!("Remote '{}' already registered. Use `cx pull` to refresh.", name);
        return Ok(());
    }
    crate::config::save(root, &config)?;

    // Copy graph artifacts
    let copied = pull_remote(root, &name, &resolved_path)?;

    eprintln!("Added '{}' from {}", name, resolved_path.display());
    if copied > 0 {
        eprintln!("  Copied {} artifact(s)", copied);
    }

    // Rebuild unified graph with the new remote included
    rebuild_unified(root)?;

    Ok(())
}

/// Run `cx pull` — refresh graphs from all registered remotes.
pub fn run_pull(root: &Path, name_filter: Option<&str>) -> Result<()> {
    let mut config = crate::config::load(root)?;
    if config.remotes.is_empty() {
        eprintln!("No remotes registered. Use `cx add <path_or_url>` first.");
        return Ok(());
    }

    let mut pulled = 0u32;
    for remote in &mut config.remotes {
        if let Some(filter) = name_filter {
            if remote.name != filter {
                continue;
            }
        }

        // For git remotes, pull latest
        let clones_dir = root.join(".cx").join("remotes").join("clones");
        let clone_path = clones_dir.join(&remote.name);
        if clone_path.exists() && clone_path.join(".git").exists() {
            eprint!("Pulling '{}'...", remote.name);
            let status = std::process::Command::new("git")
                .args(["pull", "--ff-only"])
                .current_dir(&clone_path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if let Ok(s) = status {
                if s.success() {
                    eprintln!(" updated");
                } else {
                    eprintln!(" git pull failed, using existing");
                }
            }
        }

        match pull_remote(root, &remote.name, &remote.path) {
            Ok(n) => {
                if n > 0 {
                    eprintln!("  '{}': {} artifact(s) refreshed", remote.name, n);
                    pulled += 1;
                } else {
                    eprintln!("  '{}': up to date", remote.name);
                }
            }
            Err(e) => {
                eprintln!("  '{}': {}", remote.name, e);
            }
        }
    }

    if pulled > 0 {
        rebuild_unified(root)?;
    }

    Ok(())
}

/// Resolve a remote path: local directory or git URL → (local_path, name).
fn resolve_remote(root: &Path, remote_path: &str) -> Result<(std::path::PathBuf, String)> {
    if is_git_url(remote_path) {
        let name = repo_name_from_url(remote_path);
        let clones_dir = root.join(".cx").join("remotes").join("clones");
        std::fs::create_dir_all(&clones_dir)?;
        let clone_dest = clones_dir.join(&name);

        if clone_dest.exists() {
            eprintln!("Updating existing clone for '{}'...", name);
            let _ = std::process::Command::new("git")
                .args(["pull", "--ff-only"])
                .current_dir(&clone_dest)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        } else {
            eprintln!("Cloning {}...", remote_path);
            let status = std::process::Command::new("git")
                .args(["clone", "--depth", "1", remote_path, clone_dest.to_str().unwrap_or(".")])
                .status()
                .with_context(|| format!("failed to clone {}", remote_path))?;
            if !status.success() {
                bail!("git clone failed for {}", remote_path);
            }
        }

        Ok((clone_dest, name))
    } else {
        let local = Path::new(remote_path)
            .canonicalize()
            .with_context(|| format!("path not found: {}", remote_path))?;
        let name = local
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("remote")
            .to_string();
        Ok((local, name))
    }
}

/// Copy graph artifacts from a remote's .cx/graph/ into our remotes directory.
/// Returns count of files copied.
fn pull_remote(root: &Path, name: &str, remote_path: &Path) -> Result<u32> {
    let remotes_dir = root.join(".cx").join("remotes");
    std::fs::create_dir_all(&remotes_dir)?;

    let remote_graph_dir = remote_path.join(".cx").join("graph");
    let mut copied = 0u32;

    // Copy base.cxgraph → remotes/{name}.cxgraph
    let src_graph = remote_graph_dir.join("base.cxgraph");
    let dst_graph = remotes_dir.join(format!("{}.cxgraph", name));
    if src_graph.exists() {
        std::fs::copy(&src_graph, &dst_graph)
            .with_context(|| format!("failed to copy graph for '{}'", name))?;
        copied += 1;
    }

    // Copy network.json → remotes/{name}.network.json
    let src_net = remote_graph_dir.join("network.json");
    let dst_net = remotes_dir.join(format!("{}.network.json", name));
    if src_net.exists() {
        std::fs::copy(&src_net, &dst_net)
            .with_context(|| format!("failed to copy network.json for '{}'", name))?;
        copied += 1;
    }

    Ok(copied)
}

/// Rebuild the unified graph by merging local per-repo graphs with remote graphs.
/// After merging, runs cross-repo env var resolution to create Resolves edges
/// between matching Resource nodes across repos.
fn rebuild_unified(root: &Path) -> Result<()> {
    use cx_core::graph::csr::EdgeInput;
    use cx_core::graph::edges::{EdgeKind, EDGE_IS_CROSS_REPO};
    use cx_core::graph::nodes::NodeKind;

    let cx_dir = root.join(".cx").join("graph");
    std::fs::create_dir_all(&cx_dir)?;
    let graph_path = cx_dir.join("base.cxgraph");

    // Merge all per-repo + remote graphs
    let merged = match crate::indexing::merge_per_repo_graphs(root) {
        Ok(m) => m,
        Err(_) => return Ok(()), // No graphs yet
    };

    // Cross-repo env var resolution: find Resource nodes with matching names
    // across different repos and create Resolves edges between them.
    // This links code-side os.Getenv("SERVICE_ADDR") to K8s-side SERVICE_ADDR=myservice:8080
    let mut cross_repo_edges: Vec<EdgeInput> = Vec::new();

    // Build a map: env_var_name → [(node_index, repo_id)]
    let mut env_var_nodes: rustc_hash::FxHashMap<String, Vec<(u32, u16)>> =
        rustc_hash::FxHashMap::default();

    for (idx, node) in merged.nodes.iter().enumerate() {
        if node.kind == NodeKind::Resource as u8 {
            let name = merged.strings.get(node.name);
            // Only match env-var-like names (UPPER_CASE_WITH_UNDERSCORES)
            if !name.is_empty()
                && name.len() > 1
                && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                env_var_nodes
                    .entry(name.to_string())
                    .or_default()
                    .push((idx as u32, node.repo));
            }
        }
    }

    // For each env var that appears in multiple repos, create Resolves edges
    for (_name, nodes) in &env_var_nodes {
        if nodes.len() < 2 {
            continue;
        }
        // Create edges between all cross-repo pairs
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                let (idx_a, repo_a) = nodes[i];
                let (idx_b, repo_b) = nodes[j];
                if repo_a == repo_b {
                    continue;
                }
                // Bidirectional Resolves edges
                let mut edge = EdgeInput::new(idx_a, idx_b, EdgeKind::Resolves);
                edge.confidence_u8 = 200;
                edge.flags = EDGE_IS_CROSS_REPO;
                cross_repo_edges.push(edge);

                let mut edge_rev = EdgeInput::new(idx_b, idx_a, EdgeKind::Resolves);
                edge_rev.confidence_u8 = 200;
                edge_rev.flags = EDGE_IS_CROSS_REPO;
                cross_repo_edges.push(edge_rev);
            }
        }
    }

    let cross_count = cross_repo_edges.len() / 2; // each pair is bidirectional

    // If we found cross-repo matches, rebuild with the new edges
    let final_graph = if cross_repo_edges.is_empty() {
        merged
    } else {
        // Re-merge with extra edges
        let graphs = vec![merged];
        cx_core::graph::csr::CsrGraph::merge(&graphs, cross_repo_edges)
    };

    cx_core::store::mmap::write_graph(&final_graph, &graph_path)?;
    eprintln!(
        "Unified graph: {} symbols, {} edges{}",
        final_graph.node_count(),
        final_graph.edge_count(),
        if cross_count > 0 {
            format!(" ({} cross-repo env var link(s))", cross_count)
        } else {
            String::new()
        },
    );

    Ok(())
}

fn is_git_url(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://")
        || s.starts_with("git@") || s.starts_with("ssh://")
        || s.ends_with(".git")
}

fn repo_name_from_url(url: &str) -> String {
    let s = url.trim_end_matches(".git");
    s.rsplit('/')
        .next()
        .unwrap_or("remote")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_url_detection() {
        assert!(is_git_url("https://github.com/org/repo.git"));
        assert!(is_git_url("git@github.com:org/repo.git"));
        assert!(is_git_url("https://github.com/org/repo"));
        assert!(!is_git_url("/Users/me/code/repo"));
        assert!(!is_git_url("./relative/path"));
    }

    #[test]
    fn repo_name_from_url_extracts_name() {
        assert_eq!(repo_name_from_url("https://github.com/org/my-service.git"), "my-service");
        assert_eq!(repo_name_from_url("git@github.com:org/api-gateway.git"), "api-gateway");
        assert_eq!(repo_name_from_url("https://github.com/org/repo"), "repo");
    }

    #[test]
    fn add_and_pull_local() {
        let workspace = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();

        // Set up remote with a built graph
        std::fs::write(remote.path().join("main.go"), "package main\nfunc main() {}\n").unwrap();
        crate::commands::build::run(remote.path(), &[], false).unwrap();

        // Add it
        run(workspace.path(), remote.path().to_str().unwrap()).unwrap();

        // Check artifacts were copied
        let config = crate::config::load(workspace.path()).unwrap();
        assert_eq!(config.remotes.len(), 1);

        let remotes_dir = workspace.path().join(".cx").join("remotes");
        let name = &config.remotes[0].name;
        assert!(remotes_dir.join(format!("{}.cxgraph", name)).exists());
    }
}
