;; 函数调用
(call_expression
  function: (identifier) @call.name
  arguments: (arguments) @call.arguments
) @call

;; 方法调用
(call_expression
  function: (field_expression
    field: (field_identifier) @call.name
  )
  arguments: (arguments) @call.arguments
) @call.method

;; 宏调用
(macro_invocation
  macro: (identifier) @call.name
) @call.macro
