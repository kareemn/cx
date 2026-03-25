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
fn go_os_getenv() {
    let source = r#"package main

import "os"

func loadConfig() {
    dbURL := os.Getenv("DATABASE_URL")
    port := os.Getenv("PORT")
    _ = dbURL
    _ = port
}
"#;
    let (result, strings) = extract_go(source, "config.go");

    let envvars: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Resource as u8)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        envvars.contains(&"DATABASE_URL"),
        "should detect DATABASE_URL env var, got: {:?}",
        envvars
    );
    assert!(
        envvars.contains(&"PORT"),
        "should detect PORT env var, got: {:?}",
        envvars
    );

    // Configures edges from loadConfig → envvars
    let config_edges = result
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Configures)
        .count();
    assert_eq!(config_edges, 2, "should have 2 Configures edges");
}

#[test]
fn go_os_lookupenv() {
    let source = r#"package main

import "os"

func checkEnv() {
    val, ok := os.LookupEnv("SECRET_KEY")
    _ = val
    _ = ok
}
"#;
    let (result, strings) = extract_go(source, "env.go");

    let envvars: Vec<&str> = result
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Resource as u8)
        .map(|n| strings.get(n.name))
        .collect();

    assert!(
        envvars.contains(&"SECRET_KEY"),
        "should detect LookupEnv, got: {:?}",
        envvars
    );
}
