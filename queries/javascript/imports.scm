;; ES import 声明 —
;; 注意：tree-sitter-javascript 中 import_clause / named_imports 不是 named field，
;; 结合结构和 CommonJS require
(import_statement
  (string (string_fragment) @import.path)
) @import

(call_expression
  function: (identifier) @_req
  arguments: (arguments (string (string_fragment) @import.path))
) @import.require
(#eq? @_req "require")
