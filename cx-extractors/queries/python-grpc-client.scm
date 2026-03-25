; Python gRPC client detection for CX
; Captures: @http_call.url, @http_call.site

; grpc.insecure_channel("addr"), grpc.secure_channel("addr", creds)
(call
  function: (attribute
    object: (identifier) @_mod
    attribute: (identifier) @_method)
  arguments: (argument_list
    (string) @http_call.url)
  (#eq? @_mod "grpc")
  (#match? @_method "^(insecure_channel|secure_channel)$")) @http_call.site
