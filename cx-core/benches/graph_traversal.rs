use criterion::{criterion_group, criterion_main, Criterion};

fn bench_graph_traversal(_c: &mut Criterion) {}

criterion_group!(benches, bench_graph_traversal);
criterion_main!(benches);
