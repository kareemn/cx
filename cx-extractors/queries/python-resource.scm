; Python resource detection for CX — Redis, GCS/S3, OpenAI SDK proxy
; Captures: @resource.name, @resource.def
;
; @resource.name is the package/class name or URL string when available.
; When absent, universal.rs falls back to "resource".

; --- Redis ---

; redis.Redis(host=...), redis.StrictRedis(host=...)
(call
  function: (attribute
    object: (identifier) @resource.name
    attribute: (identifier) @_method)
  (#eq? @resource.name "redis")
  (#match? @_method "^(Redis|StrictRedis)$")) @resource.def

; aioredis.from_url("redis://host")
(call
  function: (attribute
    object: (identifier) @resource.name
    attribute: (identifier) @_method2)
  (#eq? @resource.name "aioredis")
  (#eq? @_method2 "from_url")) @resource.def

; --- Cloud Storage ---

; storage.Client() — Google Cloud Storage
(call
  function: (attribute
    object: (identifier) @resource.name
    attribute: (identifier) @_method3)
  (#eq? @resource.name "storage")
  (#eq? @_method3 "Client")) @resource.def

; boto3.client('s3'), boto3.resource('s3')
(call
  function: (attribute
    object: (identifier) @resource.name
    attribute: (identifier) @_method4)
  arguments: (argument_list
    (string) @_svc)
  (#eq? @resource.name "boto3")
  (#match? @_method4 "^(client|resource)$")
  (#match? @_svc "s3")) @resource.def

; --- OpenAI SDK ---

; openai.OpenAI(base_url=...), openai.AsyncOpenAI(...)
(call
  function: (attribute
    object: (identifier) @resource.name
    attribute: (identifier) @_method5)
  (#eq? @resource.name "openai")
  (#match? @_method5 "^(OpenAI|AsyncOpenAI)$")) @resource.def
