use cx_core::graph::nodes::{Node, NodeId, NodeKind};
use cx_core::graph::string_interner::StringInterner;

/// Result of parsing a .proto file.
pub struct ProtoExtractionResult {
    pub nodes: Vec<Node>,
    /// Fully qualified service name → list of RPC method names.
    pub services: Vec<ProtoService>,
}

/// A parsed proto service with its RPC methods.
#[derive(Debug, Clone)]
pub struct ProtoService {
    pub package: String,
    pub name: String,
    /// Fully qualified: package.ServiceName
    pub fqn: String,
    pub methods: Vec<String>,
    pub file: String,
}

/// Parse a .proto file and extract service definitions, RPC methods, and message types.
///
/// This is a simple line-based parser — .proto files have a regular grammar
/// that doesn't need tree-sitter.
pub fn extract_proto(
    source: &str,
    file_path: &str,
    strings: &mut StringInterner,
    id_counter: &mut NodeId,
) -> ProtoExtractionResult {
    let mut nodes = Vec::new();
    let mut services = Vec::new();

    let mut package = String::new();
    let mut current_service: Option<(String, Vec<String>, u32)> = None; // (name, methods, start_line)

    for (line_num, line) in source.lines().enumerate() {
        let trimmed = line.trim();

        // Package declaration
        if let Some(pkg) = trimmed.strip_prefix("package ") {
            let pkg = pkg.trim_end_matches(';').trim();
            package = pkg.to_string();
            continue;
        }

        // Service declaration
        if let Some(rest) = trimmed.strip_prefix("service ") {
            let name = rest.split_whitespace().next().unwrap_or("").trim_end_matches('{');
            if !name.is_empty() {
                current_service = Some((name.to_string(), Vec::new(), line_num as u32 + 1));
            }
            continue;
        }

        // RPC method inside a service
        if let Some(ref mut svc) = current_service {
            if let Some(rest) = trimmed.strip_prefix("rpc ") {
                let method_name = rest.split('(').next().unwrap_or("").trim();
                if !method_name.is_empty() {
                    svc.1.push(method_name.to_string());

                    // Create Endpoint node for each RPC method
                    let fqn = if package.is_empty() {
                        format!("{}.{}", svc.0, method_name)
                    } else {
                        format!("{}.{}.{}", package, svc.0, method_name)
                    };
                    let name_id = strings.intern(&fqn);
                    let node_id = *id_counter;
                    *id_counter += 1;

                    let file_id = strings.intern(file_path);
                    let mut node = Node::new(node_id, NodeKind::Endpoint, name_id);
                    node.file = file_id;
                    node.line = line_num as u32 + 1;
                    nodes.push(node);
                }
            }

            // End of service block
            if trimmed == "}" {
                let (svc_name, methods, start_line) = current_service.take().unwrap();
                let fqn = if package.is_empty() {
                    svc_name.clone()
                } else {
                    format!("{}.{}", package, svc_name)
                };

                // Create Surface node for the service
                let name_id = strings.intern(&fqn);
                let file_id = strings.intern(file_path);
                let node_id = *id_counter;
                *id_counter += 1;

                let mut node = Node::new(node_id, NodeKind::Surface, name_id);
                node.file = file_id;
                node.line = start_line;
                nodes.push(node);

                services.push(ProtoService {
                    package: package.clone(),
                    name: svc_name,
                    fqn,
                    methods,
                    file: file_path.to_string(),
                });
            }
        }
    }

    ProtoExtractionResult { nodes, services }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proto_extraction() {
        // TEST proto_extraction from ARCHITECTURE.md:
        // Input: .proto file defining service OrderProcessing with 3 RPC methods.
        // PASS: 1 Surface node (proto package), 3 Endpoint nodes (RPC methods).
        let source = r#"
syntax = "proto3";

package orderservice;

service OrderProcessing {
  rpc CreateOrder (CreateOrderRequest) returns (CreateOrderResponse);
  rpc GetOrder (GetOrderRequest) returns (GetOrderResponse);
  rpc StreamingRecognize (StreamingRecognizeRequest) returns (stream StreamingRecognizeResponse);
}
"#;
        let mut strings = StringInterner::new();
        let mut id = 0u32;
        let result = extract_proto(source, "order.proto", &mut strings, &mut id);

        // 1 Surface node for the service
        let surfaces: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Surface as u8)
            .map(|n| strings.get(n.name))
            .collect();
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0], "orderservice.OrderProcessing");

        // 3 Endpoint nodes for RPC methods
        let endpoints: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Endpoint as u8)
            .map(|n| strings.get(n.name))
            .collect();
        assert_eq!(endpoints.len(), 3);
        assert!(endpoints.contains(&"orderservice.OrderProcessing.CreateOrder"));
        assert!(endpoints.contains(&"orderservice.OrderProcessing.GetOrder"));
        assert!(endpoints.contains(&"orderservice.OrderProcessing.StreamingRecognize"));

        // Services metadata
        assert_eq!(result.services.len(), 1);
        assert_eq!(result.services[0].fqn, "orderservice.OrderProcessing");
        assert_eq!(result.services[0].methods.len(), 3);
    }

    #[test]
    fn proto_multiple_services() {
        let source = r#"
syntax = "proto3";

package myapp;

service Auth {
  rpc Login (LoginRequest) returns (LoginResponse);
}

service Users {
  rpc GetUser (GetUserRequest) returns (GetUserResponse);
  rpc ListUsers (ListUsersRequest) returns (ListUsersResponse);
}
"#;
        let mut strings = StringInterner::new();
        let mut id = 0u32;
        let result = extract_proto(source, "myapp.proto", &mut strings, &mut id);

        assert_eq!(
            result
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Surface as u8)
                .count(),
            2
        );
        assert_eq!(
            result
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Endpoint as u8)
                .count(),
            3
        );
        assert_eq!(result.services.len(), 2);
    }

    #[test]
    fn proto_no_package() {
        let source = r#"
syntax = "proto3";

service Simple {
  rpc DoThing (Request) returns (Response);
}
"#;
        let mut strings = StringInterner::new();
        let mut id = 0u32;
        let result = extract_proto(source, "simple.proto", &mut strings, &mut id);

        let surfaces: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Surface as u8)
            .map(|n| strings.get(n.name))
            .collect();
        assert_eq!(surfaces[0], "Simple");

        let endpoints: Vec<&str> = result
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Endpoint as u8)
            .map(|n| strings.get(n.name))
            .collect();
        assert_eq!(endpoints[0], "Simple.DoThing");
    }

    #[test]
    fn proto_empty_file() {
        let mut strings = StringInterner::new();
        let mut id = 0u32;
        let result = extract_proto("syntax = \"proto3\";", "empty.proto", &mut strings, &mut id);
        assert!(result.nodes.is_empty());
        assert!(result.services.is_empty());
    }

    #[test]
    fn proto_line_numbers() {
        let source = "syntax = \"proto3\";\n\npackage test;\n\nservice Svc {\n  rpc Method (Req) returns (Res);\n}\n";
        let mut strings = StringInterner::new();
        let mut id = 0u32;
        let result = extract_proto(source, "test.proto", &mut strings, &mut id);

        let endpoint = result
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Endpoint as u8)
            .unwrap();
        assert_eq!(endpoint.line, 6, "RPC method should be on line 6");
    }
}
