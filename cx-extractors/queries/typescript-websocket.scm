; TypeScript/JavaScript WebSocket detection for CX
; Captures: @ws.path, @ws.path_var, @ws.def

; --- Server-side ---

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

; --- Client-side ---

; new WebSocket('ws://host/path') — browser API, ws package
(new_expression
  constructor: (identifier) @_ctor
  arguments: (arguments
    (string) @ws.path)
  (#eq? @_ctor "WebSocket")) @ws.def

; new WebSocket(urlVar) — variable reference
(new_expression
  constructor: (identifier) @_ctor2
  arguments: (arguments
    (identifier) @ws.path_var)
  (#eq? @_ctor2 "WebSocket")) @ws.def

; new WebSocket(`ws://host/${path}`) — template string
(new_expression
  constructor: (identifier) @_ctor3
  arguments: (arguments
    (template_string) @ws.path)
  (#eq? @_ctor3 "WebSocket")) @ws.def

; WebSocket.connect('ws://host/path')
(call_expression
  function: (member_expression
    object: (identifier) @_obj2
    property: (property_identifier) @_method2)
  arguments: (arguments
    (string) @ws.path)
  (#eq? @_obj2 "WebSocket")
  (#eq? @_method2 "connect")) @ws.def
