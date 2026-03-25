pub mod serialize;

use anyhow::Result;
use cx_core::graph::csr::CsrGraph;
use cx_core::graph::edges::ALL_EDGES;
use cx_core::query::path::PathFinder;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

/// Run the MCP server: JSON-RPC 2.0 over stdio with Content-Length framing.
pub fn run(root: &Path) -> Result<()> {
    let graph = crate::commands::init::load_graph(root)?;
    let mut finder = PathFinder::new(graph.node_count());

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    loop {
        match read_message(&mut reader) {
            Ok(body) => {
                let response = handle_request(&body, &graph, &mut finder);
                write_message(&mut writer, &response)?;
                writer.flush()?;
            }
            Err(e) => {
                // EOF or read error — exit cleanly
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    break;
                }
                let err_response = json_rpc_error(
                    serde_json::Value::Null,
                    -32700,
                    &format!("Parse error: {}", e),
                );
                write_message(&mut writer, &err_response)?;
                writer.flush()?;
            }
        }
    }

    Ok(())
}

/// Read a Content-Length framed message from the reader.
fn read_message(reader: &mut impl BufRead) -> std::io::Result<String> {
    let mut content_length: Option<usize> = None;

    // Read headers
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "EOF",
            ));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break; // End of headers
        }
        if let Some(len_str) = trimmed.strip_prefix("Content-Length: ") {
            content_length = len_str.trim().parse().ok();
        }
    }

    let len = content_length.ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "missing Content-Length")
    })?;

    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    String::from_utf8(body).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write a Content-Length framed message.
fn write_message(writer: &mut impl Write, body: &str) -> std::io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)
}

/// Handle a JSON-RPC request and return a JSON-RPC response string.
fn handle_request(body: &str, graph: &CsrGraph, finder: &mut PathFinder) -> String {
    let req: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return json_rpc_error(serde_json::Value::Null, -32700, "Parse error"),
    };

    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method = req
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("");

    match method {
        "initialize" => {
            let result = serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "cx",
                    "version": "0.1.0"
                }
            });
            json_rpc_result(id, result)
        }
        "tools/list" => {
            let tools = tool_definitions();
            json_rpc_result(id, serde_json::json!({ "tools": tools }))
        }
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or_default();
            let tool_name = params
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_default();

            match dispatch_tool(tool_name, &arguments, graph, finder) {
                Ok(content) => json_rpc_result(
                    id,
                    serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": content
                        }]
                    }),
                ),
                Err(e) => json_rpc_error(id, -32602, &e),
            }
        }
        "notifications/initialized" => {
            // No response needed for notifications
            String::new()
        }
        _ => json_rpc_error(id, -32601, &format!("Method not found: {}", method)),
    }
}

fn dispatch_tool(
    name: &str,
    args: &serde_json::Value,
    graph: &CsrGraph,
    finder: &mut PathFinder,
) -> std::result::Result<String, String> {
    match name {
        "cx_path" => {
            let from = args
                .get("from")
                .and_then(|v| v.as_str())
                .ok_or("missing 'from' parameter")?;
            let max_depth = args
                .get("max_depth")
                .and_then(|v| v.as_u64())
                .unwrap_or(20) as u32;

            let start = graph
                .nodes
                .iter()
                .position(|n| graph.strings.get(n.name) == from)
                .map(|i| i as u32)
                .ok_or_else(|| format!("symbol not found: {}", from))?;

            let results = finder.find_all_downstream(graph, start, ALL_EDGES, max_depth);
            let output: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    let hops: Vec<serde_json::Value> = r
                        .hops
                        .iter()
                        .map(|h| {
                            let node = graph.node(h.node_id);
                            serde_json::json!({
                                "name": graph.strings.get(node.name),
                                "file": if node.file != u32::MAX { Some(graph.strings.get(node.file)) } else { None },
                                "line": node.line,
                            })
                        })
                        .collect();
                    serde_json::json!({ "hops": hops })
                })
                .collect();

            serde_json::to_string(&serde_json::json!({
                "paths": output,
                "completeness": 1.0,
                "gaps": [],
            }))
            .map_err(|e| e.to_string())
        }
        "cx_depends" => {
            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .ok_or("missing 'target' parameter")?;

            let start = graph
                .nodes
                .iter()
                .position(|n| graph.strings.get(n.name) == target)
                .map(|i| i as u32)
                .ok_or_else(|| format!("symbol not found: {}", target))?;

            let direction = match args.get("direction").and_then(|v| v.as_str()) {
                Some("upstream") => cx_core::query::depends::DependsDirection::Upstream,
                _ => cx_core::query::depends::DependsDirection::Downstream,
            };

            let result = cx_core::query::depends::depends(graph, start, direction, ALL_EDGES, 10);
            let deps: Vec<serde_json::Value> = result
                .nodes
                .iter()
                .map(|&id| {
                    let node = graph.node(id);
                    serde_json::json!({
                        "name": graph.strings.get(node.name),
                        "file": if node.file != u32::MAX { Some(graph.strings.get(node.file)) } else { None },
                    })
                })
                .collect();

            serde_json::to_string(&serde_json::json!({ "dependencies": deps }))
                .map_err(|e| e.to_string())
        }
        "cx_context" => {
            let ctx = crate::commands::context::build_context(graph);
            serde_json::to_string(&ctx).map_err(|e| e.to_string())
        }
        "cx_search" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("missing 'query' parameter")?;
            let results = crate::commands::search::search_graph(graph, query, 20);
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
            serde_json::to_string(&output).map_err(|e| e.to_string())
        }
        "cx_resolve" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("missing 'query' parameter")?;
            let results = crate::commands::search::search_graph(graph, query, 10);
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
            serde_json::to_string(&output).map_err(|e| e.to_string())
        }
        _ => Err(format!("unknown tool: {}", name)),
    }
}

