; C symbol extraction queries for CX UniversalExtractor
; Uses standardized capture names: @func.name, @func.def, @call.name, @call.site,
; @import.path, @import.def, @type.name, @type.def

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

; ─── Include directives (system headers: #include <stdio.h>) ───────
(preproc_include
  path: (system_lib_string) @import.path) @import.def

; ─── Include directives (local headers: #include "foo.h") ──────────
(preproc_include
  path: (string_literal) @import.path) @import.def

; ─── Call expressions (plain function calls) ────────────────────────
(call_expression
  function: (identifier) @call.name) @call.site

; ─── Call via field expression (ptr->method(), s.func()) ────────────
(call_expression
  function: (field_expression
    field: (field_identifier) @call.name)) @call.site
