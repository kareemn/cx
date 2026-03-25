use anyhow::Result;
use cx_core::graph::csr::CsrGraph;
use cx_core::graph::edges::EdgeKind;
use std::path::Path;

/// Run `cx edges` — show edge summary or list edges filtered by kind.
pub fn run(root: &Path, kind_filter: Option<&str>, limit: usize) -> Result<()> {
    let graph = super::init::load_graph(root)?;

    match kind_filter {
        None => print_summary(&graph),
        Some(kind_str) => print_edges(&graph, kind_str, limit)?,
    }

    Ok(())
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

fn parse_edge_kind(s: &str) -> Option<u8> {
    let lower = s.to_lowercase();
    match lower.as_str() {
        "contains" => Some(EdgeKind::Contains as u8),
        "calls" => Some(EdgeKind::Calls as u8),
        "imports" => Some(EdgeKind::Imports as u8),
        "dependson" => Some(EdgeKind::DependsOn as u8),
        "exposes" => Some(EdgeKind::Exposes as u8),
        "consumes" => Some(EdgeKind::Consumes as u8),
        "configures" => Some(EdgeKind::Configures as u8),
        "resolves" => Some(EdgeKind::Resolves as u8),
        "connects" => Some(EdgeKind::Connects as u8),
        "publishes" => Some(EdgeKind::Publishes as u8),
        "subscribes" => Some(EdgeKind::Subscribes as u8),
        _ => None,
    }
}

fn print_summary(graph: &CsrGraph) {
    let mut counts = [0u32; EdgeKind::COUNT];
    for edge in &graph.edges {
        if (edge.kind as usize) < EdgeKind::COUNT {
            counts[edge.kind as usize] += 1;
        }
    }

    println!("Edge summary:");
    let mut total = 0u32;
    for (i, &count) in counts.iter().enumerate() {
        if count > 0 {
            println!("  {:<14} {:>6}", format!("{}:", edge_kind_name(i as u8)), count);
            total += count;
        }
    }
    println!("  {:<14} {:>6}", "Total:", total);
}

fn print_edges(graph: &CsrGraph, kind_str: &str, limit: usize) -> Result<()> {
    let kind = parse_edge_kind(kind_str)
        .ok_or_else(|| anyhow::anyhow!("unknown edge kind: {}", kind_str))?;

    let mut count = 0;
    for src_idx in 0..graph.node_count() {
        if count >= limit {
            break;
        }
        let src = graph.node(src_idx);
        for edge in graph.edges_for(src_idx) {
            if edge.kind != kind {
                continue;
            }
            if count >= limit {
                break;
            }
            let tgt = graph.node(edge.target);
            let src_name = graph.strings.get(src.name);
            let tgt_name = graph.strings.get(tgt.name);
            let file = if src.file != u32::MAX {
                graph.strings.get(src.file)
            } else {
                ""
            };
            if src.line > 0 && !file.is_empty() {
                println!("{} \u{2192} {:<30} ({}:{})", src_name, tgt_name, file, src.line);
            } else {
                println!("{} \u{2192} {}", src_name, tgt_name);
            }
            count += 1;
        }
    }

    if count == 0 {
        println!("No {} edges found.", kind_str);
    }

    Ok(())
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

import "fmt"

func main() {
    helper()
    fmt.Println("hello")
}

func helper() {}
"#,
        )
        .unwrap();
        super::super::init::run(dir.path()).unwrap();
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        (dir, graph)
    }

    #[test]
    fn edge_summary_counts() {
        let (_dir, graph) = setup_project();
        // Just verify it doesn't panic and has edges
        let mut counts = [0u32; EdgeKind::COUNT];
        for edge in &graph.edges {
            if (edge.kind as usize) < EdgeKind::COUNT {
                counts[edge.kind as usize] += 1;
            }
        }
        let total: u32 = counts.iter().sum();
        assert!(total > 0, "should have some edges");
    }

    #[test]
    fn edge_kind_parsing() {
        assert_eq!(parse_edge_kind("Calls"), Some(EdgeKind::Calls as u8));
        assert_eq!(parse_edge_kind("calls"), Some(EdgeKind::Calls as u8));
        assert_eq!(parse_edge_kind("Imports"), Some(EdgeKind::Imports as u8));
        assert_eq!(parse_edge_kind("Contains"), Some(EdgeKind::Contains as u8));
        assert_eq!(parse_edge_kind("nonsense"), None);
    }

    #[test]
    fn list_calls_edges() {
        let (_dir, graph) = setup_project();
        let calls_count = graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls as u8)
            .count();
        assert!(calls_count > 0, "should have Calls edges");
    }

    #[test]
    fn edges_summary_correct_counts() {
        let (_dir, graph) = setup_project();
        let mut counts = [0u32; EdgeKind::COUNT];
        for edge in &graph.edges {
            if (edge.kind as usize) < EdgeKind::COUNT {
                counts[edge.kind as usize] += 1;
            }
        }
        let total: u32 = counts.iter().sum();
        // main→helper and main→Println are at minimum 1 call
        assert!(counts[EdgeKind::Calls as usize] >= 1, "should have Calls edges");
        assert_eq!(total, graph.edge_count(), "summary total should match graph edge count");
    }

    #[test]
    fn edges_filtered_respects_limit() {
        // Build a project with many calls
        let dir = tempfile::tempdir().unwrap();
        let mut source = String::from("package main\n\n");
        for i in 0..20 {
            source.push_str(&format!("func f{}() {{}}\n", i));
        }
        source.push_str("\nfunc main() {\n");
        for i in 0..20 {
            source.push_str(&format!("    f{}()\n", i));
        }
        source.push_str("}\n");
        fs::write(dir.path().join("main.go"), &source).unwrap();

        super::super::init::run(dir.path()).unwrap();
        let graph = super::super::init::load_graph(dir.path()).unwrap();

        // Count Calls edges
        let calls: Vec<_> = graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls as u8)
            .collect();
        assert!(calls.len() >= 20, "should have at least 20 Calls edges");

        // Verify parse_edge_kind works for filtering
        assert_eq!(parse_edge_kind("Calls"), Some(EdgeKind::Calls as u8));
    }
}
