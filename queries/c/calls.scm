;; C 调用查询 — 提取函数调用关系

(call_expression
  function: (identifier) @caller_name
  arguments: (argument_list) @args) @call

(call_expression
  function: (field_expression
    field: (field_identifier) @method_name)
  arguments: (argument_list) @args) @method_call
