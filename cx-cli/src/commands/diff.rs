use anyhow::{Context, Result};
use cx_extractors::taint::ResolvedNetworkCall;
use std::collections::HashMap;
use std::path::Path;

/// Run `cx diff` — compare network boundaries between current state and a baseline.
///
/// Without arguments: compares current network.json against the last saved baseline.
/// With --save: saves current state as the baseline for future diffs.
/// With --branch <name>: builds the graph on that branch and diffs against current.
pub fn run(root: &Path, save: bool, branch: Option<&str>, json: bool) -> Result<()> {
    let baseline_path = root.join(".cx").join("graph").join("network.baseline.json");
    let current_path = root.join(".cx").join("graph").join("network.json");

    if save {
        if !current_path.exists() {
            anyhow::bail!("No network.json found. Run `cx build` first.");
        }
        std::fs::copy(&current_path, &baseline_path)
            .context("failed to save baseline")?;
        let calls = load_calls(&current_path);
        eprintln!("Saved baseline: {} network call(s)", calls.len());
        return Ok(());
    }

    // If --branch, build a temporary graph for that branch and diff
    let (before_calls, before_label) = if let Some(branch_name) = branch {
        let calls = build_branch_snapshot(root, branch_name)?;
        (calls, branch_name.to_string())
    } else if baseline_path.exists() {
        let calls = load_calls(&baseline_path);
        (calls, "baseline".to_string())
    } else {
        eprintln!("No baseline found. Run `cx diff --save` first to save the current state,");
        eprintln!("or use `cx diff --branch <name>` to compare against another branch.");
        return Ok(());
    };

    let after_calls = load_calls(&current_path);
    if after_calls.is_empty() {
        anyhow::bail!("No network.json found. Run `cx build` first.");
    }

    let diff = compute_diff(&before_calls, &after_calls);

    if json {
        println!("{}", serde_json::to_string_pretty(&diff.to_json()).unwrap_or_default());
    } else {
        print_diff(&diff, &before_label);
    }

    Ok(())
}

struct NetworkDiff {
    added: Vec<DiffEntry>,
    removed: Vec<DiffEntry>,
    changed: Vec<(DiffEntry, DiffEntry)>, // (before, after)
}

#[derive(Clone)]
struct DiffEntry {
    file: String,
    line: u32,
    kind: String,
    callee: String,
    source: String,
}

impl DiffEntry {
    fn from_call(call: &ResolvedNetworkCall) -> Self {
        Self {
            file: call.file.clone(),
            line: call.line,
            kind: call.net_kind.as_str().to_string(),
            callee: call.callee_fqn.clone(),
            source: crate::indexing::format_address_chain(&call.address_source),
        }
    }

    fn key(&self) -> String {
        format!("{}:{}:{}", self.file, self.line, self.callee)
    }
}

impl NetworkDiff {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "added": self.added.iter().map(|e| serde_json::json!({
                "file": e.file, "line": e.line, "kind": e.kind,
                "callee": e.callee, "source": e.source,
            })).collect::<Vec<_>>(),
            "removed": self.removed.iter().map(|e| serde_json::json!({
                "file": e.file, "line": e.line, "kind": e.kind,
                "callee": e.callee, "source": e.source,
            })).collect::<Vec<_>>(),
            "changed": self.changed.iter().map(|(b, a)| serde_json::json!({
                "file": a.file, "line": a.line,
                "before": { "kind": b.kind, "callee": b.callee, "source": b.source },
                "after": { "kind": a.kind, "callee": a.callee, "source": a.source },
            })).collect::<Vec<_>>(),
            "summary": {
                "added": self.added.len(),
                "removed": self.removed.len(),
                "changed": self.changed.len(),
            }
        })
    }
}

fn compute_diff(before: &[ResolvedNetworkCall], after: &[ResolvedNetworkCall]) -> NetworkDiff {
    let before_map: HashMap<String, DiffEntry> = before
        .iter()
        .map(|c| {
            let entry = DiffEntry::from_call(c);
            (entry.key(), entry)
        })
        .collect();

    let after_map: HashMap<String, DiffEntry> = after
        .iter()
        .map(|c| {
            let entry = DiffEntry::from_call(c);
            (entry.key(), entry)
        })
        .collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut changed = Vec::new();

    // Find added and changed
    for (key, after_entry) in &after_map {
        if let Some(before_entry) = before_map.get(key) {
            // Exists in both — check if classification or source changed
            if before_entry.kind != after_entry.kind || before_entry.source != after_entry.source {
                changed.push((before_entry.clone(), after_entry.clone()));
            }
        } else {
            added.push(after_entry.clone());
        }
    }

    // Find removed
    for (key, before_entry) in &before_map {
        if !after_map.contains_key(key) {
            removed.push(before_entry.clone());
        }
    }

    // Sort for stable output
    added.sort_by_key(|a| a.key());
    removed.sort_by_key(|a| a.key());
    changed.sort_by_key(|(a, _)| a.key());

    NetworkDiff { added, removed, changed }
}

