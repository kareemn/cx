mod bench_helpers;

use criterion::{criterion_group, criterion_main, Criterion};
use cx_core::graph::edges::{Edge, EdgeKind, EdgeKindMask, ALL_EDGES, SERVICE_EDGES};
use cx_core::graph::kind_index::KindIndex;
use cx_core::graph::nodes::NodeKind;
use cx_core::query::bfs::{BfsState, Direction};

/// BENCH summary_graph_query:
///   Summary graph: 200 Deployable nodes, 500 edges. BFS depth 3.
///   TARGET: < 10μs (microseconds). Near-instant.
fn summary_graph_query(c: &mut Criterion) {
    let graph = bench_helpers::gen_summary_graph(200, 500);
    let mut bfs = BfsState::new(graph.node_count());

    c.bench_function("summary_graph_query_200n", |b| {
        b.iter(|| {
            bfs.run(&graph, &[0], ALL_EDGES, 3, Direction::Downstream);
            bfs.result().len()
        });
    });
}

/// BENCH kind_index_lookup:
///   Graph: 1M nodes. Find all Endpoints (expect ~5K).
///   TARGET: < 1μs for the index lookup (reading two u32 values).
fn kind_index_lookup(c: &mut Criterion) {
    let graph = bench_helpers::gen_mixed_graph(1_000_000, 1_000_000);
    let kind_idx = KindIndex::build(&graph);

    c.bench_function("kind_index_lookup_1m", |b| {
        b.iter(|| {
            let (start, end) = kind_idx.range(NodeKind::Endpoint);
            // Force the compiler to use the result
            assert!(end >= start);
            (end - start) as usize
        });
    });
}

/// BENCH edge_sorted_filter_vs_unsorted:
///   Node with 200 edges. Filter for SERVICE_EDGES only (expect ~30 edges).
///   Compare sorted (binary search to range) vs unsorted (linear scan with bitmask).
///   TARGET: sorted is at least 30% faster when <20% of edges match filter.
fn edge_sorted_filter_vs_unsorted(c: &mut Criterion) {
    // Build sorted edges: mostly Contains/Calls/Imports with some DependsOn/Exposes/Consumes
    let sorted_edges: Vec<Edge> = {
        let mut edges = Vec::with_capacity(200);
        // 170 non-service edges (kinds 0-2)
        for i in 0..60u32 {
            edges.push(Edge::new(i, EdgeKind::Contains));
        }
        for i in 60..120u32 {
            edges.push(Edge::new(i, EdgeKind::Calls));
        }
        for i in 120..170u32 {
            edges.push(Edge::new(i, EdgeKind::Imports));
        }
        // 30 service edges (kinds 3-5)
        for i in 170..185u32 {
            edges.push(Edge::new(i, EdgeKind::DependsOn));
        }
        for i in 185..195u32 {
            edges.push(Edge::new(i, EdgeKind::Exposes));
        }
        for i in 195..200u32 {
            edges.push(Edge::new(i, EdgeKind::Consumes));
        }
        edges
    };

    // Unsorted: same edges but shuffled deterministically
    let unsorted_edges: Vec<Edge> = {
        let mut edges = sorted_edges.clone();
        // Deterministic shuffle via index remapping
        let n = edges.len();
        for i in 0..n {
            let j = (i * 97 + 31) % n;
            edges.swap(i, j);
        }
        edges
    };

    let mut group = c.benchmark_group("edge_filter_200");

    group.bench_function("sorted_binary_search", |b| {
        b.iter(|| {
            filter_sorted(&sorted_edges, SERVICE_EDGES)
        });
    });

    group.bench_function("unsorted_linear_scan", |b| {
        b.iter(|| {
            filter_linear(&unsorted_edges, SERVICE_EDGES)
        });
    });

    group.finish();
}

/// Filter sorted edges using binary search to find the kind range.
fn filter_sorted(edges: &[Edge], mask: EdgeKindMask) -> usize {
    let mut count = 0;
    // For each set bit in the mask, binary search for that kind
    for kind_val in 0..11u8 {
        if (1u16 << kind_val) & mask == 0 {
            continue;
        }
        // Binary search for first edge of this kind
        let start = edges.partition_point(|e| e.kind < kind_val);
        // Scan until kind changes
        let mut i = start;
        while i < edges.len() && edges[i].kind == kind_val {
            count += 1;
            i += 1;
        }
    }
    count
}

/// Filter unsorted edges with linear scan + bitmask.
fn filter_linear(edges: &[Edge], mask: EdgeKindMask) -> usize {
    let mut count = 0;
    for edge in edges {
        if (1u16 << edge.kind) & mask != 0 {
            count += 1;
        }
    }
    count
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = summary_graph_query, kind_index_lookup, edge_sorted_filter_vs_unsorted
}
criterion_main!(benches);
