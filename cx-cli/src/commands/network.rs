use anyhow::Result;
use cx_core::graph::csr::CsrGraph;
use cx_core::graph::edges::EdgeKind;
use cx_core::graph::kind_index::KindIndex;
use cx_core::graph::nodes::NodeKind;
use cx_extractors::taint::ResolvedNetworkCall;
use std::path::Path;

/// Run `cx network` — list all detected network calls and exposed APIs with provenance.
pub fn run(root: &Path, json: bool, kind: Option<&str>, direction: Option<&str>, service: Option<&str>, local_only: bool, include_all: bool) -> Result<()> {
    let graph = crate::indexing::load_graph(root)?;

    // Load local taint analysis results
    let mut taint_calls = load_network_json(root);

    // Load remote network data unless --local-only
    // Filter: only keep remote calls whose env var names match local env var reads
    if !local_only {
        let local_env_vars: std::collections::HashSet<String> = taint_calls
            .iter()
            .filter_map(|c| extract_env_var_name(&c.address_source))
            .collect();

        let remote_calls = load_remote_network_json(root);
        let filtered: Vec<_> = remote_calls
            .into_iter()
            .filter(|c| {
                // Keep if this remote call's env var matches a local read
                if let Some(var) = extract_env_var_name(&c.address_source) {
                    local_env_vars.contains(&var)
                } else if include_all {
                    true // --include-all shows everything
                } else {
                    false
                }
            })
            .collect();
        taint_calls.extend(filtered);
    }

    // Noise filtering: exclude test/archive/example/vendor paths by default
    if !include_all {
        taint_calls.retain(|c| !is_noise_path(&c.file));
    }

    // Deduplicate by (file, line) — keep highest confidence entry
    dedup_by_location(&mut taint_calls);

    let mut result = build_network_report(&graph, &taint_calls, kind, direction, service);

    // Find cross-repo matches from local report data + remote data + global index
    if !local_only {
        let matches = find_cross_repo_matches(&result, root);
        if !matches.is_empty() {
            result["cross_repo_connections"] = serde_json::Value::Array(matches);
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print_summary_header(&result);
        print_human_readable(&result);
    }

    Ok(())
}

/// Load ResolvedNetworkCall data from .cx/graph/network.json.
/// Returns empty vec if file doesn't exist or can't be parsed.
pub fn load_network_json(root: &Path) -> Vec<ResolvedNetworkCall> {
    let path = root.join(".cx").join("graph").join("network.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Extract the env var name from an AddressSource, if it mentions one.
fn extract_env_var_name(source: &cx_extractors::taint::AddressSource) -> Option<String> {
    use cx_extractors::taint::AddressSource;
    match source {
        AddressSource::EnvVar { var_name, .. } => Some(var_name.clone()),
        AddressSource::Parameter { caller_sources, .. } => {
            caller_sources.iter().find_map(extract_env_var_name)
        }
        AddressSource::FieldAccess { assignment_sources, .. } => {
            assignment_sources.iter().find_map(extract_env_var_name)
        }
        AddressSource::Concat { parts } => parts.iter().find_map(extract_env_var_name),
        _ => None,
    }
}

/// Check whether a file path looks like test, archive, example, or vendor noise.
fn is_noise_path(path: &str) -> bool {
    let p = path.to_lowercase();
    // Test directories and files
    p.contains("/test/") || p.contains("/tests/") || p.contains("e2e_test/")
        || p.contains("/testutil/") || p.contains("/testdata/")
        || p.ends_with("_test.go") || p.contains("test_")
        // Non-production directories
        || p.contains("/archive/") || p.contains("/examples/") || p.contains("/example/")
        || p.contains("/vendor/") || p.contains("/third_party/")
        // Build artifacts and bundled code
        || p.contains("/dist/") || p.contains("/build/") || p.contains("/node_modules/")
        || p.ends_with(".min.js")
        // WASM build output and demo files
        || p.contains("wasm/build/") || p.contains("/wasm/") || p.contains("demo/")
        // Paths starting with archive/ or examples/ (no leading /)
        || p.starts_with("archive/") || p.starts_with("examples/") || p.starts_with("demo/")
        || p.starts_with("e2e_test/") || p.starts_with("test/")
}

/// Load network.json files from all pulled remotes in .cx/remotes/.
/// Prefixes file paths with [remote_name] for disambiguation.
/// Deduplicates entries from remotes pointing to the same repo.
fn load_remote_network_json(root: &Path) -> Vec<ResolvedNetworkCall> {
    let remotes_dir = root.join(".cx").join("remotes");
    let mut all_calls = Vec::new();
    let entries = match std::fs::read_dir(&remotes_dir) {
        Ok(e) => e,
        Err(_) => return all_calls,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.ends_with(".network.json") { continue; }
        let remote_name = name.trim_end_matches(".network.json");
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c, Err(_) => continue,
        };
        let mut calls: Vec<ResolvedNetworkCall> = match serde_json::from_str(&content) {
            Ok(c) => c, Err(_) => continue,
        };
        for call in &mut calls {
            call.file = format!("[{}] {}", remote_name, call.file);
        }
        all_calls.extend(calls);
    }
    // Dedup across remotes pointing to same repo
    let mut seen: std::collections::HashSet<(String, u32)> = std::collections::HashSet::new();
    all_calls.retain(|call| {
        let bare = call.file.split("] ").last().unwrap_or(&call.file);
        seen.insert((bare.to_string(), call.line))
    });
    all_calls
}

/// Deduplicate network calls by (file, line), keeping highest confidence.
fn dedup_by_location(calls: &mut Vec<ResolvedNetworkCall>) {
    use std::collections::HashMap;
    let mut best: HashMap<(String, u32), usize> = HashMap::new();
    fn confidence_rank(c: cx_extractors::taint::Confidence) -> u8 {
        match c {
            cx_extractors::taint::Confidence::TypeConfirmed => 4,
            cx_extractors::taint::Confidence::LLMClassified => 3,
            cx_extractors::taint::Confidence::ImportResolved => 2,
            cx_extractors::taint::Confidence::Heuristic => 1,
        }
    }
    let mut keep = vec![false; calls.len()];
    for (i, call) in calls.iter().enumerate() {
        let key = (call.file.clone(), call.line);
        if let Some(&prev_idx) = best.get(&key) {
            if confidence_rank(call.confidence) > confidence_rank(calls[prev_idx].confidence) {
                keep[prev_idx] = false;
                keep[i] = true;
                best.insert(key, i);
            }
        } else {
            keep[i] = true;
            best.insert(key, i);
        }
    }
    let mut i = 0;
    calls.retain(|_| { let k = keep[i]; i += 1; k });
}

/// Build the full network report from the graph and taint analysis results.
pub fn build_network_report(
    graph: &CsrGraph,
    taint_calls: &[ResolvedNetworkCall],
    kind_filter: Option<&str>,
    direction_filter: Option<&str>,
    service_filter: Option<&str>,
) -> serde_json::Value {
    let kind_idx = KindIndex::build(graph);

    // Build taint lookup: (file, line) → &ResolvedNetworkCall
    let taint_index: rustc_hash::FxHashMap<(&str, u32), &ResolvedNetworkCall> = taint_calls
        .iter()
        .map(|c| ((c.file.as_str(), c.line), c))
        .collect();

    let mut network_calls = Vec::new();
    let mut exposed_apis = Vec::new();

    // Collect outbound network calls from Connects edges (Symbol → Resource),
    // enriching with taint provenance where available
    collect_connects_edges(graph, &kind_idx, &taint_index, &mut network_calls, kind_filter, service_filter);

    // Collect inbound exposed APIs from Exposes edges (Deployable/Module → Endpoint)
    collect_exposes_edges(graph, &kind_idx, &mut exposed_apis, kind_filter, service_filter);

    // Collect Publishes/Subscribes edges as network calls
    collect_pubsub_edges(graph, &kind_idx, &mut network_calls, kind_filter, service_filter);

    // Merge in taint-only calls not already covered by graph edges
    let mut seen_locations: rustc_hash::FxHashSet<(String, u32)> = rustc_hash::FxHashSet::default();
    for call in &network_calls {
        let file = call["file"].as_str().unwrap_or("").to_string();
        let line = call["line"].as_u64().unwrap_or(0) as u32;
        seen_locations.insert((file, line));
    }

    for tc in taint_calls {
        if seen_locations.contains(&(tc.file.clone(), tc.line)) {
            continue;
        }

        let kind_str = tc.net_kind.as_str();
        if let Some(kf) = kind_filter {
            if !kind_matches(kind_str, kf) {
                continue;
            }
        }

        let entry = taint_call_to_json(tc);
        network_calls.push(entry);
    }

    // Apply direction filter to network_calls entries
    if let Some(dir) = direction_filter {
        network_calls.retain(|call| {
            call["direction"].as_str().unwrap_or("outbound") == dir
        });
    }

    let show_outbound = direction_filter.is_none()
        || direction_filter == Some("outbound");
    let show_inbound = direction_filter.is_none()
        || direction_filter == Some("inbound");

    let mut result = serde_json::Map::new();

    if show_outbound || direction_filter.is_none() {
        result.insert("network_calls".to_string(), serde_json::Value::Array(network_calls));
    }
    if show_inbound || direction_filter.is_none() {
        result.insert("exposed_apis".to_string(), serde_json::Value::Array(exposed_apis));
    }

    serde_json::Value::Object(result)
}

/// A collected endpoint (inbound exposed API) for matching.
#[allow(dead_code)]
struct CollectedEndpoint {
    file: String,
    line: u64,
    kind: String,
    path: String,
    method: Option<String>,
    service: String,
    repo_name: Option<String>,
}

/// A collected outbound call for matching.
struct CollectedOutbound {
    file: String,
    line: u64,
    kind: String,
    callee: String,
    address_hint: String,
    repo_name: Option<String>,
}

/// A single cross-repo connection match.
struct CrossRepoMatch {
    client_file: String,
    client_line: u64,
    client_callee: String,
    client_kind: String,
    client_repo: Option<String>,
    server_file: String,
    server_line: u64,
    server_path: String,
    server_kind: String,
    server_repo: Option<String>,
    match_type: String,
}

/// Find cross-repo connections by matching outbound calls to inbound endpoints.
///
/// This function:
/// 1. Collects all exposed APIs (inbound) and outbound calls from the report
/// 2. Loads remote network data from .cx/remotes/*.network.json
/// 3. Loads the GlobalIndex from .cx/graph/index.json for additional matches
/// 4. Matches outbound calls to exposed APIs by path, gRPC service name, or URL
fn find_cross_repo_matches(report: &serde_json::Value, root: &Path) -> Vec<serde_json::Value> {
    let mut endpoints = collect_endpoints_from_report(report);
    let mut outbounds = collect_outbounds_from_report(report);

    // Load remote network data and add to our collections
    load_remote_network_data(root, &mut endpoints, &mut outbounds);

    // Load global index for additional endpoint/target data
    load_global_index_data(root, &mut endpoints, &mut outbounds);

    // Filter noise from both sides before matching
    endpoints.retain(|e| !is_noise_path(&e.file));
    outbounds.retain(|o| !is_noise_path(&o.file));

    // Perform matching
    let matches = match_outbounds_to_endpoints(&outbounds, &endpoints);

    // Convert matches to JSON
    matches.iter().map(|m| {
        serde_json::json!({
            "client_file": m.client_file,
            "client_line": m.client_line,
            "client_callee": m.client_callee,
            "client_kind": m.client_kind,
            "client_repo": m.client_repo,
            "server_file": m.server_file,
            "server_line": m.server_line,
            "server_path": m.server_path,
            "server_kind": m.server_kind,
            "server_repo": m.server_repo,
            "match_type": m.match_type,
        })
    }).collect()
}

/// Collect exposed API endpoints from the report JSON.
fn collect_endpoints_from_report(report: &serde_json::Value) -> Vec<CollectedEndpoint> {
    let mut endpoints = Vec::new();

    if let Some(apis) = report.get("exposed_apis").and_then(|v| v.as_array()) {
        for api in apis {
            let file = api["file"].as_str().unwrap_or("").to_string();
            let line = api["line"].as_u64().unwrap_or(0);
            let kind = api["kind"].as_str().unwrap_or("").to_string();
            let path = api["path"].as_str().unwrap_or("").to_string();
            let method = api["method"].as_str().map(String::from);
            let service = api["service"].as_str().unwrap_or("").to_string();

            endpoints.push(CollectedEndpoint {
                file, line, kind, path, method, service, repo_name: None,
            });
        }
    }

    endpoints
}

/// Collect outbound network calls from the report JSON.
fn collect_outbounds_from_report(report: &serde_json::Value) -> Vec<CollectedOutbound> {
    let mut outbounds = Vec::new();

    if let Some(calls) = report.get("network_calls").and_then(|v| v.as_array()) {
        for call in calls {
            let direction = call["direction"].as_str().unwrap_or("");
            if direction != "outbound" {
                continue;
            }

            let file = call["file"].as_str().unwrap_or("").to_string();
            let line = call["line"].as_u64().unwrap_or(0);
            let kind = call["kind"].as_str().unwrap_or("").to_string();
            let callee = call.get("callee").and_then(|v| v.as_str()).unwrap_or("").to_string();

            // Extract address hint from provenance_chain, address_source, or target
            let address_hint = extract_address_hint(call);

            outbounds.push(CollectedOutbound {
                file, line, kind, callee, address_hint, repo_name: None,
            });
        }
    }

    outbounds
}

/// Extract a usable address hint from an outbound call's data.
///
/// Looks at provenance_chain for dynamic hints containing paths (e.g. "dynamic(unresolved var: /ws/s2s)"),
/// address_source for literal values, and target name as fallback.
fn extract_address_hint(call: &serde_json::Value) -> String {
    // First check provenance_chain for dynamic hints containing paths
    if let Some(chain) = call.get("provenance_chain").and_then(|v| v.as_str()) {
        if let Some(extracted) = extract_path_from_hint(chain) {
            return extracted;
        }
        return chain.to_string();
    }

    // Check address_source object (from taint analysis)
    if let Some(source) = call.get("address_source") {
        if let Some(hint) = extract_from_address_source(source) {
            return hint;
        }
    }

    // Fall back to target name from graph edges
    if let Some(target) = call.get("target") {
        if let Some(name) = target["name"].as_str() {
            return name.to_string();
        }
    }

    String::new()
}

/// Extract a path from a hint string like "dynamic(unresolved var: /ws/s2s)".
fn extract_path_from_hint(hint: &str) -> Option<String> {
    // Look for paths starting with / inside the hint
    for part in hint.split_whitespace() {
        let cleaned = part.trim_end_matches(')');
        if cleaned.starts_with('/') && cleaned.len() > 1 {
            return Some(cleaned.to_string());
        }
    }
    // Also try to find path after "var: " or similar patterns
    if let Some(idx) = hint.find("var: ") {
        let rest = &hint[idx + 5..];
        let path = rest.trim_end_matches(')').trim();
        if path.starts_with('/') {
            return Some(path.to_string());
        }
    }
    None
}

/// Extract address info from a serialized AddressSource JSON value.
fn extract_from_address_source(source: &serde_json::Value) -> Option<String> {
    // Literal value
    if let Some(val) = source.get("Literal").and_then(|v| v.get("value")).and_then(|v| v.as_str()) {
        return Some(val.to_string());
    }
    // Dynamic hint
    if let Some(hint) = source.get("Dynamic").and_then(|v| v.get("hint")).and_then(|v| v.as_str()) {
        if let Some(path) = extract_path_from_hint(hint) {
            return Some(path);
        }
        return Some(hint.to_string());
    }
    // EnvVar with k8s resolved value
    if let Some(env) = source.get("EnvVar") {
        if let Some(k8s) = env.get("k8s_value").and_then(|v| v.as_str()) {
            return Some(k8s.to_string());
        }
        if let Some(name) = env.get("var_name").and_then(|v| v.as_str()) {
            return Some(format!("env:{}", name));
        }
    }
    None
}

/// Load remote network.json files and add their entries to the endpoint/outbound collections.
fn load_remote_network_data(
    root: &Path,
    endpoints: &mut Vec<CollectedEndpoint>,
    outbounds: &mut Vec<CollectedOutbound>,
) {
    let remotes_dir = root.join(".cx").join("remotes");
    let Ok(entries) = std::fs::read_dir(&remotes_dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !filename.ends_with(".network.json") {
            continue;
        }
        let remote_name = filename.trim_end_matches(".network.json").to_string();

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(calls): Result<Vec<ResolvedNetworkCall>, _> = serde_json::from_str(&content) else {
            continue;
        };

        for call in &calls {
            let direction = taint_direction(call.net_kind);
            let kind = call.net_kind.as_str().to_string();
            let file = call.file.clone();
            let line = call.line as u64;

            if direction == "inbound" {
                // Remote inbound = exposed API from the remote repo
                let path_str = extract_path_from_callee(&call.callee_fqn, &call.address_source);
                endpoints.push(CollectedEndpoint {
                    file, line, kind,
                    path: path_str,
                    method: None,
                    service: call.callee_fqn.clone(),
                    repo_name: Some(remote_name.clone()),
                });
            } else {
                // Remote outbound = network call from the remote repo
                let address_hint = format_address_source(&call.address_source);
                let mut hint = address_hint.clone();
                if let Some(path) = extract_path_from_hint(&hint) {
                    hint = path;
                }
                outbounds.push(CollectedOutbound {
                    file, line, kind,
                    callee: call.callee_fqn.clone(),
                    address_hint: hint,
                    repo_name: Some(remote_name.clone()),
                });
            }
        }
    }
}

/// Extract a path string from a callee name or address source.
fn extract_path_from_callee(callee: &str, source: &cx_extractors::taint::AddressSource) -> String {
    use cx_extractors::taint::AddressSource;

    // Check address source first
    match source {
        AddressSource::Literal { value } => {
            // If literal contains a path, extract it
            if let Some(path) = extract_url_path(value) {
                return path;
            }
            return value.clone();
        }
        AddressSource::Dynamic { hint } => {
            if let Some(path) = extract_path_from_hint(hint) {
                return path;
            }
        }
        _ => {}
    }

    // Try extracting from callee name
    callee.to_string()
}

/// Extract the path component from a URL string.
fn extract_url_path(url: &str) -> Option<String> {
    // Handle full URLs like "http://host:port/path" or "ws://host/path"
    if let Some(idx) = url.find("://") {
        let after_scheme = &url[idx + 3..];
        if let Some(slash_idx) = after_scheme.find('/') {
            let path = &after_scheme[slash_idx..];
            if !path.is_empty() && path != "/" {
                return Some(path.to_string());
            }
        }
    }
    // Handle bare paths
    if url.starts_with('/') && url.len() > 1 {
        return Some(url.to_string());
    }
    None
}

/// Load GlobalIndex data and add entries to endpoint/outbound collections.
fn load_global_index_data(
    root: &Path,
    endpoints: &mut Vec<CollectedEndpoint>,
    outbounds: &mut Vec<CollectedOutbound>,
) {
    let Ok(index) = crate::graph_index::GlobalIndex::load(root) else {
        return;
    };

    // Add exposed_apis from global index
    for (endpoint_key, entries) in &index.exposed_apis {
        for entry in entries {
            // Skip entries that might already be in our local data (repo_id < 1000 = local)
            if entry.repo_id < 1000 {
                continue;
            }
            let (method, path) = parse_endpoint_name(endpoint_key);
            endpoints.push(CollectedEndpoint {
                file: entry.file.clone(),
                line: entry.line as u64,
                kind: infer_kind_from_endpoint(endpoint_key).to_string(),
                path: path.to_string(),
                method: method.map(String::from),
                service: entry.symbol.clone(),
                repo_name: Some(entry.repo_name.clone()),
            });
        }
    }

    // Add outgoing_targets from global index
    for (target_key, entries) in &index.outgoing_targets {
        for entry in entries {
            if entry.repo_id < 1000 {
                continue;
            }
            outbounds.push(CollectedOutbound {
                file: entry.file.clone(),
                line: entry.line as u64,
                kind: infer_kind_from_resource(target_key).to_string(),
                callee: entry.symbol.clone(),
                address_hint: target_key.clone(),
                repo_name: Some(entry.repo_name.clone()),
            });
        }
    }

    // Add gRPC servers as endpoints
    for (service_name, entries) in &index.grpc_servers {
        for entry in entries {
            if entry.repo_id < 1000 {
                continue;
            }
            endpoints.push(CollectedEndpoint {
                file: entry.file.clone(),
                line: entry.line as u64,
                kind: "grpc_server".to_string(),
                path: format!("grpc:{}", service_name),
                method: None,
                service: entry.symbol.clone(),
                repo_name: Some(entry.repo_name.clone()),
            });
        }
    }

    // Add gRPC clients as outbound calls
    for (service_name, entries) in &index.grpc_clients {
        for entry in entries {
            if entry.repo_id < 1000 {
                continue;
            }
            outbounds.push(CollectedOutbound {
                file: entry.file.clone(),
                line: entry.line as u64,
                kind: "grpc_client".to_string(),
                callee: entry.symbol.clone(),
                address_hint: format!("grpc:{}", service_name),
                repo_name: Some(entry.repo_name.clone()),
            });
        }
    }
}

/// Match outbound calls against exposed endpoints using multiple strategies.
fn match_outbounds_to_endpoints(
    outbounds: &[CollectedOutbound],
    endpoints: &[CollectedEndpoint],
) -> Vec<CrossRepoMatch> {
    let mut matches = Vec::new();
    let mut seen = rustc_hash::FxHashSet::default();

    for outbound in outbounds {
        for endpoint in endpoints {
            // Skip matching within the same repo (both must have repo names, and they must differ,
            // or one must be local and the other remote)
            if !is_cross_repo(outbound.repo_name.as_deref(), endpoint.repo_name.as_deref()) {
                continue;
            }

            let match_type = check_match(outbound, endpoint);
            if let Some(mt) = match_type {
                // Dedup key: unique connection pair (collapse repeated matches)
                let dedup_key = format!(
                    "{}:{}->{}:{}:{}",
                    outbound.file, outbound.line, endpoint.file, endpoint.line, mt
                );
                if !seen.insert(dedup_key) {
                    continue;
                }

                matches.push(CrossRepoMatch {
                    client_file: outbound.file.clone(),
                    client_line: outbound.line,
                    client_callee: outbound.callee.clone(),
                    client_kind: format_kind(&outbound.kind).to_string(),
                    client_repo: outbound.repo_name.clone(),
                    server_file: endpoint.file.clone(),
                    server_line: endpoint.line,
                    server_path: endpoint.path.clone(),
                    server_kind: format_kind(&endpoint.kind).to_string(),
                    server_repo: endpoint.repo_name.clone(),
                    match_type: mt,
                });
            }
        }
    }

    matches
}

/// Check whether an outbound call and an endpoint are from different repos.
fn is_cross_repo(outbound_repo: Option<&str>, endpoint_repo: Option<&str>) -> bool {
    match (outbound_repo, endpoint_repo) {
        // One local (None) and one remote (Some) = cross-repo
        (None, Some(_)) | (Some(_), None) => true,
        // Both remote but different repos = cross-repo
        (Some(a), Some(b)) => a != b,
        // Both local = not cross-repo
        (None, None) => false,
    }
}

/// Check if an outbound call matches an endpoint. Returns the match type if matched.
fn check_match(outbound: &CollectedOutbound, endpoint: &CollectedEndpoint) -> Option<String> {
    // 1. Path match: outbound's address hint contains a path that matches an endpoint's path
    if let Some(mt) = try_path_match(outbound, endpoint) {
        return Some(mt);
    }

    // 2. gRPC service match: New{Service}Client ↔ Register{Service}Server
    if let Some(mt) = try_grpc_match(outbound, endpoint) {
        return Some(mt);
    }

    // 3. URL/hostname match: .svc.cluster.local or direct service name
    if let Some(mt) = try_url_match(outbound, endpoint) {
        return Some(mt);
    }

    None
}

/// Try to match by path string (e.g., both reference "/ws/s2s").
fn try_path_match(outbound: &CollectedOutbound, endpoint: &CollectedEndpoint) -> Option<String> {
    if endpoint.path.is_empty() {
        return None;
    }

    let ep_path = &endpoint.path;

    // Skip generic catch-all paths that match everything
    if ep_path == "/" || ep_path == "*" || ep_path == "websocket" || ep_path.len() < 3 {
        return None;
    }

    // Skip noise paths on either side
    if is_noise_path(&outbound.file) || is_noise_path(&endpoint.file) {
        return None;
    }

    // Direct path match in address hint — require the path to be specific (starts with /)
    if ep_path.starts_with('/') && !outbound.address_hint.is_empty()
        && outbound.address_hint.contains(ep_path.as_str())
    {
        return Some("path".to_string());
    }

    // Check if callee contains the path — only for specific paths
    if ep_path.starts_with('/') && !outbound.callee.is_empty()
        && outbound.callee.contains(ep_path.as_str())
    {
        return Some("path".to_string());
    }

    // Extract path from address hint and compare
    if let Some(hint_path) = extract_url_path(&outbound.address_hint) {
        if hint_path == *ep_path && hint_path.len() >= 3 {
            return Some("path".to_string());
        }
    }

    None
}

/// Try to match by gRPC service name.
/// Matches patterns like: New{Service}Client ↔ Register{Service}Server
/// Also matches grpc:{ServiceName} keys from the global index.
fn try_grpc_match(outbound: &CollectedOutbound, endpoint: &CollectedEndpoint) -> Option<String> {
    let out_service = extract_grpc_service_name(&outbound.callee);
    let ep_service = extract_grpc_service_name(&endpoint.service)
        .or_else(|| extract_grpc_service_name(&endpoint.path));

    if let (Some(out_svc), Some(ep_svc)) = (out_service, ep_service) {
        if out_svc.eq_ignore_ascii_case(&ep_svc) {
            return Some("grpc".to_string());
        }
    }

    // Match grpc:{name} format from global index
    let out_grpc = outbound.address_hint.strip_prefix("grpc:");
    let ep_grpc = endpoint.path.strip_prefix("grpc:");
    if let (Some(out_name), Some(ep_name)) = (out_grpc, ep_grpc) {
        if out_name.eq_ignore_ascii_case(ep_name) {
            return Some("grpc".to_string());
        }
    }

    None
}

/// Extract the gRPC service name from a callee/symbol string.
/// Handles Go: New{Service}Client, Register{Service}Server
/// Handles Python: add_{Service}Servicer_to_server, {Service}Stub
fn extract_grpc_service_name(name: &str) -> Option<String> {
    // Go client: New{Service}Client → {Service}
    if let Some(rest) = name.strip_prefix("New") {
        if let Some(service) = rest.strip_suffix("Client") {
            if !service.is_empty() {
                return Some(service.to_string());
            }
        }
    }

    // Go server: Register{Service}Server → {Service}
    if let Some(rest) = name.strip_prefix("Register") {
        if let Some(service) = rest.strip_suffix("Server") {
            if !service.is_empty() {
                return Some(service.to_string());
            }
        }
    }

    // Also handle the full FQN like "pb.NewOrderClient" → extract after last dot
    if let Some(dot_idx) = name.rfind('.') {
        let short = &name[dot_idx + 1..];
        return extract_grpc_service_name(short);
    }

    // Python: add_{Service}Servicer_to_server → {Service}
    if let Some(rest) = name.strip_prefix("add_") {
        if let Some(service) = rest.strip_suffix("Servicer_to_server") {
            if !service.is_empty() {
                return Some(service.to_string());
            }
        }
    }

    // Python: {Service}Stub → {Service}
    if let Some(service) = name.strip_suffix("Stub") {
        if !service.is_empty() {
            return Some(service.to_string());
        }
    }

    None
}

/// Try to match by URL hostname (e.g., .svc.cluster.local or service name in address).
fn try_url_match(outbound: &CollectedOutbound, endpoint: &CollectedEndpoint) -> Option<String> {
    if outbound.address_hint.is_empty() || endpoint.service.is_empty() {
        return None;
    }

    // Skip noise paths
    if is_noise_path(&outbound.file) || is_noise_path(&endpoint.file) {
        return None;
    }

    // Skip generic/short service names that match too broadly
    let service_lower = endpoint.service.to_lowercase();
    if service_lower.len() < 4 || matches!(service_lower.as_str(),
        "main" | "server" | "handler" | "service" | "app" | "http" | "grpc" | "ws") {
        return None;
    }

    // Skip catch-all endpoints
    if endpoint.path == "/" || endpoint.path == "*" {
        return None;
    }

    let hint_lower = outbound.address_hint.to_lowercase();

    // Match "service-name.svc.cluster.local" pattern
    if hint_lower.contains(".svc.cluster.local") {
        let hostname = hint_lower.split("://").last().unwrap_or(&hint_lower);
        let hostname = hostname.split(':').next().unwrap_or(hostname);
        let hostname = hostname.split('/').next().unwrap_or(hostname);
        let svc_name = hostname.split(".svc.cluster.local").next().unwrap_or(hostname);
        let bare_name = svc_name.split('.').next().unwrap_or(svc_name);

        if bare_name == service_lower || service_lower.contains(bare_name) {
            return Some("url".to_string());
        }
    }

    // Match direct service name in address — only for specific service names (>6 chars)
    if service_lower.len() > 6 && hint_lower.contains(&service_lower) {
        return Some("url".to_string());
    }

    None
}

/// Convert a ResolvedNetworkCall to a JSON value for the report.
fn taint_call_to_json(tc: &ResolvedNetworkCall) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "file": tc.file,
        "line": tc.line,
        "kind": tc.net_kind.as_str(),
        "direction": taint_direction(tc.net_kind),
        "callee": tc.callee_fqn,
        "confidence": format!("{:?}", tc.confidence).to_lowercase(),
        "address_source": tc.address_source,
    });

    // Add human-readable provenance chain
    let chain = format_address_source(&tc.address_source);
    if !chain.is_empty() {
        entry["provenance_chain"] = serde_json::Value::String(chain);
    }

    entry
}

