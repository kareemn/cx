use anyhow::Result;
use cx_core::graph::csr::CsrGraph;
use cx_core::graph::nodes::{NodeKind, StringId};
use cx_core::query::trigram::TrigramIndex;
use std::path::Path;

/// Run `cx search <query>` — fuzzy symbol search using the trigram index.
pub fn run(root: &Path, query: &str) -> Result<()> {
    let graph = super::init::load_graph(root)?;
    let results = search_graph(&graph, query, 20);

    let output: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "kind": r.kind,
                "file": r.file,
                "line": r.line,
            })
        })
        .collect();

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// A search result entry.
pub struct SearchResult {
    pub name: String,
    pub kind: String,
    pub file: Option<String>,
    pub line: u32,
}

/// Search the graph for symbols matching a query string.
pub fn search_graph(graph: &CsrGraph, query: &str, max_results: usize) -> Vec<SearchResult> {
    // Build trigram index over all symbol names
    let symbol_ids: Vec<StringId> = graph
        .nodes
        .iter()
        .map(|n| n.name)
        .collect();

    let index = TrigramIndex::build(&symbol_ids, &graph.strings);
    let matches = index.search(query, &graph.strings);

    // Deduplicate by name and collect results
    let mut seen = rustc_hash::FxHashSet::default();
    let mut results = Vec::new();

    for &name_id in &matches {
        if results.len() >= max_results {
            break;
        }

        let name = graph.strings.get(name_id);
        if !seen.insert(name.to_string()) {
            continue;
        }

        // Find the first node with this name
        if let Some(node) = graph.nodes.iter().find(|n| n.name == name_id) {
            let kind = match NodeKind::from_u8(node.kind) {
                Some(NodeKind::Symbol) => {
                    if node.sub_kind == 1 {
                        "type"
                    } else {
                        "function"
                    }
                }
                Some(NodeKind::Module) => "module",
                Some(NodeKind::Endpoint) => "endpoint",
                Some(NodeKind::Deployable) => "deployable",
                Some(NodeKind::Resource) => "resource",
                _ => "symbol",
            };

            let file = if node.file != u32::MAX {
                Some(graph.strings.get(node.file).to_string())
            } else {
                None
            };

            results.push(SearchResult {
                name: name.to_string(),
                kind: kind.to_string(),
                file,
                line: node.line,
            });
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_indexed_project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            r#"package main

func handleAudioStream() {}
func handleVideoStream() {}
func processAudio() {}
func startServer() {}
"#,
        )
        .unwrap();
        super::super::init::run(dir.path(), false).unwrap();
        dir
    }

    #[test]
    fn search_finds_exact_match() {
        let dir = setup_indexed_project();
        let graph = super::super::init::load_graph(dir.path()).unwrap();

        let results = search_graph(&graph, "handleAudioStream", 10);
        assert!(!results.is_empty(), "should find handleAudioStream");
        assert_eq!(results[0].name, "handleAudioStream");
    }

    #[test]
    fn search_finds_partial_match() {
        let dir = setup_indexed_project();
        let graph = super::super::init::load_graph(dir.path()).unwrap();

        let results = search_graph(&graph, "Audio", 10);
        assert!(!results.is_empty(), "should find Audio matches");

        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(
            names.iter().any(|n| n.contains("Audio")),
            "results should contain Audio: {:?}",
            names
        );
    }

    #[test]
    fn search_no_results() {
        let dir = setup_indexed_project();
        let graph = super::super::init::load_graph(dir.path()).unwrap();

        let results = search_graph(&graph, "xyzzyplugh", 10);
        assert!(results.is_empty(), "should find no results for nonsense query");
    }

    #[test]
    fn search_respects_max_results() {
        let dir = setup_indexed_project();
        let graph = super::super::init::load_graph(dir.path()).unwrap();

        let results = search_graph(&graph, "handle", 1);
        assert!(results.len() <= 1, "should respect max_results=1");
    }

    #[test]
    fn search_includes_file_and_line() {
        let dir = setup_indexed_project();
        let graph = super::super::init::load_graph(dir.path()).unwrap();

        let results = search_graph(&graph, "startServer", 10);
        assert!(!results.is_empty());

        let r = &results[0];
        assert_eq!(r.name, "startServer");
        assert_eq!(r.kind, "function");
        assert!(r.file.is_some(), "should have a file path");
        assert!(r.line > 0, "should have a line number");
    }
}
