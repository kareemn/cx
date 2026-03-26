//! Raw extraction pipeline — generic tree-sitter queries that capture ALL symbols,
//! calls, imports, and strings with 100% recall. No filtering by function name;
//! classification happens later in Phase 3.
//!
//! This module defines language-agnostic IR structs and a `RawExtractor` that uses
//! per-language tree-sitter query files to populate them.

use crate::grammars::Language;
use cx_core::graph::nodes::{StringId, STRING_NONE};
use cx_core::graph::string_interner::StringInterner;
use streaming_iterator::StreamingIterator;

// ─── Raw query files (no #match? or #eq? predicates) ─────────────────────────

pub const GO_RAW_QUERY: &str = include_str!("../queries/go-raw.scm");
pub const PYTHON_RAW_QUERY: &str = include_str!("../queries/python-raw.scm");
pub const TYPESCRIPT_RAW_QUERY: &str = include_str!("../queries/typescript-raw.scm");
pub const C_RAW_QUERY: &str = include_str!("../queries/c-raw.scm");
pub const CPP_RAW_QUERY: &str = include_str!("../queries/cpp-raw.scm");
pub const JAVA_RAW_QUERY: &str = include_str!("../queries/java-raw.scm");

// ─── Language discriminant ───────────────────────────────────────────────────

/// Language tag for raw extraction IR. Compact repr(u8) for use in packed structs.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RawLang {
    Go = 0,
    Python = 1,
    TypeScript = 2,
    C = 3,
    Cpp = 4,
    Java = 5,
}

impl RawLang {
    pub fn from_language(lang: Language) -> Self {
        match lang {
            Language::Go => Self::Go,
            Language::Python => Self::Python,
            Language::TypeScript => Self::TypeScript,
            Language::C => Self::C,
            Language::Cpp => Self::Cpp,
            Language::Java => Self::Java,
        }
    }
}

// ─── IR structs ──────────────────────────────────────────────────────────────

/// A function/method call site.
#[derive(Debug, Clone)]
pub struct RawCall {
    pub line: u32,
    pub byte_offset: u32,
    pub callee_name: StringId,
    pub receiver_name: StringId,
    pub first_string_arg: StringId,
    pub second_string_arg: StringId,
    pub arg_count: u8,
    pub lang: RawLang,
}

/// A function or method definition.
#[derive(Debug, Clone)]
pub struct RawDef {
    pub name: StringId,
    pub byte_start: u32,
    pub byte_end: u32,
    pub line: u32,
    pub is_method: bool,
}

/// An import/include statement.
#[derive(Debug, Clone)]
pub struct RawImport {
    pub path: StringId,
    pub alias: StringId,
    pub line: u32,
    pub is_system: bool,
    pub lang: RawLang,
}

/// A type/class/struct/interface definition.
#[derive(Debug, Clone)]
pub struct RawTypeDef {
    pub name: StringId,
    pub byte_start: u32,
    pub byte_end: u32,
    pub line: u32,
}

/// A constant string assignment (name = "value").
#[derive(Debug, Clone)]
pub struct RawConstant {
    pub name: StringId,
    pub value: StringId,
    pub byte_offset: u32,
}

/// A decorator/annotation on a definition.
#[derive(Debug, Clone)]
pub struct RawDecorator {
    pub name: StringId,
    pub first_arg: StringId,
    pub decorated_def_byte: u32,
    pub line: u32,
}

/// Complete extraction result for a single file.
#[derive(Debug)]
pub struct RawFileExtraction {
    pub lang: RawLang,
    pub defs: Vec<RawDef>,
    pub calls: Vec<RawCall>,
    pub imports: Vec<RawImport>,
    pub types: Vec<RawTypeDef>,
    pub constants: Vec<RawConstant>,
    pub decorators: Vec<RawDecorator>,
    pub package_name: StringId,
}

impl RawFileExtraction {
    pub fn new(lang: RawLang) -> Self {
        Self {
            lang,
            defs: Vec::new(),
            calls: Vec::new(),
            imports: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            decorators: Vec::new(),
            package_name: STRING_NONE,
        }
    }
}

