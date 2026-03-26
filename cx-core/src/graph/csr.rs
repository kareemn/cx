use super::edges::{Edge, EdgeKind};
use super::nodes::{Node, NodeId, StringId};
use super::string_interner::StringInterner;

/// The core CSR graph structure. All arrays are contiguous and cache-friendly.
#[derive(Debug)]
pub struct CsrGraph {
    /// Nodes sorted by (kind, id). Fixed 32 bytes each.
    pub nodes: Vec<Node>,

    /// Outgoing edges grouped by source node. Fixed 16 bytes each.
    /// All edges for node_i are in edges[offsets[i]..offsets[i+1]].
    pub edges: Vec<Edge>,

    /// offsets[i] = index into edges[] where node i's edges begin.
    /// offsets.len() == nodes.len() + 1 (sentinel at end).
    pub offsets: Vec<u32>,

    /// Reverse edge index for upstream queries.
    /// rev_edges grouped by target, rev_offsets indexes into rev_edges.
    pub rev_edges: Vec<Edge>,
    pub rev_offsets: Vec<u32>,

    /// Interned string table.
    pub strings: StringInterner,
}

/// Input edge for the graph builder: (source, target, kind).
pub struct EdgeInput {
    pub source: NodeId,
    pub target: NodeId,
    pub kind: EdgeKind,
    pub confidence_u8: u8,
    pub flags: u16,
}

impl EdgeInput {
    pub fn new(source: NodeId, target: NodeId, kind: EdgeKind) -> Self {
        Self {
            source,
            target,
            kind,
            confidence_u8: 255,
            flags: 0,
        }
    }
}

impl CsrGraph {
    /// Build a CSR graph from a list of nodes and edges.
    ///
    /// Nodes are sorted by (kind, name). Node IDs are remapped to sequential indices.
    /// Edges are sorted by (source, kind) within each source node's adjacency list.
    /// Reverse edge index is built simultaneously.
    pub fn build(
        mut nodes: Vec<Node>,
        edge_inputs: Vec<EdgeInput>,
        strings: StringInterner,
    ) -> Self {
        let n = nodes.len();

        // Sort nodes by (kind, name) for locality
        nodes.sort_unstable_by_key(|node| (node.kind, node.name));

        // Build old_id → new_id mapping
        let mut id_remap = vec![0u32; n];
        for (new_id, node) in nodes.iter_mut().enumerate() {
            id_remap[node.id as usize] = new_id as u32;
            node.id = new_id as u32;
        }

        // Remap parent references
        for node in &mut nodes {
            if (node.parent as usize) < n {
                node.parent = id_remap[node.parent as usize];
            }
        }

        // Build forward edges with remapped IDs, sorted by (source, kind)
        let mut forward: Vec<(u32, Edge)> = edge_inputs
            .iter()
            .map(|ei| {
                let source = id_remap[ei.source as usize];
                let target = id_remap[ei.target as usize];
                let mut edge = Edge::new(target, ei.kind);
                edge.confidence_u8 = ei.confidence_u8;
                edge.flags = ei.flags;
                (source, edge)
            })
            .collect();
        forward.sort_unstable_by_key(|(src, e)| (*src, e.kind, e.target));
        forward.dedup_by(|a, b| a.0 == b.0 && a.1.kind == b.1.kind && a.1.target == b.1.target);

        // Build forward offsets and edges arrays
        let mut offsets = Vec::with_capacity(n + 1);
        let mut edges = Vec::with_capacity(forward.len());
        let mut fi = 0;
        for node_idx in 0..n as u32 {
            offsets.push(edges.len() as u32);
            while fi < forward.len() && forward[fi].0 == node_idx {
                edges.push(forward[fi].1);
                fi += 1;
            }
        }
        offsets.push(edges.len() as u32);

        // Build reverse edges: sort by target, store source in edge.target field
        let mut reverse: Vec<(u32, Edge)> = edge_inputs
            .iter()
            .map(|ei| {
                let source = id_remap[ei.source as usize];
                let target = id_remap[ei.target as usize];
                let mut edge = Edge::new(source, ei.kind);
                edge.confidence_u8 = ei.confidence_u8;
                edge.flags = ei.flags;
                (target, edge)
            })
            .collect();
        reverse.sort_unstable_by_key(|(tgt, e)| (*tgt, e.kind, e.target));
        reverse.dedup_by(|a, b| a.0 == b.0 && a.1.kind == b.1.kind && a.1.target == b.1.target);

        let mut rev_offsets = Vec::with_capacity(n + 1);
        let mut rev_edges = Vec::with_capacity(reverse.len());
        let mut ri = 0;
        for node_idx in 0..n as u32 {
            rev_offsets.push(rev_edges.len() as u32);
            while ri < reverse.len() && reverse[ri].0 == node_idx {
                rev_edges.push(reverse[ri].1);
                ri += 1;
            }
        }
        rev_offsets.push(rev_edges.len() as u32);

        CsrGraph {
            nodes,
            edges,
            offsets,
            rev_edges,
            rev_offsets,
            strings,
        }
    }

