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
const CAPTURE_PKG_NAME: &str = "pkg.name";
const CAPTURE_PKG_DEF: &str = "pkg.def";

/// UniversalExtractor processes tree-sitter query matches into ExtractionResult
/// using standardized capture names.
pub struct UniversalExtractor {
    query: tree_sitter::Query,
    func_name_idx: Option<u32>,
    func_def_idx: Option<u32>,
    call_name_idx: Option<u32>,
    call_site_idx: Option<u32>,
    import_path_idx: Option<u32>,
    #[allow(dead_code)]
    import_def_idx: Option<u32>,
    type_name_idx: Option<u32>,
    type_def_idx: Option<u32>,
    pkg_name_idx: Option<u32>,
    pkg_def_idx: Option<u32>,
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
            pkg_name_idx: find_capture(CAPTURE_PKG_NAME),
            pkg_def_idx: find_capture(CAPTURE_PKG_DEF),
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

        // Track the package node ID for import edges
        let mut pkg_node_id: Option<NodeId> = None;

        // Collect import paths for creating Imports edges
        let mut import_paths: Vec<String> = Vec::new();

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

            // Package declarations → Module node (or Deployable if "main")
            if let Some(pkg_name) = self.capture_text(self.pkg_name_idx, m, file.source) {
                let name_id = strings.intern(pkg_name);
                let node_id = *id_counter;
                *id_counter += 1;

                let kind = if pkg_name == "main" {
                    NodeKind::Deployable
                } else {
                    NodeKind::Module
                };

                let mut node = Node::new(node_id, kind, name_id);
                node.file = file.path;
                node.repo = file.repo_id;
                if let Some(def_node) = self.capture_node(self.pkg_def_idx, m) {
                    node.line = def_node.start_position().row as u32 + 1;
                }

                // Don't add to defined_symbols — package decls are not
                // enclosing scopes for call edge resolution
                pkg_node_id = Some(node_id);
                result.nodes.push(node);
            }

            // Import paths → collect for creating Imports edges
            if let Some(path_text) = self.capture_text(self.import_path_idx, m, file.source) {
                import_paths.push(path_text.trim_matches('"').to_string());
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

        // Create Imports edges: package node → import target (as a Module node)
        if let Some(pkg_id) = pkg_node_id {
            for import_path in &import_paths {
                let _import_name_id = strings.intern(import_path);
                let import_node_id = *id_counter;
                *id_counter += 1;

                result
                    .edges
                    .push(EdgeInput::new(pkg_id, import_node_id, EdgeKind::Imports));
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

    const GO_QUERY: &str = include_str!("../queries/go-symbols.scm");

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

        // Find the main function (Symbol) and helper node IDs
        let helper_id = result
            .nodes
            .iter()
            .find(|n| strings.get(n.name) == "helper" && n.kind == NodeKind::Symbol as u8)
            .map(|n| n.id);
        let main_func_id = result
            .nodes
            .iter()
            .find(|n| strings.get(n.name) == "main" && n.kind == NodeKind::Symbol as u8)
            .map(|n| n.id);

        assert!(helper_id.is_some(), "should find helper node");
        assert!(main_func_id.is_some(), "should find main func node");

        let has_call_edge = result.edges.iter().any(|e| {
            e.source == main_func_id.unwrap()
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
    fn extract_go_package_as_module() {
        let source = r#"
package server

import "fmt"

func Start() {
    fmt.Println("starting")
}
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

        // Module node should come from `package server`, not from imports
        let modules: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module as u8)
            .map(|n| strings.get(n.name))
            .collect();

        assert!(modules.contains(&"server"), "should find 'server' module from package decl, got: {:?}", modules);
        assert!(!modules.iter().any(|m| m.contains("fmt")), "imports should NOT create Module nodes, got: {:?}", modules);
    }

    #[test]
    fn extract_go_deployable_from_package_main() {
        let source = r#"
package main

func main() {}
"#;
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("cmd/myapp/main.go");
        let file = ParsedFile {
            tree,
            source: source.as_bytes(),
            path: path_id,
            repo_id: 0,
        };

        let mut id_counter = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id_counter);

        // `package main` should create a Deployable node
        let deployables: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Deployable as u8)
            .map(|n| strings.get(n.name))
            .collect();

        assert!(deployables.contains(&"main"), "should create Deployable for package main, got: {:?}", deployables);

        // Should NOT create a Module node for "main"
        let modules: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module as u8)
            .map(|n| strings.get(n.name))
            .collect();

        assert!(!modules.contains(&"main"), "package main should be Deployable, not Module");
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
    fn extract_file_with_only_package_decl() {
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

        // Should have exactly 1 node: the Deployable from `package main`
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].kind, NodeKind::Deployable as u8);
        assert!(result.edges.is_empty());
    }

