mod bench_helpers;

use criterion::{criterion_group, criterion_main, Criterion};
use cx_core::graph::csr::{CsrGraph, EdgeInput};
use cx_core::graph::edges::{EdgeKind, ALL_EDGES, SERVICE_EDGES};
use cx_core::graph::nodes::{Node, NodeKind};
use cx_core::graph::string_interner::StringInterner;
use cx_core::query::depends::{self, DependsDirection};
use cx_core::query::path::PathFinder;

/// Build a graph that simulates multiple repos with service-level edges.
/// Creates `n_services` deployable nodes connected in a chain via DependsOn edges,
/// each with `symbols_per_service` symbol children connected by Calls edges.
fn build_multi_service_graph(n_services: u32, symbols_per_service: u32) -> CsrGraph {
    let mut strings = StringInterner::with_capacity((n_services * symbols_per_service * 20) as usize);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut id = 0u32;

    // Create services with symbols
    let mut service_ids = Vec::new();
    for s in 0..n_services {
        let svc_name = strings.intern(&format!("service_{}", s));
        let svc_id = id;
        id += 1;
        let mut svc_node = Node::new(svc_id, NodeKind::Deployable, svc_name);
        svc_node.repo = s as u16;
        nodes.push(svc_node);
        service_ids.push(svc_id);

        // Add symbols per service
        let mut prev_sym = None;
        for f in 0..symbols_per_service {
            let sym_name = strings.intern(&format!("svc_{}_func_{}", s, f));
            let sym_id = id;
            id += 1;
            let mut sym_node = Node::new(sym_id, NodeKind::Symbol, sym_name);
            sym_node.parent = svc_id;
            sym_node.repo = s as u16;
            nodes.push(sym_node);

            // Contains edge from service to symbol
            edges.push(EdgeInput::new(svc_id, sym_id, EdgeKind::Contains));

            // Chain calls within the service
            if let Some(prev) = prev_sym {
                edges.push(EdgeInput::new(prev, sym_id, EdgeKind::Calls));
            }
            prev_sym = Some(sym_id);
        }
    }

    // Chain services: service_0 → service_1 → ... → service_N
    for i in 0..n_services.saturating_sub(1) {
        edges.push(EdgeInput::new(service_ids[i as usize], service_ids[(i + 1) as usize], EdgeKind::DependsOn));
    }

    CsrGraph::build(nodes, edges, strings)
}

/// BENCH cx_path_5_hops:
///   Graph with 100K nodes across 5 repos. Path spanning 5 service boundaries.
///   TARGET: < 2ms
fn cx_path_5_hops(c: &mut Criterion) {
    // 5 services, 20K symbols each = 100K+ nodes
    let graph = build_multi_service_graph(5, 20_000);
    let mut finder = PathFinder::new(graph.node_count());

    // Find first and last service
    let first = graph.nodes.iter().position(|n| n.kind == NodeKind::Deployable as u8).unwrap() as u32;
    let last = graph.nodes.iter().rposition(|n| n.kind == NodeKind::Deployable as u8).unwrap() as u32;

    c.bench_function("cx_path_5_hops_100k", |b| {
        b.iter(|| {
            finder.find_path(&graph, first, last, ALL_EDGES, 20)
        });
    });
}

/// BENCH cx_path_10_hops:
///   Graph with 1M nodes across 20 repos. Path spanning 10 service boundaries.
///   TARGET: < 10ms
fn cx_path_10_hops(c: &mut Criterion) {
    let graph = build_multi_service_graph(20, 50_000);
    let mut finder = PathFinder::new(graph.node_count());

    let first = graph.nodes.iter().position(|n| n.kind == NodeKind::Deployable as u8).unwrap() as u32;
    let last = graph.nodes.iter().rposition(|n| n.kind == NodeKind::Deployable as u8).unwrap() as u32;

    let mut group = c.benchmark_group("cx_path_10_hops");
    group.sample_size(10);
    group.bench_function("1m_nodes", |b| {
        b.iter(|| {
            finder.find_path(&graph, first, last, ALL_EDGES, 30)
        });
    });
    group.finish();
}

/// BENCH cx_depends_depth3:
///   Graph with 100K nodes. Transitive dependency query, depth 3.
///   TARGET: < 2ms
fn cx_depends_depth3(c: &mut Criterion) {
    let graph = build_multi_service_graph(10, 10_000);

    let first = graph.nodes.iter().position(|n| n.kind == NodeKind::Deployable as u8).unwrap() as u32;

    c.bench_function("cx_depends_depth3_100k", |b| {
        b.iter(|| {
            depends::depends(&graph, first, DependsDirection::Downstream, SERVICE_EDGES, 3)
        });
    });
}

/// BENCH cx_depends_upstream:
fn cx_depends_upstream(c: &mut Criterion) {
    let graph = build_multi_service_graph(10, 10_000);

    let last = graph.nodes.iter().rposition(|n| n.kind == NodeKind::Deployable as u8).unwrap() as u32;

    c.bench_function("cx_depends_upstream_100k", |b| {
        b.iter(|| {
            depends::depends(&graph, last, DependsDirection::Upstream, SERVICE_EDGES, 10)
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = cx_path_5_hops, cx_path_10_hops, cx_depends_depth3, cx_depends_upstream
}
criterion_main!(benches);
