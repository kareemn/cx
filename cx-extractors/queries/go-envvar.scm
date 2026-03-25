; Go environment variable detection for CX
; Captures: @envvar.name, @envvar.site, @envvar.name_var

; os.Getenv("X"), os.LookupEnv("X")
(call_expression
  function: (selector_expression
    operand: (identifier) @_pkg
    field: (field_identifier) @_method)
  arguments: (argument_list
    (interpreted_string_literal) @envvar.name)
  (#eq? @_pkg "os")
  (#match? @_method "^(Getenv|LookupEnv)$")) @envvar.site

; Variant: os.Getenv(varName) — variable reference
(call_expression
  function: (selector_expression
    operand: (identifier) @_pkg2
    field: (field_identifier) @_method2)
  arguments: (argument_list
    (identifier) @envvar.name_var)
  (#eq? @_pkg2 "os")
  (#match? @_method2 "^(Getenv|LookupEnv)$")) @envvar.site
