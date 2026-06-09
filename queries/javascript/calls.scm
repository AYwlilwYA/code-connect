;; 函数调用
(call_expression
  function: (identifier) @call.name
  arguments: (arguments) @call.arguments
) @call

;; 方法调用（obj.method()）
(call_expression
  function: (member_expression
    property: (property_identifier) @call.name
  )
  arguments: (arguments) @call.arguments
) @call.method

;; new 表达式
(new_expression
  constructor: (identifier) @call.name
  arguments: (arguments) @call.arguments
) @call.constructor
