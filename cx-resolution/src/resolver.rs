use crate::proto_matching::{self, ProtoMatch};
use cx_extractors::grpc::{GrpcClientStub, GrpcServerRegistration};
use cx_extractors::proto::ProtoService;

/// Collected data from all repos for resolution.
pub struct ResolutionInput {
    /// (repo_name, client_stubs)
    pub client_stubs: Vec<(String, Vec<GrpcClientStub>)>,
    /// (repo_name, server_registrations)
    pub server_registrations: Vec<(String, Vec<GrpcServerRegistration>)>,
    /// (repo_name, proto_services)
    pub proto_services: Vec<(String, Vec<ProtoService>)>,
}

/// Result of a resolution pass.
pub struct ResolutionResult {
    pub proto_matches: Vec<ProtoMatch>,
    pub resolved_count: usize,
    pub unresolved_client_stubs: Vec<(String, GrpcClientStub)>,
}

/// Run the full resolution pass: match dangling edges across repos.
pub fn resolve(input: &ResolutionInput) -> ResolutionResult {
    let proto_matches = proto_matching::match_protos(
        &input.client_stubs,
        &input.server_registrations,
        &input.proto_services,
    );

    // Find unresolved client stubs (no server match)
    let matched_services: rustc_hash::FxHashSet<&str> = proto_matches
        .iter()
        .map(|m| m.service_name.as_str())
        .collect();

    let mut unresolved = Vec::new();
    for (repo, stubs) in &input.client_stubs {
        for stub in stubs {
            if !matched_services.contains(stub.service_name.as_str()) {
                unresolved.push((repo.clone(), stub.clone()));
            }
        }
    }

    let resolved_count = proto_matches.len();

    ResolutionResult {
        proto_matches,
        resolved_count,
        unresolved_client_stubs: unresolved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_matches_across_repos() {
        let input = ResolutionInput {
            client_stubs: vec![(
                "client-repo".into(),
                vec![GrpcClientStub {
                    service_name: "Auth".into(),
                    file: "main.go".into(),
                    line: 5,
                }],
            )],
            server_registrations: vec![(
                "auth-service".into(),
                vec![GrpcServerRegistration {
                    service_name: "Auth".into(),
                    file: "server.go".into(),
                    line: 10,
                }],
            )],
            proto_services: vec![(
                "auth-service".into(),
                vec![ProtoService {
                    package: "auth".into(),
                    name: "Auth".into(),
                    fqn: "auth.Auth".into(),
                    methods: vec!["Login".into()],
                    file: "auth.proto".into(),
                }],
            )],
        };

        let result = resolve(&input);
        assert_eq!(result.resolved_count, 1);
        assert!(result.unresolved_client_stubs.is_empty());
    }

    #[test]
    fn resolve_reports_unresolved() {
        let input = ResolutionInput {
            client_stubs: vec![(
                "client-repo".into(),
                vec![
                    GrpcClientStub {
                        service_name: "Auth".into(),
                        file: "main.go".into(),
                        line: 5,
                    },
                    GrpcClientStub {
                        service_name: "Missing".into(),
                        file: "main.go".into(),
                        line: 15,
                    },
                ],
            )],
            server_registrations: vec![(
                "auth-service".into(),
                vec![GrpcServerRegistration {
                    service_name: "Auth".into(),
                    file: "server.go".into(),
                    line: 10,
                }],
            )],
            proto_services: vec![],
        };

        let result = resolve(&input);
        assert_eq!(result.resolved_count, 1);
        assert_eq!(result.unresolved_client_stubs.len(), 1);
        assert_eq!(result.unresolved_client_stubs[0].1.service_name, "Missing");
    }
}
