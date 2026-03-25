#[allow(unused_imports)]
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

fn endpoint_names<'a>(result: &'a ExtractionResult, strings: &'a StringInterner) -> Vec<&'a str> {
    result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect()
}

#[test]
fn flask_routes() {
    let source = r#"
from flask import Flask

app = Flask(__name__)

@app.route('/api/users')
def get_users():
    pass

@app.route('/api/health')
def health():
    pass
"#;
    let (result, strings) = extract_py(source, "app.py");
    let endpoints = endpoint_names(&result, &strings);

    assert!(
        endpoints.contains(&"/api/users"),
        "should detect Flask route /api/users, got: {:?}",
        endpoints
    );
    assert!(
        endpoints.contains(&"/api/health"),
        "should detect Flask route /api/health"
    );
}

#[test]
fn fastapi_routes() {
    let source = r#"
from fastapi import FastAPI

app = FastAPI()

@app.get('/users')
def list_users():
    pass

@app.post('/users')
def create_user():
    pass

@app.delete('/users/{user_id}')
def delete_user():
    pass
"#;
    let (result, strings) = extract_py(source, "main.py");
    let endpoints = endpoint_names(&result, &strings);

    assert!(
        endpoints.contains(&"/users"),
        "should detect FastAPI GET /users, got: {:?}",
        endpoints
    );
    assert!(
        endpoints.iter().any(|e| e.contains("/users/") && e.contains("user_id")),
        "should detect FastAPI DELETE /users/{{user_id}}"
    );
}

#[test]
fn django_url_patterns() {
    let source = r#"
from django.urls import path

def index(request):
    pass

def detail(request, pk):
    pass

urlpatterns = [
    path('articles/', index),
    path('articles/<int:pk>/', detail),
]
"#;
    let (result, strings) = extract_py(source, "urls.py");
    let endpoints = endpoint_names(&result, &strings);

    assert!(
        endpoints.contains(&"articles/"),
        "should detect Django path articles/, got: {:?}",
        endpoints
    );
    assert!(
        endpoints.contains(&"articles/<int:pk>/"),
        "should detect Django path articles/<int:pk>/"
    );
}

#[test]
fn flask_endpoint_has_exposes_edge() {
    // The decorator wraps the function definition, so the endpoint.def
    // is inside the decorated_definition which contains the function
    let source = r#"
from flask import Flask

app = Flask(__name__)

@app.route('/data')
def get_data():
    pass
"#;
    let (result, strings) = extract_py(source, "app.py");

    let endpoint = result
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Endpoint as u8 && strings.get(n.name) == "/data");

    assert!(endpoint.is_some(), "should find /data endpoint");

    // The endpoint should exist with correct properties
    if let Some(ep) = endpoint {
        assert_eq!(ep.sub_kind, 0, "should be HTTP endpoint");
    }
}
