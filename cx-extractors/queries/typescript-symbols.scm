; TypeScript/JavaScript symbol extraction queries for CX UniversalExtractor
; Uses standardized capture names: @func.name, @func.def, @call.name, @call.site,
; @import.path, @import.def, @type.name, @type.def

; ─── Function declarations ──────────────────────────────────────────
(function_declaration
  name: (identifier) @func.name) @func.def

; ─── Arrow functions assigned to const/let/var ──────────────────────
(lexical_declaration
  (variable_declarator
    name: (identifier) @func.name
    value: (arrow_function))) @func.def

; ─── Function expressions assigned to const/let/var ─────────────────
(lexical_declaration
  (variable_declarator
    name: (identifier) @func.name
    value: (function_expression))) @func.def

; ─── Class declarations ─────────────────────────────────────────────
(class_declaration
  name: (type_identifier) @type.name) @type.def

; ─── Method definitions inside classes ──────────────────────────────
(method_definition
  name: (property_identifier) @func.name) @func.def

; ─── Import statements (import ... from 'source') ──────────────────
(import_statement
  source: (string) @import.path) @import.def

; ─── Call expressions (plain function calls) ────────────────────────
(call_expression
  function: (identifier) @call.name) @call.site

; ─── Method call expressions (obj.method()) ─────────────────────────
(call_expression
  function: (member_expression
    property: (property_identifier) @call.name)) @call.site

; ─── Constructor calls (new Foo()) ──────────────────────────────────
(new_expression
  constructor: (identifier) @call.name) @call.site
