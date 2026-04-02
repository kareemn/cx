use anyhow::Result;
use cx_core::graph::csr::CsrGraph;
use cx_core::graph::edges::{EdgeKind, ALL_EDGES};
use cx_core::graph::nodes::NodeKind;
use cx_core::query::path::PathFinder;
use std::path::Path;

/// Run `cx trace <target> [--upstream]`.
///
/// Target syntax:
///   env:DATABASE_URL           — trace an environment variable
///   call:file.go:FuncName      — trace a call site
///   <symbol_name>              — trace a symbol by name
pub fn run(root: &Path, target: &str, upstream_only: bool, downstream_only: bool, max_depth: u32, json: bool) -> Result<()> {
    let graph = crate::indexing::load_graph(root)?;

    // Parse target syntax — may resolve to multiple nodes (e.g. env:*)
    let targets = resolve_targets(&graph, target);
    if targets.is_empty() {
        // Before giving up, check if network.json mentions this target in address sources.
        // This handles env vars defined in Helm/k8s config that have no graph node
        // (e.g. MEGATTS_URL in values.yaml.gotmpl with no corresponding os.Getenv in local code).
        let network_calls = load_network_calls(root);
        let provenance = find_provenance(target, &network_calls);
        if !provenance.is_empty() {
            println!("\x1b[1mNetwork calls referencing {}\x1b[0m \x1b[2m(from taint analysis)\x1b[0m:", target);
            for line in &provenance {
                println!("  {}", line);
            }
            return Ok(());
        }

        eprintln!("Target not found: {}", target);
        let candidates = fuzzy_match(&graph, target, 5);
        if !candidates.is_empty() {
            eprintln!("Did you mean:");
            for (name, file, line) in &candidates {
                if !file.is_empty() {
                    eprintln!("  {} ({}:{})", name, file, line);
                } else {
                    eprintln!("  {}", name);
                }
            }
        }
        return Ok(());
    }

    // Default: show both directions. --upstream or --downstream restricts to one.
    let show_upstream = upstream_only || !downstream_only;
    let show_downstream = downstream_only || !upstream_only;

    // Multiple targets (glob): show compact summary table
    if targets.len() > 1 {
        println!("\x1b[1m{} env vars\x1b[0m matching '{}'\n", targets.len(), target);
        let network_calls = super::network::load_network_json(root);
        for (i, (node_id, node_name)) in targets.iter().enumerate() {
            if i > 0 {
                println!();
            }
            print_compact_summary(&graph, *node_id, node_name, &network_calls);
        }
        return Ok(());
    }

    // Single target: full trace
    let (node_id, node_name) = &targets[0];
    trace_single(root, &graph, node_name, *node_id, show_upstream, show_downstream, max_depth, json)?;

    Ok(())
}

