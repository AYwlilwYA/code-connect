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

;; 枚举量（enum class 体里的各个 entry）
;; ⚠️ 未经本项目实测：tree-sitter-kotlin 不是本 workspace 的依赖
;; （crates/parser/Cargo.toml 里被注释、无 kotlin.rs、无 KotlinParser 注册），
;; 节点名是按本地缓存的 fwcd tree-sitter-kotlin 0.3.8 核对的：
;; enum_class_body 的直接子节点为 enum_entry，enum_entry 的 name 是直接子节点
;; simple_identifier（该版本没有任何具名字段）。启用 Kotlin 时**必须整体重核**本文件。
(enum_entry
  (simple_identifier) @symbol.name) @enumerator
