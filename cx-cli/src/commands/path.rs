use anyhow::Result;
use cx_core::graph::csr::CsrGraph;
use cx_core::graph::edges::{EdgeKind, ALL_EDGES};
use cx_core::graph::nodes::NodeKind;
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

    // Downstream paths (forward edges)
    let mut finder = PathFinder::new(graph.node_count());
    let downstream = finder.find_all_downstream(&graph, start, ALL_EDGES, max_depth);

    // Upstream paths (reverse edges) — especially useful for Endpoint/Resource nodes
    // that have no outgoing edges but are targets of Exposes/Connects/Configures
    let upstream = find_upstream_paths(&graph, start, max_depth);

    let has_downstream = !downstream.is_empty();
    let has_upstream = !upstream.is_empty();

    if !has_downstream && !has_upstream {
        println!("No paths found from {}", from);
        return Ok(());
    }

    if has_downstream {
        println!("Downstream paths:");
        for (i, result) in downstream.iter().enumerate() {
            if i > 0 {
                println!();
            }
            print_path(&graph, i + 1, &result.hops);
        }
    }

    if has_upstream {
        if has_downstream {
            println!();
        }
        println!("Upstream paths (who reaches this):");
        for (i, path) in upstream.iter().enumerate() {
            if i > 0 {
                println!();
            }
            print_upstream_path(&graph, i + 1, path);
        }
    }

    Ok(())
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

/// Find upstream paths via reverse edges. Returns paths from the target node
/// back to interesting source nodes (cross-repo callers, entry points).
fn find_upstream_paths(graph: &CsrGraph, start: u32, max_depth: u32) -> Vec<Vec<(u32, u8)>> {
    use cx_core::graph::bitvec::BitVec;

    let mut visited = BitVec::new(graph.node_count());
    visited.set(start);

    let mut current = vec![start];
    let mut next = Vec::new();
    // parent[i] = (predecessor, edge_kind)
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
                // edge.target in rev_edges is the original source
                let src = edge.target;
                if visited.test(src) {
                    continue;
                }
                visited.set(src);
                parent[src as usize] = (node, edge.kind);
                next.push(src);

                // Mark interesting upstream nodes: different repo, entry points, cross-repo edges
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

    // If no interesting sources found, use all leaf sources (no incoming edges)
    if interesting_sources.is_empty() {
        for &node in &current {
            interesting_sources.push(node);
        }
    }

    // Reconstruct paths from each interesting source back to start
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

    // Limit to 20 most interesting paths
    paths.truncate(20);
    paths
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
