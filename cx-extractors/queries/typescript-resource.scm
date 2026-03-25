; TypeScript/JavaScript resource detection for CX — Redis, GCS/S3, OpenAI SDK proxy
; Captures: @resource.name, @resource.def
;
; @resource.name uses the constructor/function name when no string arg is available.
; universal.rs falls back to "resource" when absent.

; --- Redis ---

; new Redis(options) — ioredis
(new_expression
  constructor: (identifier) @resource.name
  (#eq? @resource.name "Redis")) @resource.def

; createClient() — redis package
(call_expression
  function: (identifier) @resource.name
  (#eq? @resource.name "createClient")) @resource.def

; --- Cloud Storage ---

; new Storage() — @google-cloud/storage
(new_expression
  constructor: (identifier) @resource.name
  (#eq? @resource.name "Storage")) @resource.def

; new S3Client(config) — @aws-sdk/client-s3
(new_expression
  constructor: (identifier) @resource.name
  (#eq? @resource.name "S3Client")) @resource.def

; --- OpenAI SDK ---

; new OpenAI({ baseURL: ... })
(new_expression
  constructor: (identifier) @resource.name
  (#eq? @resource.name "OpenAI")) @resource.def
