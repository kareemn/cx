use anyhow::Result;
use cx_core::graph::csr::CsrGraph;
use cx_core::graph::kind_index::KindIndex;
use cx_core::graph::nodes::NodeKind;
use std::path::Path;

/// Run `cx context` — print service structure from the indexed graph.
pub fn run(root: &Path) -> Result<()> {
    let graph = super::init::load_graph(root)?;
    let output = build_context(&graph);
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Build the context summary from a graph.
pub fn build_context(graph: &CsrGraph) -> serde_json::Value {
    let kind_idx = KindIndex::build(graph);

    let deployables = collect_kind_names(graph, &kind_idx, NodeKind::Deployable);
    let modules = collect_kind_names(graph, &kind_idx, NodeKind::Module);
    let endpoints = collect_kind_names(graph, &kind_idx, NodeKind::Endpoint);
    let symbols = collect_kind_names(graph, &kind_idx, NodeKind::Symbol);
    let resources = collect_kind_names(graph, &kind_idx, NodeKind::Resource);

    serde_json::json!({
        "summary": {
            "total_nodes": graph.node_count(),
            "total_edges": graph.edge_count(),
            "deployables": deployables.len(),
            "modules": modules.len(),
            "endpoints": endpoints.len(),
            "symbols": symbols.len(),
            "resources": resources.len(),
        },
        "deployables": deployables,
        "modules": modules,
        "endpoints": endpoints,
        "symbols": symbols,
        "resources": resources,
    })
}

fn collect_kind_names(graph: &CsrGraph, kind_idx: &KindIndex, kind: NodeKind) -> Vec<serde_json::Value> {
    let nodes = kind_idx.nodes_of_kind(kind, &graph.nodes);
    nodes
        .iter()
        .map(|n| {
            let name = graph.strings.get(n.name);
            let file = if n.file != u32::MAX {
                Some(graph.strings.get(n.file))
            } else {
                None
            };
            serde_json::json!({
                "name": name,
                "file": file,
                "line": if n.line > 0 { Some(n.line) } else { None },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn context_shows_symbols() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("main.go"),
            r#"package main

import "fmt"

type Server struct{}

func main() {
    fmt.Println("hello")
}

func helper() {}
"#,
        )
        .unwrap();

        super::super::init::run(dir.path(), false).unwrap();
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        let ctx = build_context(&graph);

        // Should have symbols
        let symbols = ctx["symbols"].as_array().unwrap();
        assert!(!symbols.is_empty(), "should have symbols");

        let symbol_names: Vec<&str> = symbols
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();

        assert!(symbol_names.contains(&"main"), "should find main");
        assert!(symbol_names.contains(&"helper"), "should find helper");
        assert!(symbol_names.contains(&"Server"), "should find Server");

        // Summary should have counts
        let summary = &ctx["summary"];
        assert!(summary["total_nodes"].as_u64().unwrap() > 0);
        assert!(summary["symbols"].as_u64().unwrap() > 0);
    }

    #[test]
    fn context_shows_modules_and_deployables() {
        let dir = tempfile::tempdir().unwrap();

        // package main → Deployable
        fs::write(
            dir.path().join("main.go"),
            "package main\n\nfunc main() {}\n",
        )
        .unwrap();

        // package server → Module
        fs::write(
            dir.path().join("server.go"),
            "package server\n\nfunc Start() {}\n",
        )
        .unwrap();

        super::super::init::run(dir.path(), false).unwrap();
        let graph = super::super::init::load_graph(dir.path()).unwrap();
        let ctx = build_context(&graph);

        let modules = ctx["modules"].as_array().unwrap();
        let module_names: Vec<&str> = modules.iter().map(|m| m["name"].as_str().unwrap()).collect();
        assert!(module_names.contains(&"server"), "should have server module");

        let deployables = ctx["deployables"].as_array().unwrap();
        assert!(!deployables.is_empty(), "should have deployables from package main");
    }
}
