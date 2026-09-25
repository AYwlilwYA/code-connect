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

;; 枚举量（enum 体里的各个成员）
;; 此前只索引了枚举类型本身，成员一个都没有。
;; @enumerator 挂在成员节点 enum_member_declaration 本身（不是 enum_member_declaration_list）。
(enum_member_declaration
  name: (identifier) @symbol.name) @enumerator

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