/// Determine direction from NetworkCategory.
fn taint_direction(cat: cx_extractors::sink_registry::NetworkCategory) -> &'static str {
    use cx_extractors::sink_registry::NetworkCategory::*;
    match cat {
        HttpServer | GrpcServer | WebsocketServer | KafkaConsumer | TcpListen => "inbound",
        HttpClient | GrpcClient | WebsocketClient | KafkaProducer
        | Database | Redis | Sqs | S3 | TcpDial => "outbound",
        Unknown => "unknown",
    }
}

/// Format an AddressSource into a human-readable provenance chain string.
fn format_address_source(source: &cx_extractors::taint::AddressSource) -> String {
    use cx_extractors::taint::AddressSource;
    match source {
        AddressSource::Literal { value } => format!("\"{}\"", value),
        AddressSource::EnvVar { var_name, k8s_value } => {
            if let Some(k8s) = k8s_value {
                format!("env({}) \u{2192} \"{}\" (k8s)", var_name, k8s)
            } else {
                format!("env({})", var_name)
            }
        }
        AddressSource::ConfigKey { key, file } => {
            if let Some(f) = file {
                format!("config(\"{}\", {})", key, f)
            } else {
                format!("config(\"{}\")", key)
            }
        }
        AddressSource::Parameter { func, param_idx, caller_sources } => {
            let callers: Vec<String> = caller_sources.iter()
                .map(format_address_source)
                .collect();
            if callers.is_empty() {
                format!("param({}, #{})", func, param_idx)
            } else {
                format!("{} \u{2192} param({}, #{})", callers.join(" | "), func, param_idx)
            }
        }
        AddressSource::FieldAccess { type_name, field, assignment_sources } => {
            let assigns: Vec<String> = assignment_sources.iter()
                .map(format_address_source)
                .collect();
            if assigns.is_empty() {
                format!("{}.{}", type_name, field)
            } else {
                format!("{} \u{2192} {}.{}", assigns.join(" | "), type_name, field)
            }
        }
        AddressSource::Concat { parts } => {
            let formatted: Vec<String> = parts.iter()
                .map(format_address_source)
                .collect();
            formatted.join(" + ")
        }
        AddressSource::Flag { flag_name, default_value } => {
            if let Some(d) = default_value {
                format!("flag(--{}, default=\"{}\")", flag_name, d)
            } else {
                format!("flag(--{})", flag_name)
            }
        }
        AddressSource::ServiceDiscovery { service_name, mechanism } => {
            format!("service-discovery({}, {})", mechanism, service_name)
        }
        AddressSource::Dynamic { hint } => {
            if hint.is_empty() {
                "dynamic".to_string()
            } else {
                format!("dynamic({})", hint)
            }
        }
    }
}

