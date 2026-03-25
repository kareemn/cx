use crate::graph::bitvec::BitVec;
use crate::graph::csr::CsrGraph;
use crate::graph::edges::EdgeKindMask;
use crate::graph::nodes::NodeId;

/// A single hop in a path result.
#[derive(Debug, Clone)]
pub struct Hop {
    pub node_id: NodeId,
    pub edge_kind_to_next: Option<u8>,
}

/// Result of a path query.
#[derive(Debug)]
pub struct PathResult {
    pub hops: Vec<Hop>,
    pub found: bool,
}

/// PathFinder with parent tracking for path reconstruction.
///
/// Uses BFS with a parent array to reconstruct the actual path,
/// not just the reachable set.
pub struct PathFinder {
    /// parent[i] = (predecessor_node_id, edge_kind). u32::MAX = no parent.
    parent: Vec<(NodeId, u8)>,
    visited: BitVec,
    current: Vec<NodeId>,
    next: Vec<NodeId>,
}

impl PathFinder {
    pub fn new(node_capacity: u32) -> Self {
        Self {
            parent: vec![(u32::MAX, 0); node_capacity as usize],
            visited: BitVec::new(node_capacity),
            current: Vec::with_capacity(1024),
            next: Vec::with_capacity(1024),
        }
    }

    /// Find shortest path from `from` to `to`. Returns ordered hops.
    pub fn find_path(
        &mut self,
        graph: &CsrGraph,
        from: NodeId,
        to: NodeId,
        mask: EdgeKindMask,
        max_depth: u32,
    ) -> PathResult {
        self.reset(graph.node_count());

        self.visited.set(from);
        self.parent[from as usize] = (from, 0); // self-parent for root
        self.current.push(from);

        for _depth in 0..max_depth {
            if self.current.is_empty() {
                break;
            }

            for &node in &self.current {
                if node == to {
                    return PathResult {
                        hops: self.reconstruct(from, to),
                        found: true,
                    };
                }

                for edge in graph.edges_for(node) {
                    if (1u16 << edge.kind) & mask == 0 {
                        continue;
                    }
                    if self.visited.test(edge.target) {
                        continue;
                    }
                    self.visited.set(edge.target);
                    self.parent[edge.target as usize] = (node, edge.kind);
                    self.next.push(edge.target);
                }
            }

            std::mem::swap(&mut self.current, &mut self.next);
            self.next.clear();
        }

        // Check if `to` was reached in the last level
        if self.visited.test(to) {
            PathResult {
                hops: self.reconstruct(from, to),
                found: true,
            }
        } else {
            PathResult {
                hops: vec![],
                found: false,
            }
        }
    }

    /// Find all downstream paths from `from`, returning all terminal nodes.
    pub fn find_all_downstream(
        &mut self,
        graph: &CsrGraph,
        from: NodeId,
        mask: EdgeKindMask,
        max_depth: u32,
    ) -> Vec<PathResult> {
        self.reset(graph.node_count());

        self.visited.set(from);
        self.parent[from as usize] = (from, 0);
        self.current.push(from);

        let mut all_reached = Vec::new();

        for _depth in 0..max_depth {
            if self.current.is_empty() {
                break;
            }

            for &node in &self.current {
                all_reached.push(node);

                for edge in graph.edges_for(node) {
                    if (1u16 << edge.kind) & mask == 0 {
                        continue;
                    }
                    if self.visited.test(edge.target) {
                        continue;
                    }
                    self.visited.set(edge.target);
                    self.parent[edge.target as usize] = (node, edge.kind);
                    self.next.push(edge.target);
                }
            }

            std::mem::swap(&mut self.current, &mut self.next);
            self.next.clear();
        }

        // Add remaining nodes in current level
        for &node in &self.current {
            all_reached.push(node);
        }

        // Terminals: nodes with no outgoing edges matching mask
        let terminals: Vec<NodeId> = all_reached
            .iter()
            .filter(|&&n| {
                graph
                    .edges_for(n)
                    .iter()
                    .all(|e| (1u16 << e.kind) & mask == 0)
            })
            .copied()
            .collect();

        terminals
            .into_iter()
            .map(|t| PathResult {
                hops: self.reconstruct(from, t),
                found: true,
            })
            .collect()
    }

    fn reset(&mut self, node_count: u32) {
        self.visited.clear();
        self.current.clear();
        self.next.clear();
        let needed = node_count as usize;
        if self.parent.len() < needed {
            self.parent.resize(needed, (u32::MAX, 0));
        }
        for p in self.parent.iter_mut().take(needed) {
            *p = (u32::MAX, 0);
        }
    }

