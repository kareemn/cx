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

; --- Database connection pools ---

; pgxpool.New(ctx, connString) — PostgreSQL via pgx
(call_expression
  function: (selector_expression
    operand: (identifier) @resource.name
    field: (field_identifier) @_method_db)
  (#eq? @resource.name "pgxpool")
  (#match? @_method_db "^(New|NewWithConfig)$")) @resource.def

; pgx.Connect(ctx, connString) — PostgreSQL via pgx (single conn)
(call_expression
  function: (selector_expression
    operand: (identifier) @resource.name
    field: (field_identifier) @_method_db2)
  (#eq? @resource.name "pgx")
  (#match? @_method_db2 "^(Connect|ConnectConfig)$")) @resource.def

; sql.Open(driver, dsn)
(call_expression
  function: (selector_expression
    operand: (identifier) @resource.name
    field: (field_identifier) @_method_db3)
  (#eq? @resource.name "sql")
  (#eq? @_method_db3 "Open")) @resource.def

; mongo.Connect(ctx, opts)
(call_expression
  function: (selector_expression
    operand: (identifier) @resource.name
    field: (field_identifier) @_method_db4)
  (#eq? @resource.name "mongo")
  (#eq? @_method_db4 "Connect")) @resource.def

; --- OpenAI SDK as service proxy ---

; openai.NewClient(option.WithBaseURL(baseURL))
(call_expression
  function: (selector_expression
    operand: (identifier) @resource.name
    field: (field_identifier) @_method4)
  (#eq? @resource.name "openai")
  (#eq? @_method4 "NewClient")) @resource.def