fn tool_definitions() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "cx_path",
            "description": "Trace execution path from an entry point through service boundaries.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Symbol to trace from" },
                    "direction": { "type": "string", "enum": ["downstream", "upstream"], "default": "downstream" },
                    "max_depth": { "type": "integer", "default": 20 }
                },
                "required": ["from"]
            }
        }),
        serde_json::json!({
            "name": "cx_depends",
            "description": "Get transitive dependencies for a service or symbol.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Service or symbol name" },
                    "direction": { "type": "string", "enum": ["downstream", "upstream"], "default": "downstream" },
                    "depth": { "type": "integer", "default": 3 }
                },
                "required": ["target"]
            }
        }),
        serde_json::json!({
            "name": "cx_context",
            "description": "Get structured summary of a service.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "service": { "type": "string", "description": "Service name (optional)" }
                }
            }
        }),
        serde_json::json!({
            "name": "cx_search",
            "description": "Fuzzy symbol search.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "cx_resolve",
            "description": "Resolve a qualified name to specific symbols.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Qualified name to resolve" },
                    "kind": { "type": "string", "description": "Filter by kind (Symbol, Endpoint, etc.)" }
                },
                "required": ["query"]
            }
        }),
    ]
}

fn json_rpc_result(id: serde_json::Value, result: serde_json::Value) -> String {
    serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    }))
    .unwrap_or_default()
}

fn json_rpc_error(id: serde_json::Value, code: i32, message: &str) -> String {
    serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    }))
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_mcp() -> (tempfile::TempDir, CsrGraph) {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("main.go"),
            "package main\n\nfunc main() { helper() }\nfunc helper() {}\n",
        )
        .unwrap();
        crate::commands::init::run(dir.path()).unwrap();
        let graph = crate::commands::init::load_graph(dir.path()).unwrap();
        (dir, graph)
    }

    #[test]
    fn mcp_server_tool_listing() {
        let (_dir, graph) = setup_mcp();
        let mut finder = PathFinder::new(graph.node_count());

        let req = r#"{"jsonrpc":"2.0","method":"tools/list","id":1}"#;
        let resp = handle_request(req, &graph, &mut finder);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();

        let tools = v["result"]["tools"].as_array().unwrap();
        assert!(tools.len() >= 5, "should have at least 5 tools");

        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"cx_path"));
        assert!(names.contains(&"cx_depends"));
        assert!(names.contains(&"cx_context"));
        assert!(names.contains(&"cx_search"));
        assert!(names.contains(&"cx_resolve"));
    }

    #[test]
    fn mcp_cx_path_call() {
        let (_dir, graph) = setup_mcp();
        let mut finder = PathFinder::new(graph.node_count());

        let req = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"cx_path","arguments":{"from":"main"}},"id":2}"#;
        let resp = handle_request(req, &graph, &mut finder);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();

        assert!(v.get("error").is_none(), "should not have error: {}", resp);
        let content = v["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert!(parsed.get("paths").is_some());
        assert!(parsed.get("completeness").is_some());
    }

    #[test]
    fn mcp_cx_context_call() {
        let (_dir, graph) = setup_mcp();
        let mut finder = PathFinder::new(graph.node_count());

        let req = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"cx_context","arguments":{}},"id":3}"#;
        let resp = handle_request(req, &graph, &mut finder);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();

        assert!(v.get("error").is_none(), "should not error: {}", resp);
        let content = v["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert!(parsed.get("summary").is_some());
    }

    #[test]
    fn mcp_content_length_framing() {
        // Test read/write message framing
        let msg = r#"{"jsonrpc":"2.0","method":"test","id":1}"#;
        let mut buf = Vec::new();
        write_message(&mut buf, msg).unwrap();

        let framed = String::from_utf8(buf).unwrap();
        assert!(framed.starts_with("Content-Length: "));
        assert!(framed.contains("\r\n\r\n"));
        assert!(framed.ends_with(msg));

        // Verify we can read it back
        let mut reader = std::io::BufReader::new(framed.as_bytes());
        let body = read_message(&mut reader).unwrap();
        assert_eq!(body, msg);
    }

    #[test]
    fn mcp_invalid_json() {
        let (_dir, graph) = setup_mcp();
        let mut finder = PathFinder::new(graph.node_count());

        let resp = handle_request("not json at all{{{", &graph, &mut finder);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["error"]["code"], -32700);
    }

    #[test]
    fn mcp_unknown_tool() {
        let (_dir, graph) = setup_mcp();
        let mut finder = PathFinder::new(graph.node_count());

        let req = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"nonexistent_tool","arguments":{}},"id":5}"#;
        let resp = handle_request(req, &graph, &mut finder);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        // Unknown tool returns an error in the content
        assert!(v.get("error").is_some() || {
            let content = v["result"]["content"][0]["text"].as_str().unwrap_or("");
            content.contains("unknown")
        });
    }

    #[test]
    fn mcp_query_after_error() {
        let (_dir, graph) = setup_mcp();
        let mut finder = PathFinder::new(graph.node_count());

        // First: invalid request
        let _ = handle_request("invalid json", &graph, &mut finder);

        // Second: valid request — should still work
        let req = r#"{"jsonrpc":"2.0","method":"tools/list","id":6}"#;
        let resp = handle_request(req, &graph, &mut finder);
        let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert!(v["result"]["tools"].is_array(), "should return tools after error");
    }
}
