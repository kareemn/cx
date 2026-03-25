use crate::helm_env_resolution::{self, EnvVarRead, HelmEnvDef, HelmEnvMatch};
use crate::image_resolution::{self, DockerImage, ImageMatch, K8sContainerImage};
use crate::proto_matching::{self, ProtoMatch};
use crate::rest_resolution::{self, HttpClientCall, HttpServerRoute, RestMatch};
use crate::websocket_resolution::{self, WsClientConnection, WsMatch, WsServerEndpoint};
use cx_extractors::grpc::{GrpcClientStub, GrpcServerRegistration};
use cx_extractors::proto::ProtoService;

/// Collected data from all repos for resolution.
pub struct ResolutionInput {
    // -- gRPC/Proto --
    /// (repo_name, client_stubs)
    pub client_stubs: Vec<(String, Vec<GrpcClientStub>)>,
    /// (repo_name, server_registrations)
    pub server_registrations: Vec<(String, Vec<GrpcServerRegistration>)>,
    /// (repo_name, proto_services)
    pub proto_services: Vec<(String, Vec<ProtoService>)>,

    // -- REST/HTTP --
    /// (repo_name, http_client_calls)
    pub http_client_calls: Vec<(String, Vec<HttpClientCall>)>,
    /// (repo_name, http_server_routes)
    pub http_server_routes: Vec<(String, Vec<HttpServerRoute>)>,

    // -- Env Var → Helm → K8s DNS --
    /// (repo_name, env_var_reads)
    pub env_var_reads: Vec<(String, Vec<EnvVarRead>)>,
    /// (repo_name, helm_env_defs)
    pub helm_env_defs: Vec<(String, Vec<HelmEnvDef>)>,

    // -- Docker Image → K8s --
    /// (repo_name, docker_images)
    pub docker_images: Vec<(String, Vec<DockerImage>)>,
    /// (repo_name, k8s_container_images)
    pub k8s_container_images: Vec<(String, Vec<K8sContainerImage>)>,

    // -- WebSocket --
    /// (repo_name, ws_client_connections)
    pub ws_clients: Vec<(String, Vec<WsClientConnection>)>,
    /// (repo_name, ws_server_endpoints)
    pub ws_servers: Vec<(String, Vec<WsServerEndpoint>)>,
}

/// Result of a resolution pass.
pub struct ResolutionResult {
    pub proto_matches: Vec<ProtoMatch>,
    pub rest_matches: Vec<RestMatch>,
    pub helm_env_matches: Vec<HelmEnvMatch>,
    pub image_matches: Vec<ImageMatch>,
    pub ws_matches: Vec<WsMatch>,

    /// Total resolved edges across all types.
    pub resolved_count: usize,
    /// Per-type counts.
    pub proto_count: usize,
    pub rest_count: usize,
    pub helm_env_count: usize,
    pub image_count: usize,
    pub ws_count: usize,

    /// Unresolved gRPC client stubs.
    pub unresolved_client_stubs: Vec<(String, GrpcClientStub)>,
}

