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

fn endpoint_names<'a>(result: &'a ExtractionResult, strings: &'a StringInterner) -> Vec<&'a str> {
    result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Endpoint as u8)
        .map(|n| strings.get(n.name))
        .collect()
}

#[test]
fn express_routes() {
    let source = r#"
const express = require('express');
const app = express();

function setupRoutes() {
    app.get('/api/users', getUsers);
    app.post('/api/users', createUser);
    app.delete('/api/users/:id', deleteUser);
}
"#;
    let (result, strings) = extract_ts(source, "routes.ts");
    let endpoints = endpoint_names(&result, &strings);

    assert!(
        endpoints.contains(&"/api/users"),
        "should detect express GET /api/users, got: {:?}",
        endpoints
    );
    assert!(
        endpoints.contains(&"/api/users/:id"),
        "should detect express DELETE /api/users/:id"
    );
}

#[test]
fn express_router() {
    let source = r#"
import { Router } from 'express';

function setupRouter() {
    const router = Router();
    router.get('/health', healthCheck);
    router.post('/webhook', handleWebhook);
}
"#;
    let (result, strings) = extract_ts(source, "router.ts");
    let endpoints = endpoint_names(&result, &strings);

    assert!(
        endpoints.contains(&"/health"),
        "should detect router.get /health, got: {:?}",
        endpoints
    );
    assert!(
        endpoints.contains(&"/webhook"),
        "should detect router.post /webhook"
    );
}

#[test]
fn express_endpoint_has_exposes_edge() {
    let source = r#"
function setupRoutes() {
    app.get('/api/data', handler);
}
"#;
    let (result, strings) = extract_ts(source, "routes.ts");

    let endpoint = result
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Endpoint as u8 && strings.get(n.name) == "/api/data")
        .expect("should find /api/data endpoint");

    assert!(
        result
            .edges
            .iter()
            .any(|e| e.kind == EdgeKind::Exposes && e.target == endpoint.id),
        "should have Exposes edge from enclosing function to endpoint"
    );
}
