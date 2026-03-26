; Python gRPC client detection for CX
; gRPC client stubs are now detected by scan_python_grpc() in grpc.rs
; (line-based scanner that produces GrpcClientStub directly)
; This file is intentionally empty — the old tree-sitter query used
; @http_call.url captures which created false HTTP Endpoint nodes.
