use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::NodeKind;
use cx_core::graph::string_interner::StringInterner;
use cx_extractors::grammars::{self, Language};
use cx_extractors::universal::ParsedFile;

fn extract_c(source: &str, path: &str) -> (cx_extractors::universal::ExtractionResult, StringInterner) {
    let lang = Language::C;
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
fn c_functions_and_structs() {
    let source = r#"
#include <stdio.h>
#include "server.h"

struct Server {
    int port;
    char* host;
};

typedef struct { int x; int y; } Point;

enum Color { RED, GREEN, BLUE };

int start(int port) {
    printf("starting %d\n", port);
    return 0;
}

void stop() {
    cleanup();
}

void cleanup() {}
"#;
    let (result, strings) = extract_c(source, "server.c");

    // Functions: start, stop, cleanup
    let funcs = func_names(&result, &strings);
    assert_eq!(funcs.len(), 3, "should find 3 functions, got: {:?}", funcs);
    assert!(funcs.contains(&"start"));
    assert!(funcs.contains(&"stop"));
    assert!(funcs.contains(&"cleanup"));

    // Types: Server (struct), Point (typedef), Color (enum)
    let types = type_names(&result, &strings);
    assert_eq!(types.len(), 3, "should find 3 types, got: {:?}", types);
    assert!(types.contains(&"Server"));
    assert!(types.contains(&"Point"));
    assert!(types.contains(&"Color"));
}

#[test]
fn c_call_edges() {
    let source = r#"
void cleanup() {}

void stop() {
    cleanup();
}

int start() {
    stop();
    cleanup();
    return 0;
}
"#;
    let (result, strings) = extract_c(source, "main.c");

    let cleanup_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "cleanup")
        .map(|n| n.id).unwrap();
    let stop_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "stop")
        .map(|n| n.id).unwrap();
    let start_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "start")
        .map(|n| n.id).unwrap();

    // stop → cleanup
    assert!(result.edges.iter().any(|e| e.source == stop_id && e.target == cleanup_id && e.kind == EdgeKind::Calls));
    // start → stop
    assert!(result.edges.iter().any(|e| e.source == start_id && e.target == stop_id && e.kind == EdgeKind::Calls));
    // start → cleanup
    assert!(result.edges.iter().any(|e| e.source == start_id && e.target == cleanup_id && e.kind == EdgeKind::Calls));

    assert_eq!(edge_count(&result, EdgeKind::Calls), 3);
}

#[test]
fn c_include_edges() {
    let source = r#"
#include <stdio.h>
#include <stdlib.h>
#include "myheader.h"

void main() {}
"#;
    let (result, _strings) = extract_c(source, "main.c");

    // 3 includes = 3 import edges
    assert_eq!(
        edge_count(&result, EdgeKind::Imports),
        3,
        "should have 3 import edges for 3 includes"
    );
}

#[test]
fn c_typedef_struct() {
    let source = r#"
typedef struct {
    double real;
    double imag;
} Complex;

typedef int ErrorCode;

void compute() {}
"#;
    let (result, strings) = extract_c(source, "types.c");

    let types = type_names(&result, &strings);
    assert!(types.contains(&"Complex"), "should find Complex typedef, got: {:?}", types);
    assert!(types.contains(&"ErrorCode"), "should find ErrorCode typedef");
}

#[test]
fn c_field_expression_calls() {
    let source = r#"
struct Ops {
    int x;
};

void init() {}

void run() {
    init();
}
"#;
    let (result, strings) = extract_c(source, "ops.c");

    let init_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "init")
        .map(|n| n.id).unwrap();
    let run_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "run")
        .map(|n| n.id).unwrap();

    assert!(result.edges.iter().any(|e| e.source == run_id && e.target == init_id && e.kind == EdgeKind::Calls));
}
