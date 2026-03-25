; Detect gRPC server registration patterns in Go
; Matches: pb.Register{Service}Server(s, &handler{})

(call_expression
  function: (selector_expression
    operand: (_) @grpc.server.pkg
    field: (field_identifier) @grpc.server.register)
  (#match? @grpc.server.register "^Register.*Server$")) @grpc.server.call
