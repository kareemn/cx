//! LSP client for type-resolved code intelligence.
//!
//! Provides a JSON-RPC 2.0 over stdio client that communicates with language
//! servers (gopls, pyright/ty, typescript-language-server, clangd, jdtls) to
//! resolve types, definitions, and references. Used by the taint tracer to
//! get fully qualified names and trace variable origins.
//!
//! LSP is always optional — all functions return `Option` or `Result` and
//! handle server-not-found gracefully.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A position in a text document (0-indexed line and character).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// A range in a text document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// A location returned by definition/references requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

/// Result of a hover request — the type or documentation string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverResult {
    pub contents: String,
}

/// Which language an LSP server handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LspLanguage {
    Go,
    Python,
    TypeScript,
    C,
    Cpp,
    Java,
    Rust,
}

impl LspLanguage {
    /// File extensions this language covers.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            LspLanguage::Go => &["go"],
            LspLanguage::Python => &["py", "pyi"],
            LspLanguage::TypeScript => &["ts", "tsx", "js", "jsx"],
            LspLanguage::C => &["c", "h"],
            LspLanguage::Cpp => &["cpp", "cxx", "cc", "hpp", "hxx"],
            LspLanguage::Java => &["java"],
            LspLanguage::Rust => &["rs"],
        }
    }

    /// The language ID string used in LSP textDocument/didOpen.
    pub fn language_id(self) -> &'static str {
        match self {
            LspLanguage::Go => "go",
            LspLanguage::Python => "python",
            LspLanguage::TypeScript => "typescript",
            LspLanguage::C => "c",
            LspLanguage::Cpp => "cpp",
            LspLanguage::Java => "java",
            LspLanguage::Rust => "rust",
        }
    }
}

/// Error type for LSP operations.
#[derive(Debug, thiserror::Error)]
pub enum LspError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("server not found: {0}")]
    ServerNotFound(String),

    #[error("server failed to initialize: {0}")]
    InitFailed(String),

    #[error("request failed: {0}")]
    RequestFailed(String),

    #[error("server stdin/stdout unavailable")]
    NoStdio,
}

pub type Result<T> = std::result::Result<T, LspError>;

// ---------------------------------------------------------------------------
// Server detection
// ---------------------------------------------------------------------------

/// A known LSP server binary and its arguments.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub language: LspLanguage,
    pub binary: String,
    pub args: Vec<String>,
}

/// Detect which LSP servers are available on this system.
/// Returns configs for all found servers. Never errors — missing servers
/// are silently skipped.
pub fn detect_servers() -> Vec<ServerConfig> {
    let candidates: &[(&str, &[&str], LspLanguage)] = &[
        ("gopls", &["serve"], LspLanguage::Go),
        ("ty", &["server"], LspLanguage::Python),
        ("pyright-langserver", &["--stdio"], LspLanguage::Python),
        (
            "typescript-language-server",
            &["--stdio"],
            LspLanguage::TypeScript,
        ),
        ("clangd", &[], LspLanguage::C),
        ("clangd", &[], LspLanguage::Cpp),
        ("jdtls", &[], LspLanguage::Java),
        ("rust-analyzer", &[], LspLanguage::Rust),
    ];

    let mut found: HashMap<LspLanguage, ServerConfig> = HashMap::new();

    for &(binary, args, lang) in candidates {
        // If we already have a server for this language, skip (first match wins = preferred).
        if found.contains_key(&lang) {
            continue;
        }
        if which_binary(binary) {
            found.insert(
                lang,
                ServerConfig {
                    language: lang,
                    binary: binary.to_string(),
                    args: args.iter().map(|s| s.to_string()).collect(),
                },
            );
        }
    }

    found.into_values().collect()
}

