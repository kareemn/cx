; Go WebSocket detection for CX
; Captures: @ws.def

; upgrader.Upgrade(w, r, nil) — gorilla/websocket
; websocket.Accept(w, r, nil) — nhooyr.io/websocket
(call_expression
  function: (selector_expression
    field: (field_identifier) @_method)
  (#match? @_method "^(Upgrade|Accept)$")) @ws.def
