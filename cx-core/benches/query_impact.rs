mod bench_helpers;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use cx_core::graph::edges::ALL_EDGES;
use cx_core::query::bfs::{BfsState, Direction};

/// Impact traversal at various depths on a 100K-node graph.
/// cx_impact is BFS collecting all reachable nodes — measures blast radius analysis.
fn impact_traversal_100k(c: &mut Criterion) {
    let graph = bench_helpers::gen_graph(100_000, 1_000_000);
    let mut bfs = BfsState::new(graph.node_count());

    let mut group = c.benchmark_group("impact_100k");

    for depth in [1, 2, 3, 5, 10] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("depth_{}", depth)),
            &depth,
            |b, &d| {
                b.iter(|| {
                    bfs.run(&graph, &[0], ALL_EDGES, d, Direction::Downstream);
                    bfs.result().len()
                });
            },
        );
    }

    group.finish();
}

/// Impact traversal at various depths on a 1M-node graph.
/// TARGET for depth 5: < 5ms.
fn impact_traversal_1m(c: &mut Criterion) {
    let graph = bench_helpers::gen_graph(1_000_000, 10_000_000);
    let mut bfs = BfsState::new(graph.node_count());

    let mut group = c.benchmark_group("impact_1m");
    group.sample_size(10);

    for depth in [1, 3, 5] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("depth_{}", depth)),
            &depth,
            |b, &d| {
                b.iter(|| {
                    bfs.run(&graph, &[0], ALL_EDGES, d, Direction::Downstream);
                    bfs.result().len()
                });
            },
        );
    }

    group.finish();
}

/// Upstream impact (who is affected if this node changes).
fn impact_upstream_100k(c: &mut Criterion) {
    let graph = bench_helpers::gen_graph(100_000, 1_000_000);
    let mut bfs = BfsState::new(graph.node_count());

    let mut group = c.benchmark_group("impact_upstream_100k");

    for depth in [1, 3, 5] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("depth_{}", depth)),
            &depth,
            |b, &d| {
                b.iter(|| {
                    bfs.run(&graph, &[500], ALL_EDGES, d, Direction::Upstream);
                    bfs.result().len()
                });
            },
        );
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = impact_traversal_100k, impact_traversal_1m, impact_upstream_100k
}
criterion_main!(benches);
