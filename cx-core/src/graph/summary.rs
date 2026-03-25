use super::csr::{CsrGraph, EdgeInput};
use super::edges::EdgeKind;
use super::nodes::{Node, NodeId, NodeKind};
use super::string_interner::StringInterner;

/// Pre-computed service-level summary graph.
/// Contains only Deployable and Resource nodes with aggregated edges.
///
/// Used for macro queries (cx_depends at service level) to avoid scanning 1M symbol nodes.
#[derive(Debug)]
pub struct SummaryGraph {
    /// The summary CSR graph containing only Deployable and Resource nodes.
    pub graph: CsrGraph,
    /// Maps full graph NodeId → summary graph NodeId (u32::MAX if not in summary).
    full_to_summary: Vec<u32>,
    /// Maps summary graph NodeId → full graph NodeId.
    summary_to_full: Vec<NodeId>,
}

impl SummaryGraph {
    /// Build a summary graph from a full CsrGraph.
    ///
    /// Collapses all Contains edges — every symbol inside a Deployable is collapsed
    /// into the Deployable node. Edges between symbols in different Deployables become
    /// a single DependsOn edge in the summary.
    pub fn build(full: &CsrGraph) -> Self {
        let n = full.node_count();

        // Step 1: Identify which nodes are Deployable or Resource (summary nodes)
        let mut full_to_summary = vec![u32::MAX; n as usize];
        let mut summary_to_full: Vec<NodeId> = Vec::new();

        for i in 0..n {
            let node = full.node(i);
            let kind = NodeKind::from_u8(node.kind);
            if matches!(kind, Some(NodeKind::Deployable) | Some(NodeKind::Resource)) {
                full_to_summary[i as usize] = summary_to_full.len() as u32;
                summary_to_full.push(i);
            }
        }

        // Step 2: Map every non-summary node to its nearest summary ancestor via parent chain.
        // Walk the parent chain until we find a node that is in the summary.
        let mut node_to_deployable = vec![u32::MAX; n as usize];
        for i in 0..n {
            if full_to_summary[i as usize] != u32::MAX {
                node_to_deployable[i as usize] = i;
                continue;
            }
            // Walk up parent chain
            let mut current = i;
            let mut visited = Vec::new();
            loop {
                if node_to_deployable[current as usize] != u32::MAX {
                    // Found — propagate back
                    let deployable = node_to_deployable[current as usize];
                    for &v in &visited {
                        node_to_deployable[v as usize] = deployable;
                    }
                    node_to_deployable[i as usize] = deployable;
                    break;
                }
                let parent = full.node(current).parent;
                if parent == u32::MAX || parent == current {
                    break; // no deployable ancestor
                }
                visited.push(current);
                current = parent;
            }
        }

        // Step 3: For each edge in the full graph between nodes in different deployables,
        // create a summary edge (DependsOn) between the deployables. Deduplicate.
        let mut summary_edge_set = rustc_hash::FxHashSet::default();
        let mut summary_edges = Vec::new();

        for src_idx in 0..n {
            let src_dep = node_to_deployable[src_idx as usize];
            if src_dep == u32::MAX {
                continue;
            }
            let src_summary = full_to_summary[src_dep as usize];
            if src_summary == u32::MAX {
                continue;
            }

            for edge in full.edges_for(src_idx) {
                let tgt_dep = node_to_deployable[edge.target as usize];
                if tgt_dep == u32::MAX || tgt_dep == src_dep {
                    continue; // same deployable or no deployable
                }
                let tgt_summary = full_to_summary[tgt_dep as usize];
                if tgt_summary == u32::MAX {
                    continue;
                }

                let key = (src_summary, tgt_summary);
                if summary_edge_set.insert(key) {
                    summary_edges.push(EdgeInput::new(
                        src_summary,
                        tgt_summary,
                        EdgeKind::DependsOn,
                    ));
                }
            }
        }

        // Step 4: Build summary nodes (copy from full graph, remap IDs)
        let summary_nodes: Vec<Node> = summary_to_full
            .iter()
            .enumerate()
            .map(|(new_id, &full_id)| {
                let orig = full.node(full_id);
                let mut node = *orig;
                node.id = new_id as u32;
                node.parent = u32::MAX; // summary nodes have no parent in summary graph
                node
            })
            .collect();

        // Build the summary CSR graph
        let summary_strings = StringInterner::new(); // summary shares strings via IDs
        let graph = CsrGraph::build(summary_nodes, summary_edges, summary_strings);

        SummaryGraph {
            graph,
            full_to_summary,
            summary_to_full,
        }
    }

    /// Get the summary node index for a full graph node, if it's in the summary.
    pub fn summary_idx(&self, full_id: NodeId) -> Option<u32> {
        let idx = self.full_to_summary[full_id as usize];
        if idx == u32::MAX {
            None
        } else {
            Some(idx)
        }
    }

