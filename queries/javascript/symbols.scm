;; 函数声明
(function_declaration
  name: (identifier) @symbol.name
  parameters: (formal_parameters) @symbol.parameters
) @symbol.function

;; 箭头函数（变量赋值）
(variable_declarator
  name: (identifier) @symbol.name
  value: (arrow_function
    parameters: (formal_parameters) @symbol.parameters
  )
) @symbol.function

;; 函数表达式赋值
(variable_declarator
  name: (identifier) @symbol.name
  value: (function_expression
    parameters: (formal_parameters) @symbol.parameters
  )
) @symbol.function

;; 类声明
(class_declaration
  name: (identifier) @symbol.name
) @symbol.class

;; 方法定义（类成员）
(method_definition
  name: (property_identifier) @symbol.name
  parameters: (formal_parameters) @symbol.parameters
) @symbol.method

;; 变量声明（非函数赋值）
(variable_declarator
  name: (identifier) @symbol.name
) @symbol.variable

;; export default function（React 组件最常见的声明方式）
;; 注：export default 在 tree-sitter 中 function_declaration 仍位于 declaration 字段下，
;; 已有的 export_statement 模式已通过 declaration 覆盖，但这里显式添加带完整参数签名的版本。
(export_statement
  declaration: (function_declaration
    name: (identifier) @symbol.name
    parameters: (formal_parameters) @symbol.parameters
  )
) @symbol.exported

;; export default class（同样 declaration 字段覆盖）
(export_statement
  declaration: (class_declaration
    name: (identifier) @symbol.name
  )
) @symbol.exported

;; 函数内部的嵌套函数声明（如闭包内 helper）
;; 注：已存在的顶层 function_declaration 模式已覆盖此场景，此模式提供显式标注。
(function_declaration
  name: (identifier) @symbol.name
  parameters: (formal_parameters) @symbol.parameters
) @symbol.function
