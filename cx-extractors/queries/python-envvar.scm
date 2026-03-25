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
