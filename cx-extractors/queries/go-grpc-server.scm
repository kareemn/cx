; Go gRPC server registration detection for CX
; Captures: @endpoint.path, @endpoint.def
; Matches: pb.Register{Service}Server(s, &handler{})
; The Register function name is used as the endpoint identifier

(call_expression
  function: (selector_expression
    field: (field_identifier) @endpoint.path)
  (#match? @endpoint.path "^Register.*Server$")) @endpoint.def
