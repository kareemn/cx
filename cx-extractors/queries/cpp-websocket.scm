; C++ WebSocket detection for CX
; Captures: @ws.path, @ws.def

; beast::websocket or boost::beast WebSocket handshake/connect patterns
; ws.async_handshake(host, "/path"), ws_.handshake(host, "/path")
(call_expression
  function: (field_expression
    field: (field_identifier) @_method)
  arguments: (argument_list
    (_)
    (string_literal) @ws.path)
  (#match? @_method "^(handshake|async_handshake)$")
  (#match? @ws.path "^\"/" )) @ws.def
