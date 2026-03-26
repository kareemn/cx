use rustc_hash::FxHashMap;

/// An HTTP client call extracted from source code.
#[derive(Debug, Clone)]
pub struct HttpClientCall {
    /// The URL path pattern (e.g., "/inference", "/v1/chat/completions").
    pub path: String,
    /// HTTP method if known (GET, POST, etc.), empty if unknown.
    pub method: String,
    /// Env var that holds the base URL, if any (e.g., "TTS_SERVICE_URL").
    pub base_url_env_var: Option<String>,
    /// File where the client call was found.
    pub file: String,
    /// Line number.
    pub line: u32,
}

/// An HTTP server route registration extracted from source code.
#[derive(Debug, Clone)]
pub struct HttpServerRoute {
    /// The route path (e.g., "/inference", "/embed").
    pub path: String,
    /// HTTP method (GET, POST, etc.).
    pub method: String,
    /// The framework (e.g., "fastapi", "express", "gin", "nextjs").
    pub framework: String,
    /// File where the route was defined.
    pub file: String,
    /// Line number.
    pub line: u32,
}

/// A resolved REST match: a client call matched to a server route.
#[derive(Debug, Clone)]
pub struct RestMatch {
    /// The matched URL path.
    pub path: String,
    /// HTTP method (empty if method-agnostic match).
    pub method: String,
    /// Client info.
    pub client_file: String,
    pub client_line: u32,
    pub client_repo: String,
    /// Server info.
    pub server_file: String,
    pub server_line: u32,
    pub server_repo: String,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f32,
}

/// Paths that are too generic to match meaningfully across repos.
const TRIVIAL_PATHS: &[&str] = &[
    "/", "/health", "/healthz", "/ready", "/readyz", "/livez",
    "/metrics", "/status", "/ping", "/version", "/favicon.ico",
];

/// Minimum path segment count for prefix matching (avoid "/" matching everything).
const MIN_PREFIX_SEGMENTS: usize = 2;

/// Normalize a URL path for matching: lowercase, strip trailing slash, collapse double slashes.
fn normalize_path(path: &str) -> String {
    let p = path.trim().to_lowercase();
    let p = p.trim_end_matches('/');
    if p.is_empty() {
        "/".to_string()
    } else {
        p.replace("//", "/")
    }
}

/// Count meaningful path segments (split by '/', skip empty).
fn path_segment_count(path: &str) -> usize {
    path.split('/').filter(|s| !s.is_empty()).count()
}

/// Check if a client path matches a server route path.
/// Returns (matches, is_exact).
fn paths_match(client_path: &str, server_path: &str) -> (bool, bool) {
    let c = normalize_path(client_path);
    let s = normalize_path(server_path);

    if c == s {
        return (true, true);
    }

    // Prefix match requires both paths to have meaningful depth
    // to avoid "/" or "/api" matching everything
    let c_segs = path_segment_count(&c);
    let s_segs = path_segment_count(&s);
    let shorter = c_segs.min(s_segs);

    if shorter >= MIN_PREFIX_SEGMENTS && (c.starts_with(&s) || s.starts_with(&c)) {
        return (true, false);
    }

    (false, false)
}

