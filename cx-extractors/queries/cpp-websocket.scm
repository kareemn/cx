; C++ WebSocket detection for CX
; Captures: @ws.path, @ws.path_var, @ws.def
;
; In Boost.Beast:
;   - handshake / async_handshake = CLIENT initiating WebSocket upgrade (outbound)
;   - accept / async_accept = SERVER accepting WebSocket upgrade (inbound)
;
; Only accept/async_accept should create @ws.def (Endpoint/Exposes) nodes.
; handshake/async_handshake are client calls and should NOT be @ws.def.

; Server: ws.accept() or ws.async_accept() — server accepting a WebSocket connection
(call_expression
  function: (field_expression
    field: (field_identifier) @_method)
  (#match? @_method "^(accept|async_accept)$")) @ws.def
