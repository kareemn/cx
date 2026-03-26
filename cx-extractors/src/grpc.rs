use streaming_iterator::StreamingIterator;

/// A detected gRPC client stub creation.
#[derive(Debug, Clone)]
pub struct GrpcClientStub {
    /// The service name extracted from New{Service}Client (e.g., "OrderProcessing").
    pub service_name: String,
    /// File where the client stub was found.
    pub file: String,
    /// Line number.
    pub line: u32,
}

/// A detected gRPC server registration.
#[derive(Debug, Clone)]
pub struct GrpcServerRegistration {
    /// The service name extracted from Register{Service}Server (e.g., "OrderProcessing").
    pub service_name: String,
    /// File where the server registration was found.
    pub file: String,
    /// Line number.
    pub line: u32,
}

/// Result of scanning a Go file for gRPC patterns.
pub struct GrpcScanResult {
    pub client_stubs: Vec<GrpcClientStub>,
    pub server_registrations: Vec<GrpcServerRegistration>,
}

pub const GO_GRPC_CLIENT_QUERY: &str = include_str!("../queries/go-grpc-client.scm");
pub const GO_GRPC_SERVER_QUERY: &str = include_str!("../queries/go-grpc-server.scm");

/// Scan a Go file for gRPC client and server patterns using tree-sitter queries.
pub fn scan_go_grpc(
    tree: &tree_sitter::Tree,
    source: &[u8],
    file_path: &str,
    language: &tree_sitter::Language,
) -> GrpcScanResult {
    let mut client_stubs = Vec::new();
    let mut server_registrations = Vec::new();

    // Scan for client patterns using call.name capture (method calls like NewXxxClient)
    {
        let client_query_src = r#"
(call_expression
  function: (selector_expression
    field: (field_identifier) @call.name)
  (#match? @call.name "^New.*Client$")) @call.site
"#;
        if let Ok(query) = tree_sitter::Query::new(language, client_query_src) {
            let name_idx = query
                .capture_names()
                .iter()
                .position(|n| *n == "call.name")
                .map(|i| i as u32);

            if let Some(idx) = name_idx {
                let mut cursor = tree_sitter::QueryCursor::new();
                let mut matches = cursor.matches(&query, tree.root_node(), source);

                while let Some(m) = matches.next() {
                    for cap in m.captures {
                        if cap.index == idx {
                            if let Ok(text) = std::str::from_utf8(&source[cap.node.byte_range()]) {
                                if let Some(svc) = text
                                    .strip_prefix("New")
                                    .and_then(|s| s.strip_suffix("Client"))
                                {
                                    if !svc.is_empty() {
                                        client_stubs.push(GrpcClientStub {
                                            service_name: svc.to_string(),
                                            file: file_path.to_string(),
                                            line: cap.node.start_position().row as u32 + 1,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Scan for server patterns using endpoint.path capture
    if let Ok(query) = tree_sitter::Query::new(language, GO_GRPC_SERVER_QUERY) {
        let register_idx = query
            .capture_names()
            .iter()
            .position(|n| *n == "endpoint.path")
            .map(|i| i as u32);

        if let Some(idx) = register_idx {
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut matches = cursor.matches(&query, tree.root_node(), source);

            while let Some(m) = matches.next() {
                for cap in m.captures {
                    if cap.index == idx {
                        if let Ok(text) = std::str::from_utf8(&source[cap.node.byte_range()]) {
                            // Extract service name from "Register{Service}Server"
                            if let Some(svc) = text
                                .strip_prefix("Register")
                                .and_then(|s| s.strip_suffix("Server"))
                            {
                                if !svc.is_empty() {
                                    server_registrations.push(GrpcServerRegistration {
                                        service_name: svc.to_string(),
                                        file: file_path.to_string(),
                                        line: cap.node.start_position().row as u32 + 1,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    GrpcScanResult {
        client_stubs,
        server_registrations,
    }
}

/// Scan a Python file for gRPC patterns using regex-style line scanning.
/// Detects:
/// - Server: `add_{Service}Servicer_to_server(impl, server)`
/// - Client: `{package}.{Service}Stub(channel)` or `{Service}Stub(channel)`
pub fn scan_python_grpc(source: &[u8], file_path: &str) -> GrpcScanResult {
    let mut client_stubs = Vec::new();
    let mut server_registrations = Vec::new();

    let text = match std::str::from_utf8(source) {
        Ok(t) => t,
        Err(_) => return GrpcScanResult { client_stubs, server_registrations },
    };

    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let line_num = (i + 1) as u32;

        // Server: add_*Servicer_to_server
        if let Some(idx) = trimmed.find("add_") {
            let rest = &trimmed[idx + 4..];
            if let Some(end) = rest.find("Servicer_to_server") {
                let svc = &rest[..end];
                if !svc.is_empty() && svc.chars().next().is_some_and(|c| c.is_uppercase()) {
                    server_registrations.push(GrpcServerRegistration {
                        service_name: svc.to_string(),
                        file: file_path.to_string(),
                        line: line_num,
                    });
                }
            }
        }

        // Client: SomeServiceStub(
        if let Some(idx) = trimmed.find("Stub(") {
            // Walk backwards to find the service name
            let before = &trimmed[..idx];
            let svc_name: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            // Strip trailing dots/module prefix
            let svc = svc_name.split('.').next_back().unwrap_or("");
            if !svc.is_empty()
                && svc.chars().next().is_some_and(|c| c.is_uppercase())
                && svc != "Stub"
            {
                // Keep full service name (e.g., "ProductCatalogService") to match Go clients
                client_stubs.push(GrpcClientStub {
                    service_name: svc.to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                });
            }
        }
    }

    GrpcScanResult { client_stubs, server_registrations }
}

/// Extract a gRPC service name from a line containing ".service" pattern.
/// e.g., "shopProto.CurrencyService.service," → "CurrencyService"
fn extract_js_service_name(line: &str) -> Option<String> {
    let idx = line.find(".service")?;
    let before = &line[..idx];
    let svc: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if !svc.is_empty() && svc.chars().next().is_some_and(|c| c.is_uppercase()) {
        Some(svc)
    } else {
        None
    }
}

/// Scan a JavaScript/TypeScript file for gRPC patterns using line scanning.
/// Detects:
/// - Server: `server.addService(proto.{Service}.service, ...)`
/// - Client: `new proto.{Service}(address)` or `new {Package}.{Service}Client(address)`
pub fn scan_js_grpc(source: &[u8], file_path: &str) -> GrpcScanResult {
    let mut client_stubs = Vec::new();
    let mut server_registrations = Vec::new();

    let text = match std::str::from_utf8(source) {
        Ok(t) => t,
        Err(_) => return GrpcScanResult { client_stubs, server_registrations },
    };

    let lines: Vec<&str> = text.lines().collect();
    let mut pending_add_service_line: Option<u32> = None;

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_num = (i + 1) as u32;

        // Server: server.addService(something.ServiceName.service, ...)
        // Handle multiline: addService( on one line, Proto.Service.service on the next
        if trimmed.contains("addService(") || trimmed.contains("addService (") {
            // Try same-line match first
            if let Some(svc) = extract_js_service_name(trimmed) {
                server_registrations.push(GrpcServerRegistration {
                    service_name: svc,
                    file: file_path.to_string(),
                    line: line_num,
                });
            } else {
                // Mark as pending — check the next line
                pending_add_service_line = Some(line_num);
            }
        } else if let Some(add_line) = pending_add_service_line {
            // Check if this line contains the .service reference
            if let Some(svc) = extract_js_service_name(trimmed) {
                server_registrations.push(GrpcServerRegistration {
                    service_name: svc,
                    file: file_path.to_string(),
                    line: add_line,
                });
            }
            pending_add_service_line = None;
        }

        // Client: new proto.ServiceNameClient( or new ServiceName(
        if trimmed.contains("Client(") {
            let before_client = trimmed.split("Client(").next().unwrap_or("");
            let svc: String = before_client
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if !svc.is_empty() && svc.chars().next().is_some_and(|c| c.is_uppercase()) {
                client_stubs.push(GrpcClientStub {
                    service_name: svc.to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                });
            }
        }
    }

    GrpcScanResult { client_stubs, server_registrations }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_go(source: &str) -> tree_sitter::Tree {
        let lang: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang).unwrap();
        parser.parse(source.as_bytes(), None).unwrap()
    }

    fn go_lang() -> tree_sitter::Language {
        tree_sitter_go::LANGUAGE.into()
    }

    #[test]
    fn grpc_client_detection_go() {
        // TEST grpc_client_detection_go from ARCHITECTURE.md
        let source = r#"
package main

import (
    "context"
    pb "example.com/proto/order"
    "google.golang.org/grpc"
)

func main() {
    conn, _ := grpc.Dial(addr, opts...)
    client := pb.NewOrderProcessingClient(conn)
    client.StreamingRecognize(ctx)
}
"#;
        let tree = parse_go(source);
        let lang = go_lang();
        let result = scan_go_grpc(&tree, source.as_bytes(), "main.go", &lang);

        // Should detect NewOrderProcessingClient
        assert_eq!(result.client_stubs.len(), 1, "should find 1 client stub");
        assert_eq!(result.client_stubs[0].service_name, "OrderProcessing");
    }

    #[test]
    fn grpc_server_detection_go() {
        // TEST grpc_server_detection_go from ARCHITECTURE.md
        let source = r#"
package main

import pb "example.com/proto/order"

func main() {
    s := grpc.NewServer()
    pb.RegisterOrderProcessingServer(s, &handler{})
    s.Serve(lis)
}
"#;
        let tree = parse_go(source);
        let lang = go_lang();
        let result = scan_go_grpc(&tree, source.as_bytes(), "server.go", &lang);

        // Should detect RegisterOrderProcessingServer
        assert_eq!(
            result.server_registrations.len(),
            1,
            "should find 1 server registration"
        );
        assert_eq!(
            result.server_registrations[0].service_name,
            "OrderProcessing"
        );
    }

    #[test]
    fn grpc_multiple_services() {
        let source = r#"
package main

func setup() {
    pb.RegisterAuthServer(s, &authHandler{})
    pb.RegisterUsersServer(s, &usersHandler{})
    client := pb.NewNotificationClient(conn)
}
"#;
        let tree = parse_go(source);
        let lang = go_lang();
        let result = scan_go_grpc(&tree, source.as_bytes(), "setup.go", &lang);

        assert_eq!(result.server_registrations.len(), 2);
        assert_eq!(result.client_stubs.len(), 1);
        assert_eq!(result.client_stubs[0].service_name, "Notification");
    }

    #[test]
    fn grpc_no_patterns() {
        let source = "package main\n\nfunc main() {}\n";
        let tree = parse_go(source);
        let lang = go_lang();
        let result = scan_go_grpc(&tree, source.as_bytes(), "main.go", &lang);
        assert!(result.client_stubs.is_empty());
        assert!(result.server_registrations.is_empty());
    }
}
