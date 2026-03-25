; Python string constant collection for CX local constant propagation
; Captures: @const.name, @const.value

; Simple assignment: path = "/ws/s2s" or PATH = "/api/v1"
(assignment
  left: (identifier) @const.name
  right: (string) @const.value)
