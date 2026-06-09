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
