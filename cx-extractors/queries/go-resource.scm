; Go resource detection for CX — Redis, GCS/S3, OpenAI SDK proxy
; Captures: @resource.name, @resource.def
;
; For calls without a string argument, the package name is used as @resource.name
; (e.g., "redis" for redis.NewClient(), "storage" for storage.NewClient())

; --- Redis ---

; redis.NewClient(...), redis.NewClusterClient(...)
(call_expression
  function: (selector_expression
    operand: (identifier) @resource.name
    field: (field_identifier) @_method)
  (#eq? @resource.name "redis")
  (#match? @_method "^(NewClient|NewClusterClient|NewFailoverClient|NewUniversalClient)$")) @resource.def

; --- Cloud Storage ---

; storage.NewClient(ctx) — Google Cloud Storage
(call_expression
  function: (selector_expression
    operand: (identifier) @resource.name
    field: (field_identifier) @_method2)
  (#eq? @resource.name "storage")
  (#eq? @_method2 "NewClient")) @resource.def

; s3.New(session) — AWS S3
(call_expression
  function: (selector_expression
    operand: (identifier) @resource.name
    field: (field_identifier) @_method3)
  (#eq? @resource.name "s3")
  (#match? @_method3 "^(New|NewFromConfig)$")) @resource.def

; --- OpenAI SDK as service proxy ---

; openai.NewClient(option.WithBaseURL(baseURL))
(call_expression
  function: (selector_expression
    operand: (identifier) @resource.name
    field: (field_identifier) @_method4)
  (#eq? @resource.name "openai")
  (#eq? @_method4 "NewClient")) @resource.def
