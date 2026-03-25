; C++ symbol extraction queries for CX UniversalExtractor
; Uses standardized capture names: @func.name, @func.def, @call.name, @call.site,
; @import.path, @import.def, @type.name, @type.def

; ─── Function definitions (free functions) ──────────────────────────
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @func.name)) @func.def

; ─── Method definitions inside classes (field_identifier) ───────────
(function_definition
  declarator: (function_declarator
    declarator: (field_identifier) @func.name)) @func.def

; ─── Out-of-class method definitions (Class::method) ─────────────
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (identifier) @func.name))) @func.def

; ─── Destructor definitions (Class::~Class) ──────────────────────
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (destructor_name) @func.name))) @func.def

; ─── Class declarations ─────────────────────────────────────────────
(class_specifier
  name: (type_identifier) @type.name
  body: (field_declaration_list)) @type.def

; ─── Struct declarations ────────────────────────────────────────────
(struct_specifier
  name: (type_identifier) @type.name
  body: (field_declaration_list)) @type.def

; ─── Enum declarations ──────────────────────────────────────────────
(enum_specifier
  name: (type_identifier) @type.name
  body: (enumerator_list)) @type.def

; ─── Namespace as package ───────────────────────────────────────────
(namespace_definition
  name: (namespace_identifier) @pkg.name) @pkg.def

; ─── Include directives (system headers: #include <iostream>) ──────
(preproc_include
  path: (system_lib_string) @import.path) @import.def

; ─── Include directives (local headers: #include "server.h") ───────
(preproc_include
  path: (string_literal) @import.path) @import.def

; ─── Call expressions (plain function calls) ────────────────────────
(call_expression
  function: (identifier) @call.name) @call.site

; ─── Method call expressions (obj.method(), ptr->method()) ──────────
(call_expression
  function: (field_expression
    field: (field_identifier) @call.name)) @call.site
