use crate::graph::bitvec::BitVec;
use crate::graph::csr::CsrGraph;
use crate::graph::edges::EdgeKindMask;
use crate::graph::nodes::NodeId;

/// Result of a depends query.
#[derive(Debug)]
pub struct DependsResult {
    /// All transitively reachable nodes.
    pub nodes: Vec<NodeId>,
}

/// Direction for dependency queries.
#[derive(Debug, Clone, Copy)]
pub enum DependsDirection {
    /// What does this node depend on?
    Downstream,
    /// What depends on this node?
    Upstream,
}

/// cx_depends: filtered transitive closure.
///
/// Uses SERVICE_EDGES mask by default for service-level queries.
/// For symbol-level, use ALL_EDGES.
pub fn depends(
    graph: &CsrGraph,
    seed: NodeId,
    direction: DependsDirection,
    mask: EdgeKindMask,
    max_depth: u32,
) -> DependsResult {
    let mut visited = BitVec::new(graph.node_count());
    let mut current = vec![seed];
    let mut result_nodes = Vec::new();

    visited.set(seed);

    for _depth in 0..max_depth {
        if current.is_empty() {
            break;
        }

        let mut next = Vec::new();
        for &node in &current {
            let edges = match direction {
                DependsDirection::Downstream => graph.edges_for(node),
                DependsDirection::Upstream => graph.rev_edges_for(node),
            };

            for edge in edges {
                if (1u16 << edge.kind) & mask == 0 {
                    continue;
                }
                if visited.test(edge.target) {
                    continue;
                }
                visited.set(edge.target);
                result_nodes.push(edge.target);
                next.push(edge.target);
            }
        }

        current = next;
    }

    DependsResult {
        nodes: result_nodes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::csr::{CsrGraph, EdgeInput};
    use crate::graph::edges::{EdgeKind, ALL_EDGES, SERVICE_EDGES};
    use crate::graph::nodes::{Node, NodeKind};
    use crate::graph::string_interner::StringInterner;

    fn build_depends_graph() -> (CsrGraph, Vec<u32>) {
        // A depends on B, B depends on C (chain: A→B→C)
        let mut strings = StringInterner::new();
        let names: Vec<_> = ["A", "B", "C"]
            .iter()
            .map(|s| strings.intern(s))
            .collect();
        let nodes: Vec<Node> = (0..3)
            .map(|i| Node::new(i, NodeKind::Deployable, names[i as usize]))
            .collect();
        let edges = vec![
            EdgeInput::new(0, 1, EdgeKind::DependsOn),
            EdgeInput::new(1, 2, EdgeKind::DependsOn),
        ];
        let graph = CsrGraph::build(nodes, edges, strings);
        let indices: Vec<u32> = (0..3)
            .map(|i| graph.find_node_by_name(names[i as usize]).unwrap())
            .collect();
        (graph, indices)
    }

    #[test]
    fn cx_depends_downstream() {
        // TEST cx_depends_downstream from ARCHITECTURE.md
        let (graph, idx) = build_depends_graph();

        let result = depends(
            &graph,
            idx[0],
            DependsDirection::Downstream,
            ALL_EDGES,
            10,
        );

        // A downstream → {B, C}
        assert_eq!(result.nodes.len(), 2);
        assert!(result.nodes.contains(&idx[1]));
        assert!(result.nodes.contains(&idx[2]));
    }

    #[test]
    fn cx_depends_upstream() {
        // TEST cx_depends_upstream from ARCHITECTURE.md
        let (graph, idx) = build_depends_graph();

        // A upstream → nothing (nobody depends on A)
        let result_a = depends(
            &graph,
            idx[0],
            DependsDirection::Upstream,
            ALL_EDGES,
            10,
        );
        assert!(result_a.nodes.is_empty(), "nothing depends on A");

        // C upstream → {B, A} (transitively)
        let result_c = depends(
            &graph,
            idx[2],
            DependsDirection::Upstream,
            ALL_EDGES,
            10,
        );
        assert_eq!(result_c.nodes.len(), 2);
        assert!(result_c.nodes.contains(&idx[0]));
        assert!(result_c.nodes.contains(&idx[1]));
    }

    #[test]
    fn depends_depth_limited() {
        let (graph, idx) = build_depends_graph();

        let result = depends(
            &graph,
            idx[0],
            DependsDirection::Downstream,
            ALL_EDGES,
            1,
        );
        // Depth 1: only B
        assert_eq!(result.nodes.len(), 1);
        assert!(result.nodes.contains(&idx[1]));
    }

    #[test]
    fn depends_with_service_edges_mask() {
        // A→B (DependsOn), A→C (Calls) — with SERVICE_EDGES, only B reachable
        let mut strings = StringInterner::new();
        let names: Vec<_> = ["A", "B", "C"]
            .iter()
            .map(|s| strings.intern(s))
            .collect();
        let nodes: Vec<Node> = (0..3)
            .map(|i| Node::new(i, NodeKind::Deployable, names[i as usize]))
            .collect();
        let edges = vec![
            EdgeInput::new(0, 1, EdgeKind::DependsOn),
            EdgeInput::new(0, 2, EdgeKind::Calls),
        ];
        let graph = CsrGraph::build(nodes, edges, strings);
        let a = graph.find_node_by_name(names[0]).unwrap();
        let b = graph.find_node_by_name(names[1]).unwrap();

        let result = depends(
            &graph,
            a,
            DependsDirection::Downstream,
            SERVICE_EDGES,
            10,
        );
        assert_eq!(result.nodes.len(), 1);
        assert!(result.nodes.contains(&b));
    }
}