/// Check if a binary is on PATH.
fn which_binary(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// JSON-RPC types (internal)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: i64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct JsonRpcNotification {
    jsonrpc: &'static str,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<i64>,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ---------------------------------------------------------------------------
// LspClient
// ---------------------------------------------------------------------------

/// A single LSP server connection over stdio.
pub struct LspClient {
    config: ServerConfig,
    process: Child,
    stdin: Mutex<Box<dyn Write + Send>>,
    stdout: Mutex<BufReader<Box<dyn Read + Send>>>,
    next_id: AtomicI64,
    _root_uri: String,
}

impl LspClient {
    /// Spawn an LSP server and perform the initialize/initialized handshake.
    ///
    /// Returns `Err(LspError::ServerNotFound)` if the binary is not on PATH.
    /// Returns `Err(LspError::InitFailed)` if the handshake fails.
    pub fn start(config: ServerConfig, workspace_root: &Path) -> Result<Self> {
        let mut child = Command::new(&config.binary)
            .args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(workspace_root)
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    LspError::ServerNotFound(config.binary.clone())
                } else {
                    LspError::Io(e)
                }
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or(LspError::NoStdio)
            .map(|s| -> Box<dyn Write + Send> { Box::new(s) })?;
        let stdout = child
            .stdout
            .take()
            .ok_or(LspError::NoStdio)
            .map(|s| -> Box<dyn Read + Send> { Box::new(s) })?;

        let root_uri = format!("file://{}", workspace_root.display());

        let mut client = Self {
            config,
            process: child,
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
            next_id: AtomicI64::new(1),
            _root_uri: root_uri.clone(),
        };

        // Initialize handshake
        let init_params = serde_json::json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "hover": {
                        "contentFormat": ["plaintext"]
                    },
                    "definition": {
                        "linkSupport": false
                    },
                    "references": {}
                }
            },
            "workspaceFolders": [{
                "uri": root_uri,
                "name": workspace_root.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            }]
        });

        let resp = client.send_request("initialize", Some(init_params))?;
        if resp.is_none() {
            return Err(LspError::InitFailed(
                "initialize returned null".to_string(),
            ));
        }

        // Send initialized notification
        client.send_notification("initialized", Some(serde_json::json!({})))?;

        Ok(client)
    }

    /// The language this client handles.
    pub fn language(&self) -> LspLanguage {
        self.config.language
    }

    /// The server binary name.
    pub fn server_name(&self) -> &str {
        &self.config.binary
    }

    // ---- LSP requests ----

    /// textDocument/hover — get type information at a position.
    ///
    /// Returns `None` if the server returns no hover data.
    pub fn hover(&mut self, file: &Path, pos: Position) -> Result<Option<HoverResult>> {
        let uri = path_to_uri(file);
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": pos.line, "character": pos.character }
        });

        let resp = self.send_request("textDocument/hover", Some(params))?;
        let Some(val) = resp else { return Ok(None) };

        // The hover result has a `contents` field that can be a string,
        // a MarkupContent, or an array of MarkedString.
        let contents = extract_hover_contents(&val);
        Ok(contents.map(|c| HoverResult { contents: c }))
    }

    /// textDocument/definition — go to the definition of a symbol.
    ///
    /// Returns an empty vec if no definition is found.
    pub fn definition(&mut self, file: &Path, pos: Position) -> Result<Vec<Location>> {
        let uri = path_to_uri(file);
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": pos.line, "character": pos.character }
        });

        let resp = self.send_request("textDocument/definition", Some(params))?;
        let Some(val) = resp else {
            return Ok(Vec::new());
        };

        Ok(parse_locations(&val))
    }

    /// textDocument/references — find all references to a symbol.
    ///
    /// Returns an empty vec if no references are found.
    pub fn references(&mut self, file: &Path, pos: Position) -> Result<Vec<Location>> {
        let uri = path_to_uri(file);
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": pos.line, "character": pos.character },
            "context": { "includeDeclaration": true }
        });

        let resp = self.send_request("textDocument/references", Some(params))?;
        let Some(val) = resp else {
            return Ok(Vec::new());
        };

        Ok(parse_locations(&val))
    }

    /// Notify the server that a file has been opened.
    pub fn did_open(&mut self, file: &Path, language_id: &str, text: &str) -> Result<()> {
        let uri = path_to_uri(file);
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": language_id,
                "version": 1,
                "text": text
            }
        });
        self.send_notification("textDocument/didOpen", Some(params))
    }

    /// Shut down the server gracefully.
    pub fn shutdown(mut self) -> Result<()> {
        let _ = self.send_request("shutdown", None);
        let _ = self.send_notification("exit", None);
        let _ = self.process.wait();
        Ok(())
    }

    // ---- JSON-RPC transport ----

    fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<Option<serde_json::Value>> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };
        let body = serde_json::to_string(&req)?;
        self.write_message(&body)?;
        self.read_response(id)
    }

    fn send_notification(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<()> {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0",
            method: method.to_string(),
            params,
        };
        let body = serde_json::to_string(&notif)?;
        self.write_message(&body)
    }

    fn write_message(&self, body: &str) -> Result<()> {
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let mut stdin = self.stdin.lock().unwrap();
        stdin.write_all(header.as_bytes())?;
        stdin.write_all(body.as_bytes())?;
        stdin.flush()?;
        Ok(())
    }

    fn read_response(&self, expected_id: i64) -> Result<Option<serde_json::Value>> {
        let mut stdout = self.stdout.lock().unwrap();
        // Read messages until we find the response matching our request ID.
        // Notifications and other messages from the server are skipped.
        loop {
            let content_length = read_content_length(&mut *stdout)?;
            let mut buf = vec![0u8; content_length];
            stdout.read_exact(&mut buf)?;

            let resp: JsonRpcResponse = serde_json::from_slice(&buf)?;

            // Skip notifications (no id) and responses for other requests
            if resp.id == Some(expected_id) {
                if let Some(err) = resp.error {
                    return Err(LspError::RequestFailed(format!(
                        "[{}] {}",
                        err.code, err.message
                    )));
                }
                return Ok(resp.result);
            }
            // Otherwise, keep reading (server notification or out-of-order response)
        }
    }
}

