use criterion::{criterion_group, criterion_main, Criterion};
use cx_core::graph::nodes::StringId;
use cx_core::query::trigram::TrigramIndex;
use std::path::Path;

/// Generate a synthetic Go file with `funcs` functions, each ~10 lines.
fn gen_go_file(pkg: &str, funcs: usize) -> String {
    let mut s = format!("package {}\n\nimport \"fmt\"\n\n", pkg);
    for i in 0..funcs {
        s.push_str(&format!(
            "func handler_{i}(ctx Context, req Request) (Response, error) {{\n\
             \tfmt.Println(\"handling request\")\n\
             \tresult := process_{i}(req)\n\
             \tif result == nil {{\n\
             \t\treturn nil, fmt.Errorf(\"failed\")\n\
             \t}}\n\
             \treturn result, nil\n\
             }}\n\n\
             func process_{i}(req Request) *Response {{\n\
             \treturn &Response{{}}\n\
             }}\n\n",
        ));
    }
    s
}

/// Generate a synthetic Go repo with `files` files, each containing `funcs_per_file` functions.
fn gen_go_repo(dir: &Path, files: usize, funcs_per_file: usize) {
    for i in 0..files {
        let subdir = dir.join(format!("pkg{}", i / 50));
        std::fs::create_dir_all(&subdir).unwrap();
        let path = subdir.join(format!("file_{}.go", i));
        let content = gen_go_file(&format!("pkg{}", i / 50), funcs_per_file);
        std::fs::write(path, content).unwrap();
    }
}

/// Estimate LOC per file (each function is ~10 lines + package/import header).
fn loc_per_file(funcs: usize) -> usize {
    4 + funcs * 12
}

/// BENCH index_small_repo:
///   Go repo: 50 files, 5K LOC.
///   TARGET: < 500ms
fn index_small_repo(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let funcs = 5000 / (50 * 12); // ~8 funcs/file to get ~5K LOC
    gen_go_repo(dir.path(), 50, funcs.max(1));

    c.bench_function("index_small_repo_50files", |b| {
        b.iter(|| {
            cx_extractors::pipeline::index_directory(dir.path()).unwrap()
        });
    });
}

/// BENCH index_medium_repo:
///   Go repo: 500 files, 50K LOC.
///   TARGET: < 2s
fn index_medium_repo(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let funcs = 50_000 / (500 * 12); // ~8 funcs/file
    gen_go_repo(dir.path(), 500, funcs.max(1));

    let mut group = c.benchmark_group("index_medium");
    group.sample_size(10);
    group.bench_function("500files", |b| {
        b.iter(|| {
            cx_extractors::pipeline::index_directory(dir.path()).unwrap()
        });
    });
    group.finish();
}

/// BENCH index_large_repo:
///   Go repo: 5000 files, 500K LOC.
///   TARGET: < 10s
fn index_large_repo(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let funcs = 500_000 / (5000 * 12); // ~8 funcs/file
    gen_go_repo(dir.path(), 5000, funcs.max(1));

    let mut group = c.benchmark_group("index_large");
    group.sample_size(10);
    group.bench_function("5000files", |b| {
        b.iter(|| {
            cx_extractors::pipeline::index_directory(dir.path()).unwrap()
        });
    });
    group.finish();
}

/// BENCH parse_throughput:
///   Parse Go files with tree-sitter in parallel.
///   TARGET: > 100K LOC/second
fn parse_throughput(c: &mut Criterion) {
    // Generate 100 files, ~100 LOC each = 10K LOC
    let dir = tempfile::tempdir().unwrap();
    gen_go_repo(dir.path(), 100, 8);
    let total_loc = 100 * loc_per_file(8);

    c.bench_function("parse_throughput_10k_loc", |b| {
        b.iter(|| {
            cx_extractors::pipeline::index_directory(dir.path()).unwrap()
        });
    });

    // Print LOC for reference
    eprintln!("Total LOC for throughput bench: {}", total_loc);
}

