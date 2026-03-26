; Go raw extraction query — captures ALL symbols, calls, imports, strings
; NO #match? or #eq? predicates — everything is captured for later classification

; ─── Package declarations ────────────────────────────────────────────
(package_clause
  (package_identifier) @pkg.name) @pkg.def

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

; ─── Import paths with optional alias ────────────────────────────────
(import_spec
  name: (package_identifier) @import.alias
  path: (interpreted_string_literal) @import.path) @import.def

(import_spec
  path: (interpreted_string_literal) @import.path) @import.def

; ─── Plain function calls ────────────────────────────────────────────
(call_expression
  function: (identifier) @call.name
  arguments: (argument_list) @call.args) @call.site

; ─── Method calls with receiver (receiver.Method()) ──────────────────
(call_expression
  function: (selector_expression
    operand: (identifier) @call.receiver
    field: (field_identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── Chained method calls (expr.Method()) — receiver is non-identifier
(call_expression
  function: (selector_expression
    field: (field_identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── String constants (const name = "value") ─────────────────────────
(const_spec
  name: (identifier) @const.name
  value: (expression_list
    (interpreted_string_literal) @const.value))

; ─── Var string assignments ──────────────────────────────────────────
(var_spec
  name: (identifier) @const.name
  value: (expression_list
    (interpreted_string_literal) @const.value))

; ─── Short var decl with string literal ──────────────────────────────
(short_var_declaration
  left: (expression_list
    (identifier) @const.name)
  right: (expression_list
    (interpreted_string_literal) @const.value))

; ─── Standalone string literals (for URL/address detection) ──────────
(interpreted_string_literal) @string.value
