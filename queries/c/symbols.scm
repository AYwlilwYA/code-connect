;; C 符号查询 — 提取函数、结构体、枚举、宏定义

(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name
    parameters: (parameter_list) @params)
  body: (compound_statement) @body) @func

(struct_specifier
  name: (type_identifier) @name
  body: (field_declaration_list) @body) @struct

(union_specifier
  name: (type_identifier) @name
  body: (field_declaration_list) @body) @union

(enum_specifier
  name: (type_identifier) @name
  body: (enumerator_list) @body) @enum

(preproc_def
  name: (identifier) @name
  value: (preproc_arg) @value) @macro

(typedef
  declarator: (type_identifier) @name) @typedef
