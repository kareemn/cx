; Python HTTP client call detection for CX
; Captures: @http_call.url, @http_call.site

; requests.get(url), requests.post(url), httpx.get(url)
(call
  function: (attribute
    object: (identifier) @_obj
    attribute: (identifier) @_method)
  arguments: (argument_list
    (string) @http_call.url)
  (#match? @_obj "^(requests|httpx)$")
  (#match? @_method "^(get|post|put|delete|patch|head|options)$")) @http_call.site

; urllib.request.urlopen(url)
(call
  function: (attribute
    attribute: (identifier) @_method)
  arguments: (argument_list
    (string) @http_call.url)
  (#eq? @_method "urlopen")) @http_call.site
