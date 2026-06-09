;; ES import 声明 —
;; 注意：tree-sitter-typescript 中 import_clause / named_imports 不是 named field
(import_statement
  (string (string_fragment) @import.path)
) @import

(call_expression
  function: (identifier) @_req
  arguments: (arguments (string (string_fragment) @import.path))
) @import.require
(#eq? @_req "require")
