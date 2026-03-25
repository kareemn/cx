; C++ environment variable detection for CX
; Captures: @envvar.name, @envvar.site

; std::getenv("VAR"), getenv("VAR")
(call_expression
  function: (identifier) @_func
  arguments: (argument_list
    (string_literal) @envvar.name)
  (#eq? @_func "getenv")) @envvar.site

; std::getenv("VAR") via qualified name
(call_expression
  function: (qualified_identifier
    name: (identifier) @_func)
  arguments: (argument_list
    (string_literal) @envvar.name)
  (#eq? @_func "getenv")) @envvar.site
