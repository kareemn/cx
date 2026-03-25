use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::NodeKind;
use cx_core::graph::string_interner::StringInterner;
use cx_extractors::grammars::{self, Language};
use cx_extractors::universal::ParsedFile;

fn extract_cpp(source: &str, path: &str) -> (cx_extractors::universal::ExtractionResult, StringInterner) {
    let lang = Language::Cpp;
    let ts_lang = lang.ts_language();
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&ts_lang).unwrap();
    let tree = parser.parse(source.as_bytes(), None).unwrap();
    let extractor = grammars::extractor_for_language(lang).unwrap();
    let mut strings = StringInterner::new();
    let path_id = strings.intern(path);
    let file = ParsedFile {
        tree,
        source: source.as_bytes(),
        path: path_id,
        path_str: path,
        repo_id: 0,
    };
    let mut id = 0u32;
    let result = extractor.extract(&file, &mut strings, &mut id);
    (result, strings)
}

fn func_names<'a>(
    result: &'a cx_extractors::universal::ExtractionResult,
    strings: &'a StringInterner,
) -> Vec<&'a str> {
    result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
        .map(|n| strings.get(n.name))
        .collect()
}

fn type_names<'a>(
    result: &'a cx_extractors::universal::ExtractionResult,
    strings: &'a StringInterner,
) -> Vec<&'a str> {
    result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 1)
        .map(|n| strings.get(n.name))
        .collect()
}

fn edge_count(result: &cx_extractors::universal::ExtractionResult, kind: EdgeKind) -> usize {
    result.edges.iter().filter(|e| e.kind == kind).count()
}

#[test]
fn cpp_functions_and_classes() {
    let source = r#"
#include <iostream>
#include "server.h"

class Server {
public:
    void start() {
        listen();
    }
    void stop() {}
};

struct Config {
    int port;
};

enum Status { OK, ERROR };

void listen() {}

void run() {
    Server s;
    s.start();
}
"#;
    let (result, strings) = extract_cpp(source, "server.cpp");

    // Functions: start, stop, listen, run
    let funcs = func_names(&result, &strings);
    assert!(funcs.contains(&"start"), "should find start method, got: {:?}", funcs);
    assert!(funcs.contains(&"stop"), "should find stop method");
    assert!(funcs.contains(&"listen"), "should find listen function");
    assert!(funcs.contains(&"run"), "should find run function");
    assert_eq!(funcs.len(), 4, "should find exactly 4 functions, got: {:?}", funcs);

    // Types: Server (class), Config (struct), Status (enum)
    let types = type_names(&result, &strings);
    assert_eq!(types.len(), 3, "should find 3 types, got: {:?}", types);
    assert!(types.contains(&"Server"));
    assert!(types.contains(&"Config"));
    assert!(types.contains(&"Status"));
}

#[test]
fn cpp_call_edges() {
    let source = r#"
void cleanup() {}

void stop() {
    cleanup();
}

void start() {
    stop();
    cleanup();
}
"#;
    let (result, strings) = extract_cpp(source, "main.cpp");

    let cleanup_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "cleanup")
        .map(|n| n.id).unwrap();
    let stop_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "stop")
        .map(|n| n.id).unwrap();
    let start_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "start")
        .map(|n| n.id).unwrap();

    assert!(result.edges.iter().any(|e| e.source == stop_id && e.target == cleanup_id && e.kind == EdgeKind::Calls));
    assert!(result.edges.iter().any(|e| e.source == start_id && e.target == stop_id && e.kind == EdgeKind::Calls));
    assert!(result.edges.iter().any(|e| e.source == start_id && e.target == cleanup_id && e.kind == EdgeKind::Calls));
    assert_eq!(edge_count(&result, EdgeKind::Calls), 3);
}

#[test]
fn cpp_include_edges() {
    let source = r#"
#include <iostream>
#include <vector>
#include "config.h"

void main() {}
"#;
    let (result, _strings) = extract_cpp(source, "main.cpp");

    assert_eq!(
        edge_count(&result, EdgeKind::Imports),
        3,
        "should have 3 import edges for 3 includes"
    );
}

#[test]
fn cpp_namespace_as_package() {
    let source = r#"
namespace net {

void listen() {}

void serve() {
    listen();
}

}
"#;
    let (result, strings) = extract_cpp(source, "net.cpp");

    // Namespace creates a Module node
    let modules: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Module as u8)
        .map(|n| strings.get(n.name))
        .collect();
    assert!(modules.contains(&"net"), "should find 'net' namespace as module, got: {:?}", modules);

    // Contains edges from namespace to functions
    assert!(edge_count(&result, EdgeKind::Contains) >= 2, "namespace should contain functions");
}

#[test]
fn cpp_method_call_via_field_expression() {
    let source = r#"
class Db {
public:
    void connect() {}
};

void init() {
    Db db;
    db.connect();
}
"#;
    let (result, strings) = extract_cpp(source, "db.cpp");

    let funcs = func_names(&result, &strings);
    assert!(funcs.contains(&"connect"));
    assert!(funcs.contains(&"init"));

    // init calls connect via field expression
    let connect_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "connect")
        .map(|n| n.id).unwrap();
    let init_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "init")
        .map(|n| n.id).unwrap();

    assert!(
        result.edges.iter().any(|e| e.source == init_id && e.target == connect_id && e.kind == EdgeKind::Calls),
        "init should call connect via db.connect()"
    );
}

#[test]
fn cpp_class_with_methods() {
    let source = r#"
class Router {
public:
    void addRoute() {
        validate();
    }
    void validate() {}
    void serve() {
        addRoute();
    }
};
"#;
    let (result, strings) = extract_cpp(source, "router.cpp");

    let funcs = func_names(&result, &strings);
    assert_eq!(funcs.len(), 3, "should find 3 methods: {:?}", funcs);
    assert!(funcs.contains(&"addRoute"));
    assert!(funcs.contains(&"validate"));
    assert!(funcs.contains(&"serve"));

    // addRoute → validate, serve → addRoute
    assert_eq!(edge_count(&result, EdgeKind::Calls), 2);
}
