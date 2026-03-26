use anyhow::{bail, Context, Result};
use std::path::Path;

/// Run `cx remote add <name> <path>` — register a remote graph source.
pub fn run_add(root: &Path, name: &str, remote_path: &str) -> Result<()> {
    let remote = Path::new(remote_path)
        .canonicalize()
        .with_context(|| format!("remote path not found: {}", remote_path))?;

    // Validate that the remote has a .cx directory
    let remote_cx = remote.join(".cx");
    if !remote_cx.exists() {
        bail!(
            "{} is not a cx workspace (no .cx/ directory). Run `cx init` there first.",
            remote.display()
        );
    }

    let mut config = crate::config::load(root).unwrap_or_default();
    let added = crate::config::add_remote(&mut config, name.to_string(), remote.clone());
    if !added {
        eprintln!("Remote '{}' already exists. Remove it first to update.", name);
        return Ok(());
    }
    crate::config::save(root, &config)?;
    eprintln!("Added remote '{}' -> {}", name, remote.display());
    Ok(())
}

/// Run `cx remote pull` — pull graphs from all (or a specific) remote.
pub fn run_pull(root: &Path, name_filter: Option<&str>) -> Result<()> {
    let mut config = crate::config::load(root)?;
    if config.remotes.is_empty() {
        eprintln!("No remotes configured. Use `cx remote add <name> <path>` first.");
        return Ok(());
    }

    let remotes_dir = root.join(".cx").join("remotes");
    std::fs::create_dir_all(&remotes_dir)?;

    let now = chrono_now();
    let mut pulled = 0u32;

    for remote in &mut config.remotes {
        if let Some(filter) = name_filter {
            if remote.name != filter {
                continue;
            }
        }

        eprintln!("Pulling from '{}'...", remote.name);

        let remote_graph_dir = remote.path.join(".cx").join("graph");
        if !remote_graph_dir.exists() {
            eprintln!("  Warning: {} has no graph data, skipping", remote.name);
            continue;
        }

        // Copy per-repo graphs: look for base.cxgraph or any repo graph
        let base_graph = remote_graph_dir.join("base.cxgraph");
        if base_graph.exists() {
            let dest = remotes_dir.join(format!("{}.cxgraph", remote.name));
            std::fs::copy(&base_graph, &dest).with_context(|| {
                format!("failed to copy graph from {}", remote.name)
            })?;
            let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            eprintln!("  Copied graph ({} bytes)", size);
        } else {
            eprintln!("  Warning: no base.cxgraph found for '{}'", remote.name);
        }

        // Copy index.json if available
        let remote_index = remote_graph_dir.join("index.json");
        if remote_index.exists() {
            let dest = remotes_dir.join(format!("{}.index.json", remote.name));
            std::fs::copy(&remote_index, &dest)?;
            eprintln!("  Copied index");
        }

        // Copy network.json if available
        let remote_network = remote_graph_dir.join("network.json");
        if remote_network.exists() {
            let dest = remotes_dir.join(format!("{}.network.json", remote.name));
            std::fs::copy(&remote_network, &dest)?;
            eprintln!("  Copied network data");
        }

        // Copy sink/taxonomy configs if they exist
        let remote_config_dir = remote.path.join(".cx").join("config");
        if remote_config_dir.exists() {
            let config_dest = remotes_dir.join(format!("{}.config", remote.name));
            std::fs::create_dir_all(&config_dest)?;
            for filename in &["sinks.toml", "taxonomy.toml"] {
                let src = remote_config_dir.join(filename);
                if src.exists() {
                    std::fs::copy(&src, config_dest.join(filename))?;
                    eprintln!("  Copied {}", filename);
                }
            }
        }

        remote.last_pulled = Some(now.clone());
        pulled += 1;
    }

    if pulled == 0 {
        if let Some(filter) = name_filter {
            bail!("Remote '{}' not found", filter);
        }
    }

    crate::config::save(root, &config)?;

    // Update global index with remote entries
    update_index_from_remotes(root)?;

    eprintln!("Pulled {} remote(s)", pulled);
    Ok(())
}

