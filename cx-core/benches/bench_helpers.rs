#![allow(dead_code)]
/// Shared helpers for benchmark graph generation.
use cx_core::graph::csr::{CsrGraph, EdgeInput};
use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::{Node, NodeKind};
use cx_core::graph::string_interner::StringInterner;

/// Generate a random graph with `n` nodes and `e` edges.
/// Edge kinds are distributed across all 11 kinds.
pub fn gen_graph(n: u32, e: u32) -> CsrGraph {
    let mut strings = StringInterner::with_capacity((n as usize) * 16);
    let nodes: Vec<Node> = (0..n)
        .map(|i| {
            let name = strings.intern(&format!("n{}", i));
            Node::new(i, NodeKind::Symbol, name)
        })
        .collect();

    let edges: Vec<EdgeInput> = (0..e)
        .map(|i| {
            let src = i % n;
            let tgt = (i.wrapping_mul(7).wrapping_add(13)) % n;
            let kind = EdgeKind::from_u8((i % 11) as u8).unwrap();
            EdgeInput::new(src, tgt, kind)
        })
        .collect();

    CsrGraph::build(nodes, edges, strings)
}

/// Generate a graph with mixed node kinds.
/// Distributes nodes as: 60% Symbol, 20% Module, 10% Endpoint, 5% Deployable, 5% other.
pub fn gen_mixed_graph(n: u32, e: u32) -> CsrGraph {
    let mut strings = StringInterner::with_capacity((n as usize) * 16);
    let nodes: Vec<Node> = (0..n)
        .map(|i| {
            let name = strings.intern(&format!("n{}", i));
            let kind = match i % 20 {
                0 => NodeKind::Deployable,
                1 => NodeKind::Endpoint,
                2..=3 => NodeKind::Module,
                _ => NodeKind::Symbol,
            };
            Node::new(i, kind, name)
        })
        .collect();

    let edges: Vec<EdgeInput> = (0..e)
        .map(|i| {
            let src = i % n;
            let tgt = (i.wrapping_mul(7).wrapping_add(13)) % n;
            let kind = EdgeKind::from_u8((i % 11) as u8).unwrap();
            EdgeInput::new(src, tgt, kind)
        })
        .collect();

    CsrGraph::build(nodes, edges, strings)
}

/// Generate a summary-sized graph (Deployable/Resource nodes only).
pub fn gen_summary_graph(n: u32, e: u32) -> CsrGraph {
    let mut strings = StringInterner::with_capacity((n as usize) * 20);
    let nodes: Vec<Node> = (0..n)
        .map(|i| {
            let name = strings.intern(&format!("svc_{}", i));
            let kind = if i % 4 == 0 {
                NodeKind::Resource
            } else {
                NodeKind::Deployable
            };
            Node::new(i, kind, name)
        })
        .collect();

    let edges: Vec<EdgeInput> = (0..e)
        .map(|i| {
            let src = i % n;
            let tgt = (i.wrapping_mul(3).wrapping_add(1)) % n;
            EdgeInput::new(src, tgt, EdgeKind::DependsOn)
        })
        .collect();

    CsrGraph::build(nodes, edges, strings)
}
