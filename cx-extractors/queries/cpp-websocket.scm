; C++ WebSocket detection for CX
; Captures: @ws.path, @ws.path_var, @ws.def

; ws.async_handshake(host, "/path"), ws_.handshake(host, "/path")
(call_expression
  function: (field_expression
    field: (field_identifier) @_method)
  arguments: (argument_list
    (_)
    (string_literal) @ws.path)
  (#match? @_method "^(handshake|async_handshake)$")
  (#match? @ws.path "^\"/" )) @ws.def

; Variant: ws.async_handshake(host, varName) — variable reference
(call_expression
  function: (field_expression
    field: (field_identifier) @_method2)
  arguments: (argument_list
    (_)
    (identifier) @ws.path_var)
  (#match? @_method2 "^(handshake|async_handshake)$")) @ws.def

; ws_.async_connect(endpoint, callback) — Boost.Beast WebSocket connect
(call_expression
  function: (field_expression
    field: (field_identifier) @_method3)
  (#match? @_method3 "^(async_connect|connect)$")) @ws.def