/// Run `cx remote push` — ensure local graph is ready for sharing.
pub fn run_push(root: &Path) -> Result<()> {
    let graph_path = root.join(".cx").join("graph").join("base.cxgraph");
    if !graph_path.exists() {
        bail!("No graph found. Run `cx init` first to build the graph.");
    }

    let size = std::fs::metadata(&graph_path)
        .map(|m| m.len())
        .unwrap_or(0);

    eprintln!("Graph ready at {}", graph_path.display());
    eprintln!("  Size: {} bytes", size);
    eprintln!("Other teams can pull this graph with:");
    eprintln!("  cx remote add <name> {}", root.display());
    Ok(())
}

/// Run `cx remote list` — show all configured remotes.
pub fn run_list(root: &Path) -> Result<()> {
    let config = crate::config::load(root)?;
    if config.remotes.is_empty() {
        eprintln!("No remotes configured.");
        return Ok(());
    }

    for remote in &config.remotes {
        let pulled = remote
            .last_pulled
            .as_deref()
            .unwrap_or("never");

        // Check if we have a pulled graph
        let graph_path = root
            .join(".cx")
            .join("remotes")
            .join(format!("{}.cxgraph", remote.name));
        let size = if graph_path.exists() {
            let bytes = std::fs::metadata(&graph_path)
                .map(|m| m.len())
                .unwrap_or(0);
            format!("{} bytes", bytes)
        } else {
            "not pulled".to_string()
        };

        println!(
            "{}\t{}\tlast_pulled: {}\tgraph: {}",
            remote.name,
            remote.path.display(),
            pulled,
            size,
        );
    }
    Ok(())
}

/// Merge remote index data into the local global index.
fn update_index_from_remotes(root: &Path) -> Result<()> {
    let remotes_dir = root.join(".cx").join("remotes");
    if !remotes_dir.exists() {
        return Ok(());
    }

    let mut index = crate::graph_index::GlobalIndex::load(root).unwrap_or_default();

    // Remote repos get repo IDs starting at 1000 to avoid collisions with local repos
    let mut remote_id = 1000u16;

    for entry in std::fs::read_dir(&remotes_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".index.json"))
        {
            let remote_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .trim_end_matches(".index")
                .to_string();

            // Remove old entries for this remote ID range
            index.remove_repo(remote_id);

            // Load the remote's index and merge it
            let content = std::fs::read_to_string(&path)?;
            if let Ok(remote_index) =
                serde_json::from_str::<crate::graph_index::GlobalIndex>(&content)
            {
                for (key, entries) in &remote_index.exposed_apis {
                    for e in entries {
                        let mut entry = e.clone();
                        entry.repo_id = remote_id;
                        entry.repo_name = format!("{}:{}", remote_name, entry.repo_name);
                        index
                            .exposed_apis
                            .entry(key.clone())
                            .or_default()
                            .push(entry);
                    }
                }
                for (key, entries) in &remote_index.outgoing_targets {
                    for e in entries {
                        let mut entry = e.clone();
                        entry.repo_id = remote_id;
                        entry.repo_name = format!("{}:{}", remote_name, entry.repo_name);
                        index
                            .outgoing_targets
                            .entry(key.clone())
                            .or_default()
                            .push(entry);
                    }
                }
            }

            remote_id += 1;
        }
    }

    index.save(root)?;
    Ok(())
}

