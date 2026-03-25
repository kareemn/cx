; TypeScript/JavaScript HTTP server endpoint detection for CX
; Captures: @endpoint.path, @endpoint.def, @endpoint.method

; app.get('/path', handler), router.post('/path', handler) — express, fastify, koa
; Path must start with quote+/ to avoid matching HTTP client calls like axios.get('https://...')
(call_expression
  function: (member_expression
    property: (property_identifier) @endpoint.method)
  arguments: (arguments
    (string) @endpoint.path)
  (#match? @endpoint.method "^(get|post|put|delete|patch|options|head|all)$")
  (#match? @endpoint.path "^['\"]/" )) @endpoint.def
