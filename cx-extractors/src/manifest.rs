/// Parsed dependency manifest information.
#[derive(Debug, Clone)]
pub struct ManifestInfo {
    /// Package/module name (e.g., "github.com/org/repo", "my-package").
    pub name: String,
    /// Declared dependencies.
    pub deps: Vec<DeclaredDep>,
}

/// A dependency declared in a manifest file.
#[derive(Debug, Clone)]
pub struct DeclaredDep {
    /// Dependency name/path.
    pub name: String,
    /// Version string.
    pub version: String,
    /// Whether this is a dev/test dependency.
    pub is_dev: bool,
}

/// Parse a `go.mod` file.
pub fn parse_go_mod(content: &str) -> ManifestInfo {
    let mut name = String::new();
    let mut deps = Vec::new();
    let mut in_require = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("module ") {
            name = rest.trim().to_string();
            continue;
        }

        if trimmed == "require (" {
            in_require = true;
            continue;
        }
        if trimmed == ")" {
            in_require = false;
            continue;
        }

        // Single-line require
        if let Some(rest) = trimmed.strip_prefix("require ") {
            if !rest.starts_with('(') {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() >= 2 {
                    deps.push(DeclaredDep {
                        name: parts[0].to_string(),
                        version: parts[1].to_string(),
                        is_dev: false,
                    });
                }
            }
            continue;
        }

        if in_require && !trimmed.is_empty() && !trimmed.starts_with("//") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                let is_indirect = trimmed.contains("// indirect");
                deps.push(DeclaredDep {
                    name: parts[0].to_string(),
                    version: parts[1].to_string(),
                    is_dev: is_indirect,
                });
            }
        }
    }

    ManifestInfo { name, deps }
}

/// Parse a `package.json` file using serde_json.
pub fn parse_package_json(content: &str) -> ManifestInfo {
    let parsed: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return ManifestInfo { name: String::new(), deps: Vec::new() },
    };

    let name = parsed
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut deps = Vec::new();

    if let Some(obj) = parsed.get("dependencies").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            deps.push(DeclaredDep {
                name: k.clone(),
                version: v.as_str().unwrap_or("").to_string(),
                is_dev: false,
            });
        }
    }

    if let Some(obj) = parsed.get("devDependencies").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            deps.push(DeclaredDep {
                name: k.clone(),
                version: v.as_str().unwrap_or("").to_string(),
                is_dev: true,
            });
        }
    }

    ManifestInfo { name, deps }
}

/// Parse a `requirements.txt` file.
pub fn parse_requirements_txt(content: &str) -> ManifestInfo {
    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
            continue;
        }

        // Handle: package==version, package>=version, package~=version, package
        let (name, version) = if let Some(idx) = trimmed.find("==") {
            (&trimmed[..idx], &trimmed[idx + 2..])
        } else if let Some(idx) = trimmed.find(">=") {
            (&trimmed[..idx], &trimmed[idx + 2..])
        } else if let Some(idx) = trimmed.find("~=") {
            (&trimmed[..idx], &trimmed[idx + 2..])
        } else if let Some(idx) = trimmed.find("<=") {
            (&trimmed[..idx], &trimmed[idx + 2..])
        } else if let Some(idx) = trimmed.find("!=") {
            (&trimmed[..idx], &trimmed[idx + 2..])
        } else {
            (trimmed, "")
        };

        // Strip extras (e.g., package[extra]==version)
        let clean_name = if let Some(bracket) = name.find('[') {
            &name[..bracket]
        } else {
            name
        };

        if !clean_name.is_empty() {
            deps.push(DeclaredDep {
                name: clean_name.trim().to_string(),
                version: version.trim().to_string(),
                is_dev: false,
            });
        }
    }

    ManifestInfo {
        name: String::new(),
        deps,
    }
}

/// Parse a `pyproject.toml` file (minimal TOML parsing for dependencies).
pub fn parse_pyproject_toml(content: &str) -> ManifestInfo {
    let mut name = String::new();
    let mut deps = Vec::new();
    let mut in_deps = false;
    let mut in_dev_deps = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Project name
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('=') {
                let val = rest
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                if !val.is_empty() {
                    name = val.to_string();
                }
            }
        }

        // Section headers
        if trimmed == "[project]" || trimmed == "[tool.poetry]" {
            in_deps = false;
            in_dev_deps = false;
            continue;
        }
        if trimmed == "dependencies = [" || trimmed.starts_with("dependencies = [") {
            in_deps = true;
            // Handle inline list on same line
            if let Some(bracket_content) = extract_inline_list(trimmed) {
                for dep_str in bracket_content {
                    if let Some(dep) = parse_pep508_dep(&dep_str, false) {
                        deps.push(dep);
                    }
                }
                if trimmed.contains(']') {
                    in_deps = false;
                }
            }
            continue;
        }
        if trimmed.starts_with("[project.optional-dependencies")
            || trimmed == "[tool.poetry.dev-dependencies]"
        {
            in_dev_deps = true;
            in_deps = false;
            continue;
        }
        if trimmed.starts_with('[') {
            in_deps = false;
            in_dev_deps = false;
            continue;
        }

        if trimmed == "]" {
            in_deps = false;
            // Don't reset in_dev_deps — it uses TOML table, not list
            continue;
        }

        if in_deps {
            let dep_str = trimmed.trim_matches(',').trim_matches('"').trim_matches('\'');
            if !dep_str.is_empty() {
                if let Some(dep) = parse_pep508_dep(dep_str, false) {
                    deps.push(dep);
                }
            }
        }

        if in_dev_deps {
            // poetry style: name = "^version"
            if let Some(eq_idx) = trimmed.find('=') {
                let dep_name = trimmed[..eq_idx].trim();
                let version = trimmed[eq_idx + 1..]
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .trim_start_matches('^')
                    .trim_start_matches('~');
                if !dep_name.is_empty() && !dep_name.starts_with('[') {
                    deps.push(DeclaredDep {
                        name: dep_name.to_string(),
                        version: version.to_string(),
                        is_dev: true,
                    });
                }
            }
        }
    }

    ManifestInfo { name, deps }
}

