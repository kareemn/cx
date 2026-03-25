use criterion::{criterion_group, criterion_main, Criterion};

fn bench_query_path(_c: &mut Criterion) {}

criterion_group!(benches, bench_query_path);
criterion_main!(benches);