fn trace_single(
    root: &Path,
    graph: &CsrGraph,
    target: &str,
    node_id: u32,
    show_upstream: bool,
    show_downstream: bool,
    max_depth: u32,
    json: bool,
) -> Result<()> {
    // Load network.json for provenance chains
    let network_calls = load_network_calls(root);

    // Find provenance by AddressSource mentions OR by enclosing function/file
    let node = graph.node(node_id);
    let node_file = if node.file != u32::MAX { graph.strings.get(node.file) } else { "" };
    let node_line = node.line;

    let mut provenance_lines = find_provenance(target, &network_calls);

    // Also find network calls located inside this function (by file + line range)
    if provenance_lines.is_empty() && !node_file.is_empty() {
        // Find the next function's start line to bound the range
        let next_func_line = graph.nodes.iter()
            .filter(|n| {
                n.kind == NodeKind::Symbol as u8
                    && n.file != u32::MAX
                    && graph.strings.get(n.file) == node_file
                    && n.line > node_line
            })
            .map(|n| n.line)
            .min()
            .unwrap_or(u32::MAX);

        for call in &network_calls {
            if call.file == node_file && call.line >= node_line && call.line < next_func_line {
                let chain = crate::indexing::format_address_chain(&call.address_source);
                let kind = call.net_kind.as_str();
                provenance_lines.push(format!(
                    "\x1b[2m{}:{}\x1b[0m  \x1b[33m{}\x1b[0m  callee=\x1b[1m{}\x1b[0m  source={}",
                    call.file, call.line, kind, call.callee_fqn, chain,
                ));
            }
        }
    }

    if !provenance_lines.is_empty() {
        println!("\x1b[1mNetwork calls\x1b[0m \x1b[2m(from taint analysis)\x1b[0m:");
        for line in &provenance_lines {
            println!("  {}", line);
        }
        println!();
    }

    let upstream_paths = if show_upstream {
        find_upstream_paths(&graph, node_id, max_depth)
    } else {
        Vec::new()
    };

    let downstream_results = if show_downstream {
        let mut finder = PathFinder::new(graph.node_count());
        finder.find_all_downstream(&graph, node_id, ALL_EDGES, max_depth)
    } else {
        Vec::new()
    };

    if upstream_paths.is_empty() && downstream_results.is_empty() {
        println!("No paths found for {}", target);
        return Ok(());
    }

    if json {
        // JSON mode: combined output
        let mut output = serde_json::json!({ "target": target });
        if !upstream_paths.is_empty() {
            let up_json: Vec<serde_json::Value> = upstream_paths.iter()
                .map(|path| {
                    let hops: Vec<serde_json::Value> = path.iter()
                        .map(|&(node_idx, edge_kind)| {
                            let node = graph.node(node_idx);
                            serde_json::json!({
                                "name": graph.strings.get(node.name),
                                "file": if node.file != u32::MAX { graph.strings.get(node.file) } else { "" },
                                "line": node.line,
                                "edge": EdgeKind::from_u8(edge_kind).map(|k| format!("{:?}", k)),
                            })
                        })
                        .collect();
                    serde_json::json!({ "hops": hops })
                })
                .collect();
            output["upstream"] = serde_json::json!(up_json);
        }
        if !downstream_results.is_empty() {
            let down_json: Vec<serde_json::Value> = downstream_results.iter()
                .map(|r| {
                    let hops: Vec<serde_json::Value> = r.hops.iter()
                        .map(|h| {
                            let node = graph.node(h.node_id);
                            serde_json::json!({
                                "name": graph.strings.get(node.name),
                                "file": if node.file != u32::MAX { graph.strings.get(node.file) } else { "" },
                                "line": node.line,
                                "edge": h.edge_kind_to_next.and_then(EdgeKind::from_u8).map(|k| format!("{:?}", k)),
                            })
                        })
                        .collect();
                    serde_json::json!({ "hops": hops })
                })
                .collect();
            output["downstream"] = serde_json::json!(down_json);
        }
        println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
        return Ok(());
    }

    // Pretty text output — use neutral directional labels that work
    // regardless of whether the target is an env var, function, or service
    if !upstream_paths.is_empty() {
        println!("\x1b[1mPaths reaching {}\x1b[0m \x1b[2m({} path(s))\x1b[0m:", target, upstream_paths.len());
        for (i, path) in upstream_paths.iter().enumerate() {
            if i > 0 {
                println!();
            }
            print_upstream_path(&graph, i + 1, path);
        }
    }

    if !upstream_paths.is_empty() && !downstream_results.is_empty() {
        println!();
    }

    if !downstream_results.is_empty() {
        println!("\x1b[1mPaths from {}\x1b[0m \x1b[2m({} path(s))\x1b[0m:", target, downstream_results.len());
        for (i, result) in downstream_results.iter().enumerate() {
            if i > 0 {
                println!();
            }
            print_path(&graph, i + 1, &result.hops);
        }
    }

    Ok(())
}