impl Drop for LspClient {
    fn drop(&mut self) {
        // Best-effort: kill the child process if still running.
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

// ---------------------------------------------------------------------------
// LspOrchestrator
// ---------------------------------------------------------------------------

/// Manages multiple LSP servers, one per language, for a workspace.
pub struct LspOrchestrator {
    clients: HashMap<LspLanguage, LspClient>,
}

impl LspOrchestrator {
    /// Start all detected LSP servers for the given workspace root.
    ///
    /// Servers that fail to start are silently skipped. Returns an orchestrator
    /// even if no servers started (all queries will return `None`).
    pub fn start(workspace_root: &Path) -> Self {
        let configs = detect_servers();
        let mut clients = HashMap::new();

        for config in configs {
            let lang = config.language;
            match LspClient::start(config, workspace_root) {
                Ok(client) => {
                    clients.insert(lang, client);
                }
                Err(_) => {
                    // Server failed to start — skip silently.
                    // cx works without LSP; results will be "heuristic" instead of "type-confirmed".
                }
            }
        }

        Self { clients }
    }

    /// Start with specific server configs (for testing or custom setups).
    pub fn start_with_configs(configs: Vec<ServerConfig>, workspace_root: &Path) -> Self {
        let mut clients = HashMap::new();
        for config in configs {
            let lang = config.language;
            if let Ok(client) = LspClient::start(config, workspace_root) {
                clients.insert(lang, client);
            }
        }
        Self { clients }
    }

    /// Which languages have active LSP servers.
    pub fn active_languages(&self) -> Vec<LspLanguage> {
        self.clients.keys().copied().collect()
    }

    /// Whether any LSP servers are active.
    pub fn has_servers(&self) -> bool {
        !self.clients.is_empty()
    }

    /// Get the client for a specific language (if available).
    pub fn client_mut(&mut self, lang: LspLanguage) -> Option<&mut LspClient> {
        self.clients.get_mut(&lang)
    }

    /// Resolve the language for a file path based on extension.
    pub fn language_for_file(path: &Path) -> Option<LspLanguage> {
        let ext = path.extension()?.to_str()?;
        let langs = [
            LspLanguage::Go,
            LspLanguage::Python,
            LspLanguage::TypeScript,
            LspLanguage::C,
            LspLanguage::Cpp,
            LspLanguage::Java,
            LspLanguage::Rust,
        ];
        langs.into_iter().find(|&lang| lang.extensions().contains(&ext))
    }

    /// Hover at a position in a file. Returns None if no LSP server is
    /// available for this file's language, or if the server returns no data.
    pub fn hover(&mut self, file: &Path, pos: Position) -> Option<HoverResult> {
        let lang = Self::language_for_file(file)?;
        let client = self.clients.get_mut(&lang)?;
        client.hover(file, pos).ok().flatten()
    }

    /// Go to definition at a position. Returns empty vec if unavailable.
    pub fn definition(&mut self, file: &Path, pos: Position) -> Vec<Location> {
        let lang = match Self::language_for_file(file) {
            Some(l) => l,
            None => return Vec::new(),
        };
        let client = match self.clients.get_mut(&lang) {
            Some(c) => c,
            None => return Vec::new(),
        };
        client.definition(file, pos).unwrap_or_default()
    }

    /// Find references at a position. Returns empty vec if unavailable.
    pub fn references(&mut self, file: &Path, pos: Position) -> Vec<Location> {
        let lang = match Self::language_for_file(file) {
            Some(l) => l,
            None => return Vec::new(),
        };
        let client = match self.clients.get_mut(&lang) {
            Some(c) => c,
            None => return Vec::new(),
        };
        client.references(file, pos).unwrap_or_default()
    }

    /// Shut down all servers gracefully.
    pub fn shutdown(self) {
        for (_, client) in self.clients {
            let _ = client.shutdown();
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a filesystem path to a file:// URI.
fn path_to_uri(path: &Path) -> String {
    // Canonicalize if possible, otherwise use as-is.
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!("file://{}", abs.display())
}

/// Read the Content-Length header from an LSP message.
fn read_content_length<R: BufRead>(reader: &mut R) -> Result<usize> {
    let mut length: Option<usize> = None;
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Err(LspError::RequestFailed("unexpected EOF".to_string()));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // End of headers
            break;
        }
        if let Some(val) = trimmed.strip_prefix("Content-Length: ") {
            length = val.parse().ok();
        }
    }

    length.ok_or_else(|| LspError::RequestFailed("missing Content-Length header".to_string()))
}

/// Extract a plain-text string from a hover result's `contents` field.
fn extract_hover_contents(val: &serde_json::Value) -> Option<String> {
    let contents = val.get("contents")?;

    // MarkupContent: { kind: "...", value: "..." }
    if let Some(value) = contents.get("value") {
        return value.as_str().map(|s| s.to_string());
    }
    // Plain string
    if let Some(s) = contents.as_str() {
        return Some(s.to_string());
    }
    // Array of MarkedString
    if let Some(arr) = contents.as_array() {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.get("value")?.as_str().map(|s| s.to_string()))
            })
            .collect();
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    None
}

/// Parse a definition/references response into Locations.
/// The response can be a single Location, an array of Locations, or an array
/// of LocationLinks.
fn parse_locations(val: &serde_json::Value) -> Vec<Location> {
    if let Some(arr) = val.as_array() {
        arr.iter().filter_map(parse_single_location).collect()
    } else {
        // Single location
        parse_single_location(val).into_iter().collect()
    }
}

fn parse_single_location(val: &serde_json::Value) -> Option<Location> {
    // Standard Location: { uri, range }
    if let Some(uri) = val.get("uri").and_then(|u| u.as_str()) {
        let range = parse_range(val.get("range")?)?;
        return Some(Location {
            uri: uri.to_string(),
            range,
        });
    }
    // LocationLink: { targetUri, targetRange, ... }
    if let Some(uri) = val.get("targetUri").and_then(|u| u.as_str()) {
        let range = parse_range(
            val.get("targetSelectionRange")
                .or_else(|| val.get("targetRange"))?,
        )?;
        return Some(Location {
            uri: uri.to_string(),
            range,
        });
    }
    None
}

fn parse_range(val: &serde_json::Value) -> Option<Range> {
    let start = val.get("start")?;
    let end = val.get("end")?;
    Some(Range {
        start: Position {
            line: start.get("line")?.as_u64()? as u32,
            character: start.get("character")?.as_u64()? as u32,
        },
        end: Position {
            line: end.get("line")?.as_u64()? as u32,
            character: end.get("character")?.as_u64()? as u32,
        },
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_serialize() {
        let pos = Position {
            line: 10,
            character: 5,
        };
        let json = serde_json::to_value(pos).unwrap();
        assert_eq!(json["line"], 10);
        assert_eq!(json["character"], 5);
    }

    #[test]
    fn test_path_to_uri() {
        let p = std::path::PathBuf::from("/tmp/test.go");
        let uri = path_to_uri(&p);
        assert!(uri.starts_with("file:///"));
        assert!(uri.contains("test.go"));
    }

    #[test]
    fn test_language_for_file() {
        assert_eq!(
            LspOrchestrator::language_for_file(Path::new("main.go")),
            Some(LspLanguage::Go)
        );
        assert_eq!(
            LspOrchestrator::language_for_file(Path::new("app.py")),
            Some(LspLanguage::Python)
        );
        assert_eq!(
            LspOrchestrator::language_for_file(Path::new("index.ts")),
            Some(LspLanguage::TypeScript)
        );
        assert_eq!(
            LspOrchestrator::language_for_file(Path::new("index.tsx")),
            Some(LspLanguage::TypeScript)
        );
        assert_eq!(
            LspOrchestrator::language_for_file(Path::new("main.c")),
            Some(LspLanguage::C)
        );
        assert_eq!(
            LspOrchestrator::language_for_file(Path::new("main.cpp")),
            Some(LspLanguage::Cpp)
        );
        assert_eq!(
            LspOrchestrator::language_for_file(Path::new("Main.java")),
            Some(LspLanguage::Java)
        );
        assert_eq!(
            LspOrchestrator::language_for_file(Path::new("lib.rs")),
            Some(LspLanguage::Rust)
        );
        assert_eq!(
            LspOrchestrator::language_for_file(Path::new("README.md")),
            None
        );
    }

    #[test]
    fn test_language_extensions() {
        assert!(LspLanguage::Go.extensions().contains(&"go"));
        assert!(LspLanguage::Python.extensions().contains(&"py"));
        assert!(LspLanguage::Python.extensions().contains(&"pyi"));
        assert!(LspLanguage::TypeScript.extensions().contains(&"js"));
    }

    #[test]
    fn test_language_id() {
        assert_eq!(LspLanguage::Go.language_id(), "go");
        assert_eq!(LspLanguage::Python.language_id(), "python");
        assert_eq!(LspLanguage::TypeScript.language_id(), "typescript");
    }

    #[test]
    fn test_extract_hover_markup_content() {
        let val = serde_json::json!({
            "contents": {
                "kind": "plaintext",
                "value": "func grpc.Dial(target string) (*grpc.ClientConn, error)"
            }
        });
        let result = extract_hover_contents(&val);
        assert_eq!(
            result,
            Some("func grpc.Dial(target string) (*grpc.ClientConn, error)".to_string())
        );
    }

    #[test]
    fn test_extract_hover_plain_string() {
        let val = serde_json::json!({
            "contents": "*redis.Client"
        });
        let result = extract_hover_contents(&val);
        assert_eq!(result, Some("*redis.Client".to_string()));
    }

    #[test]
    fn test_extract_hover_marked_string_array() {
        let val = serde_json::json!({
            "contents": [
                { "language": "go", "value": "func Dial(addr string)" },
                "Documentation here"
            ]
        });
        let result = extract_hover_contents(&val);
        assert_eq!(
            result,
            Some("func Dial(addr string)\nDocumentation here".to_string())
        );
    }

    #[test]
    fn test_extract_hover_null_contents() {
        let val = serde_json::json!({});
        let result = extract_hover_contents(&val);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_single_location() {
        let val = serde_json::json!({
            "uri": "file:///home/user/main.go",
            "range": {
                "start": { "line": 10, "character": 4 },
                "end": { "line": 10, "character": 20 }
            }
        });
        let locs = parse_locations(&val);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].uri, "file:///home/user/main.go");
        assert_eq!(locs[0].range.start.line, 10);
        assert_eq!(locs[0].range.start.character, 4);
    }

    #[test]
    fn test_parse_location_array() {
        let val = serde_json::json!([
            {
                "uri": "file:///a.go",
                "range": {
                    "start": { "line": 1, "character": 0 },
                    "end": { "line": 1, "character": 10 }
                }
            },
            {
                "uri": "file:///b.go",
                "range": {
                    "start": { "line": 5, "character": 2 },
                    "end": { "line": 5, "character": 12 }
                }
            }
        ]);
        let locs = parse_locations(&val);
        assert_eq!(locs.len(), 2);
        assert_eq!(locs[0].uri, "file:///a.go");
        assert_eq!(locs[1].uri, "file:///b.go");
    }

    #[test]
    fn test_parse_location_link() {
        let val = serde_json::json!([{
            "targetUri": "file:///target.go",
            "targetRange": {
                "start": { "line": 20, "character": 0 },
                "end": { "line": 25, "character": 1 }
            },
            "targetSelectionRange": {
                "start": { "line": 20, "character": 5 },
                "end": { "line": 20, "character": 15 }
            }
        }]);
        let locs = parse_locations(&val);
        assert_eq!(locs.len(), 1);
        assert_eq!(locs[0].uri, "file:///target.go");
        // Should prefer targetSelectionRange
        assert_eq!(locs[0].range.start.character, 5);
    }

    #[test]
    fn test_parse_empty_locations() {
        let val = serde_json::json!([]);
        let locs = parse_locations(&val);
        assert!(locs.is_empty());
    }

    #[test]
    fn test_read_content_length() {
        let input = b"Content-Length: 42\r\n\r\n";
        let mut reader = BufReader::new(&input[..]);
        let len = read_content_length(&mut reader).unwrap();
        assert_eq!(len, 42);
    }

    #[test]
    fn test_read_content_length_with_extra_headers() {
        let input = b"Content-Type: application/json\r\nContent-Length: 128\r\n\r\n";
        let mut reader = BufReader::new(&input[..]);
        let len = read_content_length(&mut reader).unwrap();
        assert_eq!(len, 128);
    }

    #[test]
    fn test_read_content_length_missing() {
        let input = b"X-Custom: foo\r\n\r\n";
        let mut reader = BufReader::new(&input[..]);
        assert!(read_content_length(&mut reader).is_err());
    }

    #[test]
    fn test_orchestrator_no_servers() {
        // With an empty config set, orchestrator should work but return None/empty.
        let orch = LspOrchestrator {
            clients: HashMap::new(),
        };
        assert!(!orch.has_servers());
        assert!(orch.active_languages().is_empty());
    }

    #[test]
    fn test_orchestrator_hover_no_server() {
        let mut orch = LspOrchestrator {
            clients: HashMap::new(),
        };
        let result = orch.hover(Path::new("main.go"), Position { line: 0, character: 0 });
        assert!(result.is_none());
    }

    #[test]
    fn test_orchestrator_definition_no_server() {
        let mut orch = LspOrchestrator {
            clients: HashMap::new(),
        };
        let result = orch.definition(Path::new("main.go"), Position { line: 0, character: 0 });
        assert!(result.is_empty());
    }

    #[test]
    fn test_orchestrator_references_no_server() {
        let mut orch = LspOrchestrator {
            clients: HashMap::new(),
        };
        let result = orch.references(Path::new("main.py"), Position { line: 0, character: 0 });
        assert!(result.is_empty());
    }

    #[test]
    fn test_orchestrator_unknown_extension() {
        let mut orch = LspOrchestrator {
            clients: HashMap::new(),
        };
        let result = orch.hover(
            Path::new("README.md"),
            Position {
                line: 0,
                character: 0,
            },
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_detect_servers_runs() {
        // Just ensure detect_servers doesn't panic. The result depends on
        // which binaries are installed on the test machine.
        let servers = detect_servers();
        // We can at least verify the structure.
        for s in &servers {
            assert!(!s.binary.is_empty());
        }
    }

    #[test]
    fn test_which_binary_nonexistent() {
        assert!(!which_binary("cx_definitely_not_a_real_binary_12345"));
    }

    #[test]
    fn test_jsonrpc_request_serialization() {
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "textDocument/hover".to_string(),
            params: Some(serde_json::json!({"textDocument": {"uri": "file:///test.go"}})),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "textDocument/hover");
    }

    #[test]
    fn test_jsonrpc_notification_no_id() {
        let notif = JsonRpcNotification {
            jsonrpc: "2.0",
            method: "initialized".to_string(),
            params: Some(serde_json::json!({})),
        };
        let json = serde_json::to_value(&notif).unwrap();
        assert!(json.get("id").is_none());
    }

    #[test]
    fn test_lsp_error_display() {
        let err = LspError::ServerNotFound("gopls".to_string());
        assert_eq!(err.to_string(), "server not found: gopls");

        let err = LspError::InitFailed("timeout".to_string());
        assert_eq!(err.to_string(), "server failed to initialize: timeout");
    }
}
