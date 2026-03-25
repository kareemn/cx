; Detect gRPC client patterns in Go
; Matches: pb.NewXxxClient(conn) and grpc.Dial(addr, opts...)

; pb.New{Service}Client(conn) — gRPC client stub creation
(call_expression
  function: (selector_expression
    operand: (_) @grpc.client.pkg
    field: (field_identifier) @grpc.client.constructor)
  (#match? @grpc.client.constructor "^New.*Client$")) @grpc.client.call

; grpc.Dial(addr, opts...) — gRPC connection
(call_expression
  function: (selector_expression
    operand: (identifier) @grpc.dial.pkg
    field: (field_identifier) @grpc.dial.method)
  (#eq? @grpc.dial.pkg "grpc")
  (#match? @grpc.dial.method "^(Dial|DialContext|NewClient)$")) @grpc.dial.call
