; Go HTTP client call detection for CX
; Captures: @http_call.url, @http_call.site

; http.Get(url), http.Post(url, ...), http.Head(url)
(call_expression
  function: (selector_expression
    operand: (identifier) @_pkg
    field: (field_identifier) @_method)
  arguments: (argument_list
    (interpreted_string_literal) @http_call.url)
  (#eq? @_pkg "http")
  (#match? @_method "^(Get|Post|Head|PostForm)$")) @http_call.site

; client.Get(url), client.Post(url, ...)
(call_expression
  function: (selector_expression
    field: (field_identifier) @_method)
  arguments: (argument_list
    (interpreted_string_literal) @http_call.url)
  (#match? @_method "^(Get|Post|Head|PostForm|Do)$")
  (#match? @http_call.url "https?://")) @http_call.site