/// Resolve a target string to one or more node IDs.
/// Supports glob patterns for env: prefix (e.g. env:*, env:AZURE_*).
fn resolve_targets(graph: &CsrGraph, target: &str) -> Vec<(u32, String)> {
    // env:PATTERN — find Resource nodes matching the pattern that look like env var names
    if let Some(pattern) = target.strip_prefix("env:") {
        let matches: Vec<(u32, String)> = graph
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                n.kind == NodeKind::Resource as u8
                    && {
                        let name = graph.strings.get(n.name);
                        glob_match(pattern, name) && looks_like_env_var(name)
                    }
            })
            .map(|(i, n)| (i as u32, graph.strings.get(n.name).to_string()))
            .collect();
        return matches;
    }

    // call:file:FuncName — find Symbol in that file
    if let Some(rest) = target.strip_prefix("call:") {
        if let Some(colon_pos) = rest.rfind(':') {
            let file_part = &rest[..colon_pos];
            let func_part = &rest[colon_pos + 1..];
            let matches: Vec<(u32, String)> = graph
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| {
                    n.kind == NodeKind::Symbol as u8
                        && glob_match(func_part, graph.strings.get(n.name))
                        && graph.strings.get(n.file).ends_with(file_part)
                })
                .map(|(i, n)| (i as u32, graph.strings.get(n.name).to_string()))
                .collect();
            return matches;
        }
    }

    // file:LINE — find the enclosing function at that file and line
    if let Some(colon_pos) = target.rfind(':') {
        if let Ok(line) = target[colon_pos + 1..].parse::<u32>() {
            let file_part = &target[..colon_pos];
            // Find the closest Symbol node at or before this line in the file
            let mut best: Option<(u32, String, u32)> = None; // (idx, name, distance)
            for (i, n) in graph.nodes.iter().enumerate() {
                if n.kind != NodeKind::Symbol as u8 {
                    continue;
                }
                let nf = graph.strings.get(n.file);
                if !nf.ends_with(file_part) {
                    continue;
                }
                if n.line <= line {
                    let dist = line - n.line;
                    if best.is_none() || dist < best.as_ref().unwrap().2 {
                        best = Some((i as u32, graph.strings.get(n.name).to_string(), dist));
                    }
                }
            }
            if let Some((idx, name, _)) = best {
                return vec![(idx, name)];
            }
        }
    }

    // Plain symbol name — exact match on graph nodes
    let exact: Vec<(u32, String)> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| graph.strings.get(n.name) == target)
        .map(|(i, n)| (i as u32, graph.strings.get(n.name).to_string()))
        .collect();
    if !exact.is_empty() {
        return exact;
    }

    // Fallback: search network.json callees for external library calls (e.g. pgxpool.New)
    // and resolve to the enclosing function that makes the call
    let network_path = std::path::Path::new(".cx/graph/network.json");
    if network_path.exists() {
        if let Ok(content) = std::fs::read_to_string(network_path) {
            if let Ok(calls) = serde_json::from_str::<Vec<cx_extractors::taint::ResolvedNetworkCall>>(&content) {
                for call in &calls {
                    if call.callee_fqn == target || call.callee_fqn.ends_with(target) {
                        // Find the enclosing function node at call.file:call.line
                        if let Some(resolved) = find_enclosing_symbol(graph, &call.file, call.line) {
                            return vec![resolved];
                        }
                    }
                }

                // Search network.json address_source for env var / config key references
                // (e.g. MEGATTS_URL defined in Helm values but read via os.Getenv in code)
                for call in &calls {
                    if mentions_target(&call.address_source, target) {
                        if let Some(resolved) = find_enclosing_symbol(graph, &call.file, call.line) {
                            return vec![resolved];
                        }
                    }
                }
            }
        }
    }

    Vec::new()
}

/// Simple glob matching: supports * as wildcard.
fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == text;
    }
    // Split on * and check that parts appear in order
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut pos = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if let Some(found) = text[pos..].find(part) {
            if i == 0 && found != 0 {
                return false; // pattern doesn't start with *, must match beginning
            }
            pos += found + part.len();
        } else {
            return false;
        }
    }
    // If pattern doesn't end with *, text must be fully consumed
    if !pattern.ends_with('*') && pos != text.len() {
        return false;
    }
    true
}

