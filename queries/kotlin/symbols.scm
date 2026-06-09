;; 类声明
(class_declaration
  name: (type_identifier) @symbol.name
) @symbol.class

;; 接口声明
(interface_declaration
  name: (type_identifier) @symbol.name
) @symbol.interface

;; 对象声明
(object_declaration
  name: (identifier) @symbol.name
) @symbol.object

;; 函数声明
(function_declaration
  name: (simple_identifier) @symbol.name
  parameters: (function_value_parameters) @symbol.parameters
  return_type: (type_reference)? @symbol.return_type
) @symbol.function

;; 属性声明
(property_declaration
  name: (simple_identifier) @symbol.name
  type: (type_reference)? @symbol.type
) @symbol.property

;; 枚举
(enum_class_body
  name: (type_identifier) @symbol.name
) @symbol.enum
