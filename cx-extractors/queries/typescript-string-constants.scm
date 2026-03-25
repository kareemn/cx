; TypeScript/JavaScript string constant collection for CX local constant propagation
; Captures: @const.name, @const.value

; const path = "/ws/s2s", let path = "/ws/s2s", var path = "/ws/s2s"
(variable_declarator
  name: (identifier) @const.name
  value: (string) @const.value)
