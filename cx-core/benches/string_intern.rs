use criterion::{criterion_group, criterion_main, Criterion};

fn bench_string_intern(_c: &mut Criterion) {}

criterion_group!(benches, bench_string_intern);
criterion_main!(benches);