/// Find the closest Symbol node at or before `line` in `file`. Returns (node_idx, name).
fn find_enclosing_symbol(graph: &CsrGraph, file: &str, line: u32) -> Option<(u32, String)> {
    let mut best: Option<(u32, String, u32)> = None;
    for (i, n) in graph.nodes.iter().enumerate() {
        if n.kind != NodeKind::Symbol as u8 {
            continue;
        }
        let nf = graph.strings.get(n.file);
        if nf != file {
            continue;
        }
        if n.line <= line {
            let dist = line - n.line;
            if best.is_none() || dist < best.as_ref().unwrap().2 {
                best = Some((i as u32, graph.strings.get(n.name).to_string(), dist));
            }
        }
    }
    best.map(|(idx, name, _)| (idx, name))
}

/// Heuristic: does this string look like an environment variable name?
/// Must be uppercase with underscores, no lowercase letters or dots.
fn looks_like_env_var(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && name.contains('_')
}

/// Print a compact one-line summary for a node — used for multi-target (glob) output.
/// Shows: name, file:line, immediate edges (who reads it, what it resolves to).
fn print_compact_summary(
    graph: &CsrGraph,
    node_id: u32,
    name: &str,
    network_calls: &[cx_extractors::taint::ResolvedNetworkCall],
) {
    let node = graph.node(node_id);
    let file = if node.file != u32::MAX {
        graph.strings.get(node.file)
    } else {
        ""
    };
    let loc = if !file.is_empty() && node.line > 0 {
        format!("{}:{}", file, node.line)
    } else {
        String::new()
    };

    // Find immediate edges: who Configures this (reads it), what it Resolves to
    let mut readers = Vec::new();
    let mut resolves_to = Vec::new();
    for edge in graph.rev_edges_for(node_id) {
        if let Some(kind) = cx_core::graph::edges::EdgeKind::from_u8(edge.kind) {
            let src_node = graph.node(edge.target); // rev edge: target is the original source
            let src_name = graph.strings.get(src_node.name);
            match kind {
                cx_core::graph::edges::EdgeKind::Configures => {
                    readers.push(src_name.to_string());
                }
                cx_core::graph::edges::EdgeKind::Resolves => {
                    resolves_to.push(src_name.to_string());
                }
                _ => {}
            }
        }
    }
    // Also check forward Resolves edges
    for edge in graph.edges_for(node_id) {
        if let Some(cx_core::graph::edges::EdgeKind::Resolves) =
            cx_core::graph::edges::EdgeKind::from_u8(edge.kind)
        {
            let tgt_node = graph.node(edge.target);
            resolves_to.push(graph.strings.get(tgt_node.name).to_string());
        }
    }

    // Find provenance from network.json
    let provenance: Vec<String> = network_calls
        .iter()
        .filter(|c| mentions_target(&c.address_source, name))
        .map(|c| crate::indexing::format_address_chain(&c.address_source))
        .collect();

    // Print: env var name + location
    print!("\x1b[1;33m{}\x1b[0m", name);
    if !loc.is_empty() {
        print!("  \x1b[2m{}\x1b[0m", loc);
    }
    println!();

    // Print readers
    if !readers.is_empty() {
        readers.dedup();
        println!("  \x1b[2mread by:\x1b[0m  {}", readers.join(", "));
    }

    // Print resolves
    if !resolves_to.is_empty() {
        resolves_to.dedup();
        println!("  \x1b[2mresolves:\x1b[0m {}", resolves_to.join(", "));
    }

    // Print provenance chains (deduped)
    if !provenance.is_empty() {
        let mut unique: Vec<&String> = provenance.iter().collect();
        unique.dedup();
        for chain in unique.iter().take(3) {
            println!("  \x1b[2mchain:\x1b[0m    {}", chain);
        }
    }
}

/// Simple fuzzy matching: find symbols whose name contains the target substring.
fn fuzzy_match(graph: &CsrGraph, target: &str, limit: usize) -> Vec<(String, String, u32)> {
    let lower = target.to_lowercase();
    let mut matches: Vec<(String, String, u32)> = graph
        .nodes
        .iter()
        .filter(|n| {
            let name = graph.strings.get(n.name).to_lowercase();
            name.contains(&lower)
        })
        .take(limit)
        .map(|n| {
            let name = graph.strings.get(n.name).to_string();
            let file = if n.file != u32::MAX {
                graph.strings.get(n.file).to_string()
            } else {
                String::new()
            };
            (name, file, n.line)
        })
        .collect();
    matches.truncate(limit);
    matches
}