    /// Get the forward edges for a given node index.
    #[inline]
    pub fn edges_for(&self, node_idx: u32) -> &[Edge] {
        let start = self.offsets[node_idx as usize] as usize;
        let end = self.offsets[node_idx as usize + 1] as usize;
        &self.edges[start..end]
    }

    /// Get the reverse edges (incoming) for a given node index.
    #[inline]
    pub fn rev_edges_for(&self, node_idx: u32) -> &[Edge] {
        let start = self.rev_offsets[node_idx as usize] as usize;
        let end = self.rev_offsets[node_idx as usize + 1] as usize;
        &self.rev_edges[start..end]
    }

    /// Number of nodes.
    pub fn node_count(&self) -> u32 {
        self.nodes.len() as u32
    }

    /// Number of forward edges.
    pub fn edge_count(&self) -> u32 {
        self.edges.len() as u32
    }

    /// Look up a node by its index.
    #[inline]
    pub fn node(&self, idx: u32) -> &Node {
        &self.nodes[idx as usize]
    }

    /// Find node index by name StringId. Linear scan within kind range if kind_index available,
    /// otherwise full scan. Returns the first match.
    pub fn find_node_by_name(&self, name: StringId) -> Option<u32> {
        self.nodes
            .iter()
            .position(|n| n.name == name)
            .map(|i| i as u32)
    }

