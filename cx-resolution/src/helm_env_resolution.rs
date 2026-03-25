use rustc_hash::FxHashMap;

/// An env var read extracted from source code.
#[derive(Debug, Clone)]
pub struct EnvVarRead {
    /// The env var name (e.g., "TTS_SERVICE_URL").
    pub var_name: String,
    /// File where the env var was read.
    pub file: String,
    /// Line number.
    pub line: u32,
}

/// An env var definition from a Helm values file.
#[derive(Debug, Clone)]
pub struct HelmEnvDef {
    /// The env var name (e.g., "TTS_SERVICE_URL").
    pub var_name: String,
    /// The value assigned (e.g., "http://tts-server-staging...").
    pub value: String,
    /// Helm values file where this was defined.
    pub file: String,
    /// Line number.
    pub line: u32,
}

/// A resolved k8s service reference parsed from a URL.
#[derive(Debug, Clone)]
pub struct K8sServiceRef {
    /// The service name (e.g., "tts-server").
    pub service_name: String,
    /// The namespace (e.g., "tts-server").
    pub namespace: String,
    /// The full hostname.
    pub hostname: String,
    /// Port if present.
    pub port: Option<u16>,
    /// Path portion of the URL (e.g., "/inference").
    pub path: String,
}

/// A match in the env var → Helm → k8s DNS chain.
#[derive(Debug, Clone)]
pub struct HelmEnvMatch {
    /// The env var name.
    pub var_name: String,
    /// Source code where the env var is read.
    pub reader_file: String,
    pub reader_line: u32,
    pub reader_repo: String,
    /// Helm values where the env var is defined.
    pub helm_file: String,
    pub helm_line: u32,
    pub helm_repo: String,
    /// The raw value from Helm.
    pub helm_value: String,
    /// Resolved k8s service, if the value contains a k8s DNS hostname.
    pub k8s_service: Option<K8sServiceRef>,
    /// Confidence score.
    pub confidence: f32,
}

/// Parse a k8s DNS hostname from a URL.
///
/// K8s DNS format: `<service>.<namespace>.svc.cluster.local`
/// or with env suffix: `<service>-<env>.<namespace>.svc.cluster.local`
pub fn parse_k8s_dns_from_url(url: &str) -> Option<K8sServiceRef> {
    // Extract hostname from URL
    let url_trimmed = url.trim();

    // Try to parse as a URL - handle with or without scheme
    let with_scheme = if url_trimmed.contains("://") {
        url_trimmed.to_string()
    } else {
        format!("http://{}", url_trimmed)
    };

    // Simple URL parsing: extract host, port, path
    let after_scheme = with_scheme
        .split("://")
        .nth(1)
        .unwrap_or(url_trimmed);

    let (host_port, path) = match after_scheme.find('/') {
        Some(i) => (&after_scheme[..i], &after_scheme[i..]),
        None => (after_scheme, "/"),
    };

    let (host, port) = match host_port.rfind(':') {
        Some(i) => {
            let port_str = &host_port[i + 1..];
            match port_str.parse::<u16>() {
                Ok(p) => (&host_port[..i], Some(p)),
                Err(_) => (host_port, None),
            }
        }
        None => (host_port, None),
    };

    // Check if it matches k8s DNS pattern: <name>.<namespace>.svc.cluster.local
    let parts: Vec<&str> = host.split('.').collect();

    // Look for "svc" in the parts to identify k8s DNS
    let svc_idx = parts.iter().position(|&p| p == "svc")?;

    if svc_idx < 2 {
        return None;
    }

    // Parts before "svc": [service_name_with_possible_env, namespace, ...]
    let raw_service = parts[0];
    let namespace = parts[1];

    // Strip environment suffix from service name (e.g., "tts-server-staging")
    // Common suffixes: -staging, -production, -prod, -dev, -{env}
    let service_name = strip_env_suffix(raw_service);

    Some(K8sServiceRef {
        service_name: service_name.to_string(),
        namespace: namespace.to_string(),
        hostname: host.to_string(),
        port,
        path: path.to_string(),
    })
}

/// Strip common environment suffixes from a k8s service name.
fn strip_env_suffix(name: &str) -> &str {
    let suffixes = [
        "-staging", "-production", "-prod", "-dev", "-test",
        "-canary", "-preview",
    ];
    for suffix in &suffixes {
        if let Some(stripped) = name.strip_suffix(suffix) {
            return stripped;
        }
    }

    // Handle templated suffixes like "svc-{env}" where {env} is a variable
    // These appear as literal strings in gotmpl: "svc-{{ .Values.env }}"
    // After template rendering they'd be "svc-staging" etc. — already handled above.

    name
}