    /// Get the full graph node ID for a summary node index.
    pub fn full_id(&self, summary_idx: u32) -> NodeId {
        self.summary_to_full[summary_idx as usize]
    }

    /// Number of nodes in the summary graph.
    pub fn node_count(&self) -> u32 {
        self.graph.node_count()
    }

    /// Number of edges in the summary graph.
    pub fn edge_count(&self) -> u32 {
        self.graph.edge_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::bitvec::BitVec;
    use crate::graph::edges::ALL_EDGES;

    /// Build test graph: 3 Deployables (A, B, C) each with 100 symbols.
    /// Symbols in A call symbols in B. Symbols in B call symbols in C.
    fn build_three_service_graph() -> (CsrGraph, Vec<NodeId>, Vec<NodeId>) {
        let mut strings = StringInterner::new();
        let dep_names = [
            strings.intern("ServiceA"),
            strings.intern("ServiceB"),
            strings.intern("ServiceC"),
        ];

        let mut nodes = Vec::new();
        let mut edge_inputs = Vec::new();

        // Create 3 deployable nodes (IDs 0, 1, 2)
        for (i, &name) in dep_names.iter().enumerate() {
            nodes.push(Node::new(i as u32, NodeKind::Deployable, name));
        }

        // Create 100 symbols per deployable
        let mut symbol_ids: Vec<Vec<NodeId>> = vec![Vec::new(); 3];
        let mut next_id = 3u32;

        for dep_idx in 0..3u32 {
            for s in 0..100u32 {
                let sym_name = strings.intern(&format!("sym_{}_{}", dep_idx, s));
                let mut sym = Node::new(next_id, NodeKind::Symbol, sym_name);
                sym.parent = dep_idx; // contained by deployable

                // Add Contains edge from deployable to symbol
                edge_inputs.push(EdgeInput::new(dep_idx, next_id, EdgeKind::Contains));

                symbol_ids[dep_idx as usize].push(next_id);
                nodes.push(sym);
                next_id += 1;
            }
        }

        // Symbols in A call symbols in B (cross-service calls)
        for i in 0..100u32 {
            edge_inputs.push(EdgeInput::new(
                symbol_ids[0][i as usize],
                symbol_ids[1][i as usize],
                EdgeKind::Calls,
            ));
        }

        // Symbols in B call symbols in C
        for i in 0..100u32 {
            edge_inputs.push(EdgeInput::new(
                symbol_ids[1][i as usize],
                symbol_ids[2][i as usize],
                EdgeKind::Calls,
            ));
        }

        let graph = CsrGraph::build(nodes, edge_inputs, strings);

        // Find remapped deployable indices
        let dep_indices: Vec<NodeId> = dep_names
            .iter()
            .map(|&name| graph.find_node_by_name(name).unwrap())
            .collect();

        (graph, dep_indices, dep_names.to_vec())
    }

    #[test]
    fn summary_graph_construction() {
        // TEST summary_graph_construction from ARCHITECTURE.md
        let (graph, dep_idx, _) = build_three_service_graph();

        let summary = SummaryGraph::build(&graph);

        // Summary has 3 nodes (A, B, C)
        assert_eq!(summary.node_count(), 3);

        // Summary has 2 edges (A→B, B→C)
        assert_eq!(summary.edge_count(), 2);

        // Verify the edges are DependsOn
        for i in 0..summary.node_count() {
            for edge in summary.graph.edges_for(i) {
                assert_eq!(edge.kind, EdgeKind::DependsOn as u8);
            }
        }

        // Verify all deployable nodes are in summary
        for &d in &dep_idx {
            assert!(summary.summary_idx(d).is_some());
        }
    }

    #[test]
    fn summary_graph_query() {
        // TEST summary_graph_query from ARCHITECTURE.md
        // cx_depends on summary graph: A downstream.
        // Returns {B, C} using only summary graph.
        let (full, dep_idx, _) = build_three_service_graph();
        let summary = SummaryGraph::build(&full);

        let a_summary = summary.summary_idx(dep_idx[0]).unwrap();

        // BFS on summary graph from A
        let mut visited = BitVec::new(summary.node_count());
        let mut current = vec![a_summary];
        let mut result = Vec::new();
        visited.set(a_summary);

        while !current.is_empty() {
            let mut next = Vec::new();
            for &node in &current {
                result.push(node);
                for edge in summary.graph.edges_for(node) {
                    if !visited.test(edge.target) {
                        visited.set(edge.target);
                        next.push(edge.target);
                    }
                }
            }
            current = next;
        }

        // Result includes A, B, C (3 nodes total)
        assert_eq!(result.len(), 3);

        // Downstream dependencies of A: B and C
        let downstream: Vec<u32> = result
            .iter()
            .filter(|&&r| r != a_summary)
            .copied()
            .collect();
        assert_eq!(downstream.len(), 2);
    }
}
