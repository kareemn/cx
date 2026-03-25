use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::NodeKind;
use cx_core::graph::string_interner::StringInterner;
use cx_extractors::grammars::{self, Language};
use cx_extractors::universal::ParsedFile;

fn extract_ts(source: &str, path: &str) -> (cx_extractors::universal::ExtractionResult, StringInterner) {
    let lang = Language::TypeScript;
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

fn symbol_names<'a>(
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
fn typescript_functions_and_classes() {
    let source = r#"
import { Router } from 'express';

class Server {
    start() {
        listen();
    }
    stop() {}
}

class Handler {
    handle() {}
}

function listen() {}

const handler = () => {
    listen();
};

const processor = function process() {};
"#;
    let (result, strings) = extract_ts(source, "server.ts");

    // Functions: start, stop, handle, listen, handler, processor
    let funcs = symbol_names(&result, &strings);
    assert!(funcs.contains(&"start"), "should find start method, got: {:?}", funcs);
    assert!(funcs.contains(&"stop"), "should find stop method");
    assert!(funcs.contains(&"handle"), "should find handle method");
    assert!(funcs.contains(&"listen"), "should find listen function");
    assert!(funcs.contains(&"handler"), "should find handler arrow fn");
    assert!(funcs.contains(&"processor"), "should find processor fn expr");

    // Classes: Server, Handler
    let types = type_names(&result, &strings);
    assert_eq!(types.len(), 2, "should find 2 classes, got: {:?}", types);
    assert!(types.contains(&"Server"));
    assert!(types.contains(&"Handler"));
}

#[test]
fn typescript_call_edges() {
    let source = r#"
function connect() {}

function start() {
    connect();
}

function run() {
    connect();
    start();
}
"#;
    let (result, strings) = extract_ts(source, "app.ts");

    let connect_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "connect")
        .map(|n| n.id).unwrap();
    let start_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "start")
        .map(|n| n.id).unwrap();
    let run_id = result.nodes.iter()
        .find(|n| strings.get(n.name) == "run")
        .map(|n| n.id).unwrap();

    assert!(result.edges.iter().any(|e| e.source == start_id && e.target == connect_id && e.kind == EdgeKind::Calls));
    assert!(result.edges.iter().any(|e| e.source == run_id && e.target == connect_id && e.kind == EdgeKind::Calls));
    assert!(result.edges.iter().any(|e| e.source == run_id && e.target == start_id && e.kind == EdgeKind::Calls));
    assert_eq!(edge_count(&result, EdgeKind::Calls), 3);
}

#[test]
fn typescript_import_edges() {
    let source = r#"
import { Router } from 'express';
import path from 'path';
import { readFile, writeFile } from 'fs';

function main() {}
"#;
    let (result, _strings) = extract_ts(source, "app.ts");

    // 3 import statements with 3 source strings
    assert_eq!(
        edge_count(&result, EdgeKind::Imports),
        3,
        "should have 3 import edges"
    );
}

#[test]
fn typescript_arrow_and_function_expressions() {
    let source = r#"
function target() {}

const arrow = () => {
    target();
};

const expr = function named() {
    target();
};
"#;
    let (result, strings) = extract_ts(source, "funcs.ts");

    let funcs = symbol_names(&result, &strings);
    assert!(funcs.contains(&"target"));
    assert!(funcs.contains(&"arrow"));
    assert!(funcs.contains(&"expr"));
    assert_eq!(funcs.len(), 3, "should find exactly 3 functions, got: {:?}", funcs);
}

#[test]
fn typescript_new_expression() {
    let source = r#"
class Database {}

function init() {
    const db = new Database();
}
"#;
    let (result, strings) = extract_ts(source, "init.ts");

    let types = type_names(&result, &strings);
    assert!(types.contains(&"Database"));

    // new Database() creates a call edge
    assert_eq!(edge_count(&result, EdgeKind::Calls), 1, "new Database() should create a call edge");
}

#[test]
fn typescript_js_compatibility() {
    // JS files use the same TypeScript language
    let lang = Language::from_extension("js");
    assert_eq!(lang, Some(Language::TypeScript));
    let lang = Language::from_extension("jsx");
    assert_eq!(lang, Some(Language::TypeScript));
}
