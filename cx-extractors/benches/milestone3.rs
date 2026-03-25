use criterion::{criterion_group, criterion_main, Criterion};
use cx_core::graph::string_interner::StringInterner;
use cx_extractors::grpc::{GrpcClientStub, GrpcServerRegistration};
use cx_extractors::proto::{self, ProtoService};

/// Generate a synthetic .proto file with `n_methods` RPC methods.
fn gen_proto(pkg: &str, service: &str, n_methods: usize) -> String {
    let mut s = format!("syntax = \"proto3\";\n\npackage {};\n\nservice {} {{\n", pkg, service);
    for i in 0..n_methods {
        s.push_str(&format!(
            "  rpc Method{} (Req{}) returns (Resp{});\n",
            i, i, i
        ));
    }
    s.push_str("}\n");
    s
}

/// BENCH proto_parse_throughput:
///   Parse 100 .proto files (average 200 lines each).
///   TARGET: < 200ms total.
fn proto_parse_throughput(c: &mut Criterion) {
    // 100 proto files, ~15 methods each ≈ 200 lines/file
    let protos: Vec<String> = (0..100)
        .map(|i| gen_proto(&format!("pkg{}", i), &format!("Service{}", i), 15))
        .collect();

    c.bench_function("proto_parse_100_files", |b| {
        b.iter(|| {
            let mut strings = StringInterner::new();
            let mut id = 0u32;
            let mut total_nodes = 0;
            for (i, source) in protos.iter().enumerate() {
                let result = proto::extract_proto(
                    source,
                    &format!("proto/service_{}.proto", i),
                    &mut strings,
                    &mut id,
                );
                total_nodes += result.nodes.len();
            }
            total_nodes
        });
    });
}

/// BENCH resolution_pass:
///   5 repos, 500 dangling edges to resolve.
///   TARGET: < 100ms.
fn resolution_pass(c: &mut Criterion) {
    // Build 500 client stubs across 5 repos, matching 500 server registrations
    let mut client_stubs: Vec<(String, Vec<GrpcClientStub>)> = Vec::new();
    let mut server_regs: Vec<(String, Vec<GrpcServerRegistration>)> = Vec::new();
    let mut proto_services: Vec<(String, Vec<ProtoService>)> = Vec::new();

    for repo in 0..5 {
        let repo_name = format!("repo-{}", repo);
        let mut stubs = Vec::new();
        let mut services = Vec::new();

        for svc in 0..100 {
            let svc_name = format!("Service{}_{}", repo, svc);
            stubs.push(GrpcClientStub {
                service_name: svc_name.clone(),
                file: format!("client_{}.go", svc),
                line: svc as u32 + 1,
            });
            services.push(ProtoService {
                package: format!("pkg{}", svc),
                name: svc_name.clone(),
                fqn: format!("pkg{}.{}", svc, svc_name),
                methods: vec!["Do".to_string()],
                file: format!("svc_{}.proto", svc),
            });
        }

        client_stubs.push((repo_name.clone(), stubs));
        proto_services.push((repo_name, services));
    }

    // Build server registrations: each repo serves its own services
    for repo in 0..5 {
        let repo_name = format!("repo-{}", repo);
        let regs: Vec<GrpcServerRegistration> = (0..100)
            .map(|svc| GrpcServerRegistration {
                service_name: format!("Service{}_{}", repo, svc),
                file: format!("server_{}.go", svc),
                line: svc + 1,
            })
            .collect();
        server_regs.push((repo_name, regs));
    }

    c.bench_function("resolution_pass_500_edges", |b| {
        b.iter(|| {
            cx_resolution::proto_matching::match_protos(
                &client_stubs,
                &server_regs,
                &proto_services,
            )
        });
    });
}

/// BENCH mcp_roundtrip:
///   Full MCP JSON-RPC roundtrip: parse request → execute query → serialize response.
///   TARGET: < 10ms for cx_path on 100K node graph.
///   (Measured at the handle_request level since we can't easily bench stdio)
fn mcp_roundtrip(c: &mut Criterion) {
    // Build a graph via pipeline
    let dir = tempfile::tempdir().unwrap();
    // Generate ~100 Go files
    for i in 0..100 {
        let content = format!(
            "package main\n\nfunc handler_{i}() {{ process_{i}() }}\nfunc process_{i}() {{}}\n"
        );
        std::fs::write(dir.path().join(format!("file_{}.go", i)), content).unwrap();
    }

    let result = cx_extractors::pipeline::index_directory(dir.path()).unwrap();

    // Write and reload via mmap (as MCP server would)
    let cx_dir = dir.path().join(".cx").join("graph");
    std::fs::create_dir_all(&cx_dir).unwrap();
    let graph_path = cx_dir.join("index.cxgraph");
    cx_core::store::mmap::write_graph(&result.graph, &graph_path).unwrap();
    let graph = cx_core::store::mmap::load_graph(&graph_path).unwrap();

    // Benchmark the underlying query operations that the MCP server calls.
    let mut finder = cx_core::query::path::PathFinder::new(graph.node_count());

    let mut group = c.benchmark_group("mcp_roundtrip");

    group.bench_function("cx_path_query", |b| {
        b.iter(|| {
            let start = graph.nodes.iter().position(|n| graph.strings.get(n.name) == "handler_0").map(|i| i as u32).unwrap_or(0);
            let results = finder.find_all_downstream(&graph, start, cx_core::graph::edges::ALL_EDGES, 20);
            results.len()
        });
    });

    group.bench_function("cx_search_query", |b| {
        b.iter(|| {
            let ids: Vec<cx_core::graph::nodes::StringId> = graph.nodes.iter().map(|n| n.name).collect();
            let index = cx_core::query::trigram::TrigramIndex::build(&ids, &graph.strings);
            let results = index.search("handler", &graph.strings);
            results.len()
        });
    });

    group.bench_function("cx_context_query", |b| {
        b.iter(|| {
            let kind_idx = cx_core::graph::kind_index::KindIndex::build(&graph);
            kind_idx.count(cx_core::graph::nodes::NodeKind::Symbol)
                + kind_idx.count(cx_core::graph::nodes::NodeKind::Deployable)
                + kind_idx.count(cx_core::graph::nodes::NodeKind::Module)
        });
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = proto_parse_throughput, resolution_pass, mcp_roundtrip
}
criterion_main!(benches);
