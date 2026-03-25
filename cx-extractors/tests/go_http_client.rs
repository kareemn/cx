use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::NodeKind;
use cx_core::graph::string_interner::StringInterner;
use cx_extractors::grammars::{self, Language};
use cx_extractors::universal::{ExtractionResult, ParsedFile};

fn extract_go(source: &str, path: &str) -> (ExtractionResult, StringInterner) {
    let lang = Language::Go;
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

#[test]
fn go_http_get() {
    let source = r#"package main

import "net/http"

func fetchData() {
    resp, err := http.Get("https://api.example.com/data")
    _ = resp
    _ = err
}
"#;
    let (result, strings) = extract_go(source, "client.go");

    let endpoints: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        endpoints.contains(&"https://api.example.com/data"),
        "should detect http.Get URL, got: {:?}",
        endpoints
    );

    // Should have a Connects edge from fetchData → endpoint
    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Connects),
        "should have Connects edge for HTTP client call"
    );
}

#[test]
fn go_http_post() {
    let source = r#"package main

import "net/http"

func sendData() {
    resp, err := http.Post("https://api.example.com/submit", "application/json", nil)
    _ = resp
    _ = err
}
"#;
    let (result, strings) = extract_go(source, "client.go");

    let endpoints: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        endpoints.contains(&"https://api.example.com/submit"),
        "should detect http.Post URL, got: {:?}",
        endpoints
    );
}

#[test]
fn go_http_client_connects_edge() {
    let source = r#"package main

import "net/http"

func callService() {
    http.Get("https://other-service/api")
}
"#;
    let (result, strings) = extract_go(source, "caller.go");

    let ep = result
        .nodes
        .iter()
        .find(|n| {
            n.kind == NodeKind::Endpoint as u8
                && strings.get(n.name) == "https://other-service/api"
        })
        .expect("should find endpoint");

    let caller = result
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Symbol as u8 && strings.get(n.name) == "callService")
        .expect("should find callService function");

    assert!(
        result
            .edges
            .iter()
            .any(|e| e.source == caller.id && e.target == ep.id && e.kind == EdgeKind::Connects),
        "should have Connects edge from callService to endpoint"
    );
}
