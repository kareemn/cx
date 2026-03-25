use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::NodeKind;
use cx_core::graph::string_interner::StringInterner;
use cx_extractors::grammars::{self, Language};
use cx_extractors::universal::{ExtractionResult, ParsedFile};

fn extract_py(source: &str, path: &str) -> (ExtractionResult, StringInterner) {
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

#[test]
fn requests_get() {
    let source = r#"
import requests

def fetch_users():
    resp = requests.get('https://api.example.com/users')
    return resp
"#;
    let (result, strings) = extract_py(source, "client.py");

    let endpoints: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        endpoints.contains(&"https://api.example.com/users"),
        "should detect requests.get URL, got: {:?}",
        endpoints
    );

    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Connects),
        "should have Connects edge"
    );
}

#[test]
fn requests_post() {
    let source = r#"
import requests

def submit_data():
    resp = requests.post('https://api.example.com/submit')
    return resp
"#;
    let (result, strings) = extract_py(source, "client.py");

    let endpoints: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        endpoints.contains(&"https://api.example.com/submit"),
        "should detect requests.post URL, got: {:?}",
        endpoints
    );
}

#[test]
fn httpx_client() {
    let source = r#"
import httpx

def fetch_data():
    resp = httpx.get('https://service.internal/data')
    return resp
"#;
    let (result, strings) = extract_py(source, "http_client.py");

    let endpoints: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        endpoints.contains(&"https://service.internal/data"),
        "should detect httpx.get URL, got: {:?}",
        endpoints
    );
}

#[test]
fn python_http_client_connects_edge() {
    let source = r#"
import requests

def call_service():
    requests.get('https://other-service/api')
"#;
    let (result, strings) = extract_py(source, "caller.py");

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
        .find(|n| n.kind == NodeKind::Symbol as u8 && strings.get(n.name) == "call_service")
        .expect("should find call_service function");

    assert!(
        result
            .edges
            .iter()
            .any(|e| e.source == caller.id && e.target == ep.id && e.kind == EdgeKind::Connects),
        "should have Connects edge from call_service to endpoint"
    );
}
