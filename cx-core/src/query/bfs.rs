use crate::graph::bitvec::BitVec;
use crate::graph::csr::CsrGraph;
use crate::graph::edges::EdgeKindMask;
use crate::graph::nodes::NodeId;

/// Traversal direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Follow forward (outgoing) edges.
    Downstream,
    /// Follow reverse (incoming) edges.
    Upstream,
}

/// PERFORMANCE CRITICAL: Two Vec swap is faster than VecDeque for BFS.
/// No modular arithmetic. No branch for wrap-around. Pure sequential writes and reads.
///
/// Pre-allocate once, reuse between queries via `clear` + `run`.
pub struct BfsState {
    current_level: Vec<NodeId>,
    next_level: Vec<NodeId>,
    visited: BitVec,
    result: Vec<NodeId>,
}

impl BfsState {
    /// Create a new BfsState pre-allocated for a graph with `node_capacity` nodes.
    pub fn new(node_capacity: u32) -> Self {
        Self {
            current_level: Vec::with_capacity(1024),
            next_level: Vec::with_capacity(1024),
            visited: BitVec::new(node_capacity),
            result: Vec::with_capacity(1024),
        }
    }

    /// Run BFS from seed nodes, following edges matching `mask`, up to `max_depth` hops.
    ///
    /// PERFORMANCE: Zero heap allocation during run if capacity is sufficient.
    /// Uses double-buffer swap instead of VecDeque.
    pub fn run(
        &mut self,
        graph: &CsrGraph,
        seeds: &[NodeId],
        mask: EdgeKindMask,
        max_depth: u32,
        direction: Direction,
    ) {
        self.visited.clear();
        self.result.clear();
        self.current_level.clear();
        self.next_level.clear();

        for &seed in seeds {
            if !self.visited.test(seed) {
                self.visited.set(seed);
                self.current_level.push(seed);
            }
        }

        for _depth in 0..=max_depth {
            if self.current_level.is_empty() {
                break;
            }

            for &node in &self.current_level {
                self.result.push(node);

                let edges = match direction {
                    Direction::Downstream => graph.edges_for(node),
                    Direction::Upstream => graph.rev_edges_for(node),
                };

                for edge in edges {
                    if (1u16 << edge.kind) & mask == 0 {
                        continue;
                    }
                    if self.visited.test(edge.target) {
                        continue;
                    }
                    self.visited.set(edge.target);
                    self.next_level.push(edge.target);
                }
            }

            // Swap buffers — no allocation, just pointer swap
            std::mem::swap(&mut self.current_level, &mut self.next_level);
            self.next_level.clear(); // clear does NOT deallocate
        }
    }

    /// Return the BFS result (nodes in BFS order).
    pub fn result(&self) -> &[NodeId] {
        &self.result
    }

