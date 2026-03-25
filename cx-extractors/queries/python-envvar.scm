; Python environment variable detection for CX
; Captures: @envvar.name, @envvar.site

; os.getenv("X")
(call
  function: (attribute
    object: (identifier) @_obj
    attribute: (identifier) @_method)
  arguments: (argument_list
    (string) @envvar.name)
  (#eq? @_obj "os")
  (#eq? @_method "getenv")) @envvar.site

; os.environ.get("X")
(call
  function: (attribute
    object: (attribute
      object: (identifier) @_obj
      attribute: (identifier) @_attr)
    attribute: (identifier) @_method)
  arguments: (argument_list
    (string) @envvar.name)
  (#eq? @_obj "os")
  (#eq? @_attr "environ")
  (#eq? @_method "get")) @envvar.site

; os.environ["X"]
(subscript
  value: (attribute
    object: (identifier) @_obj
    attribute: (identifier) @_attr)
  subscript: (string) @envvar.name
  (#eq? @_obj "os")
  (#eq? @_attr "environ")) @envvar.site

; Variant: os.getenv(varName) — variable reference
(call
  function: (attribute
    object: (identifier) @_obj2
    attribute: (identifier) @_method2)
  arguments: (argument_list
    (identifier) @envvar.name_var)
  (#eq? @_obj2 "os")
  (#eq? @_method2 "getenv")) @envvar.site