/// Collect outbound network calls from Connects edges.
fn collect_connects_edges(
    graph: &CsrGraph,
    _kind_idx: &KindIndex,
    taint_index: &rustc_hash::FxHashMap<(&str, u32), &ResolvedNetworkCall>,
    calls: &mut Vec<serde_json::Value>,
    kind_filter: Option<&str>,
    service_filter: Option<&str>,
) {
    let connects_kind = EdgeKind::Connects as u8;

    for src_idx in 0..graph.node_count() {
        let src = graph.node(src_idx);

        // Connects edges typically originate from Symbol or Module nodes
        if src.kind != NodeKind::Symbol as u8 && src.kind != NodeKind::Module as u8 {
            continue;
        }

        // Skip test nodes — test network calls are not production architecture
        if src.flags & cx_core::graph::nodes::NODE_IS_TEST != 0 {
            continue;
        }

        for edge in graph.edges_for(src_idx) {
            if edge.kind != connects_kind {
                continue;
            }

            let target = graph.node(edge.target);
            let target_name = graph.strings.get(target.name);
            let src_name = graph.strings.get(src.name);

            // Infer kind from target name prefix (resource:redis, resource:grpc, etc.)
            let inferred_kind = infer_kind_from_resource(target_name);

            // Apply service filter: check if source belongs to the requested service
            if let Some(sf) = service_filter {
                if !node_belongs_to_service(graph, src_idx, sf) {
                    continue;
                }
            }

            let file = if src.file != u32::MAX {
                Some(graph.strings.get(src.file).to_string())
            } else {
                None
            };

            // Try to enrich with taint analysis data
            let taint_match = file.as_deref()
                .and_then(|f| taint_index.get(&(f, src.line)));

            // Use taint-detected kind if available, otherwise fall back to inference
            let kind = if let Some(tc) = taint_match {
                tc.net_kind.as_str()
            } else {
                inferred_kind
            };

            // Apply kind filter with enriched kind
            if let Some(kf) = kind_filter {
                if !kind_matches(kind, kf) {
                    continue;
                }
            }

            let mut entry = serde_json::json!({
                "file": file,
                "line": if src.line > 0 { Some(src.line) } else { None },
                "kind": kind,
                "direction": "outbound",
                "target": {
                    "source": "graph_edge",
                    "name": target_name,
                },
                "symbol": src_name,
            });

            // Enrich with taint provenance if available
            if let Some(tc) = taint_match {
                entry["callee"] = serde_json::Value::String(tc.callee_fqn.clone());
                entry["confidence"] = serde_json::Value::String(
                    format!("{:?}", tc.confidence).to_lowercase()
                );
                entry["address_source"] = serde_json::to_value(&tc.address_source)
                    .unwrap_or(serde_json::Value::Null);
                let chain = format_address_source(&tc.address_source);
                if !chain.is_empty() {
                    entry["provenance_chain"] = serde_json::Value::String(chain);
                }
            } else {
                // Fall back to graph-based provenance chain
                let chain = build_provenance_chain(graph, src_idx);
                if !chain.is_empty() {
                    entry["provenance"] = serde_json::Value::Array(chain);
                }
            }

            calls.push(entry);
        }
    }
}