/// BENCH extractor_throughput:
///   Run Go extractor on parsed tree-sitter output.
///   TARGET: > 200K LOC/second
fn extractor_throughput(c: &mut Criterion) {
    use cx_extractors::grammars::{self, Language};
    use cx_extractors::universal::ParsedFile;
    use cx_core::graph::string_interner::StringInterner;

    // Generate a large Go file
    let source = gen_go_file("bench", 100); // ~1200 LOC
    let lang = Language::Go.ts_language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).unwrap();
    let tree = parser.parse(source.as_bytes(), None).unwrap();
    let extractor = grammars::extractor_for_language(Language::Go).unwrap();

    c.bench_function("extractor_throughput_1200loc", |b| {
        b.iter(|| {
            let mut strings = StringInterner::new();
            let path_id = strings.intern("bench.go");
            let file = ParsedFile {
                tree: tree.clone(),
                source: source.as_bytes(),
                path: path_id,
                repo_id: 0,
            };
            let mut id = 0u32;
            extractor.extract(&file, &mut strings, &mut id)
        });
    });
}

/// BENCH cx_context_latency:
///   Index a repo. Load graph and run context.
///   TARGET: < 5ms
fn cx_context_latency(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    gen_go_repo(dir.path(), 100, 8);

    let result = cx_extractors::pipeline::index_directory(dir.path()).unwrap();

    // Write graph
    let cx_dir = dir.path().join(".cx").join("graph");
    std::fs::create_dir_all(&cx_dir).unwrap();
    let graph_path = cx_dir.join("index.cxgraph");
    cx_core::store::mmap::write_graph(&result.graph, &graph_path).unwrap();

    // Load graph once (this is what cx context does)
    let graph = cx_core::store::mmap::load_graph(&graph_path).unwrap();

    c.bench_function("cx_context_latency", |b| {
        b.iter(|| {
            // Simulate what cx context does: build KindIndex + collect names
            let kind_idx = cx_core::graph::kind_index::KindIndex::build(&graph);
            let _symbols = kind_idx.count(cx_core::graph::nodes::NodeKind::Symbol);
            let _modules = kind_idx.count(cx_core::graph::nodes::NodeKind::Module);
        });
    });
}

/// BENCH cx_search_latency:
///   Index a repo with ~5K symbols. Run cx search.
///   TARGET: < 10ms
fn cx_search_latency(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    // ~50 files * 50 funcs = 2500 functions * 2 (handler + process) = ~5000 symbols
    gen_go_repo(dir.path(), 50, 50);

    let result = cx_extractors::pipeline::index_directory(dir.path()).unwrap();

    // Build trigram index
    let symbol_ids: Vec<StringId> = result.graph.nodes.iter().map(|n| n.name).collect();
    let index = TrigramIndex::build(&symbol_ids, &result.graph.strings);

    c.bench_function("cx_search_latency_5k_symbols", |b| {
        b.iter(|| {
            let results = index.search("handler_42", &result.graph.strings);
            results.len()
        });
    });
}

/// BENCH index_scaling:
///   Same 50K LOC Go repo. Run with RAYON_NUM_THREADS=1, 2, 4, 8, 16.
///   Report throughput (LOC/sec) at each thread count.
fn index_scaling(c: &mut Criterion) {
    use criterion::BenchmarkId;

    let dir = tempfile::tempdir().unwrap();
    let funcs_per_file = 8;
    let num_files = 500;
    gen_go_repo(dir.path(), num_files, funcs_per_file);
    let total_loc = num_files * loc_per_file(funcs_per_file);

    let mut group = c.benchmark_group("index_scaling");
    group.sample_size(10);

    for threads in [1, 2, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}_threads", threads)),
            &threads,
            |b, &t| {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(t)
                    .build()
                    .unwrap();
                b.iter(|| {
                    pool.install(|| {
                        cx_extractors::pipeline::index_directory(dir.path()).unwrap()
                    })
                });
            },
        );
    }

    group.finish();
    eprintln!("Total LOC for scaling bench: {}", total_loc);
}

criterion_group! {
    name = small_benches;
    config = Criterion::default();
    targets = index_small_repo, parse_throughput, extractor_throughput, cx_context_latency, cx_search_latency
}

criterion_group! {
    name = large_benches;
    config = Criterion::default().sample_size(10);
    targets = index_medium_repo, index_large_repo, index_scaling
}

criterion_main!(small_benches, large_benches);