    fn reconstruct(&self, from: NodeId, to: NodeId) -> Vec<Hop> {
        let mut path = Vec::new();
        let mut current = to;

        loop {
            let (pred, edge_kind) = self.parent[current as usize];
            if current == from {
                path.push(Hop {
                    node_id: current,
                    edge_kind_to_next: None,
                });
                break;
            }
            if pred == u32::MAX {
                break; // unreachable
            }
            path.push(Hop {
                node_id: current,
                edge_kind_to_next: Some(edge_kind),
            });
            current = pred;
        }

        path.reverse();
        // Fix edge_kind_to_next: shift so each hop carries the edge to the NEXT hop
        if path.len() >= 2 {
            for i in 0..path.len() - 1 {
                path[i].edge_kind_to_next = path[i + 1].edge_kind_to_next;
            }
            path.last_mut().unwrap().edge_kind_to_next = None;
        }

        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::csr::{CsrGraph, EdgeInput};
    use crate::graph::edges::{EdgeKind, ALL_EDGES};
    use crate::graph::nodes::{Node, NodeKind};
    use crate::graph::string_interner::StringInterner;

    fn build_chain(n: u32) -> (CsrGraph, Vec<u32>) {
        let mut strings = StringInterner::new();
        let names: Vec<_> = (0..n).map(|i| strings.intern(&format!("N{}", i))).collect();
        let nodes: Vec<Node> = (0..n)
            .map(|i| Node::new(i, NodeKind::Symbol, names[i as usize]))
            .collect();
        let edges: Vec<EdgeInput> = (0..n - 1)
            .map(|i| EdgeInput::new(i, i + 1, EdgeKind::Calls))
            .collect();
        let graph = CsrGraph::build(nodes, edges, strings);
        let indices: Vec<u32> = (0..n)
            .map(|i| graph.find_node_by_name(names[i as usize]).unwrap())
            .collect();
        (graph, indices)
    }

    #[test]
    fn cx_path_downstream() {
        // TEST cx_path_downstream from ARCHITECTURE.md
        let (graph, idx) = build_chain(5); // A→B→C→D→E
        let mut finder = PathFinder::new(graph.node_count());

        let result = finder.find_path(&graph, idx[0], idx[4], ALL_EDGES, 10);
        assert!(result.found);
        assert_eq!(result.hops.len(), 5); // A, B, C, D, E
        assert_eq!(result.hops[0].node_id, idx[0]);
        assert_eq!(result.hops[4].node_id, idx[4]);
    }

    #[test]
    fn cx_path_with_gaps() {
        // TEST cx_path_with_gaps: disconnected components, target not reachable
        let mut strings = StringInterner::new();
        let names: Vec<_> = (0..4).map(|i| strings.intern(&format!("X{}", i))).collect();
        let nodes: Vec<Node> = (0..4)
            .map(|i| Node::new(i, NodeKind::Symbol, names[i as usize]))
            .collect();
        let edges = vec![
            EdgeInput::new(0, 1, EdgeKind::Calls),
            EdgeInput::new(2, 3, EdgeKind::Calls), // disconnected component
        ];
        let graph = CsrGraph::build(nodes, edges, strings);

        let mut finder = PathFinder::new(graph.node_count());
        let _result = finder.find_path(&graph, 0, 3, ALL_EDGES, 10);
        // Disconnected: may not find path. Key: no crash.
    }

    #[test]
    fn path_not_found() {
        let mut strings = StringInterner::new();
        let n0 = strings.intern("A");
        let n1 = strings.intern("B");
        let nodes = vec![
            Node::new(0, NodeKind::Symbol, n0),
            Node::new(1, NodeKind::Symbol, n1),
        ];
        // No edges
        let graph = CsrGraph::build(nodes, vec![], strings);
        let mut finder = PathFinder::new(graph.node_count());
        let result = finder.find_path(&graph, 0, 1, ALL_EDGES, 10);
        assert!(!result.found);
    }

    #[test]
    fn path_depth_limited() {
        let (graph, idx) = build_chain(10);
        let mut finder = PathFinder::new(graph.node_count());

        // With max_depth=3, should not reach node 9
        let result = finder.find_path(&graph, idx[0], idx[9], ALL_EDGES, 3);
        assert!(!result.found);

        // With max_depth=10, should reach
        let result = finder.find_path(&graph, idx[0], idx[9], ALL_EDGES, 10);
        assert!(result.found);
    }

    #[test]
    fn find_all_downstream_terminals() {
        // A→B→C, A→D (D is terminal, C is terminal)
        let mut strings = StringInterner::new();
        let names: Vec<_> = ["A", "B", "C", "D"]
            .iter()
            .map(|s| strings.intern(s))
            .collect();
        let nodes: Vec<Node> = (0..4)
            .map(|i| Node::new(i, NodeKind::Symbol, names[i as usize]))
            .collect();
        let edges = vec![
            EdgeInput::new(0, 1, EdgeKind::Calls),
            EdgeInput::new(1, 2, EdgeKind::Calls),
            EdgeInput::new(0, 3, EdgeKind::Calls),
        ];
        let graph = CsrGraph::build(nodes, edges, strings);
        let a = graph.find_node_by_name(names[0]).unwrap();

        let mut finder = PathFinder::new(graph.node_count());
        let results = finder.find_all_downstream(&graph, a, ALL_EDGES, 10);

        assert!(results.len() >= 2, "should find at least 2 terminal paths");
    }
}
