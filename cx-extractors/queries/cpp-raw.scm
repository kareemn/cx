; C++ raw extraction query — captures ALL symbols, calls, imports, strings
; NO #match? or #eq? predicates — everything is captured for later classification

; ─── Function definitions (free functions) ──────────────────────────
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @func.name)) @func.def

; ─── Method definitions inside classes ──────────────────────────────
(function_definition
  declarator: (function_declarator
    declarator: (field_identifier) @func.name)) @func.def

; ─── Out-of-class method definitions (Class::method) ────────────────
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      name: (identifier) @func.name))) @func.def

; ─── Destructor definitions (Class::~Class) ─────────────────────────
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

; ─── Method calls (obj.method(), ptr->method()) ─────────────────────
(call_expression
  function: (field_expression
    argument: (identifier) @call.receiver
    field: (field_identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── Chained method calls (expr.method()) ───────────────────────────
(call_expression
  function: (field_expression
    field: (field_identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── Namespace-qualified calls (ns::func()) ─────────────────────────
(call_expression
  function: (qualified_identifier
    scope: (namespace_identifier) @call.receiver
    name: (identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── Template function calls (func<T>()) ────────────────────────────
(call_expression
  function: (template_function
    name: (identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── String literals ────────────────────────────────────────────────
(string_literal) @string.value
