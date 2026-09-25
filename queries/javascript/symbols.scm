;; 【枚举量：本语言显式不适用，非遗漏】
;; tree-sitter-javascript 0.23.1 的 node-types.json 里 "enum" 出现 0 次 ——
;; 标准 JavaScript 没有 enum 语法，grammar 里根本不存在对应节点。
;; 因此这里不能写 @enumerator 捕获：引用不存在的节点名会让 Query::new 报错，
;; 而解析器对查询编译失败是 `Err(_) => 返回空`，整套 JS 符号会**静默清零**。
;; 注：生产路径上 .js 目前由 TypeScriptParser 注册（见 cli/commands/index.rs），
;; 本文件对应的是未注册的 JavaScriptParser。

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
