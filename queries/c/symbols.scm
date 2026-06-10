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

(preproc_function_def
  name: (identifier) @name
  parameters: (preproc_params) @params
  value: (preproc_arg) @value) @macro

(type_definition
  declarator: (type_identifier) @name) @type_definition

(type_definition
  declarator: (primitive_type) @name) @type_definition
