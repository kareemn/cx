; Go WebSocket detection for CX
; Captures: @ws.path, @ws.path_var, @ws.def

; --- Server-side ---

; upgrader.Upgrade(w, r, nil) — gorilla/websocket
; websocket.Accept(w, r, nil) — nhooyr.io/websocket
(call_expression
  function: (selector_expression
    field: (field_identifier) @_method)
  (#match? @_method "^(Upgrade|Accept)$")) @ws.def

; --- Client-side ---

; websocket.Dial("ws://host/path", nil) — gorilla/websocket
(call_expression
  function: (selector_expression
    operand: (identifier) @_pkg
    field: (field_identifier) @_method2)
  arguments: (argument_list
    (interpreted_string_literal) @ws.path)
  (#eq? @_pkg "websocket")
  (#match? @_method2 "^(Dial|DefaultDialer)$")) @ws.def

; websocket.DefaultDialer.Dial("ws://host/path", nil) — gorilla/websocket
(call_expression
  function: (selector_expression
    operand: (selector_expression
      operand: (identifier) @_pkg3
      field: (field_identifier) @_dialer)
    field: (field_identifier) @_method3)
  arguments: (argument_list
    (interpreted_string_literal) @ws.path)
  (#eq? @_pkg3 "websocket")
  (#eq? @_dialer "DefaultDialer")
  (#eq? @_method3 "Dial")) @ws.def

; dialer.Dial("ws://host/path", nil) — generic dialer variable
(call_expression
  function: (selector_expression
    field: (field_identifier) @_method4)
  arguments: (argument_list
    (interpreted_string_literal) @ws.path)
  (#eq? @_method4 "Dial")
  (#match? @ws.path "^\"ws")) @ws.def

; Variant: websocket.Dial(urlVar, nil) — variable reference
(call_expression
  function: (selector_expression
    operand: (identifier) @_pkg5
    field: (field_identifier) @_method5)
  arguments: (argument_list
    (identifier) @ws.path_var)
  (#eq? @_pkg5 "websocket")
  (#match? @_method5 "^(Dial|DefaultDialer)$")) @ws.def