/// Match HTTP client calls to server routes across repos.
pub fn match_rest(
    client_calls: &[(String, Vec<HttpClientCall>)],
    server_routes: &[(String, Vec<HttpServerRoute>)],
) -> Vec<RestMatch> {
    let mut matches = Vec::new();

    // Build server index: normalized_path → Vec<(repo, route)>
    // Skip trivial paths that would create false matches
    let mut server_index: FxHashMap<String, Vec<(&str, &HttpServerRoute)>> =
        FxHashMap::default();
    for (repo, routes) in server_routes {
        for route in routes {
            let key = normalize_path(&route.path);
            if TRIVIAL_PATHS.contains(&key.as_str()) {
                continue;
            }
            server_index
                .entry(key)
                .or_default()
                .push((repo, route));
        }
    }

    for (client_repo, calls) in client_calls {
        for call in calls {
            let client_norm = normalize_path(&call.path);
            // Skip trivial client paths
            if TRIVIAL_PATHS.contains(&client_norm.as_str()) {
                continue;
            }

            // Try exact path match first
            if let Some(servers) = server_index.get(&client_norm) {
                for &(server_repo, route) in servers {
                    let method_matches = call.method.is_empty()
                        || route.method.is_empty()
                        || call.method.eq_ignore_ascii_case(&route.method);

                    let confidence = if client_repo == server_repo {
                        0.4 // same-repo REST is less interesting
                    } else if method_matches {
                        0.85 // cross-repo exact path + method match
                    } else {
                        0.6 // cross-repo exact path, method mismatch
                    };

                    matches.push(RestMatch {
                        path: route.path.clone(),
                        method: if method_matches {
                            route.method.clone()
                        } else {
                            String::new()
                        },
                        client_file: call.file.clone(),
                        client_line: call.line,
                        client_repo: client_repo.clone(),
                        server_file: route.file.clone(),
                        server_line: route.line,
                        server_repo: server_repo.to_string(),
                        confidence,
                    });
                }
                continue;
            }

            // Try prefix match against all server routes
            for (norm_path, servers) in &server_index {
                let (matched, _exact) = paths_match(&client_norm, norm_path);
                if matched {
                    for &(server_repo, route) in servers {
                        let confidence = if client_repo == server_repo {
                            0.3
                        } else {
                            0.6 // partial match
                        };

                        matches.push(RestMatch {
                            path: route.path.clone(),
                            method: String::new(),
                            client_file: call.file.clone(),
                            client_line: call.line,
                            client_repo: client_repo.clone(),
                            server_file: route.file.clone(),
                            server_line: route.line,
                            server_repo: server_repo.to_string(),
                            confidence,
                        });
                    }
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
    fn exact_path_match_cross_repo() {
        let clients = vec![(
            "api-gateway".into(),
            vec![HttpClientCall {
                path: "/inference".into(),
                method: "POST".into(),
                base_url_env_var: Some("TTS_SERVICE_URL".into()),
                file: "translator.go".into(),
                line: 42,
            }],
        )];
        let servers = vec![(
            "acme-tts".into(),
            vec![HttpServerRoute {
                path: "/inference".into(),
                method: "POST".into(),
                framework: "fastapi".into(),
                file: "app.py".into(),
                line: 15,
            }],
        )];

        let result = match_rest(&clients, &servers);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "/inference");
        assert!(result[0].confidence >= 0.85);
        assert_eq!(result[0].client_repo, "api-gateway");
        assert_eq!(result[0].server_repo, "acme-tts");
    }

    #[test]
    fn method_mismatch_lowers_confidence() {
        let clients = vec![(
            "repo-a".into(),
            vec![HttpClientCall {
                path: "/api/users".into(),
                method: "GET".into(),
                base_url_env_var: None,
                file: "client.go".into(),
                line: 10,
            }],
        )];
        let servers = vec![(
            "repo-b".into(),
            vec![HttpServerRoute {
                path: "/api/users".into(),
                method: "POST".into(),
                framework: "express".into(),
                file: "server.ts".into(),
                line: 5,
            }],
        )];

        let result = match_rest(&clients, &servers);
        assert_eq!(result.len(), 1);
        assert!(result[0].confidence < 0.85);
        assert!(result[0].confidence >= 0.5);
    }

    #[test]
    fn prefix_path_match() {
        let clients = vec![(
            "repo-a".into(),
            vec![HttpClientCall {
                path: "/v1/chat/completions".into(),
                method: "POST".into(),
                base_url_env_var: Some("LLM_BASE_URL".into()),
                file: "llm.go".into(),
                line: 20,
            }],
        )];
        let servers = vec![(
            "repo-b".into(),
            vec![HttpServerRoute {
                path: "/v1/chat".into(),
                method: "POST".into(),
                framework: "fastapi".into(),
                file: "api.py".into(),
                line: 8,
            }],
        )];

        let result = match_rest(&clients, &servers);
        assert_eq!(result.len(), 1);
        assert!(result[0].confidence >= 0.5);
        assert!(result[0].confidence < 0.85);
    }

    #[test]
    fn trivial_paths_not_matched() {
        let clients = vec![(
            "repo-a".into(),
            vec![HttpClientCall {
                path: "/".into(),
                method: "GET".into(),
                base_url_env_var: None,
                file: "client.go".into(),
                line: 10,
            }],
        )];
        let servers = vec![(
            "repo-b".into(),
            vec![HttpServerRoute {
                path: "/".into(),
                method: "GET".into(),
                framework: "express".into(),
                file: "server.ts".into(),
                line: 5,
            }],
        )];
        let result = match_rest(&clients, &servers);
        assert!(result.is_empty(), "trivial path '/' should not match");
    }

    #[test]
    fn short_prefix_not_matched() {
        // "/v1" (1 segment) should not prefix-match "/v1/chat/completions"
        let clients = vec![(
            "repo-a".into(),
            vec![HttpClientCall {
                path: "/v1/chat/completions".into(),
                method: "POST".into(),
                base_url_env_var: None,
                file: "llm.go".into(),
                line: 20,
            }],
        )];
        let servers = vec![(
            "repo-b".into(),
            vec![HttpServerRoute {
                path: "/v1".into(),
                method: "POST".into(),
                framework: "fastapi".into(),
                file: "api.py".into(),
                line: 8,
            }],
        )];
        let result = match_rest(&clients, &servers);
        // Single segment "/v1" should not prefix-match
        assert!(result.is_empty());
    }

    #[test]
    fn no_match_different_paths() {
        let clients = vec![(
            "repo-a".into(),
            vec![HttpClientCall {
                path: "/inference".into(),
                method: "POST".into(),
                base_url_env_var: None,
                file: "client.go".into(),
                line: 10,
            }],
        )];
        let servers = vec![(
            "repo-b".into(),
            vec![HttpServerRoute {
                path: "/health".into(),
                method: "GET".into(),
                framework: "fastapi".into(),
                file: "server.py".into(),
                line: 5,
            }],
        )];

        let result = match_rest(&clients, &servers);
        assert!(result.is_empty());
    }

    #[test]
    fn same_repo_lower_confidence() {
        let clients = vec![(
            "same-repo".into(),
            vec![HttpClientCall {
                path: "/api".into(),
                method: "GET".into(),
                base_url_env_var: None,
                file: "client.go".into(),
                line: 1,
            }],
        )];
        let servers = vec![(
            "same-repo".into(),
            vec![HttpServerRoute {
                path: "/api".into(),
                method: "GET".into(),
                framework: "gin".into(),
                file: "server.go".into(),
                line: 5,
            }],
        )];

        let result = match_rest(&clients, &servers);
        assert_eq!(result.len(), 1);
        assert!(result[0].confidence < 0.5);
    }

    #[test]
    fn normalize_trailing_slash() {
        assert_eq!(normalize_path("/inference/"), "/inference");
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path(""), "/");
    }

    #[test]
    fn multiple_server_matches() {
        let clients = vec![(
            "repo-a".into(),
            vec![HttpClientCall {
                path: "/embed".into(),
                method: "POST".into(),
                base_url_env_var: Some("SENTENCE_EMBEDDER_BASE_URL".into()),
                file: "embed.go".into(),
                line: 10,
            }],
        )];
        let servers = vec![
            (
                "embedder-svc".into(),
                vec![HttpServerRoute {
                    path: "/embed".into(),
                    method: "POST".into(),
                    framework: "fastapi".into(),
                    file: "embed_api.py".into(),
                    line: 20,
                }],
            ),
            (
                "other-svc".into(),
                vec![HttpServerRoute {
                    path: "/embed".into(),
                    method: "POST".into(),
                    framework: "express".into(),
                    file: "routes.ts".into(),
                    line: 30,
                }],
            ),
        ];

        let result = match_rest(&clients, &servers);
        assert_eq!(result.len(), 2);
    }
}
