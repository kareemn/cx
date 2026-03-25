mod bench_helpers;

use criterion::{criterion_group, criterion_main, Criterion};
use cx_core::graph::bitvec::BitVec;
use cx_core::graph::edges::{ALL_EDGES, SERVICE_EDGES};
use cx_core::graph::nodes::NodeId;
use cx_core::query::bfs::{BfsState, Direction};
use std::collections::VecDeque;

/// BENCH bfs_100k_nodes:
///   Random graph: 100K nodes, 1M edges. BFS from random seed, depth unlimited.
///   TARGET: < 1ms (median over 1000 runs).
fn bfs_100k_nodes(c: &mut Criterion) {
    let graph = bench_helpers::gen_graph(100_000, 1_000_000);
    let mut bfs = BfsState::new(graph.node_count());

    c.bench_function("bfs_100k_nodes", |b| {
        b.iter(|| {
            bfs.run(&graph, &[0], ALL_EDGES, u32::MAX, Direction::Downstream);
            bfs.result().len()
        });
    });
}

/// BENCH bfs_1m_nodes:
///   Random graph: 1M nodes, 10M edges. BFS from random seed, depth 5.
///   TARGET: < 5ms (median).
fn bfs_1m_nodes(c: &mut Criterion) {
    let graph = bench_helpers::gen_graph(1_000_000, 10_000_000);
    let mut bfs = BfsState::new(graph.node_count());

    c.bench_function("bfs_1m_nodes_depth5", |b| {
        b.iter(|| {
            bfs.run(&graph, &[0], ALL_EDGES, 5, Direction::Downstream);
            bfs.result().len()
        });
    });
}

/// BENCH bfs_filtered:
///   Random graph: 1M nodes, 10M edges (mixed edge kinds). BFS with SERVICE_EDGES mask.
///   TARGET: < 5ms (median). Bitmask filtering has <5% overhead vs unfiltered.
fn bfs_filtered(c: &mut Criterion) {
    let graph = bench_helpers::gen_graph(1_000_000, 10_000_000);
    let mut bfs = BfsState::new(graph.node_count());

    let mut group = c.benchmark_group("bfs_filtered_1m");

    group.bench_function("all_edges", |b| {
        b.iter(|| {
            bfs.run(&graph, &[0], ALL_EDGES, 5, Direction::Downstream);
            bfs.result().len()
        });
    });

    group.bench_function("service_edges", |b| {
        b.iter(|| {
            bfs.run(&graph, &[0], SERVICE_EDGES, 5, Direction::Downstream);
            bfs.result().len()
        });
    });

    group.finish();
}

/// BENCH bfs_double_buffer_vs_vecdeque:
///   Same graph, same query. Compare BfsState (double-buffer) vs VecDeque-based BFS.
///   TARGET: double-buffer is at least 20% faster.
fn bfs_double_buffer_vs_vecdeque(c: &mut Criterion) {
    let graph = bench_helpers::gen_graph(100_000, 1_000_000);
    let mut bfs = BfsState::new(graph.node_count());

    let mut group = c.benchmark_group("bfs_impl_compare");

    group.bench_function("double_buffer", |b| {
        b.iter(|| {
            bfs.run(&graph, &[0], ALL_EDGES, u32::MAX, Direction::Downstream);
            bfs.result().len()
        });
    });

    group.bench_function("vecdeque", |b| {
        b.iter(|| {
            vecdeque_bfs(&graph, 0, ALL_EDGES)
        });
    });

    group.finish();
}

/// VecDeque-based BFS for comparison.
fn vecdeque_bfs(
    graph: &cx_core::graph::csr::CsrGraph,
    seed: NodeId,
    mask: cx_core::graph::edges::EdgeKindMask,
) -> usize {
    let mut visited = BitVec::new(graph.node_count());
    let mut queue = VecDeque::new();
    let mut count = 0usize;

    visited.set(seed);
    queue.push_back(seed);

    while let Some(node) = queue.pop_front() {
        count += 1;
        for edge in graph.edges_for(node) {
            if (1u16 << edge.kind) & mask == 0 {
                continue;
            }
            if visited.test(edge.target) {
                continue;
            }
            visited.set(edge.target);
            queue.push_back(edge.target);
        }
    }
    count
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bfs_100k_nodes, bfs_1m_nodes, bfs_filtered, bfs_double_buffer_vs_vecdeque
}
criterion_main!(benches);
