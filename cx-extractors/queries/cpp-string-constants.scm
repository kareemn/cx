; C++ string constant collection for CX local constant propagation
; Captures: @const.name, @const.value

; const std::string path = "/ws/s2s"; or const char* path = "/ws/s2s";
(declaration
  declarator: (init_declarator
    declarator: (identifier) @const.name
    value: (string_literal) @const.value))

; Pointer declarator: const char* path = "/ws/s2s";
(declaration
  declarator: (init_declarator
    declarator: (pointer_declarator
      declarator: (identifier) @const.name)
    value: (string_literal) @const.value))

; Reference declarator: const std::string& path = "/ws/s2s";
(declaration
  declarator: (init_declarator
    declarator: (reference_declarator
      (identifier) @const.name)
    value: (string_literal) @const.value))

; #define PATH "/ws/s2s"
(preproc_def
  name: (identifier) @const.name
  value: (preproc_arg) @const.value)
