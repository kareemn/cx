; TypeScript/JavaScript HTTP server endpoint detection for CX
; Captures: @endpoint.path, @endpoint.path_var, @endpoint.def, @endpoint.method

; app.get('/path', handler), router.post('/path', handler) — express, fastify, koa
; Path must start with quote+/ to avoid matching HTTP client calls like axios.get('https://...')
(call_expression
  function: (member_expression
    property: (property_identifier) @endpoint.method)
  arguments: (arguments
    (string) @endpoint.path)
  (#match? @endpoint.method "^(get|post|put|delete|patch|options|head|all)$")
  (#match? @endpoint.path "^['\"]/" )) @endpoint.def

; Variant: app.get(varName, handler) — variable reference
(call_expression
  function: (member_expression
    property: (property_identifier) @_method2)
  arguments: (arguments
    (identifier) @endpoint.path_var)
  (#match? @_method2 "^(get|post|put|delete|patch|options|head|all)$")) @endpoint.def
