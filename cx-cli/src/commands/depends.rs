use anyhow::Result;
use cx_core::graph::edges::ALL_EDGES;
use cx_core::query::depends::{self, DependsDirection};
use std::path::Path;

/// Run `cx depends <symbol> [--upstream|--downstream]`.
pub fn run(root: &Path, symbol: &str, upstream: bool, max_depth: u32) -> Result<()> {
    let graph = super::init::load_graph(root)?;

    let start = graph
        .nodes
        .iter()
        .position(|n| graph.strings.get(n.name) == symbol)
        .map(|i| i as u32);

    let start = match start {
        Some(s) => s,
        None => {
            eprintln!("Symbol not found: {}", symbol);
            return Ok(());
        }
    };

    let direction = if upstream {
        DependsDirection::Upstream
    } else {
        DependsDirection::Downstream
    };

    let result = depends::depends(&graph, start, direction, ALL_EDGES, max_depth);

    if result.nodes.is_empty() {
        let dir_str = if upstream { "upstream" } else { "downstream" };
        println!("No {} dependencies for {}", dir_str, symbol);
        return Ok(());
    }

    let dir_str = if upstream {
        "Depends on (upstream)"
    } else {
        "Dependencies (downstream)"
    };
    println!("{} of {}:", dir_str, symbol);

    for &node_id in &result.nodes {
        let node = graph.node(node_id);
        let name = graph.strings.get(node.name);
        let file = if node.file != u32::MAX {
            graph.strings.get(node.file)
        } else {
            ""
        };
        if !file.is_empty() && node.line > 0 {
            println!("  {} ({}:{})", name, file, node.line);
        } else {
            println!("  {}", name);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn depends_downstream() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\n\nfunc a() { b() }\nfunc b() {}\n",
        )
        .unwrap();
        super::super::init::run(dir.path()).unwrap();
        assert!(run(dir.path(), "a", false, 10).is_ok());
    }

    #[test]
    fn depends_upstream() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\n\nfunc a() { b() }\nfunc b() {}\n",
        )
        .unwrap();
        super::super::init::run(dir.path()).unwrap();
        assert!(run(dir.path(), "b", true, 10).is_ok());
    }
}