    #[test]
    fn invalid_query_returns_error() {
        let lang = go_language();
        let result = UniversalExtractor::new(&lang, "(invalid_node_type @cap)");
        assert!(result.is_err());
    }

    // ─── Realistic Go pattern tests ─────────────────────────────────

    #[test]
    fn real_go_package_declaration() {
        let source = r#"package auth

import "fmt"

func Login() { fmt.Println("login") }
"#;
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("pkg/auth/login.go");
        let file = ParsedFile { tree, source: source.as_bytes(), path: path_id, repo_id: 0 };

        let mut id = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id);

        // 1 Module node with name="auth" (from package declaration)
        let modules: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Module as u8)
            .map(|n| strings.get(n.name))
            .collect();
        assert_eq!(modules, vec!["auth"], "should have 1 Module from package decl");

        // 0 Module nodes with name="fmt" — imports don't create Module nodes
        assert!(!modules.contains(&"fmt"), "imports must not create Module nodes");

        // 1 Import edge from auth module to "fmt"
        let import_edges: Vec<_> = result.edges.iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 1, "should have 1 Imports edge");

        // 1 Symbol node: Login
        let symbols: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| strings.get(n.name))
            .collect();
        assert_eq!(symbols, vec!["Login"]);
    }

    #[test]
    fn real_go_main_package() {
        let source = r#"package main

import "fmt"

func main() { fmt.Println("hello") }
"#;
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("cmd/server/main.go");
        let file = ParsedFile { tree, source: source.as_bytes(), path: path_id, repo_id: 0 };

        let mut id = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id);

        // 1 Deployable node
        let deployables: Vec<_> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Deployable as u8)
            .collect();
        assert_eq!(deployables.len(), 1, "should have 1 Deployable");

        // 1 Symbol node: main (Function)
        let symbols: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| strings.get(n.name))
            .collect();
        assert!(symbols.contains(&"main"), "should have main function symbol");

        // 1 Import edge to "fmt"
        let import_edges: Vec<_> = result.edges.iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 1, "should have 1 Imports edge");

        // 0 Module nodes named "fmt"
        let fmt_modules: Vec<_> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Module as u8 && strings.get(n.name) == "fmt")
            .collect();
        assert!(fmt_modules.is_empty(), "fmt should not be a Module node");
    }

    #[test]
    fn real_go_multiple_imports() {
        let source = r#"package router

import (
    "fmt"
    "net/http"
    "github.com/gorilla/mux"
)

func HandleRoute() {}
"#;
        let tree = parse_go(source);
        let lang = go_language();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();

        let mut strings = StringInterner::new();
        let path_id = strings.intern("pkg/router/router.go");
        let file = ParsedFile { tree, source: source.as_bytes(), path: path_id, repo_id: 0 };

        let mut id = 0u32;
        let result = extractor.extract(&file, &mut strings, &mut id);

        // 1 Module node with name="router"
        let modules: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Module as u8)
            .map(|n| strings.get(n.name))
            .collect();
        assert_eq!(modules, vec!["router"]);

        // 3 Import edges
        let import_edges: Vec<_> = result.edges.iter()
            .filter(|e| e.kind == EdgeKind::Imports)
            .collect();
        assert_eq!(import_edges.len(), 3, "should have 3 Imports edges");

        // 0 Module nodes for import paths
        assert!(!modules.contains(&"fmt"));
        assert!(!modules.iter().any(|m| m.contains("net/http")));
        assert!(!modules.iter().any(|m| m.contains("gorilla")));

        // 1 Symbol node: HandleRoute
        let symbols: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8)
            .map(|n| strings.get(n.name))
            .collect();
        assert_eq!(symbols, vec!["HandleRoute"]);
    }
}
