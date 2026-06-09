;; 类声明
(class_declaration
  (identifier) @symbol.name
) @symbol.class

;; 接口声明
(interface_declaration
  (identifier) @symbol.name
) @symbol.interface

;; 枚举声明
(enum_declaration
  (identifier) @symbol.name
) @symbol.enum

;; 方法声明
(method_declaration
  (identifier) @symbol.name
  (formal_parameters) @symbol.parameters
) @symbol.method

;; 构造函数
(constructor_declaration
  (identifier) @symbol.name
  (formal_parameters) @symbol.parameters
) @symbol.constructor

;; 字段声明
(field_declaration
  (variable_declarator
    (identifier) @symbol.name
  )
) @symbol.field

;; 注解声明
(annotation_type_declaration
  (identifier) @symbol.name
) @symbol.annotation
