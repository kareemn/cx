; Java raw extraction query — captures ALL symbols, calls, imports, strings
; NO #match? or #eq? predicates — everything is captured for later classification

; ─── Package declarations ───────────────────────────────────────────
(package_declaration
  (scoped_identifier) @pkg.name) @pkg.def

; ─── Class declarations ─────────────────────────────────────────────
(class_declaration
  name: (identifier) @type.name) @type.def

; ─── Interface declarations ─────────────────────────────────────────
(interface_declaration
  name: (identifier) @type.name) @type.def

; ─── Enum declarations ──────────────────────────────────────────────
(enum_declaration
  name: (identifier) @type.name) @type.def

; ─── Method declarations ────────────────────────────────────────────
(method_declaration
  name: (identifier) @func.name) @func.def

; ─── Constructor declarations ───────────────────────────────────────
(constructor_declaration
  name: (identifier) @func.name) @func.def

; ─── Import declarations ────────────────────────────────────────────
(import_declaration
  (scoped_identifier) @import.path) @import.def

; ─── Static import declarations ─────────────────────────────────────
(import_declaration
  (scoped_identifier) @import.path) @import.def

; ─── Method invocations with receiver (obj.method()) ────────────────
(method_invocation
  object: (identifier) @call.receiver
  name: (identifier) @call.name
  arguments: (argument_list) @call.args) @call.site

; ─── Chained method invocations (expr.method()) ─────────────────────
(method_invocation
  name: (identifier) @call.name
  arguments: (argument_list) @call.args) @call.site

; ─── Constructor invocations (new Foo()) ────────────────────────────
(object_creation_expression
  type: (type_identifier) @call.name
  arguments: (argument_list) @call.args) @call.site

; ─── Annotation declarations (decorators) ───────────────────────────
(marker_annotation
  name: (identifier) @decorator.name) @decorator.def

(annotation
  name: (identifier) @decorator.name
  arguments: (annotation_argument_list
    (element_value_pair
      value: (string_literal) @decorator.arg)?)?) @decorator.def

; ─── Field string constants ─────────────────────────────────────────
(field_declaration
  declarator: (variable_declarator
    name: (identifier) @const.name
    value: (string_literal) @const.value))

; ─── Local variable string constants ────────────────────────────────
(local_variable_declaration
  declarator: (variable_declarator
    name: (identifier) @const.name
    value: (string_literal) @const.value))

; ─── String literals ────────────────────────────────────────────────
(string_literal) @string.value
