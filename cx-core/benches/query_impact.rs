use criterion::{criterion_group, criterion_main, Criterion};

fn bench_query_impact(_c: &mut Criterion) {}

criterion_group!(benches, bench_query_impact);
criterion_main!(benches);