/// Collect exposed API endpoints from Exposes edges.
fn collect_exposes_edges(
    graph: &CsrGraph,
    _kind_idx: &KindIndex,
    apis: &mut Vec<serde_json::Value>,
    kind_filter: Option<&str>,
    service_filter: Option<&str>,
) {
    let exposes_kind = EdgeKind::Exposes as u8;

    for src_idx in 0..graph.node_count() {
        let src = graph.node(src_idx);

        // Skip test nodes — test endpoint registrations are not production APIs
        if src.flags & cx_core::graph::nodes::NODE_IS_TEST != 0 {
            continue;
        }

        for edge in graph.edges_for(src_idx) {
            if edge.kind != exposes_kind {
                continue;
            }

            let endpoint = graph.node(edge.target);

            // Also skip endpoints that are themselves from test files
            if endpoint.flags & cx_core::graph::nodes::NODE_IS_TEST != 0 {
                continue;
            }

            let endpoint_name = graph.strings.get(endpoint.name);
            let src_name = graph.strings.get(src.name);

            // Infer kind from endpoint name
            let inferred_kind = infer_kind_from_endpoint(endpoint_name);

            if let Some(kf) = kind_filter {
                if !kind_matches(inferred_kind, kf) {
                    continue;
                }
            }

            if let Some(sf) = service_filter {
                if !node_belongs_to_service(graph, src_idx, sf) {
                    continue;
                }
            }

            let file = if endpoint.file != u32::MAX {
                Some(graph.strings.get(endpoint.file).to_string())
            } else if src.file != u32::MAX {
                Some(graph.strings.get(src.file).to_string())
            } else {
                None
            };
            let line = if endpoint.line > 0 {
                Some(endpoint.line)
            } else if src.line > 0 {
                Some(src.line)
            } else {
                None
            };

            // Parse method and path from endpoint name (e.g. "GET /api/users")
            let (method, path) = parse_endpoint_name(endpoint_name);

            let entry = serde_json::json!({
                "file": file,
                "line": line,
                "kind": inferred_kind,
                "path": path,
                "method": method,
                "service": src_name,
            });

            apis.push(entry);
        }
    }
}

