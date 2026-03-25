; C++ environment variable detection for CX
; Captures: @envvar.name, @envvar.name_var, @envvar.site

; std::getenv("VAR"), getenv("VAR")
(call_expression
  function: (identifier) @_func
  arguments: (argument_list
    (string_literal) @envvar.name)
  (#eq? @_func "getenv")) @envvar.site

; std::getenv("VAR") via qualified name
(call_expression
  function: (qualified_identifier
    name: (identifier) @_func2)
  arguments: (argument_list
    (string_literal) @envvar.name)
  (#eq? @_func2 "getenv")) @envvar.site

; Variant: getenv(varName) — variable reference
(call_expression
  function: (identifier) @_func3
  arguments: (argument_list
    (identifier) @envvar.name_var)
  (#eq? @_func3 "getenv")) @envvar.site
