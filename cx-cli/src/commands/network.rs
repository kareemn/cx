use anyhow::Result;
use cx_core::graph::csr::CsrGraph;
use cx_core::graph::edges::EdgeKind;
use cx_core::graph::kind_index::KindIndex;
use cx_core::graph::nodes::NodeKind;
use cx_extractors::taint::ResolvedNetworkCall;
use std::path::Path;

/// Run `cx network` — list all detected network calls and exposed APIs with provenance.
pub fn run(root: &Path, json: bool, kind: Option<&str>, direction: Option<&str>, service: Option<&str>) -> Result<()> {
    let graph = super::init::load_graph(root)?;

    // Load local taint analysis results
    let mut taint_calls = load_network_json(root);

    // Load remote network data from .cx/remotes/*.network.json
    let remote_calls = load_remote_network_json(root);

    // Merge remote calls (prefixed with remote name)
    taint_calls.extend(remote_calls);

    // Deduplicate by (file, line) — keep highest confidence entry
    dedup_by_location(&mut taint_calls);

    let result = build_network_report(&graph, &taint_calls, kind, direction, service);

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
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

/// Load network.json files from all pulled remotes in .cx/remotes/.
/// Prefixes file paths with [remote_name] for disambiguation.
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
        if !name.ends_with(".network.json") {
            continue;
        }
        let remote_name = name.trim_end_matches(".network.json");

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut calls: Vec<ResolvedNetworkCall> = match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Prefix file paths with remote name for disambiguation
        for call in &mut calls {
            call.file = format!("[{}] {}", remote_name, call.file);
        }
        all_calls.extend(calls);
    }

    all_calls
}

/// Deduplicate network calls by (file, line).
/// When the same location appears with multiple confidence levels, keep the highest.
fn dedup_by_location(calls: &mut Vec<ResolvedNetworkCall>) {
    use std::collections::HashMap;
    let mut best: HashMap<(String, u32), usize> = HashMap::new();

    // Confidence ordering: TypeConfirmed > LLMClassified > ImportResolved > Heuristic
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
            // else: keep the previous one, skip this duplicate
        } else {
            keep[i] = true;
            best.insert(key, i);
        }
    }

    let mut i = 0;
    calls.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
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

    // Apply direction filter to network_calls (not just top-level sections)
    if let Some(dir) = direction_filter {
        network_calls.retain(|call| {
            let call_dir = call["direction"].as_str().unwrap_or("outbound");
            call_dir == dir
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

/// Convert a ResolvedNetworkCall to a JSON value for the report.
fn taint_call_to_json(tc: &ResolvedNetworkCall) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "file": tc.file,
        "line": tc.line,
        "kind": tc.net_kind.as_str(),
        "direction": taint_direction(tc.net_kind),
        "callee": tc.callee_fqn,
        "confidence": serde_json::to_value(&tc.confidence).ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "heuristic".to_string()),
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
                    serde_json::to_value(&tc.confidence).ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "heuristic".to_string())
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
                    Some("type_confirmed") => " [type-confirmed]",
                    Some("llm_classified") => " [llm-classified]",
                    Some("import_resolved") => " [import-resolved]",
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

    // If nothing was found
    let has_calls = report.get("network_calls")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    let has_apis = report.get("exposed_apis")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());

    if !has_calls && !has_apis {
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
        super::super::init::run(dir.path()).unwrap();
        let graph = super::super::init::load_graph(dir.path()).unwrap();
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
    fn format_service_discovery() {
        let source = cx_extractors::taint::AddressSource::ServiceDiscovery {
            service_name: "payment-service".to_string(),
            mechanism: "consul".to_string(),
        };
        let formatted = format_address_source(&source);
        assert_eq!(formatted, "service-discovery(consul, payment-service)");
    }

    #[test]
    fn empty_graph_no_crash() {
        let dir = tempfile::tempdir().unwrap();
        // Create a minimal file so init succeeds
        fs::write(dir.path().join("empty.txt"), "").unwrap();
        // init might fail on empty project, so use a Go file
        fs::write(dir.path().join("main.go"), "package main\n\nfunc main() {}\n").unwrap();
        super::super::init::run(dir.path()).unwrap();
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        let report = build_network_report(&graph, &[], None, None, None);
        assert!(report.is_object());
    }
}