/// Collect Publishes/Subscribes edges as network calls.
fn collect_pubsub_edges(
    graph: &CsrGraph,
    _kind_idx: &KindIndex,
    calls: &mut Vec<serde_json::Value>,
    kind_filter: Option<&str>,
    service_filter: Option<&str>,
) {
    let publishes_kind = EdgeKind::Publishes as u8;
    let subscribes_kind = EdgeKind::Subscribes as u8;

    for src_idx in 0..graph.node_count() {
        let src = graph.node(src_idx);

        for edge in graph.edges_for(src_idx) {
            let (direction, inferred_kind) = if edge.kind == publishes_kind {
                ("outbound", "kafka_producer")
            } else if edge.kind == subscribes_kind {
                ("inbound", "kafka_consumer")
            } else {
                continue;
            };

            let target = graph.node(edge.target);
            let target_name = graph.strings.get(target.name);
            let src_name = graph.strings.get(src.name);

            if let Some(kf) = kind_filter {
                if !kind_matches(inferred_kind, kf) {
                    continue;
                }
            }

            if let Some(sf) = service_filter {
                if !node_belongs_to_service(graph, src_idx, sf) {
                    continue;
                }
            }

            let file = if src.file != u32::MAX {
                Some(graph.strings.get(src.file).to_string())
            } else {
                None
            };

            let entry = serde_json::json!({
                "file": file,
                "line": if src.line > 0 { Some(src.line) } else { None },
                "kind": inferred_kind,
                "direction": direction,
                "target": {
                    "source": "graph_edge",
                    "name": target_name,
                },
                "symbol": src_name,
            });

            calls.push(entry);
        }
    }
}

/// Infer a network kind string from a Resource node name.
fn infer_kind_from_resource(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("grpc") || lower.contains("proto") {
        "grpc_client"
    } else if lower.contains("http") || lower.contains("rest") || lower.contains("api") {
        "http_client"
    } else if lower.contains("redis") {
        "redis"
    } else if lower.contains("kafka") || lower.contains("queue") || lower.contains("mq") {
        "kafka_producer"
    } else if lower.contains("postgres") || lower.contains("mysql") || lower.contains("sql")
        || lower.contains("database") || lower.contains("mongo") || lower.contains("db")
    {
        "database"
    } else if lower.contains("websocket") || lower.contains("ws://") {
        "websocket_client"
    } else if lower.contains("sqs") {
        "sqs"
    } else if lower.contains("s3") || lower.contains("bucket") {
        "s3"
    } else if lower.contains("tcp") || lower.contains("socket") {
        "tcp_dial"
    } else {
        "unknown"
    }
}

/// Infer kind from an Endpoint node name.
fn infer_kind_from_endpoint(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("grpc") || lower.contains("proto") {
        "grpc_server"
    } else if name.starts_with("Register") && name.ends_with("Server") {
        // Go gRPC pattern: RegisterXxxServer
        "grpc_server"
    } else if name.starts_with("add_") && name.ends_with("_to_server") {
        // Python gRPC pattern: add_XxxServicer_to_server
        "grpc_server"
    } else if lower.contains("websocket") || lower.contains("ws") {
        "websocket_server"
    } else {
        // Most endpoints are HTTP by default
        "http"
    }
}

/// Check if a kind string matches a filter.
fn kind_matches(kind: &str, filter: &str) -> bool {
    let filter_lower = filter.to_lowercase();
    let kind_lower = kind.to_lowercase();

    // Exact match
    if kind_lower == filter_lower {
        return true;
    }

    // Prefix match (e.g. "http" matches "http_client" and "http_server")
    if kind_lower.starts_with(&filter_lower) {
        return true;
    }

    // Category match
    match filter_lower.as_str() {
        "grpc" => kind_lower.contains("grpc"),
        "http" => kind_lower.contains("http"),
        "websocket" => kind_lower.contains("websocket"),
        "kafka" => kind_lower.contains("kafka"),
        _ => false,
    }
}

/// Check if a node belongs to a given service (by walking up Contains edges to Deployable).
fn node_belongs_to_service(graph: &CsrGraph, node_idx: u32, service: &str) -> bool {
    let service_lower = service.to_lowercase();

    // Check the node itself
    let node = graph.node(node_idx);
    if graph.strings.get(node.name).to_lowercase().contains(&service_lower) {
        return true;
    }

    // Walk up parent chain
    let mut current = node_idx;
    for _ in 0..10 {
        let n = graph.node(current);
        if n.parent == u32::MAX || n.parent == current {
            break;
        }
        let parent = graph.node(n.parent);
        if (parent.kind == NodeKind::Deployable as u8 || parent.kind == NodeKind::Module as u8)
            && graph.strings.get(parent.name).to_lowercase().contains(&service_lower) {
                return true;
            }
        current = n.parent;
    }

    false
}

