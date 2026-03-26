; Python raw extraction query — captures ALL symbols, calls, imports, strings
; NO #match? or #eq? predicates — everything is captured for later classification

; ─── Function definitions ───────────────────────────────────────────
(function_definition
  name: (identifier) @func.name) @func.def

; ─── Class definitions ──────────────────────────────────────────────
(class_definition
  name: (identifier) @type.name) @type.def

; ─── Decorated definitions with decorator info ──────────────────────
(decorated_definition
  (decorator
    (identifier) @decorator.name)
  definition: (function_definition
    name: (identifier) @func.name)) @func.def

(decorated_definition
  (decorator
    (call
      function: (identifier) @decorator.name
      arguments: (argument_list
        (string) @decorator.arg)))
  definition: (function_definition
    name: (identifier) @func.name)) @func.def

(decorated_definition
  (decorator
    (call
      function: (attribute
        attribute: (identifier) @decorator.name)
      arguments: (argument_list
        (string) @decorator.arg)))
  definition: (function_definition
    name: (identifier) @func.name)) @func.def

(decorated_definition
  (decorator
    (attribute
      attribute: (identifier) @decorator.name))
  definition: (function_definition
    name: (identifier) @func.name)) @func.def

; ─── Decorated class definitions ────────────────────────────────────
(decorated_definition
  definition: (class_definition
    name: (identifier) @type.name)) @type.def

; ─── Import statements ──────────────────────────────────────────────
(import_statement
  name: (dotted_name) @import.path) @import.def

(import_statement
  name: (aliased_import
    name: (dotted_name) @import.path
    alias: (identifier) @import.alias)) @import.def

; ─── From-import statements ─────────────────────────────────────────
(import_from_statement
  module_name: (dotted_name) @import.path) @import.def

; ─── Plain function calls ───────────────────────────────────────────
(call
  function: (identifier) @call.name
  arguments: (argument_list) @call.args) @call.site

; ─── Method/attribute calls with receiver ───────────────────────────
(call
  function: (attribute
    object: (identifier) @call.receiver
    attribute: (identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── Chained attribute calls (expr.method()) ────────────────────────
(call
  function: (attribute
    attribute: (identifier) @call.name)
  arguments: (argument_list) @call.args) @call.site

; ─── String assignments ─────────────────────────────────────────────
(assignment
  left: (identifier) @const.name
  right: (string) @const.value)

; ─── Standalone string literals ─────────────────────────────────────
(string) @string.value
