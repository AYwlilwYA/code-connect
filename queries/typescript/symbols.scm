;; 函数声明
(function_declaration
  name: (identifier) @symbol.name
  parameters: (formal_parameters) @symbol.parameters
  return_type: (type_annotation)? @symbol.return_type
) @symbol.function

;; 箭头函数（变量赋值）
(variable_declarator
  name: (identifier) @symbol.name
  value: (arrow_function
    parameters: (formal_parameters) @symbol.parameters
    return_type: (type_annotation)? @symbol.return_type
  )
) @symbol.function

;; 类声明
(class_declaration
  name: (type_identifier) @symbol.name
) @symbol.class

;; 接口声明
(interface_declaration
  name: (type_identifier) @symbol.name
) @symbol.interface

;; 枚举声明
(enum_declaration
  name: (identifier) @symbol.name
) @symbol.enum

;; 类型别名
(type_alias_declaration
  name: (type_identifier) @symbol.name
) @symbol.type_alias

;; 方法定义（类成员）
(method_definition
  name: (property_identifier) @symbol.name
  parameters: (formal_parameters) @symbol.parameters
  return_type: (type_annotation)? @symbol.return_type
) @symbol.method

;; 变量声明
(variable_declarator
  name: (identifier) @symbol.name
  type: (type_annotation)? @symbol.type
) @symbol.variable

;; 导出声明（标记公开）
(export_statement
  declaration: (function_declaration
    name: (identifier) @symbol.name
  )
) @symbol.exported

(export_statement
  declaration: (class_declaration
    name: (type_identifier) @symbol.name
  )
) @symbol.exported
