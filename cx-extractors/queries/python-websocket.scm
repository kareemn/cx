; Python WebSocket detection for CX
; Captures: @ws.path, @ws.path_var, @ws.def

; --- Server-side ---

; FastAPI: @app.websocket('/path')
(decorator
  (call
    function: (attribute
      attribute: (identifier) @_method)
    arguments: (argument_list
      (string) @ws.path))
  (#eq? @_method "websocket")) @ws.def

; websockets.serve(handler, host, port)
(call
  function: (attribute
    object: (identifier) @_obj
    attribute: (identifier) @_method2)
  (#eq? @_obj "websockets")
  (#eq? @_method2 "serve")) @ws.def

; --- Client-side ---

; websockets.connect('ws://host/path')
(call
  function: (attribute
    object: (identifier) @_obj3
    attribute: (identifier) @_method3)
  arguments: (argument_list
    (string) @ws.path)
  (#eq? @_obj3 "websockets")
  (#eq? @_method3 "connect")) @ws.def

; websocket.WebSocketApp('ws://host/path')
(call
  function: (attribute
    object: (identifier) @_obj4
    attribute: (identifier) @_method4)
  arguments: (argument_list
    (string) @ws.path)
  (#eq? @_obj4 "websocket")
  (#eq? @_method4 "WebSocketApp")) @ws.def

; await websockets.connect(urlVar) — variable reference
(call
  function: (attribute
    object: (identifier) @_obj5
    attribute: (identifier) @_method5)
  arguments: (argument_list
    (identifier) @ws.path_var)
  (#eq? @_obj5 "websockets")
  (#eq? @_method5 "connect")) @ws.def