/// Return current UTC timestamp as ISO 8601 string (no chrono dependency).
fn chrono_now() -> String {
    // Use std::process::Command to get UTC time to avoid adding a dependency
    let output = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output();
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        }
        _ => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_cx_workspace(dir: &Path) {
        fs::create_dir_all(dir.join(".cx").join("graph")).unwrap();
        // Write a minimal config
        let config = crate::config::CxConfig::default();
        crate::config::save(dir, &config).unwrap();
    }

    #[test]
    fn remote_add_and_list() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();

        setup_cx_workspace(local.path());
        setup_cx_workspace(remote.path());

        run_add(local.path(), "team-b", remote.path().to_str().unwrap()).unwrap();

        let config = crate::config::load(local.path()).unwrap();
        assert_eq!(config.remotes.len(), 1);
        assert_eq!(config.remotes[0].name, "team-b");
        assert_eq!(config.remotes[0].path, remote.path().canonicalize().unwrap());
    }

    #[test]
    fn remote_add_rejects_non_cx_dir() {
        let local = tempfile::tempdir().unwrap();
        let not_cx = tempfile::tempdir().unwrap();

        setup_cx_workspace(local.path());
        // not_cx has no .cx/ directory

        let result = run_add(local.path(), "bad", not_cx.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not a cx workspace")
        );
    }

    #[test]
    fn remote_add_duplicate_is_noop() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();

        setup_cx_workspace(local.path());
        setup_cx_workspace(remote.path());

        run_add(local.path(), "svc", remote.path().to_str().unwrap()).unwrap();
        run_add(local.path(), "svc", remote.path().to_str().unwrap()).unwrap();

        let config = crate::config::load(local.path()).unwrap();
        assert_eq!(config.remotes.len(), 1);
    }

    #[test]
    fn remote_pull_copies_graph() {
        let local = tempfile::tempdir().unwrap();
        let remote = tempfile::tempdir().unwrap();

        setup_cx_workspace(local.path());
        setup_cx_workspace(remote.path());

        // Create a fake graph in remote
        fs::write(
            remote.path().join(".cx").join("graph").join("base.cxgraph"),
            b"fake-graph-data",
        )
        .unwrap();

        run_add(local.path(), "team-b", remote.path().to_str().unwrap()).unwrap();
        run_pull(local.path(), None).unwrap();

        // Should have copied graph
        let pulled_graph = local
            .path()
            .join(".cx")
            .join("remotes")
            .join("team-b.cxgraph");
        assert!(pulled_graph.exists());
        assert_eq!(fs::read(&pulled_graph).unwrap(), b"fake-graph-data");

        // last_pulled should be set
        let config = crate::config::load(local.path()).unwrap();
        assert!(config.remotes[0].last_pulled.is_some());
    }

    #[test]
    fn remote_pull_with_filter() {
        let local = tempfile::tempdir().unwrap();
        let remote_a = tempfile::tempdir().unwrap();
        let remote_b = tempfile::tempdir().unwrap();

        setup_cx_workspace(local.path());
        setup_cx_workspace(remote_a.path());
        setup_cx_workspace(remote_b.path());

        fs::write(
            remote_a.path().join(".cx").join("graph").join("base.cxgraph"),
            b"graph-a",
        )
        .unwrap();
        fs::write(
            remote_b.path().join(".cx").join("graph").join("base.cxgraph"),
            b"graph-b",
        )
        .unwrap();

        run_add(local.path(), "svc-a", remote_a.path().to_str().unwrap()).unwrap();
        run_add(local.path(), "svc-b", remote_b.path().to_str().unwrap()).unwrap();

        // Pull only svc-a
        run_pull(local.path(), Some("svc-a")).unwrap();

        let remotes_dir = local.path().join(".cx").join("remotes");
        assert!(remotes_dir.join("svc-a.cxgraph").exists());
        assert!(!remotes_dir.join("svc-b.cxgraph").exists());
    }

    #[test]
    fn remote_push_requires_graph() {
        let local = tempfile::tempdir().unwrap();
        setup_cx_workspace(local.path());

        let result = run_push(local.path());
        assert!(result.is_err());
    }

    #[test]
    fn remote_push_succeeds_with_graph() {
        let local = tempfile::tempdir().unwrap();
        setup_cx_workspace(local.path());

        fs::write(
            local.path().join(".cx").join("graph").join("base.cxgraph"),
            b"my-graph",
        )
        .unwrap();

        run_push(local.path()).unwrap();
    }

    #[test]
    fn remote_list_empty() {
        let local = tempfile::tempdir().unwrap();
        setup_cx_workspace(local.path());
        run_list(local.path()).unwrap();
    }
}
