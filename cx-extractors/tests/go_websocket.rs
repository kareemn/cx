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
fn go_websocket_upgrade() {
    let source = r#"package main

import "github.com/gorilla/websocket"

var upgrader = websocket.Upgrader{}

func wsHandler(w http.ResponseWriter, r *http.Request) {
    conn, err := upgrader.Upgrade(w, r, nil)
    _ = conn
    _ = err
}
"#;
    let (result, strings) = extract_go(source, "ws.go");

    let ws_endpoints: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8 && n.sub_kind == 1)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        !ws_endpoints.is_empty(),
        "should detect WebSocket upgrade, got: {:?}",
        ws_endpoints
    );
    assert!(
        ws_endpoints.contains(&"websocket"),
        "WebSocket without path should be named 'websocket'"
    );

    // Exposes edge from wsHandler → websocket endpoint
    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Exposes),
        "should have Exposes edge for WebSocket handler"
    );
}

#[test]
fn go_websocket_accept() {
    let source = r#"package main

import "nhooyr.io/websocket"

func wsHandler(w http.ResponseWriter, r *http.Request) {
    conn, err := websocket.Accept(w, r, nil)
    _ = conn
    _ = err
}
"#;
    let (result, _strings) = extract_go(source, "ws.go");

    let ws_count = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8 && n.sub_kind == 1)
        .count();

    assert!(ws_count > 0, "should detect nhooyr websocket.Accept");
}
