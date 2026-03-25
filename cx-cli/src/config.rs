use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CxConfig {
    #[serde(default)]
    pub repos: Vec<RepoEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RepoEntry {
    pub path: PathBuf,
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
    config.repos.push(RepoEntry { path: repo_path });
    true
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