/// Build a simple provenance chain by walking backward through Calls edges from a symbol.
fn build_provenance_chain(graph: &CsrGraph, node_idx: u32) -> Vec<serde_json::Value> {
    let mut chain = Vec::new();
    let calls_kind = EdgeKind::Calls as u8;

    // Walk reverse Calls edges (who calls this symbol?)
    for rev_edge in graph.rev_edges_for(node_idx) {
        if rev_edge.kind != calls_kind {
            continue;
        }
        let caller = graph.node(rev_edge.target);
        let caller_name = graph.strings.get(caller.name);
        let file = if caller.file != u32::MAX {
            Some(graph.strings.get(caller.file).to_string())
        } else {
            None
        };

        chain.push(serde_json::json!({
            "symbol": caller_name,
            "file": file,
            "line": if caller.line > 0 { Some(caller.line) } else { None },
        }));

        if chain.len() >= 10 {
            break;
        }
    }

    chain
}

/// Parse an endpoint name into (method, path).
/// Handles formats like "GET /api/users", "/api/users", or just a name.
fn parse_endpoint_name(name: &str) -> (Option<&str>, &str) {
    let trimmed = name.trim();

    // Check for "METHOD /path" format
    let methods = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];
    for method in &methods {
        if let Some(rest) = trimmed.strip_prefix(method) {
            let rest = rest.trim_start();
            if rest.starts_with('/') {
                return (Some(method), rest);
            }
        }
    }

    // If it starts with /, it's just a path
    if trimmed.starts_with('/') {
        return (None, trimmed);
    }

    // Otherwise, return as-is
    (None, trimmed)
}

/// Print a summary header with counts by kind and direction.
fn print_summary_header(report: &serde_json::Value) {
    let mut inbound_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut outbound_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut remote_counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();

    if let Some(calls) = report.get("network_calls").and_then(|v| v.as_array()) {
        for call in calls {
            let kind = call["kind"].as_str().unwrap_or("unknown");
            let direction = call["direction"].as_str().unwrap_or("outbound");
            let display = format_kind(kind).to_string();
            match direction {
                "inbound" => *inbound_counts.entry(display).or_insert(0) += 1,
                _ => *outbound_counts.entry(display).or_insert(0) += 1,
            }
            let file = call["file"].as_str().unwrap_or("");
            if file.starts_with('[') {
                if let Some(end) = file.find(']') {
                    let name = &file[1..end];
                    *remote_counts.entry(name.to_string()).or_insert(0) += 1;
                }
            }
        }
    }
    if let Some(apis) = report.get("exposed_apis").and_then(|v| v.as_array()) {
        for api in apis {
            let kind = api["kind"].as_str().unwrap_or("unknown");
            let display = format_kind(kind).to_string();
            *inbound_counts.entry(display).or_insert(0) += 1;
        }
    }

    if inbound_counts.is_empty() && outbound_counts.is_empty() {
        return;
    }

    println!("Network Boundaries");
    if !inbound_counts.is_empty() {
        let parts: Vec<String> = inbound_counts.iter().map(|(k, v)| format!("{} {}", v, k)).collect();
        println!("  Inbound:  {}", parts.join(", "));
    }
    if !outbound_counts.is_empty() {
        let parts: Vec<String> = outbound_counts.iter().map(|(k, v)| format!("{} {}", v, k)).collect();
        println!("  Outbound: {}", parts.join(", "));
    }
    if !remote_counts.is_empty() {
        let parts: Vec<String> = remote_counts.iter().map(|(k, v)| format!("{} ({} calls)", k, v)).collect();
        println!("  Remotes:  {}", parts.join(", "));
    }
    if let Some(conns) = report.get("cross_repo_connections").and_then(|v| v.as_array()) {
        if !conns.is_empty() {
            println!("  Cross-repo: {} connection(s)", conns.len());
        }
    }
    println!();
}

/// Print the network report in human-readable format.
fn print_human_readable(report: &serde_json::Value) {
    if let Some(calls) = report.get("network_calls").and_then(|v| v.as_array()) {
        if !calls.is_empty() {
            println!("Network Calls:");
            for call in calls {
                let file = call["file"].as_str().unwrap_or("unknown");
                let line = call["line"].as_u64().unwrap_or(0);
                let location = if line > 0 {
                    format!("{}:{}", file, line)
                } else {
                    file.to_string()
                };
                println!("  {}", location);

                let kind = call["kind"].as_str().unwrap_or("unknown");
                let direction = call["direction"].as_str().unwrap_or("outbound");
                let confidence = call.get("confidence").and_then(|v| v.as_str());
                let conf_tag = match confidence {
                    Some("typeconfirmed") => " [type-confirmed]",
                    Some("heuristic") => " [heuristic]",
                    _ => "",
                };
                println!("    Kind:      {} ({}){}", format_kind(kind), direction, conf_tag);

                // Show callee FQN if from taint analysis
                if let Some(callee) = call.get("callee").and_then(|v| v.as_str()) {
                    println!("    Callee:    {}", callee);
                }

                // Show taint provenance chain if available
                if let Some(chain) = call.get("provenance_chain").and_then(|v| v.as_str()) {
                    println!("    Source:    {}", chain);
                } else if let Some(target) = call.get("target") {
                    // Fall back to graph-edge target
                    let target_name = target["name"].as_str().unwrap_or("unknown");
                    println!("    Target:    {}", target_name);
                }

                // Show graph-based provenance if no taint data
                if call.get("provenance_chain").is_none() {
                    if let Some(provenance) = call.get("provenance").and_then(|v| v.as_array()) {
                        if !provenance.is_empty() {
                            let chain_parts: Vec<String> = provenance.iter().map(|p| {
                                let sym = p["symbol"].as_str().unwrap_or("?");
                                sym.to_string()
                            }).collect();
                            println!("    Chain:     {}", chain_parts.join(" \u{2192} "));
                        }
                    }
                }
                println!();
            }
        }
    }

    if let Some(apis) = report.get("exposed_apis").and_then(|v| v.as_array()) {
        if !apis.is_empty() {
            println!("Exposed APIs (inbound):");
            for api in apis {
                let file = api["file"].as_str().unwrap_or("unknown");
                let line = api["line"].as_u64().unwrap_or(0);
                let location = if line > 0 {
                    format!("{}:{}", file, line)
                } else {
                    file.to_string()
                };
                println!("  {}", location);

                let kind = api["kind"].as_str().unwrap_or("unknown");
                println!("    Kind:      {}", format_kind(kind));

                let path = api["path"].as_str().unwrap_or("");
                if !path.is_empty() {
                    println!("    Path:      {}", path);
                }

                if let Some(method) = api["method"].as_str() {
                    println!("    Method:    {}", method);
                }

                println!();
            }
        }
    }

    // Print cross-repo connections
    if let Some(connections) = report.get("cross_repo_connections").and_then(|v| v.as_array()) {
        if !connections.is_empty() {
            println!("Cross-Repo Connections:");
            for conn in connections {
                let client_file = conn["client_file"].as_str().unwrap_or("unknown");
                let client_line = conn["client_line"].as_u64().unwrap_or(0);
                let server_file = conn["server_file"].as_str().unwrap_or("unknown");
                let server_line = conn["server_line"].as_u64().unwrap_or(0);

                let client_loc = if client_line > 0 {
                    format!("{}:{}", client_file, client_line)
                } else {
                    client_file.to_string()
                };
                let server_loc = if server_line > 0 {
                    format!("{}:{}", server_file, server_line)
                } else {
                    server_file.to_string()
                };

                // Show repo tags if available
                let client_tag = conn["client_repo"].as_str()
                    .map(|r| format!("[{}] ", r))
                    .unwrap_or_default();
                let server_tag = conn["server_repo"].as_str()
                    .map(|r| format!("[{}] ", r))
                    .unwrap_or_default();

                println!("  {}{}  -->  {}{}", client_tag, client_loc, server_tag, server_loc);

                let client_callee = conn["client_callee"].as_str().unwrap_or("?");
                let client_kind = conn["client_kind"].as_str().unwrap_or("?");
                let server_path = conn["server_path"].as_str().unwrap_or("?");
                let server_kind = conn["server_kind"].as_str().unwrap_or("?");
                let match_type = conn["match_type"].as_str().unwrap_or("?");

                println!(
                    "    Client: {} ({})  -->  Server: {} ({})",
                    client_callee, client_kind, server_path, server_kind,
                );
                println!("    Match:  {}", match_type);
                println!();
            }
        }
    }

    // If nothing was found
    let has_calls = report.get("network_calls")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    let has_apis = report.get("exposed_apis")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    let has_connections = report.get("cross_repo_connections")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());

    if !has_calls && !has_apis && !has_connections {
        println!("No network boundaries detected.");
        println!("Hint: Run `cx init` first, then `cx network` to see results.");
    }
}

