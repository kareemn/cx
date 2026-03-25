use criterion::{criterion_group, criterion_main, Criterion};

fn bench_graph_loading(_c: &mut Criterion) {}

criterion_group!(benches, bench_graph_loading);
criterion_main!(benches);
