;; 类声明
(class_declaration
  name: (identifier) @symbol.name
) @symbol.class

;; 接口声明
(interface_declaration
  name: (identifier) @symbol.name
) @symbol.interface

;; 结构体声明
(struct_declaration
  name: (identifier) @symbol.name
) @symbol.struct

;; 枚举声明
(enum_declaration
  name: (identifier) @symbol.name
) @symbol.enum

;; 方法声明
(method_declaration
  name: (identifier) @symbol.name
  parameters: (parameter_list) @symbol.parameters
) @symbol.method

;; 属性声明
(property_declaration
  name: (identifier) @symbol.name
) @symbol.property

;; 字段声明 — 匹配 field_declaration 内部的第一个 variable_declaration
(field_declaration
  (variable_declaration) @symbol.field
)

;; 命名空间
(namespace_declaration
  name: (qualified_name) @symbol.name
) @symbol.namespace
