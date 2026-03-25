; C++ gRPC client detection for CX
; Captures: @http_call.url, @http_call.site

; grpc::CreateChannel("addr", ...)
(call_expression
  function: (qualified_identifier
    scope: (namespace_identifier) @_ns
    name: (identifier) @_method)
  arguments: (argument_list
    (string_literal) @http_call.url)
  (#eq? @_ns "grpc")
  (#eq? @_method "CreateChannel")) @http_call.site
