;; 函数定义
(function_item
  name: (identifier) @symbol.name
  parameters: (parameters) @symbol.parameters
  return_type: (type_identifier)? @symbol.return_type
) @symbol.function

;; 结构体定义
(struct_item
  name: (type_identifier) @symbol.name
) @symbol.struct

;; trait 定义
(trait_item
  name: (type_identifier) @symbol.name
) @symbol.trait

;; 枚举定义
(enum_item
  name: (type_identifier) @symbol.name
) @symbol.enum

;; impl 块（方法定义）
(impl_item
  type: (type_identifier) @symbol.parent
  body: (declaration_list
    (function_item
      name: (identifier) @symbol.name
      parameters: (parameters) @symbol.parameters
      return_type: (type_identifier)? @symbol.return_type
    ) @symbol.method))

;; 类型别名
(type_item
  name: (type_identifier) @symbol.name
) @symbol.type_alias

;; 宏定义
(macro_definition
  name: (identifier) @symbol.name
) @symbol.macro

;; 模块声明
(mod_item
  name: (identifier) @symbol.name
) @symbol.module

;; 常量/静态变量
(let_declaration
  pattern: (identifier) @symbol.name
  type: (type_identifier)? @symbol.type
) @symbol.variable

;; 字段声明
(field_declaration
  name: (field_identifier) @symbol.name
  type: (type_identifier) @symbol.type
) @symbol.field
