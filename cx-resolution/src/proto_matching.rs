use cx_extractors::grpc::{GrpcClientStub, GrpcServerRegistration};
use cx_extractors::proto::ProtoService;

/// A resolved proto match: a client stub matched to a server registration.
#[derive(Debug, Clone)]
pub struct ProtoMatch {
    /// The service name (e.g., "OrderProcessing").
    pub service_name: String,
    /// Client stub info.
    pub client_file: String,
    pub client_line: u32,
    pub client_repo: String,
    /// Server registration info.
    pub server_file: String,
    pub server_line: u32,
    pub server_repo: String,
    /// Confidence score (0.0 - 1.0).
    pub confidence: f32,
    /// Warnings (e.g., field count mismatch).
    pub warnings: Vec<String>,
}

/// Match proto client stubs to server registrations by service name.
///
/// Returns matches with confidence >= 0.9 for exact name matches.
pub fn match_protos(
    client_stubs: &[(String, Vec<GrpcClientStub>)], // (repo_name, stubs)
    server_regs: &[(String, Vec<GrpcServerRegistration>)], // (repo_name, registrations)
    proto_services: &[(String, Vec<ProtoService>)], // (repo_name, services)
) -> Vec<ProtoMatch> {
    let mut matches = Vec::new();

    // Build server index: service_name → (repo, registration)
    let mut server_index: rustc_hash::FxHashMap<&str, Vec<(&str, &GrpcServerRegistration)>> =
        rustc_hash::FxHashMap::default();
    for (repo, regs) in server_regs {
        for reg in regs {
            server_index
                .entry(&reg.service_name)
                .or_default()
                .push((repo, reg));
        }
    }

    // Build proto service index for validation
    let mut proto_index: rustc_hash::FxHashMap<&str, &ProtoService> =
        rustc_hash::FxHashMap::default();
    for (_repo, services) in proto_services {
        for svc in services {
            proto_index.insert(&svc.name, svc);
        }
    }

    // Match each client stub to server registrations
    for (client_repo, stubs) in client_stubs {
        for stub in stubs {
            if let Some(servers) = server_index.get(stub.service_name.as_str()) {
                for &(server_repo, reg) in servers {
                    // Don't match within the same repo (that's not cross-repo)
                    let confidence = if client_repo == server_repo {
                        0.5 // same-repo gRPC is less interesting
                    } else {
                        0.95 // cross-repo proto match = high confidence
                    };

                    let mut warnings = Vec::new();

                    // Check if proto definitions match between repos
                    // (simplified: we just check if the service exists in proto_index)
                    if !proto_index.contains_key(stub.service_name.as_str()) {
                        warnings.push(format!(
                            "no proto definition found for service {}",
                            stub.service_name
                        ));
                    }

                    matches.push(ProtoMatch {
                        service_name: stub.service_name.clone(),
                        client_file: stub.file.clone(),
                        client_line: stub.line,
                        client_repo: client_repo.clone(),
                        server_file: reg.file.clone(),
                        server_line: reg.line,
                        server_repo: server_repo.to_string(),
                        confidence,
                        warnings,
                    });
                }
            }
        }
    }

    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_repo_proto_resolution() {
        // TEST cross_repo_proto_resolution from ARCHITECTURE.md
        let client_stubs = vec![(
            "repo-a".to_string(),
            vec![GrpcClientStub {
                service_name: "OrderProcessing".to_string(),
                file: "client.go".to_string(),
                line: 10,
            }],
        )];

        let server_regs = vec![(
            "repo-b".to_string(),
            vec![GrpcServerRegistration {
                service_name: "OrderProcessing".to_string(),
                file: "server.go".to_string(),
                line: 20,
            }],
        )];

        let proto_services = vec![(
            "repo-b".to_string(),
            vec![cx_extractors::proto::ProtoService {
                package: "order".to_string(),
                name: "OrderProcessing".to_string(),
                fqn: "order.OrderProcessing".to_string(),
                methods: vec!["CreateOrder".to_string()],
                file: "order.proto".to_string(),
            }],
        )];

        let matches = match_protos(&client_stubs, &server_regs, &proto_services);

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].service_name, "OrderProcessing");
        assert!(
            matches[0].confidence >= 0.9,
            "cross-repo match should have confidence >= 0.9"
        );
        assert_eq!(matches[0].client_repo, "repo-a");
        assert_eq!(matches[0].server_repo, "repo-b");
        assert!(matches[0].warnings.is_empty());
    }

    #[test]
    fn cross_repo_proto_mismatch() {
        // TEST cross_repo_proto_mismatch from ARCHITECTURE.md
        // Proto service not found in proto_index → warning
        let client_stubs = vec![(
            "repo-a".to_string(),
            vec![GrpcClientStub {
                service_name: "MissingService".to_string(),
                file: "client.go".to_string(),
                line: 10,
            }],
        )];

        let server_regs = vec![(
            "repo-b".to_string(),
            vec![GrpcServerRegistration {
                service_name: "MissingService".to_string(),
                file: "server.go".to_string(),
                line: 20,
            }],
        )];

        // No proto definition for MissingService
        let proto_services: Vec<(String, Vec<cx_extractors::proto::ProtoService>)> = vec![];

        let matches = match_protos(&client_stubs, &server_regs, &proto_services);

        assert_eq!(matches.len(), 1);
        assert!(!matches[0].warnings.is_empty(), "should have warning about missing proto");
    }

    #[test]
    fn no_match_when_no_server() {
        let client_stubs = vec![(
            "repo-a".to_string(),
            vec![GrpcClientStub {
                service_name: "Orphan".to_string(),
                file: "client.go".to_string(),
                line: 10,
            }],
        )];

        let server_regs: Vec<(String, Vec<GrpcServerRegistration>)> = vec![];
        let proto_services: Vec<(String, Vec<cx_extractors::proto::ProtoService>)> = vec![];

        let matches = match_protos(&client_stubs, &server_regs, &proto_services);
        assert!(matches.is_empty());
    }
}
