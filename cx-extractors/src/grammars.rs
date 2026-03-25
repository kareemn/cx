use crate::universal::UniversalExtractor;

/// Supported languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Go,
    TypeScript,
    Python,
}

impl Language {
    /// Detect language from file extension. Returns None for unsupported files.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "go" => Some(Self::Go),
            "ts" | "tsx" => Some(Self::TypeScript),
            "py" => Some(Self::Python),
            _ => None,
        }
    }

    /// Detect language from file path.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        Self::from_extension(ext)
    }

    /// Get the tree-sitter language for this language.
    pub fn ts_language(&self) -> tree_sitter::Language {
        match self {
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
        }
    }
}

/// Embedded query files for each language.
pub const GO_QUERY: &str = include_str!("../queries/go-symbols.scm");

/// Create a UniversalExtractor for a given language.
/// Returns None if no query is available for this language yet.
pub fn extractor_for_language(lang: Language) -> Option<UniversalExtractor> {
    let ts_lang = lang.ts_language();
    let query_src = match lang {
        Language::Go => GO_QUERY,
        // TypeScript and Python queries not yet implemented
        _ => return None,
    };
    UniversalExtractor::new(&ts_lang, query_src).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_from_extension() {
        assert_eq!(Language::from_extension("go"), Some(Language::Go));
        assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
        assert_eq!(Language::from_extension("py"), Some(Language::Python));
        assert_eq!(Language::from_extension("rs"), None);
        assert_eq!(Language::from_extension(""), None);
    }

    #[test]
    fn language_from_path() {
        use std::path::Path;
        assert_eq!(Language::from_path(Path::new("main.go")), Some(Language::Go));
        assert_eq!(Language::from_path(Path::new("src/app.ts")), Some(Language::TypeScript));
        assert_eq!(Language::from_path(Path::new("README.md")), None);
    }

    #[test]
    fn go_extractor_loads() {
        let ext = extractor_for_language(Language::Go);
        assert!(ext.is_some(), "Go extractor should load");
    }

    #[test]
    fn go_query_parses_source() {
        let ext = extractor_for_language(Language::Go).unwrap();
        let lang = Language::Go.ts_language();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();

        let source = b"package main\nfunc hello() {}\n";
        let tree = parser.parse(source, None).unwrap();

        let mut strings = cx_core::graph::string_interner::StringInterner::new();
        let path_id = strings.intern("test.go");

        let file = crate::universal::ParsedFile {
            tree,
            source,
            path: path_id,
            path_str: "test.go",
            repo_id: 0,
        };

        let mut id = 0u32;
        let result = ext.extract(&file, &mut strings, &mut id);
        assert!(!result.nodes.is_empty(), "should extract at least one symbol");
    }
}
