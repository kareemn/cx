use cx_core::graph::csr::EdgeInput;
use cx_core::graph::edges::EdgeKind;
use cx_core::graph::nodes::{Node, NodeId, NodeKind, StringId};
use cx_core::graph::string_interner::StringInterner;
use streaming_iterator::StreamingIterator;

/// Result of extraction from a single file.
pub struct ExtractionResult {
    pub nodes: Vec<Node>,
    pub edges: Vec<EdgeInput>,
}

impl ExtractionResult {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    pub fn with_capacity(nodes: usize, edges: usize) -> Self {
        Self {
            nodes: Vec::with_capacity(nodes),
            edges: Vec::with_capacity(edges),
        }
    }
}

impl Default for ExtractionResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Context about the repository being indexed.
pub struct RepoContext {
    pub repo_id: u16,
    pub repo_root: std::path::PathBuf,
}

/// A pre-parsed source file.
pub struct ParsedFile<'src> {
    pub tree: tree_sitter::Tree,
    pub source: &'src [u8],
    pub path: StringId,
    pub repo_id: u16,
}

/// Standardized capture names used in tree-sitter query files.
const CAPTURE_FUNC_NAME: &str = "func.name";
const CAPTURE_FUNC_DEF: &str = "func.def";
const CAPTURE_CALL_NAME: &str = "call.name";
const CAPTURE_CALL_SITE: &str = "call.site";
const CAPTURE_IMPORT_PATH: &str = "import.path";
const CAPTURE_IMPORT_DEF: &str = "import.def";
const CAPTURE_TYPE_NAME: &str = "type.name";
const CAPTURE_TYPE_DEF: &str = "type.def";

/// UniversalExtractor processes tree-sitter query matches into ExtractionResult
/// using standardized capture names.
pub struct UniversalExtractor {
    query: tree_sitter::Query,
    func_name_idx: Option<u32>,
    func_def_idx: Option<u32>,
    call_name_idx: Option<u32>,
    call_site_idx: Option<u32>,
    import_path_idx: Option<u32>,
    import_def_idx: Option<u32>,
    type_name_idx: Option<u32>,
    type_def_idx: Option<u32>,
}

impl UniversalExtractor {
    /// Create a new UniversalExtractor from a tree-sitter query string and language.
    pub fn new(
        language: &tree_sitter::Language,
        query_source: &str,
    ) -> Result<Self, tree_sitter::QueryError> {
        let query = tree_sitter::Query::new(language, query_source)?;

        let find_capture = |name: &str| -> Option<u32> {
            query
                .capture_names()
                .iter()
                .position(|n| *n == name)
                .map(|i| i as u32)
        };

        Ok(Self {
            func_name_idx: find_capture(CAPTURE_FUNC_NAME),
            func_def_idx: find_capture(CAPTURE_FUNC_DEF),
            call_name_idx: find_capture(CAPTURE_CALL_NAME),
            call_site_idx: find_capture(CAPTURE_CALL_SITE),
            import_path_idx: find_capture(CAPTURE_IMPORT_PATH),
            import_def_idx: find_capture(CAPTURE_IMPORT_DEF),
            type_name_idx: find_capture(CAPTURE_TYPE_NAME),
            type_def_idx: find_capture(CAPTURE_TYPE_DEF),
            query,
        })
    }

    /// Extract symbols from a parsed file.
    pub fn extract(
        &self,
        file: &ParsedFile<'_>,
        strings: &mut StringInterner,
        id_counter: &mut NodeId,
    ) -> ExtractionResult {
        let mut result = ExtractionResult::with_capacity(64, 128);

        // Track defined symbols: name StringId → (NodeId, def_start_byte, def_end_byte)
        let mut defined_symbols: Vec<(StringId, NodeId, usize, usize)> = Vec::new();

        // Collect call sites for second pass: (call_name_text, call_byte_offset)
        let mut call_sites: Vec<(String, usize)> = Vec::new();

        // Single pass: collect definitions and call sites
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&self.query, file.tree.root_node(), file.source);

