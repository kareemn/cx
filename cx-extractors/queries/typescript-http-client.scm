; TypeScript/JavaScript HTTP client call detection for CX
; Captures: @http_call.url, @http_call.site

; fetch('/api/endpoint')
(call_expression
  function: (identifier) @_fn
  arguments: (arguments
    (string) @http_call.url)
  (#eq? @_fn "fetch")) @http_call.site

; fetch(`/api/${id}`)
(call_expression
  function: (identifier) @_fn
  arguments: (arguments
    (template_string) @http_call.url)
  (#eq? @_fn "fetch")) @http_call.site

; axios.get('https://...'), got.post('http://...')
(call_expression
  function: (member_expression
    property: (property_identifier) @_method)
  arguments: (arguments
    (string) @http_call.url)
  (#match? @_method "^(get|post|put|delete|patch|head|options|request)$")
  (#match? @http_call.url "https?://")) @http_call.site