    /// Return whether a node was visited during the last run.
    pub fn was_visited(&self, id: NodeId) -> bool {
        self.visited.test(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::csr::{CsrGraph, EdgeInput};
    use crate::graph::edges::{EdgeKind, ALL_EDGES};
    use crate::graph::nodes::{Node, NodeKind};
    use crate::graph::string_interner::StringInterner;

    fn make_linear_graph(n: u32) -> (CsrGraph, Vec<u32>) {
        // A→B→C→D→E (linear chain of n nodes)
        let mut strings = StringInterner::new();
        let names: Vec<_> = (0..n)
            .map(|i| strings.intern(&format!("N{}", i)))
            .collect();

        let nodes: Vec<Node> = (0..n)
            .map(|i| Node::new(i, NodeKind::Symbol, names[i as usize]))
            .collect();

        let edges: Vec<EdgeInput> = (0..n.saturating_sub(1))
            .map(|i| EdgeInput::new(i, i + 1, EdgeKind::Calls))
            .collect();

        let graph = CsrGraph::build(nodes, edges, strings);

        // Find remapped indices
        let mut indices = Vec::with_capacity(n as usize);
        for i in 0..n {
            indices.push(graph.find_node_by_name(names[i as usize]).unwrap());
        }

        (graph, indices)
    }

    #[test]
    fn bfs_double_buffer() {
        // TEST bfs_double_buffer from ARCHITECTURE.md:
        // Build graph: A→B→C→D→E (linear chain).
        // Run BfsState::run(seed=A, max_depth=3).
        // Result contains {A, B, C, D}. E excluded (depth 4).
        let (graph, idx) = make_linear_graph(5);

        let mut bfs = BfsState::new(graph.node_count());
        bfs.run(&graph, &[idx[0]], ALL_EDGES, 3, Direction::Downstream);

        let result = bfs.result();
        assert_eq!(result.len(), 4, "expected 4 nodes (depth 0-3)");

        // A is first
        assert_eq!(result[0], idx[0]);

        // Contains A, B, C, D
        for i in 0..4 {
            assert!(
                result.contains(&idx[i]),
                "expected node {} in result",
                i
            );
        }

        // E excluded (depth 4)
        assert!(
            !result.contains(&idx[4]),
            "E should be excluded at depth 4"
        );
    }

    #[test]
    fn bfs_downstream_full() {
        // Full traversal of A→B→C→D→E
        let (graph, idx) = make_linear_graph(5);
        let mut bfs = BfsState::new(graph.node_count());
        bfs.run(&graph, &[idx[0]], ALL_EDGES, u32::MAX, Direction::Downstream);

        assert_eq!(bfs.result().len(), 5);
    }

    #[test]
    fn bfs_upstream() {
        let (graph, idx) = make_linear_graph(5);
        let mut bfs = BfsState::new(graph.node_count());
        bfs.run(&graph, &[idx[4]], ALL_EDGES, u32::MAX, Direction::Upstream);

        // E→D→C→B→A (all 5 upstream)
        assert_eq!(bfs.result().len(), 5);
        assert_eq!(bfs.result()[0], idx[4]); // E is seed
    }

    #[test]
    fn bfs_reuse() {
        // Verify BfsState can be reused between queries
        let (graph, idx) = make_linear_graph(5);
        let mut bfs = BfsState::new(graph.node_count());

        // First query from A, depth 1
        bfs.run(&graph, &[idx[0]], ALL_EDGES, 1, Direction::Downstream);
        assert_eq!(bfs.result().len(), 2); // A, B

        // Second query from C, depth 1 — should not have stale data
        bfs.run(&graph, &[idx[2]], ALL_EDGES, 1, Direction::Downstream);
        assert_eq!(bfs.result().len(), 2); // C, D

        // A and B should NOT be in the second result
        assert!(!bfs.result().contains(&idx[0]));
        assert!(!bfs.result().contains(&idx[1]));
    }

    #[test]
    fn bfs_depth_zero() {
        let (graph, idx) = make_linear_graph(5);
        let mut bfs = BfsState::new(graph.node_count());
        bfs.run(&graph, &[idx[0]], ALL_EDGES, 0, Direction::Downstream);

        // Only the seed
        assert_eq!(bfs.result().len(), 1);
        assert_eq!(bfs.result()[0], idx[0]);
    }

    #[test]
    fn bfs_multiple_seeds() {
        let (graph, idx) = make_linear_graph(5);
        let mut bfs = BfsState::new(graph.node_count());
        bfs.run(
            &graph,
            &[idx[0], idx[4]],
            ALL_EDGES,
            u32::MAX,
            Direction::Downstream,
        );

        // Both seeds + everything reachable downstream (all 5 nodes)
        assert_eq!(bfs.result().len(), 5);
    }

    #[test]
    fn bfs_no_capacity_growth_when_preallocated() {
        // Verify that pre-allocating with sufficient capacity means no reallocation
        let (graph, idx) = make_linear_graph(5);
        let mut bfs = BfsState::new(graph.node_count());

        // Pre-warm with a query to establish capacity
        bfs.run(&graph, &[idx[0]], ALL_EDGES, u32::MAX, Direction::Downstream);
        let cap_after_first = bfs.result.capacity();

        // Second run should not grow
        bfs.run(&graph, &[idx[0]], ALL_EDGES, u32::MAX, Direction::Downstream);
        assert_eq!(bfs.result.capacity(), cap_after_first);
    }
}
