; TypeScript/JavaScript raw extraction query — captures ALL symbols, calls, imports, strings
; NO #match? or #eq? predicates — everything is captured for later classification

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

; ─── require() calls — CRITICAL for CommonJS modules ────────────────
(call_expression
  function: (identifier) @require.name
  arguments: (arguments
    (string) @require.path)) @require.site

; ─── Plain function calls ───────────────────────────────────────────
(call_expression
  function: (identifier) @call.name
  arguments: (arguments) @call.args) @call.site

; ─── Method calls with receiver (obj.method()) ─────────────────────
(call_expression
  function: (member_expression
    object: (identifier) @call.receiver
    property: (property_identifier) @call.name)
  arguments: (arguments) @call.args) @call.site

; ─── Chained method calls (expr.method()) ───────────────────────────
(call_expression
  function: (member_expression
    property: (property_identifier) @call.name)
  arguments: (arguments) @call.args) @call.site

; ─── Constructor calls (new Foo()) ──────────────────────────────────
(new_expression
  constructor: (identifier) @call.name
  arguments: (arguments) @call.args) @call.site

; ─── Constructor calls with member expression (new mod.Foo()) ───────
(new_expression
  constructor: (member_expression
    object: (identifier) @call.receiver
    property: (property_identifier) @call.name)
  arguments: (arguments) @call.args) @call.site

; ─── Variable string assignments ────────────────────────────────────
(lexical_declaration
  (variable_declarator
    name: (identifier) @const.name
    value: (string) @const.value))

; ─── Standalone string literals ─────────────────────────────────────
(string) @string.value

; ─── Template string literals ───────────────────────────────────────
(template_string) @string.value
