; TypeScript/JavaScript gRPC client detection for CX
; Captures: @http_call.url, @http_call.site

; new grpc.Client("addr", creds), grpc.credentials.createInsecure()
; new XxxClient("addr", ...)
(new_expression
  constructor: (identifier) @_cls
  arguments: (arguments
    (string) @http_call.url)
  (#match? @_cls "Client$")) @http_call.site

; @grpc/grpc-js: new Client("addr")
(new_expression
  constructor: (member_expression
    property: (property_identifier) @_cls)
  arguments: (arguments
    (string) @http_call.url)
  (#match? @_cls "Client$")) @http_call.site
