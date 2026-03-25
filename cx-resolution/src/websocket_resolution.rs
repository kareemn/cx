use crate::helm_env_resolution::parse_k8s_dns_from_url;
use rustc_hash::FxHashMap;

/// A WebSocket client connection extracted from source code.
#[derive(Debug, Clone)]
pub struct WsClientConnection {
    /// The URL or path pattern (e.g., "ws://host:port/ws/s2s" or "/ws/s2s").
    pub url_or_path: String,
    /// File where the connection was found.
    pub file: String,
    /// Line number.
    pub line: u32,
}

/// A WebSocket server endpoint.
#[derive(Debug, Clone)]
pub struct WsServerEndpoint {
    /// The path this endpoint serves (e.g., "/ws/s2s").
    pub path: String,
    /// File where the endpoint was defined.
    pub file: String,
    /// Line number.
    pub line: u32,
}

/// A resolved WebSocket match.
#[derive(Debug, Clone)]
pub struct WsMatch {
    /// The matched path.
    pub path: String,
    /// Client info.
    pub client_file: String,
    pub client_line: u32,
    pub client_repo: String,
    /// Server info.
    pub server_file: String,
    pub server_line: u32,
    pub server_repo: String,
    /// Resolved k8s service name, if the URL contained k8s DNS.
    pub k8s_service_name: Option<String>,
    /// Confidence score.
    pub confidence: f32,
}

/// Extract the path from a WebSocket URL.
/// "ws://host:port/ws/s2s" → "/ws/s2s"
/// "/ws/s2s" → "/ws/s2s"
fn extract_ws_path(url_or_path: &str) -> &str {
    let s = url_or_path.trim();
    if s.starts_with('/') {
        return s;
    }
    // Strip scheme
    let after_scheme = if let Some(i) = s.find("://") {
        &s[i + 3..]
    } else {
        s
    };
    // Find path start
    match after_scheme.find('/') {
        Some(i) => &after_scheme[i..],
        None => "/",
    }
}

/// Normalize a WS path for matching.
fn normalize_ws_path(path: &str) -> String {
    let p = path.trim().to_lowercase();
    let p = p.trim_end_matches('/');
    if p.is_empty() { "/".to_string() } else { p.to_string() }
}

/// Match WebSocket client connections to server endpoints.
pub fn match_websockets(
    ws_clients: &[(String, Vec<WsClientConnection>)],
    ws_servers: &[(String, Vec<WsServerEndpoint>)],
) -> Vec<WsMatch> {
    let mut matches = Vec::new();

    // Build server index: normalized path → Vec<(repo, endpoint)>
    let mut server_index: FxHashMap<String, Vec<(&str, &WsServerEndpoint)>> =
        FxHashMap::default();
    for (repo, endpoints) in ws_servers {
        for ep in endpoints {
            let key = normalize_ws_path(&ep.path);
            server_index.entry(key).or_default().push((repo, ep));
        }
    }

    for (client_repo, connections) in ws_clients {
        for conn in connections {
            let client_path = extract_ws_path(&conn.url_or_path);
            let client_norm = normalize_ws_path(client_path);

            // Try to extract k8s service from the URL
            let k8s_svc = parse_k8s_dns_from_url(&conn.url_or_path);

            // Exact path match
            if let Some(servers) = server_index.get(&client_norm) {
                for &(server_repo, ep) in servers {
                    let confidence = if client_repo == server_repo {
                        0.4
                    } else if k8s_svc.is_some() {
                        0.85 // path match + k8s DNS
                    } else {
                        0.75 // path match only
                    };

                    matches.push(WsMatch {
                        path: ep.path.clone(),
                        client_file: conn.file.clone(),
                        client_line: conn.line,
                        client_repo: client_repo.clone(),
                        server_file: ep.file.clone(),
                        server_line: ep.line,
                        server_repo: server_repo.to_string(),
                        k8s_service_name: k8s_svc
                            .as_ref()
                            .map(|s| s.service_name.clone()),
                        confidence,
                    });
                }
                continue;
            }

            // Prefix match
            for (norm_path, servers) in &server_index {
                if client_norm.starts_with(norm_path.as_str())
                    || norm_path.starts_with(client_norm.as_str())
                {
                    for &(server_repo, ep) in servers {
                        let confidence = if client_repo == server_repo {
                            0.3
                        } else {
                            0.5
                        };

                        matches.push(WsMatch {
                            path: ep.path.clone(),
                            client_file: conn.file.clone(),
                            client_line: conn.line,
                            client_repo: client_repo.clone(),
                            server_file: ep.file.clone(),
                            server_line: ep.line,
                            server_repo: server_repo.to_string(),
                            k8s_service_name: k8s_svc
                                .as_ref()
                                .map(|s| s.service_name.clone()),
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
    fn extract_path_from_ws_url() {
        assert_eq!(extract_ws_path("ws://host:8080/ws/s2s"), "/ws/s2s");
        assert_eq!(extract_ws_path("wss://host/ws/s2s"), "/ws/s2s");
        assert_eq!(extract_ws_path("/ws/s2s"), "/ws/s2s");
        assert_eq!(extract_ws_path("ws://host"), "/");
    }

    #[test]
    fn exact_ws_path_match() {
        let clients = vec![(
            "native-client".into(),
            vec![WsClientConnection {
                url_or_path: "ws://10.0.0.1:8080/ws/s2s".into(),
                file: "client.cpp".into(),
                line: 42,
            }],
        )];
        let servers = vec![(
            "api-gateway".into(),
            vec![WsServerEndpoint {
                path: "/ws/s2s".into(),
                file: "handler.go".into(),
                line: 15,
            }],
        )];

        let result = match_websockets(&clients, &servers);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "/ws/s2s");
        assert!(result[0].confidence >= 0.7);
    }

    #[test]
    fn ws_with_k8s_dns_higher_confidence() {
        let clients = vec![(
            "browser-ext".into(),
            vec![WsClientConnection {
                url_or_path: "wss://ws-svc.ws-ns.svc.cluster.local/ws/s2s".into(),
                file: "ws.ts".into(),
                line: 10,
            }],
        )];
        let servers = vec![(
            "ws-server".into(),
            vec![WsServerEndpoint {
                path: "/ws/s2s".into(),
                file: "main.go".into(),
                line: 20,
            }],
        )];

        let result = match_websockets(&clients, &servers);
        assert_eq!(result.len(), 1);
        assert!(result[0].confidence >= 0.85);
        assert!(result[0].k8s_service_name.is_some());
    }

    #[test]
    fn no_match_different_paths() {
        let clients = vec![(
            "repo-a".into(),
            vec![WsClientConnection {
                url_or_path: "ws://host/ws/chat".into(),
                file: "client.ts".into(),
                line: 5,
            }],
        )];
        let servers = vec![(
            "repo-b".into(),
            vec![WsServerEndpoint {
                path: "/ws/s2s".into(),
                file: "server.go".into(),
                line: 10,
            }],
        )];

        let result = match_websockets(&clients, &servers);
        assert!(result.is_empty());
    }
}
