use anyhow::{Context, Result};
use cx_extractors::lsp::LspOrchestrator;
use cx_extractors::pipeline::{self, IndexResult, MergedResult};
use cx_extractors::sink_registry;
use cx_extractors::taint::Confidence;
use std::path::PathBuf;

/// Run the full indexing pipeline with cross-repo resolution:
/// 1. Extract and merge all repos
/// 2. Run resolution engine (gRPC, REST, env→Helm→k8s, Docker image, WebSocket)
/// 3. Optionally upgrade heuristic results via LSP
/// 4. Build the unified CSR graph
pub fn index_repos_with_resolution(repos: &[(PathBuf, u16)], verbose: bool, custom_sinks: &cx_extractors::custom_sinks::CustomSinkConfig, model_only: bool) -> Result<IndexResult> {
    let mut merged = pipeline::extract_and_merge_repos(repos, custom_sinks)
        .context("failed to extract repos")?;

    if verbose {
        eprintln!("Raw extraction: {} total function calls across {} files",
            merged.raw_call_count, merged.file_count);
    }

    let resolved = resolve_cross_repo(&mut merged);
    if resolved > 0 {
        eprintln!("Resolved {} cross-repo connection(s)", resolved);
    }

    // Backward pass: add Resolves edges linking network targets to their env var sources.
    let resolves_added = add_resolves_edges(&mut merged);
    if resolves_added > 0 {
        eprintln!("Backward pass: {} Resolves edge(s) linking env vars to network calls", resolves_added);
    }

    if model_only {
        // In model-only mode, mark ALL network calls as Heuristic so the LLM processes them all.
        // This skips LSP upgrade and lets us compare pure-LLM accuracy against the static pipeline.
        for call in &mut merged.network_calls {
            call.confidence = cx_extractors::taint::Confidence::Heuristic;
        }
    } else {
        // LSP integration: try to upgrade Heuristic network calls to TypeConfirmed
        if !merged.network_calls.is_empty() {
            let workspace_root = repos.first().map(|(p, _)| p.as_path());
            if let Some(root) = workspace_root {
                upgrade_via_lsp(&mut merged, root, verbose);
            }
        }
    }

    // LLM integration: classify heuristic calls via Claude CLI
    // In model-only mode, this processes ALL calls. In normal mode, only unresolved ones.
    if !merged.network_calls.is_empty() {
        let workspace_root = repos.first().map(|(p, _)| p.as_path());
        if let Some(root) = workspace_root {
            upgrade_via_llm(&mut merged.network_calls, root, verbose);
        }
    }

    // Second pass: LLM classification may have added new network calls that now
    // have Connects edges (from tree-sitter resource queries). Re-run resolves
    // to link them to env var sources.
    let resolves_added_2 = add_resolves_edges(&mut merged);
    if resolves_added_2 > 0 {
        eprintln!("Backward pass (post-LLM): {} additional Resolves edge(s)", resolves_added_2);
    }

    Ok(pipeline::build_index(merged))
}

/// Format an AddressSource chain as a human-readable provenance string.
/// e.g. "self.wsURL (field) <- NewClient.url (param) <- EnvVar(WS_URI)"
pub fn format_address_chain(src: &cx_extractors::taint::AddressSource) -> String {
    use cx_extractors::taint::AddressSource;
    match src {
        AddressSource::Literal { value } => format!("\"{}\"", truncate(value, 50)),
        AddressSource::EnvVar { var_name, k8s_value } => {
            if let Some(v) = k8s_value {
                format!("env({}) = \"{}\"", var_name, truncate(v, 40))
            } else {
                format!("env({})", var_name)
            }
        }
        AddressSource::ConfigKey { key, file } => {
            if let Some(f) = file {
                format!("config({}, {})", key, f)
            } else {
                format!("config({})", key)
            }
        }
        AddressSource::Parameter { func, param_idx, caller_sources } => {
            let base = format!("{}.param[{}]", func, param_idx);
            if caller_sources.is_empty() {
                base
            } else {
                let callers: Vec<String> = caller_sources.iter()
                    .map(format_address_chain)
                    .collect();
                format!("{} <- [{}]", base, callers.join(", "))
            }
        }
        AddressSource::FieldAccess { type_name, field, assignment_sources } => {
            let base = format!("{}.{}", type_name, field);
            if assignment_sources.is_empty() {
                base
            } else {
                let assigns: Vec<String> = assignment_sources.iter()
                    .map(format_address_chain)
                    .collect();
                format!("{} <- [{}]", base, assigns.join(", "))
            }
        }
        AddressSource::Concat { parts } => {
            let pieces: Vec<String> = parts.iter()
                .map(format_address_chain)
                .collect();
            pieces.join(" + ")
        }
        AddressSource::Flag { flag_name, default_value } => {
            if let Some(d) = default_value {
                format!("flag({}, default=\"{}\")", flag_name, d)
            } else {
                format!("flag({})", flag_name)
            }
        }
        AddressSource::ServiceDiscovery { service_name, mechanism } => {
            format!("svc_discovery({}, {})", service_name, mechanism)
        }
        AddressSource::Dynamic { hint } => {
            if hint.is_empty() { "dynamic".to_string() } else { format!("dynamic({})", hint) }
        }
    }
}

/// Extract ~30 lines of source context around a call site for LLM analysis.
fn extract_call_context(file_path: &str, line: u32, workspace_root: &std::path::Path) -> Option<String> {
    let full_path = workspace_root.join(file_path);
    let content = std::fs::read_to_string(&full_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let target = (line as usize).saturating_sub(1);
    let start = target.saturating_sub(15);
    let end = (target + 15).min(lines.len());
    Some(lines[start..end].join("\n"))
}

/// Upgrade heuristic network calls via Claude CLI with source context.
/// Sends function context to Haiku and asks for both classification AND target resolution.
/// Silently skips if `claude` CLI is not available.
/// Load natural language context from .cx/config/context.md if it exists.
fn load_context_md(workspace_root: &std::path::Path) -> Option<String> {
    let path = workspace_root.join(".cx").join("config").join("context.md");
    std::fs::read_to_string(&path).ok()
}

/// A cached LLM classification result, keyed by file:line:callee.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct CachedClassification {
    kind: String,
    direction: String,
    target: String,
    target_source: String,
    context_hash: u64,
}

