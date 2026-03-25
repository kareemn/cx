; C environment variable detection for CX
; Captures: @envvar.name, @envvar.site

; getenv("VAR")
(call_expression
  function: (identifier) @_func
  arguments: (argument_list
    (string_literal) @envvar.name)
  (#eq? @_func "getenv")) @envvar.site
