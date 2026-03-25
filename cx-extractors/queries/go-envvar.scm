; Go environment variable detection for CX
; Captures: @envvar.name, @envvar.site

; os.Getenv("X"), os.LookupEnv("X")
(call_expression
  function: (selector_expression
    operand: (identifier) @_pkg
    field: (field_identifier) @_method)
  arguments: (argument_list
    (interpreted_string_literal) @envvar.name)
  (#eq? @_pkg "os")
  (#match? @_method "^(Getenv|LookupEnv)$")) @envvar.site
