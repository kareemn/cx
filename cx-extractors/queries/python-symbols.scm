; Python symbol extraction queries for CX UniversalExtractor
; Uses standardized capture names: @func.name, @func.def, @call.name, @call.site,
; @import.path, @import.def, @type.name, @type.def

; ─── Function definitions ───────────────────────────────────────────
(function_definition
  name: (identifier) @func.name) @func.def

; ─── Class definitions ──────────────────────────────────────────────
(class_definition
  name: (identifier) @type.name) @type.def

; ─── Decorated function definitions ─────────────────────────────────
(decorated_definition
  definition: (function_definition
    name: (identifier) @func.name)) @func.def

; ─── Decorated class definitions ────────────────────────────────────
(decorated_definition
  definition: (class_definition
    name: (identifier) @type.name)) @type.def

; ─── Import statements (import x, import x.y) ──────────────────────
(import_statement
  name: (dotted_name) @import.path) @import.def

; ─── From-import statements (from x import y) ──────────────────────
(import_from_statement
  module_name: (dotted_name) @import.path) @import.def

; ─── Call expressions (plain function calls) ────────────────────────
(call
  function: (identifier) @call.name) @call.site

; ─── Method/attribute call expressions (obj.method()) ───────────────
(call
  function: (attribute
    attribute: (identifier) @call.name)) @call.site
