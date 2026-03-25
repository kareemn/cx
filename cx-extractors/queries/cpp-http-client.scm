; C++ HTTP client detection for CX
; Captures: @http_call.url, @http_call.url_var, @http_call.site

; Any string literal containing http:// or https:// passed as a call argument
(call_expression
  arguments: (argument_list
    (string_literal) @http_call.url)
  (#match? @http_call.url "https?://")) @http_call.site
