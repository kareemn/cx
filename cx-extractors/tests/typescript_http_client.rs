use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::NodeKind;
use cx_core::graph::string_interner::StringInterner;
use cx_extractors::grammars::{self, Language};
use cx_extractors::universal::{ExtractionResult, ParsedFile};

fn extract_ts(source: &str, path: &str) -> (ExtractionResult, StringInterner) {
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

#[test]
fn fetch_with_string_url() {
    let source = r#"
function loadData() {
    fetch('/api/data');
}
"#;
    let (result, strings) = extract_ts(source, "client.ts");

    let endpoints: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        endpoints.contains(&"/api/data"),
        "should detect fetch URL, got: {:?}",
        endpoints
    );

    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Connects),
        "should have Connects edge for fetch call"
    );
}

#[test]
fn fetch_connects_edge() {
    let source = r#"
function callApi() {
    fetch('/api/endpoint');
}
"#;
    let (result, strings) = extract_ts(source, "api.ts");

    let endpoint = result
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Endpoint as u8 && strings.get(n.name) == "/api/endpoint")
        .expect("should find endpoint");

    let caller = result
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Symbol as u8 && strings.get(n.name) == "callApi")
        .expect("should find callApi function");

    assert!(
        result
            .edges
            .iter()
            .any(|e| e.source == caller.id && e.target == endpoint.id && e.kind == EdgeKind::Connects),
        "should have Connects edge from callApi to endpoint"
    );
}

#[test]
fn axios_with_http_url() {
    let source = r#"
function fetchRemote() {
    axios.get('https://api.example.com/users');
    axios.post('https://api.example.com/submit');
}
"#;
    let (result, strings) = extract_ts(source, "remote.ts");

    let endpoints: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        endpoints.contains(&"https://api.example.com/users"),
        "should detect axios.get URL, got: {:?}",
        endpoints
    );
    assert!(
        endpoints.contains(&"https://api.example.com/submit"),
        "should detect axios.post URL"
    );
}
