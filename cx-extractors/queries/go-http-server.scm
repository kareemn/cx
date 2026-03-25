; Go HTTP server endpoint detection for CX
; Captures: @endpoint.path, @endpoint.def, @endpoint.method

; http.HandleFunc("/path", handler), http.Handle("/path", handler)
(call_expression
  function: (selector_expression
    field: (field_identifier) @_method)
  arguments: (argument_list
    (interpreted_string_literal) @endpoint.path)
  (#match? @_method "^(HandleFunc|Handle)$")) @endpoint.def

; router.GET("/path", handler) — gorilla/mux, chi, gin, echo
; r.POST("/path", handler), e.GET("/path", handler)
; Path must start with "/" to avoid matching HTTP client calls like http.Get("https://...")
(call_expression
  function: (selector_expression
    field: (field_identifier) @endpoint.method)
  arguments: (argument_list
    (interpreted_string_literal) @endpoint.path)
  (#match? @endpoint.method "^(GET|POST|PUT|DELETE|PATCH|OPTIONS|HEAD|Get|Post|Put|Delete|Patch|Options|Head|Any|Group)$")
  (#match? @endpoint.path "^\"/")) @endpoint.def
