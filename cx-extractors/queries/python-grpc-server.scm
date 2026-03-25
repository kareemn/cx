; Python gRPC server registration detection for CX
; Captures: @endpoint.path, @endpoint.def

; pb2_grpc.add_{Service}Servicer_to_server(servicer, server)
(call
  function: (attribute
    attribute: (identifier) @endpoint.path)
  (#match? @endpoint.path "^add_.*_to_server$")) @endpoint.def
