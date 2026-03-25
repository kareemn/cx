use super::csr::CsrGraph;
use super::nodes::{Node, NodeKind};

/// PERFORMANCE CRITICAL: Avoid scanning all 1M nodes when you only want Endpoints.
/// The kind_index tells you exactly where each node kind starts and ends.
///
/// Requires nodes to be sorted by (kind, ...) — which CsrGraph::build guarantees.
#[derive(Debug, Clone, Copy)]
pub struct KindIndex {
    /// kind_ranges[k] = (start, end) indices into the nodes array for NodeKind k.
    /// Find all Endpoint nodes: nodes[kind_ranges[4].0 .. kind_ranges[4].1]
    pub kind_ranges: [(u32, u32); NodeKind::COUNT],
}

impl KindIndex {
    /// Build a KindIndex from a CsrGraph whose nodes are sorted by kind.
    pub fn build(graph: &CsrGraph) -> Self {
        Self::build_from_nodes(&graph.nodes)
    }

    /// Build from a slice of nodes sorted by kind.
    pub fn build_from_nodes(nodes: &[Node]) -> Self {
        let mut kind_ranges = [(0u32, 0u32); NodeKind::COUNT];
        let n = nodes.len() as u32;

        if n == 0 {
            return Self { kind_ranges };
        }

        let mut current_kind = nodes[0].kind;
        let mut start = 0u32;

        for i in 1..n {
            if nodes[i as usize].kind != current_kind {
                if (current_kind as usize) < NodeKind::COUNT {
                    kind_ranges[current_kind as usize] = (start, i);
                }
                current_kind = nodes[i as usize].kind;
                start = i;
            }
        }
        // Close the last range
        if (current_kind as usize) < NodeKind::COUNT {
            kind_ranges[current_kind as usize] = (start, n);
        }

        Self { kind_ranges }
    }

    /// Get the (start, end) range for a given node kind.
    #[inline]
    pub fn range(&self, kind: NodeKind) -> (u32, u32) {
        self.kind_ranges[kind as usize]
    }

    /// Get all nodes of a given kind from the graph.
    #[inline]
    pub fn nodes_of_kind<'a>(&self, kind: NodeKind, nodes: &'a [Node]) -> &'a [Node] {
        let (start, end) = self.range(kind);
        &nodes[start as usize..end as usize]
    }

    /// Count of nodes for a given kind.
    #[inline]
    pub fn count(&self, kind: NodeKind) -> u32 {
        let (start, end) = self.range(kind);
        end - start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::csr::CsrGraph;
    use crate::graph::string_interner::StringInterner;

    #[test]
    fn kind_index() {
        // TEST kind_index from ARCHITECTURE.md:
        // Build graph: 50 Symbol nodes, 10 Endpoint nodes, 5 Deployable nodes.
        // Use KindIndex to find all Endpoints.
        // kind_ranges returns exactly the 10 Endpoint nodes.
        // Zero scanning of Symbol or Deployable nodes.
        let mut strings = StringInterner::new();
        let mut nodes = Vec::new();
        let mut id = 0u32;

        // 5 Deployable nodes
        for i in 0..5u32 {
            let name = strings.intern(&format!("dep_{}", i));
            nodes.push(Node::new(id, NodeKind::Deployable, name));
            id += 1;
        }

        // 50 Symbol nodes
        for i in 0..50u32 {
            let name = strings.intern(&format!("sym_{}", i));
            nodes.push(Node::new(id, NodeKind::Symbol, name));
            id += 1;
        }

        // 10 Endpoint nodes
        for i in 0..10u32 {
            let name = strings.intern(&format!("ep_{}", i));
            nodes.push(Node::new(id, NodeKind::Endpoint, name));
            id += 1;
        }

        let graph = CsrGraph::build(nodes, vec![], strings);
        let kind_idx = KindIndex::build(&graph);

        // kind_ranges returns exactly the 10 Endpoint nodes
        assert_eq!(kind_idx.count(NodeKind::Endpoint), 10);
        assert_eq!(kind_idx.count(NodeKind::Symbol), 50);
        assert_eq!(kind_idx.count(NodeKind::Deployable), 5);

        // Verify the endpoint nodes are correct
        let endpoints = kind_idx.nodes_of_kind(NodeKind::Endpoint, &graph.nodes);
        assert_eq!(endpoints.len(), 10);
        for ep in endpoints {
            assert_eq!(ep.kind, NodeKind::Endpoint as u8);
        }

        // Verify zero scanning: the range for endpoints doesn't overlap with others
        let (ep_start, ep_end) = kind_idx.range(NodeKind::Endpoint);
        let (sym_start, sym_end) = kind_idx.range(NodeKind::Symbol);
        let (dep_start, dep_end) = kind_idx.range(NodeKind::Deployable);

        // Ranges should not overlap
        assert!(ep_end <= sym_start || ep_start >= sym_end);
        assert!(ep_end <= dep_start || ep_start >= dep_end);

        // Empty kinds have zero-length ranges
        assert_eq!(kind_idx.count(NodeKind::Repo), 0);
        assert_eq!(kind_idx.count(NodeKind::Resource), 0);
    }

    #[test]
    fn kind_index_empty_graph() {
        let strings = StringInterner::new();
        let graph = CsrGraph::build(vec![], vec![], strings);
        let kind_idx = KindIndex::build(&graph);

        for k in 0..NodeKind::COUNT {
            let kind = NodeKind::from_u8(k as u8).unwrap();
            assert_eq!(kind_idx.count(kind), 0);
        }
    }

    #[test]
    fn kind_index_single_kind() {
        let mut strings = StringInterner::new();
        let nodes: Vec<Node> = (0..10)
            .map(|i| {
                let name = strings.intern(&format!("s{}", i));
                Node::new(i, NodeKind::Symbol, name)
            })
            .collect();

        let graph = CsrGraph::build(nodes, vec![], strings);
        let kind_idx = KindIndex::build(&graph);

        assert_eq!(kind_idx.count(NodeKind::Symbol), 10);
        for k in 0..NodeKind::COUNT {
            let kind = NodeKind::from_u8(k as u8).unwrap();
            if kind != NodeKind::Symbol {
                assert_eq!(kind_idx.count(kind), 0);
            }
        }
    }
}
