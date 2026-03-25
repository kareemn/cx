; Go string constant collection for CX local constant propagation
; Captures: @const.name, @const.value

; Short variable declaration: path := "/ws/s2s"
(short_var_declaration
  left: (expression_list (identifier) @const.name)
  right: (expression_list (interpreted_string_literal) @const.value))

; Var spec: var path = "/ws/s2s"
(var_spec
  name: (identifier) @const.name
  value: (expression_list (interpreted_string_literal) @const.value))

; Const spec: const path = "/ws/s2s"
(const_spec
  name: (identifier) @const.name
  value: (expression_list (interpreted_string_literal) @const.value))
