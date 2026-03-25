; Go gRPC client detection for CX
; Captures: @http_call.url, @http_call.site

; grpc.Dial("addr"), grpc.DialContext(ctx, "addr"), grpc.NewClient("addr")
(call_expression
  function: (selector_expression
    operand: (identifier) @_pkg
    field: (field_identifier) @_method)
  arguments: (argument_list
    .
    (_)* @_skip
    (interpreted_string_literal) @http_call.url)
  (#eq? @_pkg "grpc")
  (#match? @_method "^(Dial|DialContext|NewClient)$")) @http_call.site
