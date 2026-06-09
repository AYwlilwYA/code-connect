;; 方法调用
(method_invocation
  (identifier) @call.name
  (argument_list) @call.arguments
) @call

;; 对象方法调用（obj.method()）
(method_invocation
  (_) @call.object
  (identifier) @call.name
  (argument_list) @call.arguments
) @call.method

;; 构造函数调用（new ClassName()）
(object_creation_expression
  (type_identifier) @call.name
  (argument_list) @call.arguments
) @call.constructor
