; C raw extraction query — captures ALL symbols, calls, imports, strings
; NO #match? or #eq? predicates — everything is captured for later classification

; ─── Function definitions ───────────────────────────────────────────
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @func.name)) @func.def

; ─── Struct declarations ────────────────────────────────────────────
(struct_specifier
  name: (type_identifier) @type.name
  body: (field_declaration_list)) @type.def

; ─── Enum declarations ──────────────────────────────────────────────
(enum_specifier
  name: (type_identifier) @type.name
  body: (enumerator_list)) @type.def

; ─── Typedef declarations ───────────────────────────────────────────
(type_definition
  declarator: (type_identifier) @type.name) @type.def

; ─── Include directives (system headers) ────────────────────────────
(preproc_include
  path: (system_lib_string) @import.path) @import.def

; ─── Include directives (local headers) ─────────────────────────────
(preproc_include
  path: (string_literal) @import.path) @import.def

; ─── Plain function calls ───────────────────────────────────────────
(call_expression
  function: (identifier) @call.name
  arguments: (argument_list) @call.args) @call.site

; ─── Calls via field expression (ptr->method(), s.func()) ───────────
(call_expression
  function: (field_expression
    argument: (identifier) @call.receiver
    field: (field_identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── Calls via field expression (chained, expr->func()) ─────────────
(call_expression
  function: (field_expression
    field: (field_identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── #define constants ──────────────────────────────────────────────
(preproc_def
  name: (identifier) @const.name
  value: (preproc_arg) @const.value)

; ─── String literals ────────────────────────────────────────────────
(string_literal) @string.value
