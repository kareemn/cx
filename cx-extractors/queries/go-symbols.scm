; Go symbol extraction queries for CX UniversalExtractor
; Uses standardized capture names: @func.name, @func.def, @call.name, @call.site,
; @import.path, @import.def, @type.name, @type.def

; ─── Function declarations ───────────────────────────────────────────
(function_declaration
  name: (identifier) @func.name) @func.def

; ─── Method declarations ─────────────────────────────────────────────
(method_declaration
  name: (field_identifier) @func.name) @func.def

; ─── Type declarations ───────────────────────────────────────────────
(type_declaration
  (type_spec
    name: (type_identifier) @type.name)) @type.def

; ─── Import paths ────────────────────────────────────────────────────
(import_spec
  path: (interpreted_string_literal) @import.path) @import.def

; ─── Call expressions (plain function calls) ─────────────────────────
(call_expression
  function: (identifier) @call.name) @call.site

; ─── Method call expressions (receiver.Method()) ─────────────────────
(call_expression
  function: (selector_expression
    field: (field_identifier) @call.name)) @call.site