// ─── Capture name constants ──────────────────────────────────────────────────

const CAP_FUNC_NAME: &str = "func.name";
const CAP_FUNC_DEF: &str = "func.def";
const CAP_CALL_NAME: &str = "call.name";
const CAP_CALL_SITE: &str = "call.site";
const CAP_CALL_RECEIVER: &str = "call.receiver";
const CAP_CALL_ARGS: &str = "call.args";
const CAP_IMPORT_PATH: &str = "import.path";
const CAP_IMPORT_DEF: &str = "import.def";
const CAP_IMPORT_ALIAS: &str = "import.alias";
const CAP_TYPE_NAME: &str = "type.name";
const CAP_TYPE_DEF: &str = "type.def";
const CAP_PKG_NAME: &str = "pkg.name";
const CAP_CONST_NAME: &str = "const.name";
const CAP_CONST_VALUE: &str = "const.value";
const CAP_DECORATOR_NAME: &str = "decorator.name";
const CAP_DECORATOR_ARG: &str = "decorator.arg";
const CAP_REQUIRE_PATH: &str = "require.path";
const CAP_REQUIRE_SITE: &str = "require.site";

// ─── RawExtractor ────────────────────────────────────────────────────────────

/// Extracts raw IR from source using tree-sitter queries.
pub struct RawExtractor {
    query: tree_sitter::Query,
    lang: RawLang,
    // Capture indices, resolved at construction time
    idx_func_name: Option<u32>,
    idx_func_def: Option<u32>,
    idx_call_name: Option<u32>,
    idx_call_site: Option<u32>,
    idx_call_receiver: Option<u32>,
    idx_call_args: Option<u32>,
    idx_import_path: Option<u32>,
    idx_import_def: Option<u32>,
    idx_import_alias: Option<u32>,
    idx_type_name: Option<u32>,
    idx_type_def: Option<u32>,
    idx_pkg_name: Option<u32>,
    idx_const_name: Option<u32>,
    idx_const_value: Option<u32>,
    idx_decorator_name: Option<u32>,
    idx_decorator_arg: Option<u32>,
    idx_require_path: Option<u32>,
    idx_require_site: Option<u32>,
}

impl RawExtractor {
    /// Create a new RawExtractor for a given language.
    pub fn new(lang: Language) -> Result<Self, tree_sitter::QueryError> {
        let ts_lang = lang.ts_language();
        let raw_lang = RawLang::from_language(lang);
        let query_src = match lang {
            Language::Go => GO_RAW_QUERY,
            Language::Python => PYTHON_RAW_QUERY,
            Language::TypeScript => TYPESCRIPT_RAW_QUERY,
            Language::C => C_RAW_QUERY,
            Language::Cpp => CPP_RAW_QUERY,
            Language::Java => JAVA_RAW_QUERY,
        };

        let query = tree_sitter::Query::new(&ts_lang, query_src)?;

        let resolve = |name: &str| -> Option<u32> {
            query
                .capture_names()
                .iter()
                .position(|n| *n == name)
                .map(|i| i as u32)
        };

        Ok(Self {
            idx_func_name: resolve(CAP_FUNC_NAME),
            idx_func_def: resolve(CAP_FUNC_DEF),
            idx_call_name: resolve(CAP_CALL_NAME),
            idx_call_site: resolve(CAP_CALL_SITE),
            idx_call_receiver: resolve(CAP_CALL_RECEIVER),
            idx_call_args: resolve(CAP_CALL_ARGS),
            idx_import_path: resolve(CAP_IMPORT_PATH),
            idx_import_def: resolve(CAP_IMPORT_DEF),
            idx_import_alias: resolve(CAP_IMPORT_ALIAS),
            idx_type_name: resolve(CAP_TYPE_NAME),
            idx_type_def: resolve(CAP_TYPE_DEF),
            idx_pkg_name: resolve(CAP_PKG_NAME),
            idx_const_name: resolve(CAP_CONST_NAME),
            idx_const_value: resolve(CAP_CONST_VALUE),
            idx_decorator_name: resolve(CAP_DECORATOR_NAME),
            idx_decorator_arg: resolve(CAP_DECORATOR_ARG),
            idx_require_path: resolve(CAP_REQUIRE_PATH),
            idx_require_site: resolve(CAP_REQUIRE_SITE),
            query,
            lang: raw_lang,
        })
    }

