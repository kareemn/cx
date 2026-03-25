; C++ HTTP client detection for CX
; Captures: @http_call.url, @http_call.site

; curl_easy_setopt(curl, CURLOPT_URL, "https://...")
; httplib::Client cli("https://...")
; Any string literal containing http:// or https:// passed as a call argument
(call_expression
  arguments: (argument_list
    (string_literal) @http_call.url)
  (#match? @http_call.url "https?://")) @http_call.site