        while let Some(m) = matches.next() {
            // Function definitions
            if let Some((name_text, def_node)) = self.get_func_def(m, file.source) {
                let name_id = strings.intern(name_text);
                let node_id = *id_counter;
                *id_counter += 1;

                let mut node = Node::new(node_id, NodeKind::Symbol, name_id);
                node.file = file.path;
                node.repo = file.repo_id;
                node.line = def_node.start_position().row as u32 + 1;

                let start = def_node.start_byte();
                let end = def_node.end_byte();
                defined_symbols.push((name_id, node_id, start, end));
                result.nodes.push(node);
            }

            // Type definitions
            if let Some((name_text, def_node)) = self.get_type_def(m, file.source) {
                let name_id = strings.intern(name_text);
                let node_id = *id_counter;
                *id_counter += 1;

                let mut node = Node::new(node_id, NodeKind::Symbol, name_id);
                node.file = file.path;
                node.repo = file.repo_id;
                node.sub_kind = 1; // type
                node.line = def_node.start_position().row as u32 + 1;

                let start = def_node.start_byte();
                let end = def_node.end_byte();
                defined_symbols.push((name_id, node_id, start, end));
                result.nodes.push(node);
            }

            // Import definitions
            if let Some(path_text) = self.capture_text(self.import_path_idx, m, file.source) {
                let name_id = strings.intern(path_text);
                let node_id = *id_counter;
                *id_counter += 1;

                let mut node = Node::new(node_id, NodeKind::Module, name_id);
                node.file = file.path;
                node.repo = file.repo_id;
                if let Some(def_node) = self.capture_node(self.import_def_idx, m) {
                    node.line = def_node.start_position().row as u32 + 1;
                }

                result.nodes.push(node);
            }

            // Call sites — record for second pass
            if let Some(call_name) = self.capture_text(self.call_name_idx, m, file.source) {
                let byte_offset = self
                    .capture_node(self.call_site_idx, m)
                    .or_else(|| self.capture_node(self.call_name_idx, m))
                    .map(|n| n.start_byte())
                    .unwrap_or(0);
                call_sites.push((call_name.to_string(), byte_offset));
            }
        }

        // Second pass: resolve call edges
        for (call_name, byte_offset) in &call_sites {
            let call_name_id = strings.intern(call_name);

            // Find the target: a defined symbol with this name
            let target = defined_symbols
                .iter()
                .find(|(name, _, _, _)| *name == call_name_id);

            if let Some(&(_, target_id, _, _)) = target {
                // Find the enclosing function: the defined symbol whose byte range contains this call
                let caller = defined_symbols
                    .iter()
                    .find(|(_, id, start, end)| {
                        *id != target_id && *start <= *byte_offset && *byte_offset < *end
                    });

                if let Some(&(_, caller_id, _, _)) = caller {
                    result
                        .edges
                        .push(EdgeInput::new(caller_id, target_id, EdgeKind::Calls));
                }
            }
        }