/// Format a kind string for human display.
fn format_kind(kind: &str) -> &str {
    match kind {
        "http_client" => "HTTP client",
        "http_server" => "HTTP server",
        "http" => "HTTP",
        "grpc_client" => "gRPC client",
        "grpc_server" => "gRPC server",
        "websocket_client" => "WebSocket client",
        "websocket_server" => "WebSocket server",
        "kafka_producer" => "Kafka producer",
        "kafka_consumer" => "Kafka consumer",
        "database" => "Database",
        "redis" => "Redis",
        "sqs" => "SQS",
        "s3" => "S3",
        "tcp_dial" => "TCP dial",
        "tcp_listen" => "TCP listen",
        _ => kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_project() -> (tempfile::TempDir, CsrGraph) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            r#"package main

import (
    "fmt"
    "net/http"
)

func main() {
    http.HandleFunc("/", handler)
    http.HandleFunc("/api/health", healthHandler)
    http.ListenAndServe(":8080", nil)
}

func handler(w http.ResponseWriter, r *http.Request) {
    fmt.Fprintf(w, "Hello")
}

func healthHandler(w http.ResponseWriter, r *http.Request) {
    fmt.Fprintf(w, "OK")
}
"#,
        )
        .unwrap();
        crate::commands::build::run(dir.path(), &[], false, false).unwrap();
        let graph = crate::indexing::load_graph(dir.path()).unwrap();
        (dir, graph)
    }

    #[test]
    fn network_report_returns_valid_json() {
        let (_dir, graph) = setup_project();
        let report = build_network_report(&graph, &[], None, None, None);
        // Should be a valid JSON object
        assert!(report.is_object());
        // Should have the expected top-level keys
        assert!(report.get("network_calls").is_some() || report.get("exposed_apis").is_some());
    }

    #[test]
    fn network_report_direction_filter() {
        let (_dir, graph) = setup_project();

        let outbound_only = build_network_report(&graph, &[], None, Some("outbound"), None);
        assert!(outbound_only.get("network_calls").is_some());
        assert!(outbound_only.get("exposed_apis").is_none());

        let inbound_only = build_network_report(&graph, &[], None, Some("inbound"), None);
        assert!(inbound_only.get("exposed_apis").is_some());
        assert!(inbound_only.get("network_calls").is_none());
    }

    #[test]
    fn network_report_kind_filter() {
        let (_dir, graph) = setup_project();
        // Filter for HTTP — should not error
        let report = build_network_report(&graph, &[], Some("http"), None, None);
        assert!(report.is_object());
    }

    #[test]
    fn parse_endpoint_name_method_and_path() {
        assert_eq!(parse_endpoint_name("GET /api/users"), (Some("GET"), "/api/users"));
        assert_eq!(parse_endpoint_name("POST /api/data"), (Some("POST"), "/api/data"));
        assert_eq!(parse_endpoint_name("/api/health"), (None, "/api/health"));
        assert_eq!(parse_endpoint_name("myEndpoint"), (None, "myEndpoint"));
    }

    #[test]
    fn kind_matches_exact_and_prefix() {
        assert!(kind_matches("http_client", "http"));
        assert!(kind_matches("http_server", "http"));
        assert!(kind_matches("grpc_client", "grpc"));
        assert!(kind_matches("database", "database"));
        assert!(!kind_matches("redis", "http"));
    }

    #[test]
    fn infer_kind_from_resource_names() {
        assert_eq!(infer_kind_from_resource("resource:redis"), "redis");
        assert_eq!(infer_kind_from_resource("resource:grpc:myservice"), "grpc_client");
        assert_eq!(infer_kind_from_resource("resource:http:api"), "http_client");
        assert_eq!(infer_kind_from_resource("resource:postgres:db"), "database");
        assert_eq!(infer_kind_from_resource("resource:kafka:topic"), "kafka_producer");
        assert_eq!(infer_kind_from_resource("something_unknown"), "unknown");
    }

    #[test]
    fn format_kind_labels() {
        assert_eq!(format_kind("http_client"), "HTTP client");
        assert_eq!(format_kind("grpc_server"), "gRPC server");
        assert_eq!(format_kind("database"), "Database");
        assert_eq!(format_kind("unknown_thing"), "unknown_thing");
    }

    #[test]
    fn human_readable_output_no_panic() {
        let (_dir, graph) = setup_project();
        let report = build_network_report(&graph, &[], None, None, None);
        // Just ensure it doesn't panic
        print_human_readable(&report);
    }

    #[test]
    fn empty_graph_no_crash() {
        let dir = tempfile::tempdir().unwrap();
        // Create a minimal file so init succeeds
        fs::write(dir.path().join("empty.txt"), "").unwrap();
        // init might fail on empty project, so use a Go file
        fs::write(dir.path().join("main.go"), "package main\n\nfunc main() {}\n").unwrap();
        crate::commands::build::run(dir.path(), &[], false, false).unwrap();
        let graph = crate::indexing::load_graph(dir.path()).unwrap();
        let report = build_network_report(&graph, &[], None, None, None);
        assert!(report.is_object());
    }

    // ─── Cross-repo matching tests ────────────────────────────────────

    #[test]
    fn extract_path_from_hint_dynamic() {
        // "dynamic(unresolved var: /ws/s2s)" → "/ws/s2s"
        assert_eq!(
            extract_path_from_hint("dynamic(unresolved var: /ws/s2s)"),
            Some("/ws/s2s".to_string()),
        );
        // No path
        assert_eq!(extract_path_from_hint("dynamic(unknown)"), None);
        // Path in middle
        assert_eq!(
            extract_path_from_hint("some context /api/health)"),
            Some("/api/health".to_string()),
        );
    }

    #[test]
    fn extract_url_path_from_full_url() {
        assert_eq!(
            extract_url_path("http://example.com/api/v1/users"),
            Some("/api/v1/users".to_string()),
        );
        assert_eq!(
            extract_url_path("ws://host:8080/ws/s2s"),
            Some("/ws/s2s".to_string()),
        );
        assert_eq!(extract_url_path("http://example.com/"), None);
        assert_eq!(extract_url_path("http://example.com"), None);
        assert_eq!(
            extract_url_path("/api/health"),
            Some("/api/health".to_string()),
        );
    }

    #[test]
    fn extract_grpc_service_name_go_patterns() {
        assert_eq!(
            extract_grpc_service_name("NewS2SClient"),
            Some("S2S".to_string()),
        );
        assert_eq!(
            extract_grpc_service_name("RegisterS2SServer"),
            Some("S2S".to_string()),
        );
        assert_eq!(
            extract_grpc_service_name("NewOrderProcessingClient"),
            Some("OrderProcessing".to_string()),
        );
        assert_eq!(
            extract_grpc_service_name("RegisterOrderProcessingServer"),
            Some("OrderProcessing".to_string()),
        );
        // With package prefix
        assert_eq!(
            extract_grpc_service_name("pb.NewOrderClient"),
            Some("Order".to_string()),
        );
    }

    #[test]
    fn extract_grpc_service_name_python_patterns() {
        assert_eq!(
            extract_grpc_service_name("add_OrderServicer_to_server"),
            Some("Order".to_string()),
        );
        assert_eq!(
            extract_grpc_service_name("OrderStub"),
            Some("Order".to_string()),
        );
    }

    #[test]
    fn is_cross_repo_checks() {
        // Local vs remote = cross-repo
        assert!(is_cross_repo(None, Some("remote-svc")));
        assert!(is_cross_repo(Some("remote-svc"), None));
        // Different remotes = cross-repo
        assert!(is_cross_repo(Some("svc-a"), Some("svc-b")));
        // Same repo = not cross-repo
        assert!(!is_cross_repo(Some("svc-a"), Some("svc-a")));
        // Both local = not cross-repo
        assert!(!is_cross_repo(None, None));
    }

    #[test]
    fn path_match_websocket() {
        let outbound = CollectedOutbound {
            file: "s2s_client.cpp".to_string(),
            line: 188,
            kind: "websocket_client".to_string(),
            callee: "ws_.async_handshake".to_string(),
            address_hint: "/ws/s2s".to_string(),
            repo_name: Some("native-client".to_string()),
        };
        let endpoint = CollectedEndpoint {
            file: "transport/ws.go".to_string(),
            line: 471,
            kind: "websocket_server".to_string(),
            path: "/ws/s2s".to_string(),
            method: None,
            service: "wsHandler".to_string(),
            repo_name: None,
        };

        let result = check_match(&outbound, &endpoint);
        assert_eq!(result, Some("path".to_string()));
    }

    #[test]
    fn grpc_match_client_to_server() {
        let outbound = CollectedOutbound {
            file: "client.go".to_string(),
            line: 20,
            kind: "grpc_client".to_string(),
            callee: "pb.NewOrderProcessingClient".to_string(),
            address_hint: "localhost:50051".to_string(),
            repo_name: Some("frontend".to_string()),
        };
        let endpoint = CollectedEndpoint {
            file: "server.go".to_string(),
            line: 10,
            kind: "grpc_server".to_string(),
            path: "RegisterOrderProcessingServer".to_string(),
            method: None,
            service: "RegisterOrderProcessingServer".to_string(),
            repo_name: None,
        };

        let result = check_match(&outbound, &endpoint);
        assert_eq!(result, Some("grpc".to_string()));
    }

    #[test]
    fn url_match_svc_cluster_local() {
        let outbound = CollectedOutbound {
            file: "api.go".to_string(),
            line: 30,
            kind: "http_client".to_string(),
            callee: "http.Get".to_string(),
            address_hint: "http://order-service.default.svc.cluster.local:8080/api/orders".to_string(),
            repo_name: Some("frontend".to_string()),
        };
        let endpoint = CollectedEndpoint {
            file: "handler.go".to_string(),
            line: 15,
            kind: "http_server".to_string(),
            path: "/api/orders".to_string(),
            method: Some("GET".to_string()),
            service: "order-service".to_string(),
            repo_name: None,
        };

        let result = check_match(&outbound, &endpoint);
        // Should match on URL hostname
        assert!(result.is_some());
    }

    #[test]
    fn match_outbounds_to_endpoints_dedup() {
        let outbounds = vec![
            CollectedOutbound {
                file: "client.cpp".to_string(),
                line: 100,
                kind: "websocket_client".to_string(),
                callee: "connect".to_string(),
                address_hint: "/ws/s2s".to_string(),
                repo_name: Some("native-client".to_string()),
            },
        ];
        let endpoints = vec![
            CollectedEndpoint {
                file: "ws.go".to_string(),
                line: 50,
                kind: "websocket_server".to_string(),
                path: "/ws/s2s".to_string(),
                method: None,
                service: "wsHandler".to_string(),
                repo_name: None,
            },
            // Same endpoint listed again (e.g., from index + graph)
            CollectedEndpoint {
                file: "ws.go".to_string(),
                line: 50,
                kind: "websocket_server".to_string(),
                path: "/ws/s2s".to_string(),
                method: None,
                service: "wsHandler2".to_string(),
                repo_name: None,
            },
        ];

        let matches = match_outbounds_to_endpoints(&outbounds, &endpoints);
        // Should dedup: same client:line → server:line pair should appear once
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn no_match_within_same_repo() {
        let outbounds = vec![
            CollectedOutbound {
                file: "client.go".to_string(),
                line: 10,
                kind: "http_client".to_string(),
                callee: "http.Get".to_string(),
                address_hint: "/api/internal".to_string(),
                repo_name: None,
            },
        ];
        let endpoints = vec![
            CollectedEndpoint {
                file: "server.go".to_string(),
                line: 20,
                kind: "http_server".to_string(),
                path: "/api/internal".to_string(),
                method: None,
                service: "internalHandler".to_string(),
                repo_name: None,
            },
        ];

        let matches = match_outbounds_to_endpoints(&outbounds, &endpoints);
        // Both are local (None repo) → not cross-repo
        assert!(matches.is_empty());
    }

    #[test]
    fn find_cross_repo_matches_from_report() {
        // Build a report with local endpoints and inject remote calls
        let report = serde_json::json!({
            "exposed_apis": [
                {
                    "file": "transport/ws.go",
                    "line": 471,
                    "kind": "websocket_server",
                    "path": "/ws/s2s",
                    "method": null,
                    "service": "wsTransport",
                }
            ],
            "network_calls": [
                {
                    "file": "s2s_client.cpp",
                    "line": 188,
                    "kind": "websocket_client",
                    "direction": "outbound",
                    "callee": "ws_.async_handshake",
                    "provenance_chain": "dynamic(unresolved var: /ws/s2s)",
                }
            ]
        });

        // Without a root dir, remote data won't load, but local matching
        // still won't match because both sides have repo_name=None (same repo).
        // This is correct behavior: local-to-local is not cross-repo.
        let dir = tempfile::tempdir().unwrap();
        let matches = find_cross_repo_matches(&report, dir.path());
        assert!(matches.is_empty(), "local-to-local should not produce cross-repo matches");
    }

    #[test]
    fn find_cross_repo_matches_with_remote_network_data() {
        let dir = tempfile::tempdir().unwrap();

        // Create .cx/remotes directory with remote network data
        let remotes_dir = dir.path().join(".cx").join("remotes");
        fs::create_dir_all(&remotes_dir).unwrap();

        // Write a remote network.json for "native-client"
        let remote_calls = serde_json::json!([
            {
                "net_kind": "websocket_client",
                "callee_fqn": "ws_.async_handshake",
                "address_source": {
                    "Dynamic": { "hint": "unresolved var: /ws/s2s" }
                },
                "file": "native/src/s2s_client.cpp",
                "line": 188,
                "confidence": "heuristic"
            }
        ]);
        fs::write(
            remotes_dir.join("native-client.network.json"),
            serde_json::to_string(&remote_calls).unwrap(),
        ).unwrap();

        // Build a report with local exposed APIs
        let report = serde_json::json!({
            "exposed_apis": [
                {
                    "file": "transport/ws.go",
                    "line": 471,
                    "kind": "websocket_server",
                    "path": "/ws/s2s",
                    "method": null,
                    "service": "wsTransport",
                }
            ],
            "network_calls": []
        });

        let matches = find_cross_repo_matches(&report, dir.path());
        assert_eq!(matches.len(), 1, "should find one cross-repo match");
        assert_eq!(matches[0]["client_repo"], "native-client");
        assert_eq!(matches[0]["server_path"], "/ws/s2s");
        assert_eq!(matches[0]["match_type"], "path");
    }

    #[test]
    fn find_cross_repo_matches_bidirectional_with_index() {
        let dir = tempfile::tempdir().unwrap();

        // Create .cx/graph directory
        let graph_dir = dir.path().join(".cx").join("graph");
        fs::create_dir_all(&graph_dir).unwrap();

        // Create a global index with a remote gRPC server
        let index = crate::graph_index::GlobalIndex {
            exposed_apis: std::collections::HashMap::new(),
            outgoing_targets: std::collections::HashMap::new(),
            grpc_servers: {
                let mut m = std::collections::HashMap::new();
                m.insert("OrderProcessing".to_string(), vec![
                    crate::graph_index::IndexEntry {
                        repo_id: 1000,
                        repo_name: "order-svc".to_string(),
                        file: "server.go".to_string(),
                        line: 10,
                        symbol: "RegisterOrderProcessingServer".to_string(),
                    },
                ]);
                m
            },
            grpc_clients: std::collections::HashMap::new(),
        };
        index.save(dir.path()).unwrap();

        // Build a report with a local gRPC client call
        let report = serde_json::json!({
            "exposed_apis": [],
            "network_calls": [
                {
                    "file": "client.go",
                    "line": 20,
                    "kind": "grpc_client",
                    "direction": "outbound",
                    "callee": "pb.NewOrderProcessingClient",
                    "provenance_chain": "\"localhost:50051\"",
                }
            ]
        });

        let matches = find_cross_repo_matches(&report, dir.path());
        assert_eq!(matches.len(), 1, "should find gRPC cross-repo match from index");
        assert_eq!(matches[0]["match_type"], "grpc");
        assert_eq!(matches[0]["server_repo"], "order-svc");
    }

    #[test]
    fn human_readable_cross_repo_output() {
        let report = serde_json::json!({
            "network_calls": [],
            "exposed_apis": [],
            "cross_repo_connections": [
                {
                    "client_file": "native/src/s2s_client.cpp",
                    "client_line": 188,
                    "client_callee": "ws_.async_handshake",
                    "client_kind": "WebSocket client",
                    "client_repo": "native-client",
                    "server_file": "transport/ws.go",
                    "server_line": 471,
                    "server_path": "/ws/s2s",
                    "server_kind": "WebSocket server",
                    "server_repo": null,
                    "match_type": "path",
                }
            ]
        });

        // Should not panic
        print_human_readable(&report);
    }

    #[test]
    fn extract_from_address_source_literal() {
        let source = serde_json::json!({
            "Literal": { "value": "http://example.com/api/v1" }
        });
        assert_eq!(
            extract_from_address_source(&source),
            Some("http://example.com/api/v1".to_string()),
        );
    }

    #[test]
    fn extract_from_address_source_dynamic() {
        let source = serde_json::json!({
            "Dynamic": { "hint": "unresolved var: /ws/s2s" }
        });
        assert_eq!(
            extract_from_address_source(&source),
            Some("/ws/s2s".to_string()),
        );
    }

    #[test]
    fn extract_from_address_source_env_with_k8s() {
        let source = serde_json::json!({
            "EnvVar": { "var_name": "SVC_ADDR", "k8s_value": "productcatalog:3550" }
        });
        assert_eq!(
            extract_from_address_source(&source),
            Some("productcatalog:3550".to_string()),
        );
    }
}