/// Run the full resolution pass: match dangling edges across repos.
pub fn resolve(input: &ResolutionInput) -> ResolutionResult {
    // 1. Proto/gRPC resolution
    let proto_matches = proto_matching::match_protos(
        &input.client_stubs,
        &input.server_registrations,
        &input.proto_services,
    );

    // Find unresolved gRPC client stubs
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

    // 2. REST/HTTP resolution
    let rest_matches =
        rest_resolution::match_rest(&input.http_client_calls, &input.http_server_routes);

    // 3. Env Var → Helm → K8s DNS chain
    let helm_env_matches =
        helm_env_resolution::match_helm_env(&input.env_var_reads, &input.helm_env_defs);

    // 4. Docker Image → K8s
    let image_matches =
        image_resolution::match_images(&input.docker_images, &input.k8s_container_images);

    // 5. WebSocket
    let ws_matches =
        websocket_resolution::match_websockets(&input.ws_clients, &input.ws_servers);

    let proto_count = proto_matches.len();
    let rest_count = rest_matches.len();
    let helm_env_count = helm_env_matches.len();
    let image_count = image_matches.len();
    let ws_count = ws_matches.len();
    let resolved_count = proto_count + rest_count + helm_env_count + image_count + ws_count;

    ResolutionResult {
        proto_matches,
        rest_matches,
        helm_env_matches,
        image_matches,
        ws_matches,
        resolved_count,
        proto_count,
        rest_count,
        helm_env_count,
        image_count,
        ws_count,
        unresolved_client_stubs: unresolved,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_input() -> ResolutionInput {
        ResolutionInput {
            client_stubs: vec![],
            server_registrations: vec![],
            proto_services: vec![],
            http_client_calls: vec![],
            http_server_routes: vec![],
            env_var_reads: vec![],
            helm_env_defs: vec![],
            docker_images: vec![],
            k8s_container_images: vec![],
            ws_clients: vec![],
            ws_servers: vec![],
        }
    }

    #[test]
    fn resolve_empty_input() {
        let result = resolve(&empty_input());
        assert_eq!(result.resolved_count, 0);
    }

    #[test]
    fn resolve_proto_matches_across_repos() {
        let mut input = empty_input();
        input.client_stubs = vec![(
            "client-repo".into(),
            vec![GrpcClientStub {
                service_name: "Auth".into(),
                file: "main.go".into(),
                line: 5,
            }],
        )];
        input.server_registrations = vec![(
            "auth-service".into(),
            vec![GrpcServerRegistration {
                service_name: "Auth".into(),
                file: "server.go".into(),
                line: 10,
            }],
        )];
        input.proto_services = vec![(
            "auth-service".into(),
            vec![ProtoService {
                package: "auth".into(),
                name: "Auth".into(),
                fqn: "auth.Auth".into(),
                methods: vec!["Login".into()],
                file: "auth.proto".into(),
            }],
        )];

        let result = resolve(&input);
        assert_eq!(result.proto_count, 1);
        assert_eq!(result.resolved_count, 1);
        assert!(result.unresolved_client_stubs.is_empty());
    }

    #[test]
    fn resolve_rest_and_helm_together() {
        let mut input = empty_input();

        // REST: client calls /inference, server exposes /inference
        input.http_client_calls = vec![(
            "api-gateway".into(),
            vec![HttpClientCall {
                path: "/inference".into(),
                method: "POST".into(),
                base_url_env_var: Some("TTS_SERVICE_URL".into()),
                file: "translator.go".into(),
                line: 42,
            }],
        )];
        input.http_server_routes = vec![(
            "tts-service".into(),
            vec![HttpServerRoute {
                path: "/inference".into(),
                method: "POST".into(),
                framework: "fastapi".into(),
                file: "app.py".into(),
                line: 15,
            }],
        )];

        // Helm env: TTS_SERVICE_URL → k8s DNS → tts-service
        input.env_var_reads = vec![(
            "api-gateway".into(),
            vec![EnvVarRead {
                var_name: "TTS_SERVICE_URL".into(),
                file: "config.go".into(),
                line: 10,
            }],
        )];
        input.helm_env_defs = vec![(
            "infra-k8s-config".into(),
            vec![HelmEnvDef {
                var_name: "TTS_SERVICE_URL".into(),
                value: "http://tts-server-staging.tts-server.svc.cluster.local:8000/inference".into(),
                file: "values.yaml.gotmpl".into(),
                line: 42,
            }],
        )];

        let result = resolve(&input);
        assert_eq!(result.rest_count, 1);
        assert_eq!(result.helm_env_count, 1);
        assert_eq!(result.resolved_count, 2);

        // Verify the helm env match resolves to k8s DNS
        assert!(result.helm_env_matches[0].k8s_service.is_some());
        let svc = result.helm_env_matches[0].k8s_service.as_ref().unwrap();
        assert_eq!(svc.service_name, "tts-server");
    }

    #[test]
    fn resolve_reports_unresolved_grpc() {
        let mut input = empty_input();
        input.client_stubs = vec![(
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
        )];
        input.server_registrations = vec![(
            "auth-service".into(),
            vec![GrpcServerRegistration {
                service_name: "Auth".into(),
                file: "server.go".into(),
                line: 10,
            }],
        )];

        let result = resolve(&input);
        assert_eq!(result.proto_count, 1);
        assert_eq!(result.unresolved_client_stubs.len(), 1);
        assert_eq!(result.unresolved_client_stubs[0].1.service_name, "Missing");
    }

    #[test]
    fn full_myapp_chain_resolution() {
        let mut input = empty_input();

        // gRPC: native-client → api-gateway
        input.client_stubs = vec![(
            "native-client".into(),
            vec![GrpcClientStub {
                service_name: "TranslationStreaming".into(),
                file: "client.cpp".into(),
                line: 100,
            }],
        )];
        input.server_registrations = vec![(
            "api-gateway".into(),
            vec![GrpcServerRegistration {
                service_name: "TranslationStreaming".into(),
                file: "server.go".into(),
                line: 50,
            }],
        )];

        // REST: translation-server → tts-service /inference
        input.http_client_calls = vec![(
            "api-gateway".into(),
            vec![HttpClientCall {
                path: "/inference".into(),
                method: "POST".into(),
                base_url_env_var: Some("TTS_SERVICE_URL".into()),
                file: "tts_client.go".into(),
                line: 30,
            }],
        )];
        input.http_server_routes = vec![(
            "tts-service".into(),
            vec![HttpServerRoute {
                path: "/inference".into(),
                method: "POST".into(),
                framework: "fastapi".into(),
                file: "app.py".into(),
                line: 15,
            }],
        )];

        // Helm env chain
        input.env_var_reads = vec![(
            "api-gateway".into(),
            vec![EnvVarRead {
                var_name: "TTS_SERVICE_URL".into(),
                file: "config.go".into(),
                line: 10,
            }],
        )];
        input.helm_env_defs = vec![(
            "infra-k8s-config".into(),
            vec![HelmEnvDef {
                var_name: "TTS_SERVICE_URL".into(),
                value: "http://tts-server-staging.tts-server.svc.cluster.local:8000/inference".into(),
                file: "values.yaml.gotmpl".into(),
                line: 42,
            }],
        )];

        // Docker image → k8s
        input.docker_images = vec![(
            "tts-service".into(),
            vec![image_resolution::DockerImage {
                image_ref: "gcr.io/example-org/myapp/tts-server".into(),
                file: "Dockerfile".into(),
            }],
        )];
        input.k8s_container_images = vec![(
            "infra-k8s-config".into(),
            vec![image_resolution::K8sContainerImage {
                image_ref: "gcr.io/example-org/myapp/tts-server:v2.0".into(),
                file: "values.yaml".into(),
                line: 15,
                deployment_name: Some("tts-server".into()),
            }],
        )];

        // WebSocket: cpp-client → translation-server
        input.ws_clients = vec![(
            "native-client".into(),
            vec![WsClientConnection {
                url_or_path: "ws://10.0.0.1:8080/ws/s2s".into(),
                file: "ws_client.cpp".into(),
                line: 200,
            }],
        )];
        input.ws_servers = vec![(
            "api-gateway".into(),
            vec![WsServerEndpoint {
                path: "/ws/s2s".into(),
                file: "ws_handler.go".into(),
                line: 80,
            }],
        )];

        let result = resolve(&input);

        // Should have matches across all types
        assert_eq!(result.proto_count, 1, "gRPC match");
        assert_eq!(result.rest_count, 1, "REST match");
        assert_eq!(result.helm_env_count, 1, "Helm env match");
        assert_eq!(result.image_count, 1, "Image match");
        assert_eq!(result.ws_count, 1, "WebSocket match");
        assert_eq!(result.resolved_count, 5, "total resolved");
        assert!(result.unresolved_client_stubs.is_empty());
    }
}
