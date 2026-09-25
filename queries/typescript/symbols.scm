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

;; 枚举量
;; tree-sitter-typescript 的 enum_body 上没有独立成员节点：
;; 无初始值的成员直接以 name: (property_identifier) 挂在 enum_body 上，
;; 带初始值的成员则是 enum_assignment，两条都要，否则 enum { A = 1, B } 会漏掉 A。
;; 与其它语言不同，这里 @enumerator 只能与 @symbol.name 一起挂在 property_identifier 上 ——
;; TS 没有独立成员节点，标识符本身就是「成员节点」；容器 enum_body 从 `{` 起，
;; 挂上去 3 个成员会全部落在 `{` 的同一个位置（实测过，行号全错）。
;; 覆盖度审计对这类「无独立成员节点」的语法走容器包含兜底（见 coverage.rs）。
(enum_body
  name: (property_identifier) @symbol.name @enumerator)

(enum_assignment
  name: (property_identifier) @symbol.name @enumerator)

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

;; export default function（React 组件最常见的声明方式）
;; 注：export default 在 tree-sitter-typescript 中 function_declaration 仍位于 declaration 字段下，
;; 已有的 export_statement + declaration 模式已经覆盖。这里添加带完整参数签名的版本，
;; 确保 export default function 的 symbol 能正确标记为 exported 并携带完整参数信息。
(export_statement
  declaration: (function_declaration
    name: (identifier) @symbol.name
    parameters: (formal_parameters) @symbol.parameters
    return_type: (type_annotation)? @symbol.return_type
  )
) @symbol.exported

;; export default class（同样 declaration 字段覆盖）
(export_statement
  declaration: (class_declaration
    name: (type_identifier) @symbol.name
  )
) @symbol.exported