    /// Merge multiple CsrGraphs into a single unified graph.
    ///
    /// Each input graph has its own node ID space and string table.
    /// This function remaps all IDs so the result is a coherent single graph.
    /// Additional cross-repo edges can be injected via `extra_edges`.
    pub fn merge(graphs: &[CsrGraph], extra_edges: Vec<EdgeInput>) -> Self {
        let total_nodes: usize = graphs.iter().map(|g| g.nodes.len()).sum();
        let total_edges: usize = graphs.iter().map(|g| g.edges.len()).sum();

        let mut merged_strings = StringInterner::new();
        let mut merged_nodes = Vec::with_capacity(total_nodes);
        let mut merged_edges = Vec::with_capacity(total_edges + extra_edges.len());

        let mut node_offset: u32 = 0;

        for graph in graphs {
            // Build string remap: old StringId → new StringId in merged table
            let mut string_remap = rustc_hash::FxHashMap::default();
            // Walk the string table by re-reading each node's strings
            for node in &graph.nodes {
                if node.name != super::nodes::STRING_NONE && !string_remap.contains_key(&node.name) {
                    let s = graph.strings.get(node.name);
                    string_remap.insert(node.name, merged_strings.intern(s));
                }
                if node.file != super::nodes::STRING_NONE && !string_remap.contains_key(&node.file) {
                    let s = graph.strings.get(node.file);
                    string_remap.insert(node.file, merged_strings.intern(s));
                }
            }

            let remap_string = |sid: super::nodes::StringId| -> super::nodes::StringId {
                if sid == super::nodes::STRING_NONE {
                    super::nodes::STRING_NONE
                } else {
                    *string_remap.get(&sid).unwrap_or(&sid)
                }
            };

            // Remap nodes
            for node in &graph.nodes {
                let mut new_node = *node;
                new_node.id = node.id + node_offset;
                new_node.name = remap_string(node.name);
                new_node.file = remap_string(node.file);
                if node.parent != super::nodes::NODE_NONE {
                    new_node.parent = node.parent + node_offset;
                }
                merged_nodes.push(new_node);
            }

            // Extract forward edges and remap
            for (src_idx, node) in graph.nodes.iter().enumerate() {
                let _ = node;
                let edges = graph.edges_for(src_idx as u32);
                for edge in edges {
                    merged_edges.push(EdgeInput {
                        source: src_idx as u32 + node_offset,
                        target: edge.target + node_offset,
                        kind: EdgeKind::from_u8(edge.kind).unwrap_or(EdgeKind::Calls),
                        confidence_u8: edge.confidence_u8,
                        flags: edge.flags,
                    });
                }
            }

            node_offset += graph.nodes.len() as u32;
        }

        // Add extra cross-repo edges (already in global ID space)
        merged_edges.extend(extra_edges);

        CsrGraph::build(merged_nodes, merged_edges, merged_strings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::bitvec::BitVec;
    use crate::graph::edges::{
        EdgeKindMask, ALL_EDGES, CODE_EDGES, EDGE_IS_CROSS_REPO,
    };
    use crate::graph::nodes::NodeKind;

    /// Helper: build a simple graph and return (graph, name→node_idx mapping).
    fn build_test_graph() -> (CsrGraph, Vec<u32>) {
        // Graph: A→B→C→D, A→E→F, B→G
        // 7 nodes: A(0), B(1), C(2), D(3), E(4), F(5), G(6)
        let mut strings = StringInterner::new();
        let names: Vec<StringId> = ["A", "B", "C", "D", "E", "F", "G"]
            .iter()
            .map(|s| strings.intern(s))
            .collect();

        let nodes: Vec<Node> = (0..7)
            .map(|i| Node::new(i, NodeKind::Symbol, names[i as usize]))
            .collect();

        let edges = vec![
            EdgeInput::new(0, 1, EdgeKind::Calls), // A→B
            EdgeInput::new(1, 2, EdgeKind::Calls), // B→C
            EdgeInput::new(2, 3, EdgeKind::Calls), // C→D
            EdgeInput::new(0, 4, EdgeKind::Calls), // A→E
            EdgeInput::new(4, 5, EdgeKind::Calls), // E→F
            EdgeInput::new(1, 6, EdgeKind::Calls), // B→G
        ];

        let graph = CsrGraph::build(nodes, edges, strings);

        // Find remapped indices by name
        let mut indices = vec![0u32; 7];
        for (i, _name) in ["A", "B", "C", "D", "E", "F", "G"].iter().enumerate() {
            indices[i] = graph.find_node_by_name(names[i]).unwrap();
        }

        (graph, indices)
    }

    /// Simple BFS helper for testing (before BfsState is implemented).
    fn bfs_downstream(graph: &CsrGraph, seed: u32, mask: EdgeKindMask) -> Vec<u32> {
        let mut visited = BitVec::new(graph.node_count());
        let mut current = vec![seed];
        let mut result = Vec::new();
        visited.set(seed);

        while !current.is_empty() {
            let mut next = Vec::new();
            for &node in &current {
                result.push(node);
                for edge in graph.edges_for(node) {
                    if !edge.matches_mask(mask) {
                        continue;
                    }
                    if visited.test(edge.target) {
                        continue;
                    }
                    visited.set(edge.target);
                    next.push(edge.target);
                }
            }
            current = next;
        }
        result
    }

    /// Simple BFS upstream (using reverse edges).
    fn bfs_upstream(graph: &CsrGraph, seed: u32, mask: EdgeKindMask) -> Vec<u32> {
        let mut visited = BitVec::new(graph.node_count());
        let mut current = vec![seed];
        let mut result = Vec::new();
        visited.set(seed);

        while !current.is_empty() {
            let mut next = Vec::new();
            for &node in &current {
                result.push(node);
                for edge in graph.rev_edges_for(node) {
                    if !edge.matches_mask(mask) {
                        continue;
                    }
                    if visited.test(edge.target) {
                        continue;
                    }
                    visited.set(edge.target);
                    next.push(edge.target);
                }
            }
            current = next;
        }
        result
    }

    #[test]
    fn graph_traversal_downstream() {
        // TEST graph_traversal_downstream from ARCHITECTURE.md
        let (graph, idx) = build_test_graph();
        let result = bfs_downstream(&graph, idx[0], ALL_EDGES);

        // Result should contain all 7 nodes (A,B,C,D,E,F,G)
        assert_eq!(result.len(), 7);

        // All nodes reachable
        let mut found = vec![false; 7];
        for &r in &result {
            for (i, &expected) in idx.iter().enumerate() {
                if r == expected {
                    found[i] = true;
                }
            }
        }
        for (i, &f) in found.iter().enumerate() {
            assert!(f, "node {} not found in BFS result", i);
        }

        // BFS order: A first
        assert_eq!(result[0], idx[0]);
    }

    #[test]
    fn graph_traversal_upstream() {
        // TEST graph_traversal_upstream from ARCHITECTURE.md
        let (graph, idx) = build_test_graph();
        let result = bfs_upstream(&graph, idx[3], ALL_EDGES); // start from D

        // D→C→B→A (upstream)
        assert_eq!(result.len(), 4);

        let names: Vec<&str> = result
            .iter()
            .map(|&i| graph.strings.get(graph.node(i).name))
            .collect();

        assert_eq!(names[0], "D");
        assert!(names.contains(&"C"));
        assert!(names.contains(&"B"));
        assert!(names.contains(&"A"));
    }

    #[test]
    fn graph_edge_filtering() {
        // TEST graph_edge_filtering from ARCHITECTURE.md
        // A→B (Calls), A→C (DependsOn), B→D (Calls)
        let mut strings = StringInterner::new();
        let na = strings.intern("A");
        let nb = strings.intern("B");
        let nc = strings.intern("C");
        let nd = strings.intern("D");

        let nodes = vec![
            Node::new(0, NodeKind::Symbol, na),
            Node::new(1, NodeKind::Symbol, nb),
            Node::new(2, NodeKind::Deployable, nc),
            Node::new(3, NodeKind::Symbol, nd),
        ];

        let edges = vec![
            EdgeInput::new(0, 1, EdgeKind::Calls),     // A→B
            EdgeInput::new(0, 2, EdgeKind::DependsOn),  // A→C
            EdgeInput::new(1, 3, EdgeKind::Calls),      // B→D
        ];

        let graph = CsrGraph::build(nodes, edges, strings);

        let a_idx = graph.find_node_by_name(na).unwrap();
        let c_idx = graph.find_node_by_name(nc).unwrap();

        // BFS with CODE_EDGES only (Calls | Imports)
        let result = bfs_downstream(&graph, a_idx, CODE_EDGES);

        let result_names: Vec<&str> = result
            .iter()
            .map(|&i| graph.strings.get(graph.node(i).name))
            .collect();

        // Should contain A, B, D but NOT C
        assert!(result_names.contains(&"A"));
        assert!(result_names.contains(&"B"));
        assert!(result_names.contains(&"D"));
        assert!(!result_names.contains(&"C"));

        // C should not be in result
        assert!(!result.contains(&c_idx));
    }

    #[test]
    fn graph_cross_repo_filtering() {
        // TEST graph_cross_repo_filtering from ARCHITECTURE.md
        // Build graph with nodes in repo_1 and repo_2.
        let mut strings = StringInterner::new();
        let na = strings.intern("A");
        let nb = strings.intern("B");
        let nc = strings.intern("C");
        let nd = strings.intern("D");

        let mut n0 = Node::new(0, NodeKind::Symbol, na);
        n0.repo = 1;
        let mut n1 = Node::new(1, NodeKind::Symbol, nb);
        n1.repo = 1;
        let mut n2 = Node::new(2, NodeKind::Symbol, nc);
        n2.repo = 2; // different repo
        let mut n3 = Node::new(3, NodeKind::Symbol, nd);
        n3.repo = 2;

        // A→B (same repo), B→C (cross repo), C→D (same repo)
        let mut e_bc = EdgeInput::new(1, 2, EdgeKind::Calls);
        e_bc.flags = EDGE_IS_CROSS_REPO;

        let edges = vec![
            EdgeInput::new(0, 1, EdgeKind::Calls),
            e_bc,
            EdgeInput::new(2, 3, EdgeKind::Calls),
        ];

        let graph = CsrGraph::build(vec![n0, n1, n2, n3], edges, strings);

        let a_idx = graph.find_node_by_name(na).unwrap();

        // BFS with cross-repo filter: only follow edges with IS_CROSS_REPO
        let mut visited = BitVec::new(graph.node_count());
        let mut current = vec![a_idx];
        let mut cross_repo_targets = Vec::new();
        visited.set(a_idx);

        while !current.is_empty() {
            let mut next = Vec::new();
            for &node in &current {
                for edge in graph.edges_for(node) {
                    if visited.test(edge.target) {
                        continue;
                    }
                    visited.set(edge.target);
                    if edge.flags & EDGE_IS_CROSS_REPO != 0 {
                        cross_repo_targets.push(edge.target);
                    }
                    next.push(edge.target);
                }
            }
            current = next;
        }

        // Only the B→C edge crosses repos
        assert_eq!(cross_repo_targets.len(), 1);
        let cross_name = graph.strings.get(graph.node(cross_repo_targets[0]).name);
        assert_eq!(cross_name, "C");
    }

    #[test]
    fn edge_sorting_within_node() {
        // TEST edge_sorting_within_node from ARCHITECTURE.md
        // Node A has 10 edges: 3 Calls, 2 Imports, 3 DependsOn, 2 Exposes
        let mut strings = StringInterner::new();
        let names: Vec<StringId> = (0..11)
            .map(|i| strings.intern(&format!("N{}", i)))
            .collect();

        let nodes: Vec<Node> = (0..11)
            .map(|i| Node::new(i, NodeKind::Symbol, names[i as usize]))
            .collect();

        let edges = vec![
            // 3 Calls
            EdgeInput::new(0, 1, EdgeKind::Calls),
            EdgeInput::new(0, 2, EdgeKind::Calls),
            EdgeInput::new(0, 3, EdgeKind::Calls),
            // 2 Imports
            EdgeInput::new(0, 4, EdgeKind::Imports),
            EdgeInput::new(0, 5, EdgeKind::Imports),
            // 3 DependsOn
            EdgeInput::new(0, 6, EdgeKind::DependsOn),
            EdgeInput::new(0, 7, EdgeKind::DependsOn),
            EdgeInput::new(0, 8, EdgeKind::DependsOn),
            // 2 Exposes
            EdgeInput::new(0, 9, EdgeKind::Exposes),
            EdgeInput::new(0, 10, EdgeKind::Exposes),
        ];

        let graph = CsrGraph::build(nodes, edges, strings);

        // Find N0's remapped index
        let n0_idx = graph.find_node_by_name(names[0]).unwrap();
        let node_edges = graph.edges_for(n0_idx);

        assert_eq!(node_edges.len(), 10);

        // Verify edges are sorted by kind
        let kinds: Vec<u8> = node_edges.iter().map(|e| e.kind).collect();
        let mut sorted_kinds = kinds.clone();
        sorted_kinds.sort_unstable();
        assert_eq!(kinds, sorted_kinds);

        // Verify expected order: Calls(1), Calls, Calls, Imports(2), Imports, DependsOn(3)x3, Exposes(4)x2
        assert_eq!(kinds, vec![1, 1, 1, 2, 2, 3, 3, 3, 4, 4]);
    }

    #[test]
    fn graph_build_basic() {
        let mut strings = StringInterner::new();
        let na = strings.intern("hello");
        let nb = strings.intern("world");

        let nodes = vec![
            Node::new(0, NodeKind::Symbol, na),
            Node::new(1, NodeKind::Symbol, nb),
        ];

        let edges = vec![EdgeInput::new(0, 1, EdgeKind::Calls)];

        let graph = CsrGraph::build(nodes, edges, strings);

        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
        assert_eq!(graph.offsets.len(), 3); // n+1

        // Reverse index
        assert_eq!(graph.rev_edges.len(), 1);
        assert_eq!(graph.rev_offsets.len(), 3);
    }

    #[test]
    fn graph_no_edges() {
        let strings = StringInterner::new();
        let nodes: Vec<Node> = (0..5)
            .map(|i| Node::new(i, NodeKind::Symbol, 0))
            .collect();

        let graph = CsrGraph::build(nodes, vec![], strings);

        assert_eq!(graph.node_count(), 5);
        assert_eq!(graph.edge_count(), 0);

        for i in 0..5 {
            assert!(graph.edges_for(i).is_empty());
            assert!(graph.rev_edges_for(i).is_empty());
        }
    }
}