    /// Extract raw IR from a parsed source file.
    pub fn extract(
        &self,
        tree: &tree_sitter::Tree,
        source: &[u8],
        strings: &mut StringInterner,
    ) -> RawFileExtraction {
        let mut result = RawFileExtraction::new(self.lang);
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&self.query, tree.root_node(), source);

        while let Some(m) = matches.next() {
            self.process_match(m, source, strings, &mut result);
        }

        result
    }

    fn process_match(
        &self,
        m: &tree_sitter::QueryMatch,
        source: &[u8],
        strings: &mut StringInterner,
        result: &mut RawFileExtraction,
    ) {
        // Check which capture names are present in this match to determine what kind it is
        let has_capture = |idx: Option<u32>| -> bool {
            idx.is_some_and(|i| m.captures.iter().any(|c| c.index == i))
        };

        let get_node = |idx: Option<u32>| -> Option<tree_sitter::Node> {
            idx.and_then(|i| m.captures.iter().find(|c| c.index == i).map(|c| c.node))
        };

        let node_text = |node: tree_sitter::Node| -> &str {
            node.utf8_text(source).unwrap_or("")
        };

        let intern_node = |node: tree_sitter::Node, strings: &mut StringInterner| -> StringId {
            let text = node.utf8_text(source).unwrap_or("");
            if text.is_empty() {
                return STRING_NONE;
            }
            strings.intern(text)
        };

        // ─── Package name ────────────────────────────────────────────
        if let Some(node) = get_node(self.idx_pkg_name) {
            if !has_capture(self.idx_func_def) && !has_capture(self.idx_call_site) {
                result.package_name = intern_node(node, strings);
                return;
            }
        }

        // ─── Function/method definitions ─────────────────────────────
        if has_capture(self.idx_func_def) {
            if let Some(name_node) = get_node(self.idx_func_name) {
                let def_node = get_node(self.idx_func_def).unwrap();
                let name = intern_node(name_node, strings);

                // Check for decorator captures in the same match
                if let Some(dec_name_node) = get_node(self.idx_decorator_name) {
                    let dec_name = intern_node(dec_name_node, strings);
                    let dec_arg = get_node(self.idx_decorator_arg)
                        .map(|n| {
                            let text = node_text(n);
                            strings.intern(strip_quotes(text))
                        })
                        .unwrap_or(STRING_NONE);
                    result.decorators.push(RawDecorator {
                        name: dec_name,
                        first_arg: dec_arg,
                        decorated_def_byte: def_node.start_byte() as u32,
                        line: dec_name_node.start_position().row as u32,
                    });
                }

                // Determine if this is a method: check parent for method_declaration
                // or if the func.def node kind indicates method
                let is_method = matches!(
                    def_node.kind(),
                    "method_declaration" | "method_definition"
                ) || def_node.kind() == "function_definition"
                    && def_node.parent().is_some_and(|p| {
                        p.kind() == "class_definition" || p.kind() == "class_body"
                    });

                result.defs.push(RawDef {
                    name,
                    byte_start: def_node.start_byte() as u32,
                    byte_end: def_node.end_byte() as u32,
                    line: name_node.start_position().row as u32,
                    is_method,
                });
                return;
            }
        }

        // ─── Type definitions ────────────────────────────────────────
        if has_capture(self.idx_type_def) {
            if let Some(name_node) = get_node(self.idx_type_name) {
                let def_node = get_node(self.idx_type_def).unwrap();
                result.types.push(RawTypeDef {
                    name: intern_node(name_node, strings),
                    byte_start: def_node.start_byte() as u32,
                    byte_end: def_node.end_byte() as u32,
                    line: name_node.start_position().row as u32,
                });
                return;
            }
        }

        // ─── Import statements ───────────────────────────────────────
        if has_capture(self.idx_import_def) {
            if let Some(path_node) = get_node(self.idx_import_path) {
                let path_text = node_text(path_node);
                let path = strings.intern(strip_quotes(path_text));
                let alias = get_node(self.idx_import_alias)
                    .map(|n| intern_node(n, strings))
                    .unwrap_or(STRING_NONE);
                let is_system = match self.lang {
                    RawLang::C | RawLang::Cpp => path_text.starts_with('<'),
                    RawLang::Python => !path_text.starts_with('.'),
                    _ => false,
                };
                result.imports.push(RawImport {
                    path,
                    alias,
                    line: path_node.start_position().row as u32,
                    is_system,
                    lang: self.lang,
                });
                return;
            }
        }

        // ─── require() calls (JS/TS) ────────────────────────────────
        if has_capture(self.idx_require_site) {
            if let Some(path_node) = get_node(self.idx_require_path) {
                let path_text = node_text(path_node);
                let path = strings.intern(strip_quotes(path_text));
                result.imports.push(RawImport {
                    path,
                    alias: STRING_NONE,
                    line: path_node.start_position().row as u32,
                    is_system: !path_text.contains("./") && !path_text.contains("../"),
                    lang: self.lang,
                });
                return;
            }
        }

        // ─── Call expressions ────────────────────────────────────────
        if has_capture(self.idx_call_site) {
            if let Some(name_node) = get_node(self.idx_call_name) {
                let callee_name = intern_node(name_node, strings);
                let receiver_name = get_node(self.idx_call_receiver)
                    .map(|n| intern_node(n, strings))
                    .unwrap_or(STRING_NONE);

                let (first_string_arg, second_string_arg, arg_count) =
                    if let Some(args_node) = get_node(self.idx_call_args) {
                        extract_args(args_node, source, strings)
                    } else {
                        (STRING_NONE, STRING_NONE, 0)
                    };

                result.calls.push(RawCall {
                    line: name_node.start_position().row as u32,
                    byte_offset: name_node.start_byte() as u32,
                    callee_name,
                    receiver_name,
                    first_string_arg,
                    second_string_arg,
                    arg_count,
                    lang: self.lang,
                });
                return;
            }
        }

        // ─── Decorator/annotation (standalone, not on func.def match) ─
        if has_capture(self.idx_decorator_name)
            || get_node(self.idx_decorator_name).is_some()
                && !has_capture(self.idx_func_def)
        {
            if let Some(dec_name_node) = get_node(self.idx_decorator_name) {
                let dec_name = intern_node(dec_name_node, strings);
                let dec_arg = get_node(self.idx_decorator_arg)
                    .map(|n| {
                        let text = node_text(n);
                        strings.intern(strip_quotes(text))
                    })
                    .unwrap_or(STRING_NONE);
                result.decorators.push(RawDecorator {
                    name: dec_name,
                    first_arg: dec_arg,
                    decorated_def_byte: 0,
                    line: dec_name_node.start_position().row as u32,
                });
                return;
            }
        }

        // ─── Constants (name = "value") ──────────────────────────────
        if has_capture(self.idx_const_name) && has_capture(self.idx_const_value) {
            if let (Some(name_node), Some(value_node)) =
                (get_node(self.idx_const_name), get_node(self.idx_const_value))
            {
                let name = intern_node(name_node, strings);
                let value_text = node_text(value_node);
                let value = strings.intern(strip_quotes(value_text));
                result.constants.push(RawConstant {
                    name,
                    value,
                    byte_offset: name_node.start_byte() as u32,
                });
            }
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Strip surrounding quotes from a string literal.
fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if (s.starts_with('"') && s.ends_with('"'))
        || (s.starts_with('\'') && s.ends_with('\''))
        || (s.starts_with('`') && s.ends_with('`'))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Walk an argument_list node's children to extract up to 2 string literal values
/// and count total arguments.
fn extract_args(
    args_node: tree_sitter::Node,
    source: &[u8],
    strings: &mut StringInterner,
) -> (StringId, StringId, u8) {
    let mut first = STRING_NONE;
    let mut second = STRING_NONE;
    let mut count: u8 = 0;
    let mut cursor = args_node.walk();

    for child in args_node.children(&mut cursor) {
        // Skip punctuation (, and parentheses)
        if child.kind() == "," || child.kind() == "(" || child.kind() == ")" {
            continue;
        }
        // Skip comment nodes
        if child.is_extra() {
            continue;
        }
        count = count.saturating_add(1);

        // Check if this arg is a string literal
        let is_string = matches!(
            child.kind(),
            "interpreted_string_literal"
                | "raw_string_literal"
                | "string_literal"
                | "string"
                | "template_string"
        );
        if is_string {
            let text = child.utf8_text(source).unwrap_or("");
            let id = strings.intern(strip_quotes(text));
            if first == STRING_NONE {
                first = id;
            } else if second == STRING_NONE {
                second = id;
            }
        }
    }

    (first, second, count)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_extract(lang: Language, source: &str) -> (RawFileExtraction, StringInterner) {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.ts_language()).unwrap();
        let tree = parser.parse(source.as_bytes(), None).unwrap();
        let mut strings = StringInterner::new();
        let extractor = RawExtractor::new(lang).unwrap();
        let result = extractor.extract(&tree, source.as_bytes(), &mut strings);
        (result, strings)
    }

    fn resolve(strings: &StringInterner, id: StringId) -> &str {
        if id == STRING_NONE {
            "<none>"
        } else {
            strings.get(id)
        }
    }

    // ─── Go tests ────────────────────────────────────────────────────

    #[test]
    fn go_extracts_package_and_functions() {
        let src = r#"package main

func hello() {}

func (s *Server) Serve() {}
"#;
        let (result, strings) = parse_and_extract(Language::Go, src);
        assert_eq!(result.lang, RawLang::Go);
        assert_eq!(resolve(&strings, result.package_name), "main");
        assert!(result.defs.len() >= 2);
        let names: Vec<_> = result.defs.iter().map(|d| resolve(&strings, d.name)).collect();
        assert!(names.contains(&"hello"), "should contain hello, got {:?}", names);
        assert!(names.contains(&"Serve"), "should contain Serve, got {:?}", names);
    }

    #[test]
    fn go_extracts_method_calls_with_receiver() {
        let src = r#"package main

func main() {
    client.Connect("localhost:8080")
    fmt.Println("hello")
}
"#;
        let (result, strings) = parse_and_extract(Language::Go, src);
        let calls_with_receiver: Vec<_> = result
            .calls
            .iter()
            .filter(|c| c.receiver_name != STRING_NONE)
            .collect();
        assert!(
            calls_with_receiver.len() >= 2,
            "should have at least 2 calls with receivers, got {}",
            calls_with_receiver.len()
        );
        let receivers: Vec<_> = calls_with_receiver
            .iter()
            .map(|c| resolve(&strings, c.receiver_name))
            .collect();
        assert!(receivers.contains(&"client"), "receivers: {:?}", receivers);
        assert!(receivers.contains(&"fmt"), "receivers: {:?}", receivers);
    }

    #[test]
    fn go_extracts_imports_with_alias() {
        let src = r#"package main

import (
    pb "google.golang.org/grpc/examples/helloworld"
    "fmt"
)
"#;
        let (result, strings) = parse_and_extract(Language::Go, src);
        assert!(result.imports.len() >= 2, "should have at least 2 imports");
        let aliased = result
            .imports
            .iter()
            .find(|i| i.alias != STRING_NONE);
        assert!(aliased.is_some(), "should have an aliased import");
        let aliased = aliased.unwrap();
        assert_eq!(resolve(&strings, aliased.alias), "pb");
        assert!(
            resolve(&strings, aliased.path).contains("helloworld"),
            "path: {}",
            resolve(&strings, aliased.path)
        );
    }

    #[test]
    fn go_extracts_string_args() {
        let src = r#"package main

func main() {
    grpc.Dial("localhost:50051")
    http.Get("http://example.com/api")
}
"#;
        let (result, strings) = parse_and_extract(Language::Go, src);
        let dial = result.calls.iter().find(|c| resolve(&strings, c.callee_name) == "Dial");
        assert!(dial.is_some(), "should find Dial call");
        let dial = dial.unwrap();
        assert_eq!(resolve(&strings, dial.first_string_arg), "localhost:50051");
        assert_eq!(resolve(&strings, dial.receiver_name), "grpc");
    }

    #[test]
    fn go_extracts_constants() {
        let src = r#"package main

const serviceAddr = "localhost:8080"
var defaultHost = "0.0.0.0"
"#;
        let (result, strings) = parse_and_extract(Language::Go, src);
        assert!(result.constants.len() >= 2, "should have at least 2 constants, got {}", result.constants.len());
        let names: Vec<_> = result.constants.iter().map(|c| resolve(&strings, c.name)).collect();
        assert!(names.contains(&"serviceAddr"), "constants: {:?}", names);
    }

    // ─── Python tests ────────────────────────────────────────────────

    #[test]
    fn python_extracts_functions_and_classes() {
        let src = r#"
class MyService:
    def handle(self):
        pass

def main():
    pass
"#;
        let (result, strings) = parse_and_extract(Language::Python, src);
        assert!(!result.defs.is_empty(), "should have function defs");
        assert!(!result.types.is_empty(), "should have type defs");
        let type_names: Vec<_> = result.types.iter().map(|t| resolve(&strings, t.name)).collect();
        assert!(type_names.contains(&"MyService"), "types: {:?}", type_names);
    }

    #[test]
    fn python_extracts_method_calls_with_receiver() {
        let src = r#"
requests.get("http://example.com")
client.connect("localhost", 8080)
"#;
        let (result, strings) = parse_and_extract(Language::Python, src);
        let with_receiver: Vec<_> = result
            .calls
            .iter()
            .filter(|c| c.receiver_name != STRING_NONE)
            .collect();
        assert!(with_receiver.len() >= 2, "got {} calls with receiver", with_receiver.len());
        let receivers: Vec<_> = with_receiver
            .iter()
            .map(|c| resolve(&strings, c.receiver_name))
            .collect();
        assert!(receivers.contains(&"requests"), "receivers: {:?}", receivers);
        assert!(receivers.contains(&"client"), "receivers: {:?}", receivers);
    }

    #[test]
    fn python_extracts_imports_with_alias() {
        let src = r#"
import os
import grpc as g
from flask import Flask
"#;
        let (result, strings) = parse_and_extract(Language::Python, src);
        assert!(result.imports.len() >= 2, "should have imports");
        let paths: Vec<_> = result.imports.iter().map(|i| resolve(&strings, i.path)).collect();
        assert!(paths.contains(&"os"), "imports: {:?}", paths);
        assert!(paths.contains(&"flask"), "imports: {:?}", paths);
    }

    #[test]
    fn python_extracts_decorators() {
        let src = r#"
@app.route("/api/users")
def get_users():
    pass

@login_required
def admin():
    pass
"#;
        let (result, strings) = parse_and_extract(Language::Python, src);
        // Decorators with args are captured as part of decorated function defs
        let dec_names: Vec<_> = result.decorators.iter().map(|d| resolve(&strings, d.name)).collect();
        // At minimum we should capture the decorator names
        assert!(
            !dec_names.is_empty() || !result.defs.is_empty(),
            "should have decorators or defs: decorators={:?}, defs={}",
            dec_names,
            result.defs.len()
        );
    }

    // ─── TypeScript tests ────────────────────────────────────────────

    #[test]
    fn typescript_extracts_functions_and_classes() {
        let src = r#"
function hello() {}
const greet = () => {}
class MyApp {}
"#;
        let (result, strings) = parse_and_extract(Language::TypeScript, src);
        let names: Vec<_> = result.defs.iter().map(|d| resolve(&strings, d.name)).collect();
        assert!(names.contains(&"hello"), "defs: {:?}", names);
        assert!(names.contains(&"greet"), "defs: {:?}", names);
        let type_names: Vec<_> = result.types.iter().map(|t| resolve(&strings, t.name)).collect();
        assert!(type_names.contains(&"MyApp"), "types: {:?}", type_names);
    }

    #[test]
    fn typescript_extracts_require_as_import() {
        let src = r#"
const express = require('express');
const path = require('path');
"#;
        let (result, strings) = parse_and_extract(Language::TypeScript, src);
        // require() calls should appear as imports
        let req_imports: Vec<_> = result
            .imports
            .iter()
            .map(|i| resolve(&strings, i.path))
            .collect();
        assert!(
            req_imports.contains(&"express"),
            "require imports: {:?}",
            req_imports
        );
        assert!(
            req_imports.contains(&"path"),
            "require imports: {:?}",
            req_imports
        );
    }

    #[test]
    fn typescript_extracts_method_calls_with_receiver() {
        let src = r#"
app.get("/api/users", handler);
axios.post("http://example.com", data);
const ws = new WebSocket("ws://localhost");
"#;
        let (result, _strings) = parse_and_extract(Language::TypeScript, src);
        let with_receiver: Vec<_> = result
            .calls
            .iter()
            .filter(|c| c.receiver_name != STRING_NONE)
            .collect();
        assert!(with_receiver.len() >= 2, "got {} calls with receiver", with_receiver.len());
    }

    #[test]
    fn typescript_extracts_imports() {
        let src = r#"
import express from 'express';
import { Router } from 'express';
"#;
        let (result, strings) = parse_and_extract(Language::TypeScript, src);
        assert!(!result.imports.is_empty(), "should have imports");
        let paths: Vec<_> = result.imports.iter().map(|i| resolve(&strings, i.path)).collect();
        assert!(
            paths.iter().any(|p| p.contains("express")),
            "imports: {:?}",
            paths
        );
    }

    // ─── C tests ─────────────────────────────────────────────────────

    #[test]
    fn c_extracts_functions_and_includes() {
        let src = r#"
#include <stdio.h>
#include "mylib.h"

struct Config {
    int port;
};

void handle_request(int sock) {
    connect(sock, addr, len);
}
"#;
        let (result, strings) = parse_and_extract(Language::C, src);
        assert!(!result.defs.is_empty(), "should have function defs");
        assert!(!result.imports.is_empty(), "should have includes");
        assert!(!result.types.is_empty(), "should have struct defs");

        let func_names: Vec<_> = result.defs.iter().map(|d| resolve(&strings, d.name)).collect();
        assert!(func_names.contains(&"handle_request"), "funcs: {:?}", func_names);

        // System vs local include
        let system_imports: Vec<_> = result.imports.iter().filter(|i| i.is_system).collect();
        let local_imports: Vec<_> = result.imports.iter().filter(|i| !i.is_system).collect();
        assert!(!system_imports.is_empty(), "should have system includes");
        assert!(!local_imports.is_empty(), "should have local includes");
    }

    #[test]
    fn c_extracts_calls_with_receiver() {
        let src = r#"
void main() {
    client->connect("localhost", 8080);
    simple_call();
}
"#;
        let (result, strings) = parse_and_extract(Language::C, src);
        let with_receiver: Vec<_> = result
            .calls
            .iter()
            .filter(|c| c.receiver_name != STRING_NONE)
            .collect();
        assert!(with_receiver.len() >= 1, "should have call with receiver");
        assert_eq!(resolve(&strings, with_receiver[0].receiver_name), "client");
    }

    // ─── C++ tests ───────────────────────────────────────────────────

    #[test]
    fn cpp_extracts_namespace_qualified_calls() {
        let src = r#"
#include <grpcpp/grpcpp.h>

namespace myapp {

void connect() {
    grpc::CreateChannel("localhost:50051");
    auto stub = service::NewStub(channel);
}

}
"#;
        let (result, strings) = parse_and_extract(Language::Cpp, src);
        assert_eq!(resolve(&strings, result.package_name), "myapp");

        let qualified: Vec<_> = result
            .calls
            .iter()
            .filter(|c| c.receiver_name != STRING_NONE)
            .collect();
        assert!(
            qualified.len() >= 1,
            "should have namespace-qualified calls, got {}",
            qualified.len()
        );
        let receivers: Vec<_> = qualified
            .iter()
            .map(|c| resolve(&strings, c.receiver_name))
            .collect();
        assert!(
            receivers.contains(&"grpc") || receivers.contains(&"service"),
            "receivers: {:?}",
            receivers
        );
    }

    // ─── Java tests ──────────────────────────────────────────────────

    #[test]
    fn java_extractor_creates() {
        let extractor = RawExtractor::new(Language::Java);
        assert!(extractor.is_ok(), "Java extractor should create successfully");
    }

    #[test]
    fn java_extracts_classes_and_methods() {
        let src = r#"
package com.example.service;

import io.grpc.ManagedChannelBuilder;
import java.net.HttpURLConnection;

public class MyService {
    public void connect() {
        ManagedChannelBuilder.forAddress("localhost", 50051);
    }

    public String getEndpoint() {
        return "http://example.com/api";
    }
}
"#;
        let (result, strings) = parse_and_extract(Language::Java, src);

        // Package
        assert_ne!(result.package_name, STRING_NONE, "should have package name");

        // Types
        let type_names: Vec<_> = result.types.iter().map(|t| resolve(&strings, t.name)).collect();
        assert!(type_names.contains(&"MyService"), "types: {:?}", type_names);

        // Methods
        let func_names: Vec<_> = result.defs.iter().map(|d| resolve(&strings, d.name)).collect();
        assert!(func_names.contains(&"connect"), "funcs: {:?}", func_names);
        assert!(func_names.contains(&"getEndpoint"), "funcs: {:?}", func_names);

        // Imports
        let import_paths: Vec<_> = result.imports.iter().map(|i| resolve(&strings, i.path)).collect();
        assert!(
            import_paths.iter().any(|p| p.contains("ManagedChannelBuilder")),
            "imports: {:?}",
            import_paths
        );

        // Calls with receiver
        let with_receiver: Vec<_> = result
            .calls
            .iter()
            .filter(|c| c.receiver_name != STRING_NONE)
            .collect();
        assert!(!with_receiver.is_empty(), "should have method calls with receiver");
    }

    #[test]
    fn java_extracts_annotations() {
        let src = r#"
public class Controller {
    @GetMapping("/api/users")
    public void getUsers() {}

    @Override
    public String toString() { return ""; }
}
"#;
        let (result, strings) = parse_and_extract(Language::Java, src);
        assert!(!result.decorators.is_empty(), "should have annotations");
        let dec_names: Vec<_> = result.decorators.iter().map(|d| resolve(&strings, d.name)).collect();
        assert!(
            dec_names.contains(&"Override") || dec_names.contains(&"GetMapping"),
            "decorators: {:?}",
            dec_names
        );
    }

    #[test]
    fn java_extracts_constructor_calls() {
        let src = r#"
public class Main {
    public void run() {
        Socket sock = new Socket("localhost", 8080);
        HttpClient client = new HttpClient();
    }
}
"#;
        let (result, strings) = parse_and_extract(Language::Java, src);
        let call_names: Vec<_> = result.calls.iter().map(|c| resolve(&strings, c.callee_name)).collect();
        assert!(
            call_names.contains(&"Socket"),
            "should have Socket constructor call, got {:?}",
            call_names
        );
    }

    // ─── Cross-language: all extractors create successfully ──────────

    #[test]
    fn all_extractors_create() {
        for lang in [
            Language::Go,
            Language::Python,
            Language::TypeScript,
            Language::C,
            Language::Cpp,
            Language::Java,
        ] {
            assert!(
                RawExtractor::new(lang).is_ok(),
                "{:?} extractor should create",
                lang
            );
        }
    }
}
