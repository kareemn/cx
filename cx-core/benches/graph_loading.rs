mod bench_helpers;

use criterion::{criterion_group, criterion_main, Criterion};
use cx_core::store::mmap;

/// BENCH graph_load_mmap:
///   Generate graph: 100K nodes, 1M edges. Write to disk. Mmap load.
///   TARGET: < 10ms
fn graph_load_mmap(c: &mut Criterion) {
    let graph = bench_helpers::gen_graph(100_000, 1_000_000);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench_100k.cxgraph");
    mmap::write_graph(&graph, &path).unwrap();

    c.bench_function("graph_load_mmap_100k", |b| {
        b.iter(|| {
            let _g = mmap::load_graph(&path).unwrap();
        });
    });
}

/// BENCH graph_load_mmap_large:
///   Generate graph: 1M nodes, 10M edges (~350MB file). Write to disk. Mmap load.
///   TARGET: < 50ms
fn graph_load_mmap_large(c: &mut Criterion) {
    let graph = bench_helpers::gen_graph(1_000_000, 10_000_000);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench_1m.cxgraph");
    mmap::write_graph(&graph, &path).unwrap();

    c.bench_function("graph_load_mmap_1m", |b| {
        b.iter(|| {
            let _g = mmap::load_graph(&path).unwrap();
        });
    });
}

/// BENCH mmap_cold_start:
///   Mmap graph file (100MB). Run first query (pages faulted in by OS).
///   TARGET: < 200ms for cold start.
///   NOTE: In CI/warm caches this will be much faster; the target is for cold page-in.
fn mmap_cold_start(c: &mut Criterion) {
    use cx_core::graph::edges::ALL_EDGES;
    use cx_core::query::bfs::{BfsState, Direction};

    let graph = bench_helpers::gen_graph(100_000, 1_000_000);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench_cold.cxgraph");
    mmap::write_graph(&graph, &path).unwrap();

    c.bench_function("mmap_cold_start", |b| {
        b.iter(|| {
            let g = mmap::load_graph(&path).unwrap();
            // Run a query to force page-in
            let mut bfs = BfsState::new(g.node_count());
            bfs.run(&g, &[0], ALL_EDGES, 3, Direction::Downstream);
            bfs.result().len()
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = graph_load_mmap, graph_load_mmap_large, mmap_cold_start
}
criterion_main!(benches);
