use anyhow::Result;
use cx_core::graph::csr::CsrGraph;
use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::NodeKind;
use std::path::Path;

/// Run `cx inspect <symbol>` — show a symbol's edges.
pub fn run(root: &Path, symbol: &str) -> Result<()> {
    let graph = super::init::load_graph(root)?;
    let output = inspect_symbol(&graph, symbol);
    print!("{}", output);
    Ok(())
}

fn kind_label(kind: u8, sub_kind: u8) -> &'static str {
    match NodeKind::from_u8(kind) {
        Some(NodeKind::Symbol) => {
            if sub_kind == 1 { "Type" } else { "Function" }
        }
        Some(NodeKind::Module) => "Module",
        Some(NodeKind::Deployable) => "Deployable",
        Some(NodeKind::Endpoint) => "Endpoint",
        Some(NodeKind::Resource) => "Resource",
        Some(NodeKind::Repo) => "Repo",
        Some(NodeKind::Surface) => "Surface",
        Some(NodeKind::InfraConfig) => "InfraConfig",
        None => "Unknown",
    }
}

fn edge_kind_name(kind: u8) -> &'static str {
    match EdgeKind::from_u8(kind) {
        Some(EdgeKind::Contains) => "Contains",
        Some(EdgeKind::Calls) => "Calls",
        Some(EdgeKind::Imports) => "Imports",
        Some(EdgeKind::DependsOn) => "DependsOn",
        Some(EdgeKind::Exposes) => "Exposes",
        Some(EdgeKind::Consumes) => "Consumes",
        Some(EdgeKind::Configures) => "Configures",
        Some(EdgeKind::Resolves) => "Resolves",
        Some(EdgeKind::Connects) => "Connects",
        Some(EdgeKind::Publishes) => "Publishes",
        Some(EdgeKind::Subscribes) => "Subscribes",
        None => "Unknown",
    }
}

fn format_node(graph: &CsrGraph, idx: u32) -> String {
    let node = graph.node(idx);
    let name = graph.strings.get(node.name);
    let kind = kind_label(node.kind, node.sub_kind);
    let file = if node.file != u32::MAX {
        graph.strings.get(node.file)
    } else {
        ""
    };
    if node.line > 0 && !file.is_empty() {
        format!("{:<24} {:<12} {}:{}", name, kind, file, node.line)
    } else if !file.is_empty() {
        format!("{:<24} {:<12} {}", name, kind, file)
    } else {
        format!("{:<24} {}", name, kind)
    }
}

pub fn inspect_symbol(graph: &CsrGraph, symbol: &str) -> String {
    // Find all nodes matching the symbol name
    let matches: Vec<u32> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| graph.strings.get(n.name) == symbol)
        .map(|(i, _)| i as u32)
        .collect();

    if matches.is_empty() {
        return format!("Symbol not found: {}\n", symbol);
    }

    let mut out = String::new();

    for &node_idx in &matches {
        out.push_str(&format_node(graph, node_idx));
        out.push('\n');

        // Outgoing edges grouped by kind
        let forward = graph.edges_for(node_idx);
        let mut calls = Vec::new();
        let mut imports = Vec::new();
        let mut other = Vec::new();

        for edge in forward {
            match EdgeKind::from_u8(edge.kind) {
                Some(EdgeKind::Calls) => calls.push(edge.target),
                Some(EdgeKind::Imports) => imports.push(edge.target),
                _ => other.push((edge.kind, edge.target)),
            }
        }

        if !calls.is_empty() {
            out.push_str("\n  Calls:\n");
            for &target in &calls {
                out.push_str(&format!("    \u{2192} {}\n", format_node(graph, target)));
            }
        }

        // Incoming Calls edges (reverse index)
        let reverse = graph.rev_edges_for(node_idx);
        let callers: Vec<u32> = reverse
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls as u8)
            .map(|e| e.target) // in rev_edges, target = source of original edge
            .collect();

        if !callers.is_empty() {
            out.push_str("\n  Called by:\n");
            for &caller in &callers {
                out.push_str(&format!("    \u{2190} {}\n", format_node(graph, caller)));
            }
        }

        if !imports.is_empty() {
            out.push_str("\n  Imports: (module-level)\n");
            for &target in &imports {
                if (target as usize) < graph.nodes.len() {
                    out.push_str(&format!("    \u{2192} {}\n", graph.strings.get(graph.node(target).name)));
                }
            }
        }

        if !other.is_empty() {
            out.push_str("\n  Other edges:\n");
            for &(kind, target) in &other {
                out.push_str(&format!(
                    "    \u{2192} {} ({})\n",
                    format_node(graph, target),
                    edge_kind_name(kind),
                ));
            }
        }

        if calls.is_empty() && callers.is_empty() && imports.is_empty() && other.is_empty() {
            out.push_str("  (no edges)\n");
        }

        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            r#"package main

func main() {
    helper()
}

func helper() {
    doWork()
}

func doWork() {}
"#,
        )
        .unwrap();
        super::super::init::run(dir.path()).unwrap();
        dir
    }

    #[test]
    fn inspect_finds_symbol() {
        let dir = setup_project();
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        let output = inspect_symbol(&graph, "helper");
        assert!(output.contains("helper"), "should show helper");
        assert!(output.contains("Function"), "should show kind");
    }

    #[test]
    fn inspect_shows_outgoing_calls() {
        let dir = setup_project();
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        let output = inspect_symbol(&graph, "helper");
        // helper calls doWork
        assert!(
            output.contains("Calls:") && output.contains("doWork"),
            "should show outgoing call to doWork, got:\n{}",
            output
        );
    }

    #[test]
    fn inspect_shows_incoming_calls() {
        let dir = setup_project();
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        let output = inspect_symbol(&graph, "helper");
        // main calls helper
        assert!(
            output.contains("Called by:") && output.contains("main"),
            "should show main as caller, got:\n{}",
            output
        );
    }

    #[test]
    fn inspect_not_found() {
        let dir = setup_project();
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        let output = inspect_symbol(&graph, "nonexistent");
        assert!(output.contains("not found"));
    }

    #[test]
    fn inspect_ambiguous_same_name_different_files() {
        let dir = tempfile::tempdir().unwrap();

        // Two files each defining a function named "Run"
        fs::create_dir_all(dir.path().join("pkg/a")).unwrap();
        fs::write(
            dir.path().join("pkg/a/a.go"),
            "package a\n\nfunc Run() {}\n",
        )
        .unwrap();

        fs::create_dir_all(dir.path().join("pkg/b")).unwrap();
        fs::write(
            dir.path().join("pkg/b/b.go"),
            "package b\n\nfunc Run() {}\n",
        )
        .unwrap();

        super::super::init::run(dir.path()).unwrap();
        let graph = super::super::init::load_graph(dir.path()).unwrap();

        let output = inspect_symbol(&graph, "Run");

        // Should list both with file paths so user can distinguish
        assert!(output.contains("pkg/a/a.go"), "should show first file path");
        assert!(output.contains("pkg/b/b.go"), "should show second file path");

        // Should appear twice as separate entries
        let run_count = output.matches("Run").count();
        assert!(run_count >= 2, "should show Run at least twice, got {}", run_count);
    }
}
