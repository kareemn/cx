use anyhow::Result;
use cx_core::graph::edges::ALL_EDGES;
use cx_core::query::path::PathFinder;
use std::path::Path;

/// Run `cx path --from <symbol> [--downstream|--upstream]`.
pub fn run(root: &Path, from: &str, max_depth: u32) -> Result<()> {
    let graph = super::init::load_graph(root)?;

    // Find the starting node by name
    let start = graph
        .nodes
        .iter()
        .position(|n| graph.strings.get(n.name) == from)
        .map(|i| i as u32);

    let start = match start {
        Some(s) => s,
        None => {
            eprintln!("Symbol not found: {}", from);
            return Ok(());
        }
    };

    let mut finder = PathFinder::new(graph.node_count());
    let results = finder.find_all_downstream(&graph, start, ALL_EDGES, max_depth);

    if results.is_empty() {
        println!("No downstream paths found from {}", from);
        return Ok(());
    }

    for (i, result) in results.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("Path {}:", i + 1);
        for hop in &result.hops {
            let node = graph.node(hop.node_id);
            let name = graph.strings.get(node.name);
            let file = if node.file != u32::MAX {
                graph.strings.get(node.file)
            } else {
                ""
            };
            let edge_str = hop
                .edge_kind_to_next
                .and_then(cx_core::graph::edges::EdgeKind::from_u8)
                .map(|k| format!(" --{:?}-->", k))
                .unwrap_or_default();
            if !file.is_empty() && node.line > 0 {
                println!("  {} ({}:{}) {}", name, file, node.line, edge_str);
            } else {
                println!("  {} {}", name, edge_str);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn path_finds_chain() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\n\nfunc a() { b() }\nfunc b() { c() }\nfunc c() {}\n",
        )
        .unwrap();
        super::super::init::run(dir.path()).unwrap();

        // Just verify it doesn't panic
        let result = run(dir.path(), "a", 10);
        assert!(result.is_ok());
    }

    #[test]
    fn path_symbol_not_found() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("main.go"), "package main\nfunc main() {}\n").unwrap();
        super::super::init::run(dir.path()).unwrap();

        let result = run(dir.path(), "nonexistent", 10);
        assert!(result.is_ok()); // prints message, doesn't error
    }
}
