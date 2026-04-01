use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CxConfig {
    #[serde(default)]
    pub repos: Vec<RepoEntry>,
    #[serde(default)]
    pub remotes: Vec<RemoteEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoEntry {
    pub path: PathBuf,
    /// Git HEAD hash at last index time, for change detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_hash: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RemoteEntry {
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_pulled: Option<String>,
}

fn config_path(root: &Path) -> PathBuf {
    root.join(".cx").join("config.toml")
}

/// Load config from .cx/config.toml, returning default if it doesn't exist.
pub fn load(root: &Path) -> Result<CxConfig> {
    let path = config_path(root);
    if !path.exists() {
        return Ok(CxConfig::default());
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

/// Save config to .cx/config.toml.
pub fn save(root: &Path, config: &CxConfig) -> Result<()> {
    let cx_dir = root.join(".cx");
    std::fs::create_dir_all(&cx_dir)?;
    let path = config_path(root);
    let content = toml::to_string_pretty(config).context("failed to serialize config")?;
    std::fs::write(&path, content).with_context(|| format!("failed to write {}", path.display()))
}

/// Add a repo path to the config if not already present. Returns true if added.
pub fn add_repo(config: &mut CxConfig, repo_path: PathBuf) -> bool {
    if config.repos.iter().any(|r| r.path == repo_path) {
        return false;
    }
    let git_hash = git_head_hash(&repo_path);
    config.repos.push(RepoEntry {
        path: repo_path,
        git_hash,
    });
    true
}

/// Update the git hash for a repo in the config. Returns true if found and updated.
#[allow(dead_code)]
pub fn update_git_hash(config: &mut CxConfig, repo_path: &Path) -> bool {
    if let Some(entry) = config.repos.iter_mut().find(|r| r.path == repo_path) {
        entry.git_hash = git_head_hash(repo_path);
        true
    } else {
        false
    }
}

/// Get the git HEAD hash for a repo path, or None if not a git repo.
pub fn git_head_hash(repo_path: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Add a remote to the config if not already present by name. Returns true if added.
pub fn add_remote(config: &mut CxConfig, name: String, path: PathBuf) -> bool {
    if config.remotes.iter().any(|r| r.name == name) {
        return false;
    }
    config.remotes.push(RemoteEntry {
        name,
        path,
        last_pulled: None,
    });
    true
}

/// Derive a short repo name from a path (last component).
pub fn repo_name(repo_path: &Path) -> String {
    repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("repo")
        .to_string()
}

/// Generate the per-repo graph filename: NNNN-reponame.cxgraph
pub fn per_repo_filename(index: usize, repo_path: &Path) -> String {
    format!("{:04}-{}.cxgraph", index, repo_name(repo_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = CxConfig::default();
        add_repo(&mut config, PathBuf::from("/tmp/repo1"));
        add_repo(&mut config, PathBuf::from("/tmp/repo2"));

        save(dir.path(), &config).unwrap();
        let loaded = load(dir.path()).unwrap();

        assert_eq!(loaded.repos.len(), 2);
        assert_eq!(loaded.repos[0].path, PathBuf::from("/tmp/repo1"));
        assert_eq!(loaded.repos[1].path, PathBuf::from("/tmp/repo2"));
    }

    #[test]
    fn add_repo_dedup() {
        let mut config = CxConfig::default();
        assert!(add_repo(&mut config, PathBuf::from("/tmp/repo1")));
        assert!(!add_repo(&mut config, PathBuf::from("/tmp/repo1")));
        assert_eq!(config.repos.len(), 1);
    }

    #[test]
    fn load_missing_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = load(dir.path()).unwrap();
        assert!(config.repos.is_empty());
    }
}