/// Simple hash of a string for cache invalidation.
fn hash_context(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Load LLM classification cache from .cx/graph/llm_cache.json.
fn load_llm_cache(workspace_root: &std::path::Path) -> std::collections::HashMap<String, CachedClassification> {
    let path = workspace_root.join(".cx").join("graph").join("llm_cache.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return std::collections::HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

/// Save LLM classification cache to .cx/graph/llm_cache.json.
fn save_llm_cache(workspace_root: &std::path::Path, cache: &std::collections::HashMap<String, CachedClassification>) {
    let path = workspace_root.join(".cx").join("graph").join("llm_cache.json");
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        let _ = std::fs::write(&path, json);
    }
}

fn upgrade_via_llm(network_calls: &mut Vec<cx_extractors::taint::ResolvedNetworkCall>, workspace_root: &std::path::Path, verbose: bool) {
    // Check if claude CLI is available
    let claude_check = std::process::Command::new("claude")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match claude_check {
        Ok(s) if s.success() => {}
        _ => {
            if verbose {
                eprintln!("LLM: claude CLI not found, skipping");
            }
            return;
        }
    }

    // Load LLM cache and apply cached results before collecting heuristic indices
    let mut llm_cache = load_llm_cache(workspace_root);
    let mut cache_hits = 0u32;

    for call in network_calls.iter_mut() {
        if call.confidence != Confidence::Heuristic {
            continue;
        }
        let key = format!("{}:{}:{}", call.file, call.line, call.callee_fqn);
        let context = extract_call_context(&call.file, call.line, workspace_root)
            .unwrap_or_default();
        let ctx_hash = hash_context(&context);

        if let Some(cached) = llm_cache.get(&key) {
            if cached.context_hash == ctx_hash {
                // Cache hit — apply without LLM call
                if cached.kind != "not_network" && !cached.kind.is_empty() {
                    if let Some(cat) = parse_network_category(&cached.kind) {
                        call.net_kind = cat;
                        call.confidence = Confidence::LLMClassified;
                    }
                }
                cache_hits += 1;
            }
        }
    }

    // Collect remaining heuristic calls (not resolved by cache)
    let heuristic_indices: Vec<usize> = network_calls
        .iter()
        .enumerate()
        .filter(|(_, c)| c.confidence == Confidence::Heuristic)
        .map(|(i, _)| i)
        .collect();

    if heuristic_indices.is_empty() {
        if cache_hits > 0 {
            eprintln!("LLM: {} call(s) resolved from cache, 0 new", cache_hits);
        } else if verbose {
            eprintln!("LLM: no heuristic calls remaining, skipping");
        }
        return;
    }

    // ANSI color codes
    const DIM: &str = "\x1b[2m";
    const BOLD: &str = "\x1b[1m";
    const GREEN: &str = "\x1b[32m";
    const YELLOW: &str = "\x1b[33m";
    const CYAN: &str = "\x1b[36m";
    const RED: &str = "\x1b[31m";
    const MAGENTA: &str = "\x1b[35m";
    const RESET: &str = "\x1b[0m";

    // Load natural language context if available
    let context_md = load_context_md(workspace_root);
    if context_md.is_some() {
        eprintln!("  {DIM}loaded .cx/config/context.md{RESET}");
    }

    let batch_size: usize = 50;
    let concurrency: usize = 8;
    let total_batches = (heuristic_indices.len() + batch_size - 1) / batch_size;
    eprintln!(
        "\n{BOLD}LLM{RESET}  classifying {CYAN}{}{RESET} heuristic calls via Claude CLI  {DIM}({} batches of {}, {}x parallel){RESET}\n",
        heuristic_indices.len(), total_batches, batch_size, concurrency,
    );

    // ── Phase 1: Build all prompts upfront ──────────────────────────────

    let system_prompt =
        "Classify these call sites. Some are confirmed network calls needing re-classification;\n\
         others (marked current_kind=unknown) are CANDIDATES that may or may not be network calls.\n\
         For each, respond with ONLY a JSON array.\n\
         Each entry: {\"idx\": N, \"kind\": \"...\", \"direction\": \"inbound|outbound\", \"target\": \"...\", \"target_source\": \"...\"}\n\n\
         Kinds: http_client, http_server, grpc_client, grpc_server, websocket_client, websocket_server,\n\
                kafka_producer, kafka_consumer, database, redis, sqs, s3, tcp_dial, tcp_listen, not_network\n\n\
         Use 'not_network' for calls that are NOT actually making or receiving network connections\n\
         (e.g. protobuf serialization, logging, string manipulation, context management).\n\n\
         For 'target': trace the variable chain to its origin. Use the provenance chain provided.\n\
         - If it resolves to an env var, return the env var name (e.g. \"DATABASE_URL\")\n\
         - If it resolves to a config key, return the key (e.g. \"db.host\")\n\
         - If it resolves to a literal, return the literal value\n\
         - If it resolves to a field, return type.field (e.g. \"Config.wsURL\")\n\
         - If it resolves to a parameter, return func.param[N] (e.g. \"NewClient.param[0]\")\n\
         - Only use \"dynamic\" if truly unresolvable after following the chain\n\n\
         For 'target_source': literal|env_var|config|parameter|field|concat|service_discovery|flag|dynamic\n\n";

    struct BatchJob {
        batch_num: usize,
        call_indices: Vec<usize>,
        prompt: String,
        preview: String,
    }

    let mut jobs: Vec<BatchJob> = Vec::with_capacity(total_batches);

    for (batch_num, batch) in heuristic_indices.chunks(batch_size).enumerate() {
        let files_in_batch: Vec<String> = batch.iter().map(|&i| {
            let c = &network_calls[i];
            format!("{}:{}", c.file, c.line)
        }).collect();
        let preview: String = if files_in_batch.len() <= 3 {
            files_in_batch.join(", ")
        } else {
            format!("{}, {} +{} more",
                files_in_batch[0], files_in_batch[1], files_in_batch.len() - 2)
        };

        let mut prompt = String::from(system_prompt);

        if let Some(ref ctx) = context_md {
            prompt.push_str("Project context (provided by the developer):\n");
            prompt.push_str(ctx);
            prompt.push_str("\n\n");
        }

        for (batch_idx, &call_idx) in batch.iter().enumerate() {
            let call = &network_calls[call_idx];
            let context = extract_call_context(&call.file, call.line, workspace_root)
                .unwrap_or_default();
            let chain = format_address_chain(&call.address_source);
            prompt.push_str(&format!(
                "[{}] {}:{} callee={} current_kind={}\n  provenance: {}\n{}\n\n",
                batch_idx, call.file, call.line, call.callee_fqn,
                call.net_kind.as_str(), chain, context
            ));
        }

        prompt.push_str("Respond ONLY with a JSON array.\n");

        jobs.push(BatchJob {
            batch_num,
            call_indices: batch.to_vec(),
            prompt,
            preview,
        });
    }

    // ── Phase 2: Run batches in parallel ────────────────────────────────

    struct BatchResult {
        batch_num: usize,
        call_indices: Vec<usize>,
        preview: String,
        elapsed: std::time::Duration,
        outcome: Result<Vec<serde_json::Value>, String>,
    }

    let batch_start_time = std::time::Instant::now();

    // Semaphore: bounded channel with `concurrency` tokens — created before scope
    // so it outlives all spawned threads.
    let (sem_tx, sem_rx) = std::sync::mpsc::sync_channel::<()>(concurrency);
    for _ in 0..concurrency {
        sem_tx.send(()).ok();
    }
    let sem_rx = std::sync::Mutex::new(sem_rx);

    // Live progress counters (shared across threads)
    let done_count = std::sync::atomic::AtomicUsize::new(0);
    let inflight_count = std::sync::atomic::AtomicUsize::new(0);
    let progress_lock = std::sync::Mutex::new(());  // serializes stderr writes

    let results: Vec<BatchResult> = std::thread::scope(|scope| {
        let sem_rx_ref = &sem_rx;
        let sem_tx_ref = &sem_tx;
        let done_ref = &done_count;
        let inflight_ref = &inflight_count;
        let progress_ref = &progress_lock;
        let start_ref = &batch_start_time;

        let handles: Vec<_> = jobs.into_iter().map(|job| {
            scope.spawn(move || {
                // Acquire slot
                let _permit = sem_rx_ref.lock().unwrap().recv();
                inflight_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                // Update progress: starting
                {
                    let _lock = progress_ref.lock().unwrap();
                    let done = done_ref.load(std::sync::atomic::Ordering::Relaxed);
                    let inflight = inflight_ref.load(std::sync::atomic::Ordering::Relaxed);
                    let elapsed = start_ref.elapsed().as_secs_f64();
                    eprint!(
                        "\r  {DIM}[{done}/{total_batches}]{RESET} {YELLOW}{inflight} in-flight{RESET}  {DIM}{elapsed:.0}s{RESET}    ",
                    );
                }

                let call_start = std::time::Instant::now();
                let result = std::process::Command::new("claude")
                    .args(["-p", &job.prompt, "--output-format", "json", "--model", "haiku"])
                    .output();
                let elapsed = call_start.elapsed();

                // Release slot
                sem_tx_ref.send(()).ok();
                inflight_ref.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                let done = done_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;

                // Update progress: completed
                {
                    let _lock = progress_ref.lock().unwrap();
                    let inflight = inflight_ref.load(std::sync::atomic::Ordering::Relaxed);
                    let wall = start_ref.elapsed().as_secs_f64();

                    // Progress bar
                    let bar_width = 20usize;
                    let filled = (done * bar_width) / total_batches;
                    let bar: String = format!(
                        "{}{}", "█".repeat(filled), "░".repeat(bar_width - filled),
                    );

                    eprint!(
                        "\r  {bar} {DIM}{done}/{total_batches}{RESET}  {YELLOW}{inflight} in-flight{RESET}  {DIM}{wall:.0}s{RESET}    ",
                    );
                }

                let outcome = match result {
                    Ok(o) if o.status.success() => {
                        let stdout = String::from_utf8_lossy(&o.stdout);
                        let result_text = if let Ok(wrapper) = serde_json::from_str::<serde_json::Value>(&stdout) {
                            wrapper.get("result").and_then(|v| v.as_str()).unwrap_or(&stdout).to_string()
                        } else {
                            stdout.to_string()
                        };
                        let classifications: Vec<serde_json::Value> = if let Some(start) = result_text.find('[') {
                            if let Some(end) = result_text.rfind(']') {
                                serde_json::from_str(&result_text[start..=end]).unwrap_or_default()
                            } else { Vec::new() }
                        } else { Vec::new() };
                        Ok(classifications)
                    }
                    Ok(o) => {
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        let stdout = String::from_utf8_lossy(&o.stdout);
                        let reason = if !stderr.is_empty() {
                            stderr.lines().last().unwrap_or("").trim().to_string()
                        } else if !stdout.is_empty() {
                            stdout.lines().last().unwrap_or("").trim().to_string()
                        } else {
                            format!("exit code {}", o.status)
                        };
                        Err(reason)
                    }
                    Err(e) => Err(e.to_string()),
                };

                BatchResult {
                    batch_num: job.batch_num,
                    call_indices: job.call_indices,
                    preview: job.preview,
                    elapsed,
                    outcome,
                }
            })
        }).collect();

        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // Clear progress line
    eprint!("\r{}\r", " ".repeat(80));

    // ── Phase 3: Apply results (deterministic order) ────────────────────

    let mut upgraded = 0u32;
    let mut not_network = 0u32;
    let mut failed_batches = 0u32;
    let mut parse_failures = 0u32;

    let mut sorted_results = results;
    sorted_results.sort_by_key(|r| r.batch_num);

    for br in &sorted_results {
        eprintln!(
            "  {DIM}[{}/{}]{RESET} {DIM}{}{RESET}  {DIM}({:.1}s){RESET}",
            br.batch_num + 1, total_batches, br.preview, br.elapsed.as_secs_f64(),
        );

        let classifications = match &br.outcome {
            Ok(c) => c,
            Err(reason) => {
                failed_batches += 1;
                eprintln!("         {RED}failed: {}{RESET}", reason);
                continue;
            }
        };

        if classifications.is_empty() && verbose {
            eprintln!("         {RED}no parseable JSON in response{RESET}");
        }

        for entry in classifications {
            let idx = entry.get("idx").and_then(|v| v.as_u64()).unwrap_or(u64::MAX) as usize;
            let kind = entry.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let direction = entry.get("direction").and_then(|v| v.as_str()).unwrap_or("?");
            let target = entry.get("target").and_then(|v| v.as_str()).unwrap_or("?");
            let target_source = entry.get("target_source").and_then(|v| v.as_str()).unwrap_or("?");

            if idx >= br.call_indices.len() {
                parse_failures += 1;
                continue;
            }

            let call_idx = br.call_indices[idx];
            let call_loc = format!("{}:{}", network_calls[call_idx].file, network_calls[call_idx].line);

            // Cache this result
            let context = extract_call_context(&network_calls[call_idx].file, network_calls[call_idx].line, workspace_root)
                .unwrap_or_default();
            let cache_key = format!("{}:{}:{}", network_calls[call_idx].file, network_calls[call_idx].line, network_calls[call_idx].callee_fqn);
            llm_cache.insert(cache_key, CachedClassification {
                kind: kind.to_string(),
                direction: direction.to_string(),
                target: target.to_string(),
                target_source: target_source.to_string(),
                context_hash: hash_context(&context),
            });

            if kind == "not_network" || kind.is_empty() {
                not_network += 1;
                if verbose {
                    eprintln!("         {DIM}{} -> skip (not network){RESET}", call_loc);
                }
                continue;
            }

            if let Some(cat) = parse_network_category(kind) {
                network_calls[call_idx].net_kind = cat;
                network_calls[call_idx].confidence = Confidence::LLMClassified;
                upgraded += 1;

                let kind_colored = match direction {
                    "outbound" => format!("{MAGENTA}{}{RESET}", kind),
                    "inbound" => format!("{CYAN}{}{RESET}", kind),
                    _ => kind.to_string(),
                };
                let target_colored = match target_source {
                    "literal" => format!("{GREEN}\"{}\"{RESET}", target),
                    "env_var" => format!("{YELLOW}${}{RESET}", target),
                    "config" => format!("{CYAN}{}{RESET}", target),
                    "dynamic" => format!("{DIM}{}{RESET}", target),
                    _ => target.to_string(),
                };
                let arrow = if direction == "inbound" {
                    format!("{CYAN}<-{RESET}")
                } else {
                    format!("{MAGENTA}->{RESET}")
                };
                eprintln!(
                    "         {DIM}{}{RESET}  {kind_colored} {arrow} {target_colored} {DIM}({target_source}){RESET}",
                    call_loc,
                );
            } else {
                parse_failures += 1;
                if verbose {
                    eprintln!("         {DIM}{}{RESET}  {RED}unknown: {}{RESET}", call_loc, kind);
                }
            }
        }
    }

    // Save updated cache
    save_llm_cache(workspace_root, &llm_cache);

    let total_elapsed = batch_start_time.elapsed();
    let cache_str = if cache_hits > 0 {
        format!("{DIM}{} cached, {RESET}", cache_hits)
    } else {
        String::new()
    };
    eprintln!(
        "\n{BOLD}LLM{RESET}  {cache_str}{GREEN}{} classified{RESET}, {DIM}{} not network, {} failed, {} errors{RESET}  {DIM}({:.1}s){RESET}\n",
        upgraded, not_network, failed_batches, parse_failures, total_elapsed.as_secs_f64(),
    );
}

fn parse_network_category(kind: &str) -> Option<sink_registry::NetworkCategory> {
    use sink_registry::NetworkCategory::*;
    match kind {
        "http_client" => Some(HttpClient), "http_server" => Some(HttpServer),
        "grpc_client" => Some(GrpcClient), "grpc_server" => Some(GrpcServer),
        "websocket_client" => Some(WebsocketClient), "websocket_server" => Some(WebsocketServer),
        "kafka_producer" => Some(KafkaProducer), "kafka_consumer" => Some(KafkaConsumer),
        "database" => Some(Database), "redis" => Some(Redis),
        "sqs" => Some(Sqs), "s3" => Some(S3),
        "tcp_dial" => Some(TcpDial), "tcp_listen" => Some(TcpListen),
        "unknown" => Some(Unknown),
        _ => None,
    }
}

/// Try to upgrade heuristic network call classifications using LSP type info.
/// This is best-effort — if no LSP servers are available, results stay as Heuristic.
/// Backward pass: add Resolves edges linking network targets to env var sources.
///
/// Strategy 1: For each Connects edge (function → resource), find Configures edges
/// from the SAME function. If found, add a Resolves edge: resource → env_var_resource.
///
/// Strategy 2: For each Configures edge (function reads env var), walk forward through
/// Calls edges to find functions that have Connects edges. Add Resolves from the
/// Connects target back to the env var.
///
/// Strategy 3: Walk backward from Connects source through Calls edges (up to 5 hops)
/// to find any function with a Configures edge.
fn add_resolves_edges(merged: &mut MergedResult) -> usize {
    use cx_core::graph::edges::EdgeKind;
    use cx_core::graph::nodes::NodeKind;

    let mut new_edges: Vec<cx_core::graph::csr::EdgeInput> = Vec::new();

    // Build node ID → index lookup (node IDs are NOT sequential array indices)
    let node_by_id: rustc_hash::FxHashMap<u32, usize> = merged.nodes.iter()
        .enumerate()
        .map(|(idx, n)| (n.id, idx))
        .collect();

    // Index: node_id → list of (target_node_id, edge_kind)
    let mut outgoing: rustc_hash::FxHashMap<u32, Vec<(u32, EdgeKind)>> = rustc_hash::FxHashMap::default();
    // Index: node_id → list of caller_node_ids (reverse Calls)
    let mut callers: rustc_hash::FxHashMap<u32, Vec<u32>> = rustc_hash::FxHashMap::default();

    for edge in &merged.edges {
        outgoing.entry(edge.source).or_default().push((edge.target, edge.kind));
        if edge.kind == EdgeKind::Calls {
            callers.entry(edge.target).or_default().push(edge.source);
        }
    }

    // Collect existing Resolves edges to avoid duplicates
    let mut existing_resolves: rustc_hash::FxHashSet<(u32, u32)> = rustc_hash::FxHashSet::default();
    for edge in &merged.edges {
        if edge.kind == EdgeKind::Resolves {
            existing_resolves.insert((edge.source, edge.target));
        }
    }

    // Strategy 1: Same-function Connects + Configures
    // If a function has both Connects→X and Configures→Y, add X→Resolves→Y
    for (&func_id, edges) in &outgoing {
        let node_idx = match node_by_id.get(&func_id) {
            Some(&idx) => idx,
            None => continue,
        };
        let node = &merged.nodes[node_idx];
        if node.kind != NodeKind::Symbol as u8 {
            continue;
        }

        let connects_targets: Vec<u32> = edges.iter()
            .filter(|(_, k)| *k == EdgeKind::Connects)
            .map(|(t, _)| *t)
            .collect();
        let configures_targets: Vec<u32> = edges.iter()
            .filter(|(_, k)| *k == EdgeKind::Configures)
            .map(|(t, _)| *t)
            .collect();

        // Link each Connects target to each Configures target (env var) in the same function
        for &conn_target in &connects_targets {
            for &conf_target in &configures_targets {
                // Only link if the Configures target looks like a network address env var
                let conf_name = node_by_id.get(&conf_target)
                    .map(|&idx| merged.strings.get(merged.nodes[idx].name))
                    .unwrap_or("");
                if is_network_env_var(conf_name) && !existing_resolves.contains(&(conn_target, conf_target)) {
                    new_edges.push(cx_core::graph::csr::EdgeInput::new(
                        conn_target, conf_target, EdgeKind::Resolves,
                    ));
                    existing_resolves.insert((conn_target, conf_target));
                }
            }
        }
    }

    // Strategy 2: Walk backward from Connects source through Calls (up to 5 hops)
    // to find functions with Configures edges that feed the network call
    let connects_edges: Vec<(u32, u32)> = merged.edges.iter()
        .filter(|e| e.kind == EdgeKind::Connects)
        .map(|e| (e.source, e.target))
        .collect();
    for (conn_source, conn_target) in &connects_edges {
        let conn_target = *conn_target;
        let conn_source = *conn_source;

        // BFS backward through callers
        let mut visited = rustc_hash::FxHashSet::default();
        visited.insert(conn_source);
        let mut frontier = vec![conn_source];

        for _depth in 0..5 {
            let mut next_frontier = Vec::new();
            for &func_id in &frontier {
                if let Some(caller_ids) = callers.get(&func_id) {
                    for &caller_id in caller_ids {
                        if !visited.insert(caller_id) {
                            continue;
                        }
                        next_frontier.push(caller_id);

                        // Check if this caller has Configures edges
                        if let Some(caller_edges) = outgoing.get(&caller_id) {
                            for &(target, kind) in caller_edges {
                                if kind == EdgeKind::Configures {
                                    let conf_name = node_by_id.get(&target)
                                        .map(|&idx| merged.strings.get(merged.nodes[idx].name))
                                        .unwrap_or("");
                                    if is_network_env_var(conf_name)
                                        && !existing_resolves.contains(&(conn_target, target))
                                    {
                                        new_edges.push(cx_core::graph::csr::EdgeInput::new(
                                            conn_target, target, EdgeKind::Resolves,
                                        ));
                                        existing_resolves.insert((conn_target, target));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            frontier = next_frontier;
            if frontier.is_empty() {
                break;
            }
        }
    }

    // Strategy 3: For Configures edges where there's NO Connects edge in the same function,
    // walk FORWARD through Calls to find functions that make network calls (have Connects).
    // This handles: NewASRClient reads ASR_WS_URI → connect() does the websocket.Dial
    let configures_edges: Vec<(u32, u32)> = merged.edges.iter()
        .filter(|e| e.kind == EdgeKind::Configures)
        .map(|e| (e.source, e.target))
        .collect();
    for (reader_func, env_var_node) in &configures_edges {
        let env_var_node = *env_var_node;
        let reader_func = *reader_func;
        let env_name = node_by_id.get(&env_var_node)
            .map(|&idx| merged.strings.get(merged.nodes[idx].name))
            .unwrap_or("");
        if !is_network_env_var(env_name) {
            continue;
        }

        // Does this function also have a Connects edge? If so, Strategy 1 handled it
        let has_connects = outgoing.get(&reader_func)
            .map(|e| e.iter().any(|(_, k)| *k == EdgeKind::Connects))
            .unwrap_or(false);
        if has_connects {
            continue;
        }

        // Walk forward from the reader function through Calls edges (up to 5 hops)
        // to find a function that has a Connects edge
        let mut visited = rustc_hash::FxHashSet::default();
        visited.insert(reader_func);
        let mut frontier = vec![reader_func];

        for _depth in 0..5 {
            let mut next_frontier = Vec::new();
            for &func_id in &frontier {
                if let Some(func_edges) = outgoing.get(&func_id) {
                    for &(target, kind) in func_edges {
                        if kind == EdgeKind::Calls && visited.insert(target) {
                            next_frontier.push(target);

                            // Does this called function have Connects edges?
                            if let Some(callee_edges) = outgoing.get(&target) {
                                for &(conn_target, ck) in callee_edges {
                                    if ck == EdgeKind::Connects
                                        && !existing_resolves.contains(&(conn_target, env_var_node))
                                    {
                                        new_edges.push(cx_core::graph::csr::EdgeInput::new(
                                            conn_target, env_var_node, EdgeKind::Resolves,
                                        ));
                                        existing_resolves.insert((conn_target, env_var_node));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            frontier = next_frontier;
            if frontier.is_empty() {
                break;
            }
        }
    }

    let count = new_edges.len();
    merged.edges.extend(new_edges);
    count
}

/// Check if an env var name looks like it holds a network address.
fn is_network_env_var(name: &str) -> bool {
    let upper = name.to_uppercase();
    upper.contains("URL") || upper.contains("URI") || upper.contains("ADDR")
        || upper.contains("HOST") || upper.contains("ENDPOINT")
        || upper.contains("DATABASE") || upper.contains("REDIS")
        || upper.contains("KAFKA") || upper.ends_with("_DSN")
        || upper.contains("CONNECTION")
}

fn upgrade_via_lsp(merged: &mut MergedResult, workspace_root: &std::path::Path, verbose: bool) {
    let mut orchestrator = LspOrchestrator::start(workspace_root);

    if !orchestrator.has_servers() {
        if verbose {
            eprintln!("LSP: no language servers found on PATH, skipping");
        }
        return;
    }

    let active = orchestrator.active_languages();
    eprintln!("LSP: attempting to upgrade heuristic network calls...");
    if verbose {
        let names: Vec<&str> = active.iter().map(|l| l.language_id()).collect();
        eprintln!("  servers: {}", names.join(", "));
    }

    let heuristic_count = merged.network_calls.iter()
        .filter(|c| c.confidence == Confidence::Heuristic)
        .count();
    let total = heuristic_count;
    let mut processed = 0usize;
    let mut upgraded = 0u32;
    let mut skipped_no_lang = 0u32;
    let mut skipped_no_hover = 0u32;
    let mut skipped_no_sink = 0u32;

    for call in &mut merged.network_calls {
        if call.confidence != Confidence::Heuristic {
            continue;
        }
        processed += 1;

        // Try to resolve the callee FQN via LSP hover
        let file_path = std::path::Path::new(&call.file);
        if LspOrchestrator::language_for_file(file_path).is_none() {
            skipped_no_lang += 1;
            if verbose {
                eprintln!("  [{}/{}] skip {}:{} (unsupported language)", processed, total, call.file, call.line);
            }
            continue;
        }

        let pos = cx_extractors::lsp::Position {
            line: call.line.saturating_sub(1),
            character: 0,
        };

        if let Some(hover) = orchestrator.hover(file_path, pos) {
            // Check if the hover type matches a known sink in the registry
            let hover_text = &hover.contents;
            if sink_registry::lookup_sink(hover_text).is_some() {
                if verbose {
                    eprintln!("  [{}/{}] UPGRADED {}:{} -> TypeConfirmed ({})", processed, total, call.file, call.line, hover_text);
                }
                call.callee_fqn = hover_text.clone();
                call.confidence = Confidence::TypeConfirmed;
                upgraded += 1;
            } else {
                skipped_no_sink += 1;
                if verbose {
                    eprintln!("  [{}/{}] skip {}:{} (hover '{}' not in sink registry)", processed, total, call.file, call.line, truncate(hover_text, 60));
                }
            }
        } else {
            skipped_no_hover += 1;
            if verbose {
                eprintln!("  [{}/{}] skip {}:{} (no hover data)", processed, total, call.file, call.line);
            }
        }
    }

    eprintln!("LSP: {} upgraded, {} no hover, {} no sink match, {} unsupported language",
        upgraded, skipped_no_hover, skipped_no_sink, skipped_no_lang);

    orchestrator.shutdown();
}

/// Run the full resolution engine on merged extraction data.
/// Creates DependsOn edges for all resolved cross-repo connections.
fn resolve_cross_repo(merged: &mut MergedResult) -> usize {
    use cx_resolution::resolver::{self, ResolutionInput};

    let input = ResolutionInput {
        client_stubs: merged.grpc_clients.clone(),
        server_registrations: merged.grpc_servers.clone(),
        proto_services: merged.proto_services.clone(),
        http_client_calls: merged.http_client_calls.iter().map(|(repo, calls)| {
            (repo.clone(), calls.iter().map(|c| cx_resolution::rest_resolution::HttpClientCall {
                path: c.path.clone(), method: c.method.clone(),
                base_url_env_var: c.base_url_env_var.clone(),
                file: c.file.clone(), line: c.line,
            }).collect())
        }).collect(),
        http_server_routes: merged.http_server_routes.iter().map(|(repo, routes)| {
            (repo.clone(), routes.iter().map(|r| cx_resolution::rest_resolution::HttpServerRoute {
                path: r.path.clone(), method: r.method.clone(),
                framework: r.framework.clone(), file: r.file.clone(), line: r.line,
            }).collect())
        }).collect(),
        env_var_reads: merged.env_var_reads.iter().map(|(repo, reads)| {
            (repo.clone(), reads.iter().map(|r| cx_resolution::helm_env_resolution::EnvVarRead {
                var_name: r.var_name.clone(), file: r.file.clone(), line: r.line,
            }).collect())
        }).collect(),
        helm_env_defs: merged.helm_env_defs.iter().map(|(repo, defs)| {
            (repo.clone(), defs.iter().map(|d| cx_resolution::helm_env_resolution::HelmEnvDef {
                var_name: d.var_name.clone(), value: d.value.clone(),
                file: d.file.clone(), line: d.line,
            }).collect())
        }).collect(),
        docker_images: merged.docker_images.iter().map(|(repo, imgs)| {
            (repo.clone(), imgs.iter().map(|i| cx_resolution::image_resolution::DockerImage {
                image_ref: i.image_ref.clone(), file: i.file.clone(),
            }).collect())
        }).collect(),
        k8s_container_images: merged.k8s_container_images.iter().map(|(repo, imgs)| {
            (repo.clone(), imgs.iter().map(|i| cx_resolution::image_resolution::K8sContainerImage {
                image_ref: i.image_ref.clone(), file: i.file.clone(),
                line: i.line, deployment_name: i.deployment_name.clone(),
            }).collect())
        }).collect(),
        ws_clients: merged.ws_clients.iter().map(|(repo, conns)| {
            (repo.clone(), conns.iter().map(|c| cx_resolution::websocket_resolution::WsClientConnection {
                url_or_path: c.url_or_path.clone(), file: c.file.clone(), line: c.line,
            }).collect())
        }).collect(),
        ws_servers: merged.ws_servers.iter().map(|(repo, eps)| {
            (repo.clone(), eps.iter().map(|e| cx_resolution::websocket_resolution::WsServerEndpoint {
                path: e.path.clone(), file: e.file.clone(), line: e.line,
            }).collect())
        }).collect(),
        k8s_env_bindings: merged.k8s_env_bindings.iter().flat_map(|(_, bindings)| {
            bindings.iter().map(|b| cx_resolution::k8s_resolution::K8sEnvBinding {
                var_name: b.var_name.clone(),
                value: b.value.clone(),
                file: b.file.clone(),
                line: b.line,
                deployment_name: b.deployment_name.clone(),
            })
        }).collect(),
    };

    let result = resolver::resolve(&input);

    if !result.unresolved_client_stubs.is_empty() {
        for (repo, stub) in &result.unresolved_client_stubs {
            eprintln!(
                "  unresolved gRPC client: {} in {} ({}:{})",
                stub.service_name, repo, stub.file, stub.line
            );
        }
    }

    let mut edges_added = 0;

    // Proto/gRPC matches → DependsOn edges
    for m in &result.proto_matches {
        edges_added += add_cross_repo_edge(merged, &m.client_file, m.client_line, &m.server_file, m.server_line, m.confidence);
        eprintln!("  gRPC: {} ({}) → {} ({}) [{}]", m.client_file, m.client_repo, m.server_file, m.server_repo, m.service_name);
    }

    // REST matches → DependsOn edges
    for m in &result.rest_matches {
        edges_added += add_cross_repo_edge(merged, &m.client_file, m.client_line, &m.server_file, m.server_line, m.confidence);
        eprintln!("  REST: {} ({}) → {} ({}) [{}]", m.client_file, m.client_repo, m.server_file, m.server_repo, m.path);
    }

    // Helm env matches → Resolves edges
    for m in &result.helm_env_matches {
        edges_added += add_cross_repo_edge(merged, &m.reader_file, m.reader_line, &m.helm_file, m.helm_line, m.confidence);
        eprintln!("  Env: {} ({}) → {} ({}) [{}={}]",
            m.reader_file, m.reader_repo, m.helm_file, m.helm_repo, m.var_name, truncate(&m.helm_value, 60));
    }

    // Image matches → DependsOn edges (Dockerfile repo builds what k8s deploys)
    for m in &result.image_matches {
        edges_added += add_cross_repo_edge(merged, &m.dockerfile, 1, &m.k8s_file, m.k8s_line, m.confidence);
        eprintln!("  Image: {} ({}) → {} ({}) [{}]", m.dockerfile, m.dockerfile_repo, m.k8s_file, m.k8s_repo, m.image_path);
    }

    // WebSocket matches → DependsOn edges
    for m in &result.ws_matches {
        edges_added += add_cross_repo_edge(merged, &m.client_file, m.client_line, &m.server_file, m.server_line, m.confidence);
        eprintln!("  WS: {} ({}) → {} ({}) [{}]", m.client_file, m.client_repo, m.server_file, m.server_repo, m.path);
    }

    // K8s env matches → DependsOn edges (code env read → resolved service target)
    // Unlike other match types, the K8s manifest target may not have a graph node.
    // We find the code-side node (function reading the env var) and create a
    // DependsOn edge to the Resource node representing the env var.
    for m in &result.k8s_matches {
        // First try the standard cross-repo edge (works when k8s manifest has nodes)
        let added = add_cross_repo_edge(merged, &m.code_file, m.code_line, &m.k8s_file, m.k8s_line, m.confidence);
        if added == 0 {
            // Fallback: find the code-side function and the env var Resource node,
            // then create an edge from the function to the Resource node.
            let code_node = find_node_at(&merged.nodes, &merged.strings, &m.code_file, m.code_line);
            // Find the Resource node for this env var name
            let envvar_node = merged.nodes.iter().find(|n| {
                n.kind == cx_core::graph::nodes::NodeKind::Resource as u8
                    && merged.strings.get(n.name) == m.env_var_name
            }).map(|n| n.id);

            if let (Some(code_id), Some(env_id)) = (code_node, envvar_node) {
                if code_id != env_id {
                    let conf_u8 = (m.confidence * 255.0) as u8;
                    let mut edge = cx_core::graph::csr::EdgeInput::new(
                        code_id, env_id,
                        cx_core::graph::edges::EdgeKind::DependsOn,
                    );
                    edge.confidence_u8 = conf_u8;
                    merged.edges.push(edge);
                    edges_added += 1;
                }
            }
        } else {
            edges_added += added;
        }

        let target = if let Some(port) = m.target_port {
            format!("{}:{}", m.target_service, port)
        } else {
            m.target_service.clone()
        };
        eprintln!("  K8s: {} → {} [{}={}]",
            m.code_file, target, m.env_var_name, truncate(&m.env_value, 60));
    }

    eprintln!(
        "  Resolution summary: {} gRPC, {} REST, {} env→Helm, {} image, {} WebSocket, {} K8s env",
        result.proto_count, result.rest_count, result.helm_env_count,
        result.image_count, result.ws_count, result.k8s_count
    );

    edges_added
}

/// Add a cross-repo DependsOn edge between nodes at the given file:line locations.
/// Returns 1 if the edge was added, 0 if nodes couldn't be found.
fn add_cross_repo_edge(
    merged: &mut MergedResult,
    client_file: &str, client_line: u32,
    server_file: &str, server_line: u32,
    confidence: f32,
) -> usize {
    let client_node = find_node_at(&merged.nodes, &merged.strings, client_file, client_line);
    let server_node = find_node_at(&merged.nodes, &merged.strings, server_file, server_line);

    if let (Some(client_id), Some(server_id)) = (client_node, server_node) {
        if client_id != server_id {
            let conf_u8 = (confidence * 255.0) as u8;
            let mut edge = cx_core::graph::csr::EdgeInput::new(
                client_id, server_id,
                cx_core::graph::edges::EdgeKind::DependsOn,
            );
            edge.confidence_u8 = conf_u8;
            edge.flags = cx_core::graph::edges::EDGE_IS_CROSS_REPO;
            merged.edges.push(edge);
            return 1;
        }
    }
    0
}

/// Index a single repo without cross-repo resolution.
/// Returns the IndexResult containing only this repo's graph.
#[allow(dead_code)] // Used when cx remote is re-added
pub fn index_single_repo(repo_path: &std::path::Path, repo_id: u16) -> Result<IndexResult> {
    let repos = vec![(repo_path.to_path_buf(), repo_id)];
    let merged = pipeline::extract_and_merge_repos(&repos, &cx_extractors::custom_sinks::CustomSinkConfig::default())
        .context("failed to extract repo")?;
    Ok(pipeline::build_index(merged))
}

/// Merge all per-repo .cxgraph files from .cx/graph/repos/ AND remote graphs
/// from .cx/remotes/*.cxgraph into a unified graph.
/// Loads the overlay graph and injects cross-repo edges into the merge.
pub fn merge_per_repo_graphs(root: &std::path::Path) -> Result<cx_core::graph::csr::CsrGraph> {
    let repos_dir = root.join(".cx").join("graph").join("repos");
    let remotes_dir = root.join(".cx").join("remotes");

    let mut graph_paths: Vec<std::path::PathBuf> = Vec::new();

    // Load local per-repo graphs
    if repos_dir.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(&repos_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "cxgraph")
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            graph_paths.push(entry.path());
        }
    }

    // Load remote graphs
    if remotes_dir.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(&remotes_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "cxgraph")
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            graph_paths.push(entry.path());
        }
    }

    if graph_paths.is_empty() {
        anyhow::bail!("no .cxgraph files found in repos/ or remotes/");
    }

    let mut graphs = Vec::with_capacity(graph_paths.len());
    for path in &graph_paths {
        let graph = cx_core::store::mmap::load_graph(path)
            .with_context(|| format!("failed to load {}", path.display()))?;
        graphs.push(graph);
    }

    // Load overlay and resolve cross-repo edges to EdgeInputs
    let overlay = crate::overlay::OverlayGraph::load(root).unwrap_or_default();
    let extra_edges = overlay.to_edge_inputs(&graphs);
    let overlay_count = extra_edges.len();

    let merged = cx_core::graph::csr::CsrGraph::merge(&graphs, extra_edges);

    if overlay_count > 0 {
        eprintln!("  Applied {} overlay edge(s)", overlay_count);
    }

    Ok(merged)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}...", &s[..max]) }
}

/// Find the Symbol node closest to a given file:line location.
/// Looks for the nearest function/method node at or just before the given line.
fn find_node_at(
    nodes: &[cx_core::graph::nodes::Node],
    strings: &cx_core::graph::string_interner::StringInterner,
    file: &str,
    line: u32,
) -> Option<u32> {
    let mut best: Option<(u32, u32)> = None; // (node_id, line_distance)

    for node in nodes {
        if node.kind != cx_core::graph::nodes::NodeKind::Symbol as u8 {
            continue;
        }
        if node.file == u32::MAX {
            continue;
        }
        let node_file = strings.get(node.file);
        if node_file != file {
            continue;
        }
        // Prefer the enclosing function (line <= target) with smallest distance
        if node.line <= line {
            let dist = line - node.line;
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((node.id, dist));
            }
        }
    }

    best.map(|(id, _)| id)
}

/// Load the unified graph from .cx/graph/base.cxgraph.
/// Falls back to merging per-repo graphs if base.cxgraph doesn't exist.
pub fn load_graph(root: &std::path::Path) -> Result<cx_core::graph::csr::CsrGraph> {
    let graph_path = root.join(".cx").join("graph").join("base.cxgraph");
    if graph_path.exists() {
        return cx_core::store::mmap::load_graph(&graph_path).context("failed to load graph");
    }

    // Try layered loading (per-repo graphs + overlay)
    let repos_dir = root.join(".cx").join("graph").join("repos");
    if repos_dir.exists() {
        return merge_per_repo_graphs(root);
    }

    anyhow::bail!("index not found: run `cx build` first")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn grpc_resolution_creates_depends_on_edge() {
        let server_repo = tempfile::tempdir().unwrap();
        let client_repo = tempfile::tempdir().unwrap();

        // Server repo: registers OrderProcessing gRPC server
        fs::write(
            server_repo.path().join("server.go"),
            r#"package main

import pb "example.com/proto/order"

func StartServer() {
    s := grpc.NewServer()
    pb.RegisterOrderProcessingServer(s, &handler{})
    s.Serve(lis)
}
"#,
        )
        .unwrap();

        // Client repo: creates OrderProcessing gRPC client
        fs::write(
            client_repo.path().join("client.go"),
            r#"package main

import pb "example.com/proto/order"

func CallService() {
    conn, _ := grpc.Dial("localhost:50051")
    client := pb.NewOrderProcessingClient(conn)
    _ = client
}
"#,
        )
        .unwrap();

        let repos = vec![
            (server_repo.path().to_path_buf(), 0u16),
            (client_repo.path().to_path_buf(), 1u16),
        ];

        let result = index_repos_with_resolution(&repos, false, &cx_extractors::custom_sinks::CustomSinkConfig::default(), false).unwrap();
        let graph = &result.graph;

        // Should have a DependsOn edge from client → server
        let has_depends_on = graph.edges.iter().any(|e| {
            e.kind == cx_core::graph::edges::EdgeKind::DependsOn as u8
                && e.flags & cx_core::graph::edges::EDGE_IS_CROSS_REPO != 0
        });

        assert!(
            has_depends_on,
            "should have a cross-repo DependsOn edge from gRPC resolution"
        );
    }

    #[test]
    fn k8s_env_resolution_creates_depends_on_edge() {
        let repo = tempfile::tempdir().unwrap();

        // Go code that reads PRODUCT_CATALOG_SERVICE_ADDR env var
        fs::write(
            repo.path().join("main.go"),
            r#"package main

import "os"

func GetCatalog() {
    addr := os.Getenv("PRODUCT_CATALOG_SERVICE_ADDR")
    conn, _ := grpc.Dial(addr)
    _ = conn
}
"#,
        )
        .unwrap();

        // K8s deployment manifest with the env var binding
        fs::create_dir_all(repo.path().join("kubernetes")).unwrap();
        fs::write(
            repo.path().join("kubernetes").join("deployment.yaml"),
            r#"apiVersion: apps/v1
kind: Deployment
metadata:
  name: frontend
spec:
  template:
    spec:
      containers:
      - name: server
        image: frontend:latest
        env:
        - name: PRODUCT_CATALOG_SERVICE_ADDR
          value: "productcatalogservice:3550"
        - name: CURRENCY_SERVICE_ADDR
          value: "currencyservice:7000"
"#,
        )
        .unwrap();

        let repos = vec![(repo.path().to_path_buf(), 0u16)];
        let result = index_repos_with_resolution(&repos, false, &cx_extractors::custom_sinks::CustomSinkConfig::default(), false).unwrap();

        // Verify the K8s env bindings were extracted
        // The resolution should find PRODUCT_CATALOG_SERVICE_ADDR → productcatalogservice:3550
        // Check that we have a DependsOn edge from the code to the k8s manifest
        let graph = &result.graph;

        let has_depends_on = graph.edges.iter().any(|e| {
            e.kind == cx_core::graph::edges::EdgeKind::DependsOn as u8
        });

        assert!(
            has_depends_on,
            "should have a DependsOn edge from K8s env resolution (PRODUCT_CATALOG_SERVICE_ADDR → productcatalogservice:3550)"
        );
    }
}
