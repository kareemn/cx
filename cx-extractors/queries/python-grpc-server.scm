; Python gRPC server registration detection for CX
; gRPC server registrations are now detected by scan_python_grpc() in grpc.rs
; (line-based scanner that produces GrpcServerRegistration directly)
; This file is intentionally empty — the old tree-sitter query used
; @endpoint.path captures which created false HTTP Endpoint nodes
; (e.g., "add_EmailServiceServicer_to_server" as an endpoint name).
