;; 方法调用
(invocation_expression
  function: (identifier) @call.name
  arguments: (argument_list) @call.arguments
) @call

;; 成员方法调用（obj.Method()）
(invocation_expression
  function: (member_access_expression
    name: (identifier) @call.name
  )
  arguments: (argument_list) @call.arguments
) @call.method

;; 构造函数调用（new ClassName()）
(object_creation_expression
  type: (identifier) @call.name
  arguments: (argument_list)? @call.arguments
) @call.constructor
