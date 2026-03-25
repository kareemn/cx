; Python HTTP server endpoint detection for CX
; Captures: @endpoint.path, @endpoint.def, @endpoint.method

; Flask: @app.route('/path'), FastAPI: @app.get('/path'), @app.post('/path')
(decorator
  (call
    function: (attribute
      attribute: (identifier) @endpoint.method)
    arguments: (argument_list
      (string) @endpoint.path))
  (#match? @endpoint.method "^(route|get|post|put|delete|patch|options|head)$")) @endpoint.def

; Django: path('url/', view), re_path(r'^url/$', view)
(call
  function: (identifier) @_fn
  arguments: (argument_list
    (string) @endpoint.path)
  (#match? @_fn "^(path|re_path|url)$")
  (#match? @endpoint.path "/")) @endpoint.def

; Variant: app.get(varName, handler) — variable reference
(call
  function: (attribute
    attribute: (identifier) @_method2)
  arguments: (argument_list
    (identifier) @endpoint.path_var)
  (#match? @_method2 "^(route|get|post|put|delete|patch|options|head)$")) @endpoint.def