/// Match env var reads to Helm env var definitions, resolving the k8s DNS chain.
pub fn match_helm_env(
    env_reads: &[(String, Vec<EnvVarRead>)],
    helm_defs: &[(String, Vec<HelmEnvDef>)],
) -> Vec<HelmEnvMatch> {
    let mut matches = Vec::new();

    // Build index: var_name → Vec<(repo, def)>
    let mut helm_index: FxHashMap<&str, Vec<(&str, &HelmEnvDef)>> = FxHashMap::default();
    for (repo, defs) in helm_defs {
        for def in defs {
            helm_index
                .entry(&def.var_name)
                .or_default()
                .push((repo, def));
        }
    }

    for (reader_repo, reads) in env_reads {
        for read in reads {
            if let Some(defs) = helm_index.get(read.var_name.as_str()) {
                for &(helm_repo, def) in defs {
                    let k8s_service = parse_k8s_dns_from_url(&def.value);

                    let confidence = if k8s_service.is_some() {
                        0.85 // env var + k8s DNS = high confidence
                    } else {
                        0.7 // env var match without k8s DNS resolution
                    };

                    matches.push(HelmEnvMatch {
                        var_name: read.var_name.clone(),
                        reader_file: read.file.clone(),
                        reader_line: read.line,
                        reader_repo: reader_repo.clone(),
                        helm_file: def.file.clone(),
                        helm_line: def.line,
                        helm_repo: helm_repo.to_string(),
                        helm_value: def.value.clone(),
                        k8s_service,
                        confidence,
                    });
                }
            }
        }
    }

    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_k8s_dns_full_url() {
        let url = "http://tts-server-staging.tts-server.svc.cluster.local:8000/inference";
        let result = parse_k8s_dns_from_url(url).expect("should parse k8s DNS");
        assert_eq!(result.service_name, "tts-server");
        assert_eq!(result.namespace, "tts-server");
        assert_eq!(result.port, Some(8000));
        assert_eq!(result.path, "/inference");
    }

    #[test]
    fn parse_k8s_dns_no_port() {
        let url = "http://my-service.my-ns.svc.cluster.local/api";
        let result = parse_k8s_dns_from_url(url).expect("should parse");
        assert_eq!(result.service_name, "my-service");
        assert_eq!(result.namespace, "my-ns");
        assert_eq!(result.port, None);
        assert_eq!(result.path, "/api");
    }

    #[test]
    fn parse_k8s_dns_strips_env_suffix() {
        let url = "http://svc-prod.ns.svc.cluster.local/";
        let result = parse_k8s_dns_from_url(url).expect("should parse");
        assert_eq!(result.service_name, "svc");
    }

    #[test]
    fn parse_non_k8s_url_returns_none() {
        let url = "http://api.example.com:8080/v1";
        assert!(parse_k8s_dns_from_url(url).is_none());
    }

    #[test]
    fn parse_empty_url_returns_none() {
        assert!(parse_k8s_dns_from_url("").is_none());
    }

    #[test]
    fn match_env_var_to_helm_with_k8s() {
        let reads = vec![(
            "api-gateway".into(),
            vec![EnvVarRead {
                var_name: "TTS_SERVICE_URL".into(),
                file: "config.go".into(),
                line: 10,
            }],
        )];
        let defs = vec![(
            "infra-k8s-config".into(),
            vec![HelmEnvDef {
                var_name: "TTS_SERVICE_URL".into(),
                value: "http://tts-server-staging.tts-server.svc.cluster.local:8000/inference".into(),
                file: "values.yaml.gotmpl".into(),
                line: 42,
            }],
        )];

        let result = match_helm_env(&reads, &defs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].var_name, "TTS_SERVICE_URL");
        assert!(result[0].confidence >= 0.85);
        assert!(result[0].k8s_service.is_some());

        let svc = result[0].k8s_service.as_ref().unwrap();
        assert_eq!(svc.service_name, "tts-server");
        assert_eq!(svc.path, "/inference");
    }

    #[test]
    fn match_env_var_non_k8s_url() {
        let reads = vec![(
            "repo-a".into(),
            vec![EnvVarRead {
                var_name: "API_URL".into(),
                file: "main.go".into(),
                line: 5,
            }],
        )];
        let defs = vec![(
            "infra-repo".into(),
            vec![HelmEnvDef {
                var_name: "API_URL".into(),
                value: "https://api.external.com/v1".into(),
                file: "values.yaml".into(),
                line: 10,
            }],
        )];

        let result = match_helm_env(&reads, &defs);
        assert_eq!(result.len(), 1);
        assert!(result[0].k8s_service.is_none());
        assert!(result[0].confidence < 0.85); // lower without k8s DNS
        assert!(result[0].confidence >= 0.7);
    }

    #[test]
    fn no_match_different_var_names() {
        let reads = vec![(
            "repo-a".into(),
            vec![EnvVarRead {
                var_name: "TTS_SERVICE_URL".into(),
                file: "main.go".into(),
                line: 5,
            }],
        )];
        let defs = vec![(
            "repo-b".into(),
            vec![HelmEnvDef {
                var_name: "OTHER_URL".into(),
                value: "http://foo.bar.svc.cluster.local/api".into(),
                file: "values.yaml".into(),
                line: 10,
            }],
        )];

        let result = match_helm_env(&reads, &defs);
        assert!(result.is_empty());
    }

    #[test]
    fn multiple_helm_defs_for_same_var() {
        let reads = vec![(
            "repo-a".into(),
            vec![EnvVarRead {
                var_name: "SVC_URL".into(),
                file: "main.go".into(),
                line: 5,
            }],
        )];
        let defs = vec![
            (
                "infra-staging".into(),
                vec![HelmEnvDef {
                    var_name: "SVC_URL".into(),
                    value: "http://svc-staging.ns.svc.cluster.local:8000/api".into(),
                    file: "staging/values.yaml".into(),
                    line: 10,
                }],
            ),
            (
                "infra-prod".into(),
                vec![HelmEnvDef {
                    var_name: "SVC_URL".into(),
                    value: "http://svc-prod.ns.svc.cluster.local:8000/api".into(),
                    file: "prod/values.yaml".into(),
                    line: 10,
                }],
            ),
        ];

        let result = match_helm_env(&reads, &defs);
        assert_eq!(result.len(), 2);
    }
}
