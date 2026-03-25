//! Connection pattern query files for HTTP, WebSocket, message queue, and env var detection.
//!
//! Each constant embeds a tree-sitter query file that detects connection patterns
//! in source code. These queries are concatenated with the language's symbol queries
//! to create a combined extractor.

use crate::grammars::Language;

// Go connection patterns
pub const GO_HTTP_SERVER: &str = include_str!("../queries/go-http-server.scm");
pub const GO_HTTP_CLIENT: &str = include_str!("../queries/go-http-client.scm");
pub const GO_WEBSOCKET: &str = include_str!("../queries/go-websocket.scm");
pub const GO_MESSAGEQUEUE: &str = include_str!("../queries/go-messagequeue.scm");
pub const GO_ENVVAR: &str = include_str!("../queries/go-envvar.scm");

// TypeScript/JavaScript connection patterns
pub const TS_HTTP_SERVER: &str = include_str!("../queries/typescript-http-server.scm");
pub const TS_HTTP_CLIENT: &str = include_str!("../queries/typescript-http-client.scm");
pub const TS_WEBSOCKET: &str = include_str!("../queries/typescript-websocket.scm");
pub const TS_MESSAGEQUEUE: &str = include_str!("../queries/typescript-messagequeue.scm");
pub const TS_ENVVAR: &str = include_str!("../queries/typescript-envvar.scm");

// Python connection patterns
pub const PY_HTTP_SERVER: &str = include_str!("../queries/python-http-server.scm");
pub const PY_HTTP_CLIENT: &str = include_str!("../queries/python-http-client.scm");
pub const PY_WEBSOCKET: &str = include_str!("../queries/python-websocket.scm");
pub const PY_MESSAGEQUEUE: &str = include_str!("../queries/python-messagequeue.scm");
pub const PY_ENVVAR: &str = include_str!("../queries/python-envvar.scm");

/// Get all connection pattern queries for a language, concatenated into a single string.
/// Returns empty string for languages without connection pattern support.
pub fn connection_queries(lang: Language) -> String {
    match lang {
        Language::Go => [GO_HTTP_SERVER, GO_HTTP_CLIENT, GO_WEBSOCKET, GO_MESSAGEQUEUE, GO_ENVVAR]
            .join("\n"),
        Language::TypeScript => {
            [TS_HTTP_SERVER, TS_HTTP_CLIENT, TS_WEBSOCKET, TS_MESSAGEQUEUE, TS_ENVVAR].join("\n")
        }
        Language::Python => {
            [PY_HTTP_SERVER, PY_HTTP_CLIENT, PY_WEBSOCKET, PY_MESSAGEQUEUE, PY_ENVVAR].join("\n")
        }
        _ => String::new(),
    }
}
