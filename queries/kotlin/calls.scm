;; 函数调用
(call_expression
  callee: (simple_identifier) @call.name
  arguments: (call_suffix (value_arguments) @call.arguments)
) @call

;; 成员调用（obj.method()）
(call_expression
  callee: (navigation_expression
    name: (simple_identifier) @call.name
  )
  arguments: (call_suffix (value_arguments) @call.arguments)
) @call.method