fn print_diff(diff: &NetworkDiff, before_label: &str) {
    if diff.is_empty() {
        println!("No network boundary changes (vs {})", before_label);
        return;
    }

    println!(
        "\x1b[1mNetwork boundary diff\x1b[0m \x1b[2m(vs {})\x1b[0m",
        before_label
    );
    println!(
        "  \x1b[32m+{} added\x1b[0m  \x1b[31m-{} removed\x1b[0m  \x1b[33m~{} changed\x1b[0m\n",
        diff.added.len(),
        diff.removed.len(),
        diff.changed.len(),
    );

    if !diff.added.is_empty() {
        println!("\x1b[32mAdded:\x1b[0m");
        for entry in &diff.added {
            println!(
                "  \x1b[32m+\x1b[0m \x1b[2m{}:{}\x1b[0m  {} {} → {}",
                entry.file, entry.line, entry.kind, entry.callee, entry.source,
            );
        }
        println!();
    }

    if !diff.removed.is_empty() {
        println!("\x1b[31mRemoved:\x1b[0m");
        for entry in &diff.removed {
            println!(
                "  \x1b[31m-\x1b[0m \x1b[2m{}:{}\x1b[0m  {} {} → {}",
                entry.file, entry.line, entry.kind, entry.callee, entry.source,
            );
        }
        println!();
    }

    if !diff.changed.is_empty() {
        println!("\x1b[33mChanged:\x1b[0m");
        for (before, after) in &diff.changed {
            println!(
                "  \x1b[33m~\x1b[0m \x1b[2m{}:{}\x1b[0m  {}",
                after.file, after.line, after.callee,
            );
            if before.kind != after.kind {
                println!(
                    "    kind: \x1b[31m{}\x1b[0m → \x1b[32m{}\x1b[0m",
                    before.kind, after.kind
                );
            }
            if before.source != after.source {
                println!(
                    "    source: \x1b[31m{}\x1b[0m → \x1b[32m{}\x1b[0m",
                    before.source, after.source
                );
            }
        }
    }
}

/// Build a temporary graph snapshot for a different branch.
fn build_branch_snapshot(root: &Path, branch: &str) -> Result<Vec<ResolvedNetworkCall>> {
    // Stash current changes, checkout branch, build, read network.json, return
    let current_branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(root)
        .output()
        .context("failed to get current branch")?;
    let current = String::from_utf8_lossy(&current_branch.stdout).trim().to_string();

    // Stash
    let _ = std::process::Command::new("git")
        .args(["stash", "--include-untracked"])
        .current_dir(root)
        .output();

    // Checkout target branch
    let checkout = std::process::Command::new("git")
        .args(["checkout", branch])
        .current_dir(root)
        .output()
        .context("failed to checkout branch")?;
    if !checkout.status.success() {
        // Restore
        let _ = std::process::Command::new("git")
            .args(["checkout", &current])
            .current_dir(root)
            .output();
        let _ = std::process::Command::new("git")
            .args(["stash", "pop"])
            .current_dir(root)
            .output();
        anyhow::bail!("failed to checkout branch: {}", branch);
    }

    // Build on the target branch
    eprintln!("Building graph on branch '{}'...", branch);
    let custom = cx_extractors::custom_sinks::CustomSinkConfig::load(root);
    let repos = vec![(root.to_path_buf(), 0u16)];
    let result = crate::indexing::index_repos_with_resolution(&repos, false, &custom, false);

    // Checkout back
    let _ = std::process::Command::new("git")
        .args(["checkout", &current])
        .current_dir(root)
        .output();
    let _ = std::process::Command::new("git")
        .args(["stash", "pop"])
        .current_dir(root)
        .output();

    let result = result.context("failed to build graph on target branch")?;
    Ok(result.network_calls)
}

fn load_calls(path: &Path) -> Vec<ResolvedNetworkCall> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}