fn print_path(graph: &CsrGraph, idx: usize, hops: &[cx_core::query::path::Hop]) {
    println!("Path {}:", idx);
    for hop in hops {
        let node = graph.node(hop.node_id);
        let name = graph.strings.get(node.name);
        let file = if node.file != u32::MAX {
            graph.strings.get(node.file)
        } else {
            ""
        };
        let edge_str = hop
            .edge_kind_to_next
            .and_then(EdgeKind::from_u8)
            .map(|k| format!(" --{:?}-->", k))
            .unwrap_or_default();
        if !file.is_empty() && node.line > 0 {
            println!("  {} ({}:{}) {}", name, file, node.line, edge_str);
        } else {
            println!("  {} {}", name, edge_str);
        }
    }
}

fn print_upstream_path(graph: &CsrGraph, idx: usize, path: &[(u32, u8)]) {
    println!("Path {}:", idx);
    for (i, &(node_idx, edge_kind)) in path.iter().enumerate() {
        let node = graph.node(node_idx);
        let name = graph.strings.get(node.name);
        let file = if node.file != u32::MAX {
            graph.strings.get(node.file)
        } else {
            ""
        };
        let edge_str = if i + 1 < path.len() {
            EdgeKind::from_u8(edge_kind)
                .map(|k| format!(" <--{:?}--", k))
                .unwrap_or_default()
        } else {
            String::new()
        };
        if !file.is_empty() && node.line > 0 {
            println!("  {} ({}:{}){}", name, file, node.line, edge_str);
        } else {
            println!("  {}{}", name, edge_str);
        }
    }
}

/// Find upstream paths via reverse edges.
fn find_upstream_paths(graph: &CsrGraph, start: u32, max_depth: u32) -> Vec<Vec<(u32, u8)>> {
    use cx_core::graph::bitvec::BitVec;

    let mut visited = BitVec::new(graph.node_count());
    visited.set(start);

    let mut current = vec![start];
    let mut next = Vec::new();
    let mut parent: Vec<(u32, u8)> = vec![(u32::MAX, 0); graph.node_count() as usize];
    parent[start as usize] = (start, 0);

    let mut interesting_sources = Vec::new();

    for _depth in 0..max_depth {
        if current.is_empty() {
            break;
        }
        for &node in &current {
            for edge in graph.rev_edges_for(node) {
                if (1u16 << edge.kind) & ALL_EDGES == 0 {
                    continue;
                }
                let src = edge.target;
                if visited.test(src) {
                    continue;
                }
                visited.set(src);
                parent[src as usize] = (node, edge.kind);
                next.push(src);

                let src_node = graph.node(src);
                let start_node = graph.node(start);
                let is_cross_repo = src_node.repo != start_node.repo;
                let is_entry = src_node.kind == NodeKind::Deployable as u8
                    || src_node.kind == NodeKind::Symbol as u8
                        && graph.rev_edges_for(src).is_empty();

                if is_cross_repo || is_entry {
                    interesting_sources.push(src);
                }
            }
        }
        std::mem::swap(&mut current, &mut next);
        next.clear();
    }

    if interesting_sources.is_empty() {
        for idx in 0..graph.node_count() {
            if !visited.test(idx) || idx == start {
                continue;
            }
            let has_unvisited_upstream = graph
                .rev_edges_for(idx)
                .iter()
                .any(|e| (1u16 << e.kind) & ALL_EDGES != 0 && !visited.test(e.target));
            if !has_unvisited_upstream {
                interesting_sources.push(idx);
            }
        }
    }
    if interesting_sources.is_empty() {
        for &node in &current {
            interesting_sources.push(node);
        }
    }

    let mut paths = Vec::new();
    for &src in &interesting_sources {
        let mut path = Vec::new();
        let mut cur = src;
        let mut steps = 0;
        while cur != start && steps < max_depth {
            let (pred, kind) = parent[cur as usize];
            if pred == u32::MAX {
                break;
            }
            path.push((cur, kind));
            cur = pred;
            steps += 1;
        }
        path.push((start, 0));
        if path.len() > 1 {
            paths.push(path);
        }
    }

    // Deduplicate: remove paths that are a suffix of a longer path
    paths.sort_by(|a, b| b.len().cmp(&a.len())); // longest first
    let mut deduped: Vec<Vec<(u32, u8)>> = Vec::new();
    'outer: for path in &paths {
        let path_nodes: Vec<u32> = path.iter().map(|&(n, _)| n).collect();
        for existing in &deduped {
            let existing_nodes: Vec<u32> = existing.iter().map(|&(n, _)| n).collect();
            // Check if path_nodes is a suffix of existing_nodes
            if path_nodes.len() < existing_nodes.len()
                && existing_nodes.ends_with(&path_nodes)
            {
                continue 'outer;
            }
        }
        deduped.push(path.clone());
    }

    deduped.truncate(20);
    deduped
}

