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

fn endpoint_names<'a>(result: &'a ExtractionResult, strings: &'a StringInterner) -> Vec<&'a str> {
    result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect()
}

#[test]
fn go_http_handlefunc() {
    let source = r#"package main

import "net/http"

func usersHandler(w http.ResponseWriter, r *http.Request) {
    http.HandleFunc("/api/users", usersHandler)
}

func main() {
    http.Handle("/api/health", healthHandler)
}
"#;
    let (result, strings) = extract_go(source, "cmd/server/main.go");
    let endpoints = endpoint_names(&result, &strings);
    assert!(
        endpoints.contains(&"/api/users"),
        "should detect /api/users endpoint, got: {:?}",
        endpoints
    );
    assert!(
        endpoints.contains(&"/api/health"),
        "should detect /api/health endpoint, got: {:?}",
        endpoints
    );

    // All endpoints should be HTTP (sub_kind=0)
    for n in result.nodes.iter().filter(|n| n.kind == NodeKind::Endpoint as u8) {
        assert_eq!(n.sub_kind, 0, "HTTP endpoints should have sub_kind=0");
    }

    // Exposes edge from enclosing function → endpoint
    assert!(
        result.edges.iter().any(|e| e.kind == EdgeKind::Exposes),
        "should have Exposes edges"
    );
}

#[test]
fn go_router_methods() {
    let source = r#"package main

import "github.com/gin-gonic/gin"

func setupRoutes() {
    r := gin.Default()
    r.GET("/api/users", getUsers)
    r.POST("/api/users", createUser)
    r.DELETE("/api/users/:id", deleteUser)
    r.PUT("/api/settings", updateSettings)
}
"#;
    let (result, strings) = extract_go(source, "cmd/api/main.go");
    let endpoints = endpoint_names(&result, &strings);

    assert!(
        endpoints.contains(&"/api/users"),
        "should detect GET /api/users, got: {:?}",
        endpoints
    );
    assert!(
        endpoints.contains(&"/api/users/:id"),
        "should detect DELETE /api/users/:id"
    );
    assert!(
        endpoints.contains(&"/api/settings"),
        "should detect PUT /api/settings"
    );
}

#[test]
fn go_chi_router() {
    let source = r#"package main

import "github.com/go-chi/chi"

func setupRoutes() {
    r := chi.NewRouter()
    r.Get("/health", healthCheck)
    r.Post("/webhook", handleWebhook)
}
"#;
    let (result, strings) = extract_go(source, "cmd/server/routes.go");
    let endpoints = endpoint_names(&result, &strings);

    assert!(
        endpoints.contains(&"/health"),
        "should detect chi Get route, got: {:?}",
        endpoints
    );
    assert!(
        endpoints.contains(&"/webhook"),
        "should detect chi Post route"
    );
}

#[test]
fn go_endpoint_has_exposes_edge() {
    let source = r#"package main

func handler(w http.ResponseWriter, r *http.Request) {
    http.HandleFunc("/api/data", handler)
}
"#;
    let (result, strings) = extract_go(source, "server.go");

    let endpoint = result
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Endpoint as u8 && strings.get(n.name) == "/api/data")
        .expect("should find /api/data endpoint");

    // Should have an Exposes edge from the enclosing function
    let exposes = result
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Exposes && e.target == endpoint.id)
        .count();
    assert_eq!(exposes, 1, "should have exactly 1 Exposes edge to the endpoint");
}
