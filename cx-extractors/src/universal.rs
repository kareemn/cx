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
    /// Raw path string for pattern checks (e.g., _test.go detection).
    pub path_str: &'src str,
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
                let is_test_file = file.path_str.ends_with("_test.go");
                let is_main = pkg_name == "main" && !is_test_file;

                let (kind, display_name) = if is_main {
                    // Deployable: use the directory path as the name
                    let dir = std::path::Path::new(file.path_str)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| ".".to_string());
                    let dir = if dir.is_empty() { ".".to_string() } else { dir };
                    (NodeKind::Deployable, dir)
                } else {
                    (NodeKind::Module, pkg_name.to_string())
                };

                let name_id = strings.intern(&display_name);
                let node_id = *id_counter;
                *id_counter += 1;

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

        // If no package declaration but we have symbols or imports,
        // create a file-level Module node (e.g. for C/C++ files without namespaces).
        if pkg_node_id.is_none() && (!defined_symbols.is_empty() || !import_paths.is_empty()) {
            let file_name = std::str::from_utf8(file.source)
                .ok()
                .map(|_| file.path_str);
            let mod_name = file_name
                .and_then(|p| std::path::Path::new(p).file_stem())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".to_string());
            let name_id = strings.intern(&mod_name);
            let node_id = *id_counter;
            *id_counter += 1;
            let mut node = Node::new(node_id, NodeKind::Module, name_id);
            node.file = file.path;
            node.repo = file.repo_id;
            result.nodes.push(node);
            pkg_node_id = Some(node_id);
        }

        // Create Contains edges: package/deployable → each symbol in this file
        if let Some(pkg_id) = pkg_node_id {
            for &(_, symbol_id, _, _) in &defined_symbols {
                result
                    .edges
                    .push(EdgeInput::new(pkg_id, symbol_id, EdgeKind::Contains));
            }
        }

        // Create Imports edges: package node → import target (as a Module node)
        if let Some(pkg_id) = pkg_node_id {
            for import_path in &import_paths {
                let import_name_id = strings.intern(import_path);
                let import_node_id = *id_counter;
                *id_counter += 1;

                // Create a Module node for the import target so the edge
                // survives ID remapping in the merge step.
                let mut import_node = Node::new(import_node_id, NodeKind::Module, import_name_id);
                import_node.repo = file.repo_id;
                result.nodes.push(import_node);

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

    /// Helper: parse Go source at a given path and extract.
    fn extract_go(source: &str, path: &str) -> (ExtractionResult, StringInterner) {
        let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        let tree = parser.parse(source.as_bytes(), None).unwrap();
        let extractor = UniversalExtractor::new(&lang, GO_QUERY).unwrap();
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

    const GO_QUERY: &str = include_str!("../queries/go-symbols.scm");

    #[test]
    fn extract_go_functions() {
        let (result, strings) = extract_go(
            "package main\n\nfunc hello() {\n    println(\"hello\")\n}\n\nfunc world() {\n    hello()\n}\n",
            "main.go",
        );
        let func_names: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| strings.get(n.name))
            .collect();
        assert!(func_names.contains(&"hello"));
        assert!(func_names.contains(&"world"));
    }

    #[test]
    fn extract_go_call_edges() {
        let (result, strings) = extract_go(
            "package main\n\nfunc helper() {}\n\nfunc main() {\n    helper()\n}\n",
            "main.go",
        );
        let helper_id = result.nodes.iter()
            .find(|n| strings.get(n.name) == "helper" && n.kind == NodeKind::Symbol as u8)
            .map(|n| n.id).unwrap();
        let main_id = result.nodes.iter()
            .find(|n| strings.get(n.name) == "main" && n.kind == NodeKind::Symbol as u8)
            .map(|n| n.id).unwrap();
        assert!(result.edges.iter().any(|e| e.source == main_id && e.target == helper_id && e.kind == EdgeKind::Calls));
    }

    #[test]
    fn extract_go_types() {
        let (result, strings) = extract_go(
            "package main\n\ntype Server struct {\n    port int\n}\n\ntype Handler interface {\n    Handle()\n}\n",
            "types.go",
        );
        let type_names: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 1)
            .map(|n| strings.get(n.name))
            .collect();
        assert!(type_names.contains(&"Server"));
        assert!(type_names.contains(&"Handler"));
    }

    #[test]
    fn extract_go_methods() {
        let (result, strings) = extract_go(
            "package main\n\ntype Server struct{}\n\nfunc (s *Server) Start() {}\nfunc (s *Server) Stop() {}\n",
            "server.go",
        );
        let names: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| strings.get(n.name))
            .collect();
        assert!(names.contains(&"Start"));
        assert!(names.contains(&"Stop"));
    }

    #[test]
    fn extract_line_numbers() {
        let (result, strings) = extract_go("package main\n\nfunc foo() {}\n\nfunc bar() {}\n", "test.go");
        let foo = result.nodes.iter().find(|n| strings.get(n.name) == "foo").unwrap();
        let bar = result.nodes.iter().find(|n| strings.get(n.name) == "bar").unwrap();
        assert_eq!(foo.line, 3);
        assert_eq!(bar.line, 5);
    }

    #[test]
    fn extract_file_with_only_package_decl() {
        let (result, _strings) = extract_go("package main\n", "empty.go");
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].kind, NodeKind::Deployable as u8);
        assert!(result.edges.is_empty());
    }

    #[test]
    fn invalid_query_returns_error() {
        let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
        assert!(UniversalExtractor::new(&lang, "(invalid_node_type @cap)").is_err());
    }

    // ─── Realistic Go pattern tests ─────────────────────────────────

    #[test]
    fn real_go_package_declaration() {
        let (result, strings) = extract_go(
            "package auth\n\nimport \"fmt\"\n\nfunc Login() { fmt.Println(\"login\") }\n",
            "pkg/auth/login.go",
        );
        // The package's own Module node
        let pkg_modules: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Module as u8 && n.line > 0)
            .map(|n| strings.get(n.name)).collect();
        assert_eq!(pkg_modules, vec!["auth"]);
        // Import target Module node for "fmt"
        assert!(result.nodes.iter().any(|n| n.kind == NodeKind::Module as u8 && strings.get(n.name) == "fmt"));
        assert_eq!(result.edges.iter().filter(|e| e.kind == EdgeKind::Imports).count(), 1);
        assert_eq!(result.edges.iter().filter(|e| e.kind == EdgeKind::Contains).count(), 1);
        let symbols: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8 && n.sub_kind == 0)
            .map(|n| strings.get(n.name)).collect();
        assert_eq!(symbols, vec!["Login"]);
    }

    #[test]
    fn real_go_main_package() {
        let (result, strings) = extract_go(
            "package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Println(\"hello\") }\n",
            "cmd/server/main.go",
        );
        let deployables: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Deployable as u8)
            .map(|n| strings.get(n.name)).collect();
        assert_eq!(deployables.len(), 1);
        assert_eq!(deployables[0], "cmd/server", "deployable name should be the directory");
        assert!(result.nodes.iter().any(|n| strings.get(n.name) == "main" && n.kind == NodeKind::Symbol as u8));
        assert_eq!(result.edges.iter().filter(|e| e.kind == EdgeKind::Imports).count(), 1);
        assert_eq!(result.edges.iter().filter(|e| e.kind == EdgeKind::Contains).count(), 1);
        // Import target Module node for "fmt" is now created
        assert!(result.nodes.iter().any(|n| n.kind == NodeKind::Module as u8 && strings.get(n.name) == "fmt"));
    }

    #[test]
    fn real_go_multiple_imports() {
        let (result, strings) = extract_go(
            "package router\n\nimport (\n    \"fmt\"\n    \"net/http\"\n    \"github.com/gorilla/mux\"\n)\n\nfunc HandleRoute() {}\n",
            "pkg/router/router.go",
        );
        // The package's own Module node
        let pkg_modules: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Module as u8 && n.line > 0)
            .map(|n| strings.get(n.name)).collect();
        assert_eq!(pkg_modules, vec!["router"]);
        // 3 import target Module nodes
        assert_eq!(result.nodes.iter().filter(|n| n.kind == NodeKind::Module as u8 && n.line == 0).count(), 3);
        assert_eq!(result.edges.iter().filter(|e| e.kind == EdgeKind::Imports).count(), 3);
        assert_eq!(result.edges.iter().filter(|e| e.kind == EdgeKind::Contains).count(), 1);
        let symbols: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Symbol as u8)
            .map(|n| strings.get(n.name)).collect();
        assert_eq!(symbols, vec!["HandleRoute"]);
    }

    #[test]
    fn test_file_not_deployable() {
        // _test.go files with package main should NOT create Deployable nodes
        let (result, strings) = extract_go(
            "package main\n\nimport \"testing\"\n\nfunc TestSomething(t *testing.T) {}\n",
            "cmd/server/main_test.go",
        );
        let deployables: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Deployable as u8)
            .map(|n| strings.get(n.name)).collect();
        assert!(deployables.is_empty(), "test files should not create deployables, got: {:?}", deployables);
        // Should create a Module node instead
        let modules: Vec<&str> = result.nodes.iter()
            .filter(|n| n.kind == NodeKind::Module as u8)
            .map(|n| strings.get(n.name)).collect();
        assert!(modules.contains(&"main"), "test file with package main should be Module");
    }

    #[test]
    fn deployable_named_by_directory() {
        // Deployable name should be the directory, not "main"
        let (result, strings) = extract_go("package main\n\nfunc main() {}\n", "cmd/gh/main.go");
        let dep = result.nodes.iter().find(|n| n.kind == NodeKind::Deployable as u8).unwrap();
        assert_eq!(strings.get(dep.name), "cmd/gh");

        let (result2, strings2) = extract_go("package main\n\nfunc main() {}\n", "main.go");
        let dep2 = result2.nodes.iter().find(|n| n.kind == NodeKind::Deployable as u8).unwrap();
        // Root-level main.go: directory is empty, should be "."
        assert!(!strings2.get(dep2.name).is_empty());
    }
}
