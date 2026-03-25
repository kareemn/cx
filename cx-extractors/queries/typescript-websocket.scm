; TypeScript/JavaScript WebSocket detection for CX
; Captures: @ws.def

; new WebSocket.Server({ ... })
(new_expression
  constructor: (member_expression
    object: (identifier) @_obj
    property: (property_identifier) @_prop)
  (#eq? @_obj "WebSocket")
  (#eq? @_prop "Server")) @ws.def

; wss.on('connection', handler), io.on('connection', handler) — ws, socket.io
(call_expression
  function: (member_expression
    property: (property_identifier) @_method)
  arguments: (arguments
    (string) @_event)
  (#eq? @_method "on")
  (#match? @_event "connection")) @ws.def
