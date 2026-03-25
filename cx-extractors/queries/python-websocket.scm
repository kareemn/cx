; Python WebSocket detection for CX
; Captures: @ws.path, @ws.def

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
    attribute: (identifier) @_method)
  (#eq? @_obj "websockets")
  (#eq? @_method "serve")) @ws.def