/// Parse a PEP 508 dependency string: "requests>=2.28.0" → DeclaredDep.
fn parse_pep508_dep(s: &str, is_dev: bool) -> Option<DeclaredDep> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('#') {
        return None;
    }

    // Find the version specifier start
    let version_start = s.find(['>', '<', '=', '~', '!']);
    let (name, version) = if let Some(idx) = version_start {
        (&s[..idx], s[idx..].trim_start_matches(['>', '<', '=', '~', '!']))
    } else {
        (s, "")
    };

    // Strip extras
    let clean_name = if let Some(bracket) = name.find('[') {
        &name[..bracket]
    } else {
        name
    };

    if clean_name.trim().is_empty() {
        return None;
    }

    Some(DeclaredDep {
        name: clean_name.trim().to_string(),
        version: version.trim().to_string(),
        is_dev,
    })
}

/// Extract items from an inline TOML list like `["foo>=1.0", "bar"]`.
fn extract_inline_list(line: &str) -> Option<Vec<String>> {
    let start = line.find('[')?;
    let content = &line[start + 1..];
    let end = content.find(']').unwrap_or(content.len());
    let inner = &content[..end];

    let items: Vec<String> = inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Some(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_go_mod_basic() {
        let content = r#"module github.com/GoogleCloudPlatform/microservices-demo/src/frontend

go 1.21

require (
	cloud.google.com/go/compute/metadata v0.2.3
	github.com/google/uuid v1.6.0
	github.com/gorilla/mux v1.8.0
	google.golang.org/grpc v1.64.0
	google.golang.org/grpc/examples v0.0.0 // indirect
)

require github.com/single/dep v1.0.0
"#;
        let info = parse_go_mod(content);
        assert_eq!(
            info.name,
            "github.com/GoogleCloudPlatform/microservices-demo/src/frontend"
        );
        assert!(info.deps.len() >= 5);

        let grpc = info.deps.iter().find(|d| d.name == "google.golang.org/grpc");
        assert!(grpc.is_some());
        assert_eq!(grpc.unwrap().version, "v1.64.0");
        assert!(!grpc.unwrap().is_dev);

        let indirect = info
            .deps
            .iter()
            .find(|d| d.name == "google.golang.org/grpc/examples");
        assert!(indirect.is_some());
        assert!(indirect.unwrap().is_dev);

        let single = info.deps.iter().find(|d| d.name == "github.com/single/dep");
        assert!(single.is_some());
    }

    #[test]
    fn parse_package_json_basic() {
        let content = r#"{
  "name": "my-service",
  "version": "1.0.0",
  "dependencies": {
    "express": "^4.18.0",
    "grpc": "~1.24.0"
  },
  "devDependencies": {
    "jest": "^29.0.0"
  }
}"#;
        let info = parse_package_json(content);
        assert_eq!(info.name, "my-service");
        assert_eq!(info.deps.len(), 3);

        let express = info.deps.iter().find(|d| d.name == "express").unwrap();
        assert!(!express.is_dev);

        let jest = info.deps.iter().find(|d| d.name == "jest").unwrap();
        assert!(jest.is_dev);
    }

    #[test]
    fn parse_requirements_txt_basic() {
        let content = r#"# Core deps
requests==2.28.0
flask>=2.0.0
grpcio~=1.50.0
redis
boto3[crt]>=1.26.0

# Comments
-r other-requirements.txt
"#;
        let info = parse_requirements_txt(content);
        assert!(info.name.is_empty());
        assert_eq!(info.deps.len(), 5);

        let requests = info.deps.iter().find(|d| d.name == "requests").unwrap();
        assert_eq!(requests.version, "2.28.0");

        let boto = info.deps.iter().find(|d| d.name == "boto3").unwrap();
        assert_eq!(boto.version, "1.26.0");
    }

    #[test]
    fn parse_pyproject_toml_basic() {
        let content = r#"[project]
name = "my-service"
version = "1.0.0"

dependencies = [
    "requests>=2.28.0",
    "grpcio~=1.50.0",
    "flask",
]

[tool.poetry.dev-dependencies]
pytest = "^7.0"
"#;
        let info = parse_pyproject_toml(content);
        assert_eq!(info.name, "my-service");
        assert!(info.deps.len() >= 3);

        let requests = info.deps.iter().find(|d| d.name == "requests").unwrap();
        assert!(!requests.is_dev);

        let pytest = info.deps.iter().find(|d| d.name == "pytest");
        assert!(pytest.is_some());
        assert!(pytest.unwrap().is_dev);
    }

    #[test]
    fn parse_package_json_invalid() {
        let info = parse_package_json("not json");
        assert!(info.name.is_empty());
        assert!(info.deps.is_empty());
    }

    #[test]
    fn parse_requirements_txt_empty() {
        let info = parse_requirements_txt("");
        assert!(info.deps.is_empty());
    }
}
