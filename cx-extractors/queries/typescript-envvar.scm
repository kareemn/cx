; TypeScript/JavaScript environment variable detection for CX
; Captures: @envvar.name, @envvar.site

; process.env.VARIABLE
(member_expression
  object: (member_expression
    object: (identifier) @_process
    property: (property_identifier) @_env)
  property: (property_identifier) @envvar.name
  (#eq? @_process "process")
  (#eq? @_env "env")) @envvar.site

; process.env["VARIABLE"]
(subscript_expression
  object: (member_expression
    object: (identifier) @_process
    property: (property_identifier) @_env)
  index: (string) @envvar.name
  (#eq? @_process "process")
  (#eq? @_env "env")) @envvar.site
