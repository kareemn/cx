use criterion::{criterion_group, criterion_main, Criterion};
use cx_core::graph::bitvec::BitVec;
use cx_core::graph::nodes::StringId;
use cx_core::graph::string_interner::StringInterner;
use cx_core::query::trigram::TrigramIndex;

/// BENCH string_intern_throughput:
///   Intern 1M unique strings (average 30 chars each).
///   TARGET: > 2M interns/second.
fn string_intern_throughput(c: &mut Criterion) {
    // Pre-generate strings to avoid measuring format! overhead
    let strings: Vec<String> = (0..1_000_000)
        .map(|i| format!("com.example.service.symbol_{:08}", i))
        .collect();

    c.bench_function("string_intern_1m", |b| {
        b.iter(|| {
            let mut interner = StringInterner::with_capacity(30_000_000);
            for s in &strings {
                interner.intern(s);
            }
            interner.len()
        });
    });
}

/// BENCH string_lookup_throughput:
///   Lookup 1M StringIds in packed interner.
///   TARGET: > 10M lookups/second.
fn string_lookup_throughput(c: &mut Criterion) {
    let mut interner = StringInterner::with_capacity(30_000_000);
    let ids: Vec<StringId> = (0..1_000_000)
        .map(|i| interner.intern(&format!("com.example.service.symbol_{:08}", i)))
        .collect();

    c.bench_function("string_lookup_1m", |b| {
        b.iter(|| {
            let mut total_len = 0usize;
            for &id in &ids {
                total_len += interner.get(id).len();
            }
            total_len
        });
    });
}

/// BENCH bitvec_set_test:
///   Set and test 1M nodes in sequence.
///   TARGET: > 100M operations/second.
fn bitvec_set_test(c: &mut Criterion) {
    c.bench_function("bitvec_set_test_1m", |b| {
        let mut bv = BitVec::new(1_000_000);
        b.iter(|| {
            bv.clear();
            for i in 0..1_000_000u32 {
                bv.set(i);
            }
            let mut count = 0u32;
            for i in 0..1_000_000u32 {
                if bv.test(i) {
                    count += 1;
                }
            }
            count
        });
    });
}

/// BENCH trigram_index_build:
///   Build trigram index over symbol names.
fn trigram_index_build(c: &mut Criterion) {
    let mut interner = StringInterner::with_capacity(10_000_000);
    let ids: Vec<StringId> = (0..100_000)
        .map(|i| interner.intern(&format!("handleServiceRequest_{:06}", i)))
        .collect();

    c.bench_function("trigram_build_100k", |b| {
        b.iter(|| {
            let idx = TrigramIndex::build(&ids, &interner);
            idx
        });
    });
}

/// BENCH trigram_search:
///   Search trigram index for matching symbols.
fn trigram_search(c: &mut Criterion) {
    let mut interner = StringInterner::with_capacity(10_000_000);
    let ids: Vec<StringId> = (0..100_000)
        .map(|i| interner.intern(&format!("handleServiceRequest_{:06}", i)))
        .collect();
    let index = TrigramIndex::build(&ids, &interner);

    c.bench_function("trigram_search_100k", |b| {
        b.iter(|| {
            let results = index.search("ServiceRequest", &interner);
            results.len()
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = string_intern_throughput, string_lookup_throughput, bitvec_set_test, trigram_index_build, trigram_search
}
criterion_main!(benches);
