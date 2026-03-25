use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::NodeKind;
use cx_core::graph::string_interner::StringInterner;
use cx_extractors::grammars::{self, Language};
use cx_extractors::universal::ParsedFile;

fn extract_python(source: &str, path: &str) -> (cx_extractors::universal::ExtractionResult, StringInterner) {
    let lang = Language::Python;
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

fn node_names<'a>(
    result: &'a cx_extractors::universal::ExtractionResult,
    strings: &'a StringInterner,
    kind: NodeKind,
) -> Vec<&'a str> {
    result
        .nodes
        .iter()
        .filter(|n| n.kind == kind as u8)
        .map(|n| strings.get(n.name))
        .collect()
}

fn symbol_names<'a>(
    result: &'a cx_extractors::universal::ExtractionResult,
    strings: &'a StringInterner,
) -> Vec<&'a str> {
    node_names(result, strings, NodeKind::Symbol)
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
fn python_functions_and_classes() {
    let source = r#"
import os
from pathlib import Path

class Server:
    def __init__(self, port):
        self.port = port

    def start(self):
        listen()

class Handler:
    def handle(self):
        pass

def listen():
    pass

def helper():
    listen()
"#;
    let (result, strings) = extract_python(source, "server.py");

    // Functions: __init__, start, handle, listen, helper
    let funcs: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
        .map(|n| strings.get(n.name))
        .collect();
    assert_eq!(funcs.len(), 5, "should find 5 functions, got: {:?}", funcs);
    assert!(funcs.contains(&"__init__"));
    assert!(funcs.contains(&"start"));
    assert!(funcs.contains(&"handle"));
    assert!(funcs.contains(&"listen"));
    assert!(funcs.contains(&"helper"));

    // Classes: Server, Handler
    let types = type_names(&result, &strings);
    assert_eq!(types.len(), 2, "should find 2 classes, got: {:?}", types);
    assert!(types.contains(&"Server"));
    assert!(types.contains(&"Handler"));
}

#[test]
fn python_call_edges() {
    let source = r#"
def run():
    pass

def start():
    run()

def helper():
    run()
"#;
    let (result, strings) = extract_python(source, "app.py");

    let run_id = result
        .nodes
        .iter()
        .find(|n| strings.get(n.name) == "run" && n.kind == NodeKind::Symbol as u8)
        .map(|n| n.id)
        .unwrap();
    let start_id = result
        .nodes
        .iter()
        .find(|n| strings.get(n.name) == "start" && n.kind == NodeKind::Symbol as u8)
        .map(|n| n.id)
        .unwrap();
    let helper_id = result
        .nodes
        .iter()
        .find(|n| strings.get(n.name) == "helper" && n.kind == NodeKind::Symbol as u8)
        .map(|n| n.id)
        .unwrap();

    // start → run
    assert!(
        result.edges.iter().any(|e| e.source == start_id && e.target == run_id && e.kind == EdgeKind::Calls),
        "start should call run"
    );
    // helper → run
    assert!(
        result.edges.iter().any(|e| e.source == helper_id && e.target == run_id && e.kind == EdgeKind::Calls),
        "helper should call run"
    );

    assert_eq!(edge_count(&result, EdgeKind::Calls), 2, "should have exactly 2 call edges");
}

#[test]
fn python_import_edges() {
    let source = r#"
import os
from pathlib import Path
from collections import OrderedDict

def main():
    pass
"#;
    let (result, _strings) = extract_python(source, "app.py");

    // 3 import statements: "os", "pathlib", "collections"
    assert_eq!(
        edge_count(&result, EdgeKind::Imports),
        3,
        "should have 3 import edges"
    );
}

#[test]
fn python_decorated_functions() {
    let source = r#"
def my_decorator(f):
    return f

@my_decorator
def decorated():
    pass

def plain():
    decorated()
"#;
    let (result, strings) = extract_python(source, "deco.py");

    let funcs = symbol_names(&result, &strings);
    assert!(funcs.contains(&"my_decorator"));
    assert!(funcs.contains(&"decorated"));
    assert!(funcs.contains(&"plain"));
}

#[test]
fn python_method_calls_within_class() {
    let source = r#"
def connect():
    pass

class Client:
    def init(self):
        connect()

    def run(self):
        connect()
"#;
    let (result, strings) = extract_python(source, "client.py");

    let connect_id = result
        .nodes
        .iter()
        .find(|n| strings.get(n.name) == "connect" && n.kind == NodeKind::Symbol as u8)
        .map(|n| n.id)
        .unwrap();

    // Both init and run call connect
    let calls_to_connect = result
        .edges
        .iter()
        .filter(|e| e.target == connect_id && e.kind == EdgeKind::Calls)
        .count();
    assert_eq!(calls_to_connect, 2, "init and run should both call connect");
}