        result
    }

    fn get_func_def<'a>(
        &self,
        m: &tree_sitter::QueryMatch<'_, 'a>,
        source: &'a [u8],
    ) -> Option<(&'a str, tree_sitter::Node<'a>)> {
        let name = self.capture_text(self.func_name_idx, m, source)?;
        let def = self.capture_node(self.func_def_idx, m)?;
        Some((name, def))
    }

    fn get_type_def<'a>(
        &self,
        m: &tree_sitter::QueryMatch<'_, 'a>,
        source: &'a [u8],
    ) -> Option<(&'a str, tree_sitter::Node<'a>)> {
        let name = self.capture_text(self.type_name_idx, m, source)?;
        let def = self.capture_node(self.type_def_idx, m)?;
        Some((name, def))
    }

    fn capture_text<'a>(
        &self,
        capture_idx: Option<u32>,
        m: &tree_sitter::QueryMatch<'_, 'a>,
        source: &'a [u8],
    ) -> Option<&'a str> {
        let idx = capture_idx?;
        for cap in m.captures {
            if cap.index == idx {
                return std::str::from_utf8(&source[cap.node.byte_range()]).ok();
            }
        }
        None
    }

    fn capture_node<'a>(
        &self,
        capture_idx: Option<u32>,
        m: &tree_sitter::QueryMatch<'_, 'a>,
    ) -> Option<tree_sitter::Node<'a>> {
        let idx = capture_idx?;
        for cap in m.captures {
            if cap.index == idx {
                return Some(cap.node);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn go_language() -> tree_sitter::Language {
        tree_sitter_go::LANGUAGE.into()
    }

    fn parse_go(source: &str) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&go_language()).unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    const GO_QUERY: &str = r#"
; Function declarations
(function_declaration
  name: (identifier) @func.name) @func.def

; Method declarations
(method_declaration
  name: (field_identifier) @func.name) @func.def

; Type declarations
(type_declaration
  (type_spec
    name: (type_identifier) @type.name)) @type.def

; Import paths
(import_spec
  path: (interpreted_string_literal) @import.path) @import.def

; Call expressions
(call_expression
  function: (identifier) @call.name) @call.site

; Method call expressions
(call_expression
  function: (selector_expression
    field: (field_identifier) @call.name)) @call.site
"#;

    #[test]
    fn extract_go_functions() {
        let source = r#"
package main

func hello() {
    println("hello")
}

func world() {
    hello()
}
"#;
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("main.go");
        let file = ParsedFile {
            tree,
            source: source.as_bytes(),
            path: path_id,
            repo_id: 0,
        };

        let mut id_counter = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id_counter);

        let func_names: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| strings.get(n.name))
            .collect();

        assert!(func_names.contains(&"hello"), "should find hello, got: {:?}", func_names);
        assert!(func_names.contains(&"world"), "should find world, got: {:?}", func_names);
    }

    #[test]
    fn extract_go_call_edges() {
        let source = r#"
package main

func helper() {}

func main() {
    helper()
}
"#;
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("main.go");
        let file = ParsedFile {
            tree,
            source: source.as_bytes(),
            path: path_id,
            repo_id: 0,
        };

        let mut id_counter = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id_counter);

        // Should have an edge from main → helper
        assert!(
            !result.edges.is_empty(),
            "should have at least one call edge"
        );

        // Find the main and helper node IDs
        let helper_id = result
            .nodes
            .iter()
            .find(|n| strings.get(n.name) == "helper")
            .map(|n| n.id);
        let main_id = result
            .nodes
            .iter()
            .find(|n| strings.get(n.name) == "main")
            .map(|n| n.id);

        assert!(helper_id.is_some(), "should find helper node");
        assert!(main_id.is_some(), "should find main node");

        let has_call_edge = result.edges.iter().any(|e| {
            e.source == main_id.unwrap()
                && e.target == helper_id.unwrap()
                && e.kind == EdgeKind::Calls
        });
        assert!(has_call_edge, "should have main→helper call edge");
    }

    #[test]
    fn extract_go_types() {
        let source = r#"
package main

type Server struct {
    port int
}

type Handler interface {
    Handle()
}
"#;
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("types.go");
        let file = ParsedFile {
            tree,
            source: source.as_bytes(),
            path: path_id,
            repo_id: 0,
        };

        let mut id_counter = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id_counter);

        let type_names: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 1)
            .map(|n| strings.get(n.name))
            .collect();

        assert!(type_names.contains(&"Server"), "should find Server type");
        assert!(type_names.contains(&"Handler"), "should find Handler type");
    }

    #[test]
    fn extract_go_imports() {
        let source = r#"
package main

import (
    "fmt"
    "net/http"
)

func main() {
    fmt.Println("hello")
}
"#;
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("main.go");
        let file = ParsedFile {
            tree,
            source: source.as_bytes(),
            path: path_id,
            repo_id: 0,
        };

        let mut id_counter = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id_counter);

        let import_paths: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module as u8)
            .map(|n| strings.get(n.name))
            .collect();

        assert!(
            import_paths.iter().any(|p| p.contains("fmt")),
            "should find fmt import, got: {:?}",
            import_paths
        );
        assert!(
            import_paths.iter().any(|p| p.contains("net/http")),
            "should find net/http import, got: {:?}",
            import_paths
        );
    }

    #[test]
    fn extract_go_methods() {
        let source = r#"
package main

type Server struct{}

func (s *Server) Start() {}
func (s *Server) Stop() {}
"#;
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("server.go");
        let file = ParsedFile {
            tree,
            source: source.as_bytes(),
            path: path_id,
            repo_id: 0,
        };

        let mut id_counter = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id_counter);

        let method_names: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| strings.get(n.name))
            .collect();

        assert!(method_names.contains(&"Start"), "should find Start method");
        assert!(method_names.contains(&"Stop"), "should find Stop method");
    }

    #[test]
    fn extract_line_numbers() {
        let source = "package main\n\nfunc foo() {}\n\nfunc bar() {}\n";
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("test.go");
        let file = ParsedFile {
            tree,
            source: source.as_bytes(),
            path: path_id,
            repo_id: 0,
        };

        let mut id_counter = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id_counter);

        let foo = result.nodes.iter().find(|n| strings.get(n.name) == "foo").expect("should find foo");
        let bar = result.nodes.iter().find(|n| strings.get(n.name) == "bar").expect("should find bar");

        assert_eq!(foo.line, 3, "foo should be on line 3");
        assert_eq!(bar.line, 5, "bar should be on line 5");
    }

    #[test]
    fn extract_empty_file() {
        let source = "package main\n";
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("empty.go");
        let file = ParsedFile {
            tree,
            source: source.as_bytes(),
            path: path_id,
            repo_id: 0,
        };

        let mut id_counter = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id_counter);

        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
    }

    #[test]
    fn invalid_query_returns_error() {
        let lang = go_language();
        let result = UniversalExtractor::new(&lang, "(invalid_node_type @cap)");
        assert!(result.is_err());
    }
}