/// Load network calls from .cx/graph/network.json.
fn load_network_calls(root: &Path) -> Vec<cx_extractors::taint::ResolvedNetworkCall> {
    super::network::load_network_json(root)
}

/// Find provenance chains from network.json that reference the given target name.
/// Returns formatted lines showing how network calls use this env var / symbol.
fn find_provenance(
    target: &str,
    calls: &[cx_extractors::taint::ResolvedNetworkCall],
) -> Vec<String> {
    let mut lines = Vec::new();

    for call in calls {
        if mentions_target(&call.address_source, target) {
            let chain = crate::indexing::format_address_chain(&call.address_source);
            let kind = call.net_kind.as_str();
            lines.push(format!(
                "\x1b[2m{}:{}\x1b[0m  \x1b[33m{}\x1b[0m  {}",
                call.file, call.line, kind, chain,
            ));
        }
    }

    lines.dedup();
    lines
}

/// Check if an AddressSource tree mentions a target name (env var, field, etc.)
fn mentions_target(src: &cx_extractors::taint::AddressSource, target: &str) -> bool {
    use cx_extractors::taint::AddressSource;
    match src {
        AddressSource::EnvVar { var_name, .. } => var_name == target,
        AddressSource::ConfigKey { key, .. } => key == target,
        AddressSource::Flag { flag_name, .. } => flag_name == target,
        AddressSource::Parameter { func, caller_sources, .. } => {
            func == target || caller_sources.iter().any(|s| mentions_target(s, target))
        }
        AddressSource::FieldAccess { type_name, field, assignment_sources, .. } => {
            let full = format!("{}.{}", type_name, field);
            full == target || field == target
                || assignment_sources.iter().any(|s| mentions_target(s, target))
        }
        AddressSource::Concat { parts } => parts.iter().any(|p| mentions_target(p, target)),
        AddressSource::Literal { value } => value == target,
        AddressSource::ServiceDiscovery { service_name, .. } => service_name == target,
        AddressSource::Dynamic { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn trace_downstream() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\n\nfunc a() { b() }\nfunc b() { c() }\nfunc c() {}\n",
        )
        .unwrap();
        super::super::build::run(dir.path(), &[], false, false).unwrap();

        let result = run(dir.path(), "a", false, true, 10, false);
        assert!(result.is_ok());
    }

    #[test]
    fn trace_upstream() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\n\nfunc a() { b() }\nfunc b() {}\n",
        )
        .unwrap();
        super::super::build::run(dir.path(), &[], false, false).unwrap();

        let result = run(dir.path(), "b", true, false, 10, false);
        assert!(result.is_ok());
    }

    #[test]
    fn trace_not_found_shows_suggestions() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();
        super::super::build::run(dir.path(), &[], false, false).unwrap();

        let result = run(dir.path(), "nonexistent", false, false, 10, false);
        assert!(result.is_ok());
    }
}
