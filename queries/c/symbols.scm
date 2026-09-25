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

;; 枚举量（enum Color { RED, GREEN } 里的 RED/GREEN）
;; 此前完全未索引 —— 枚举类型建了符号、它身上的每个成员都没有，
;; 于是「加一个枚举项」这类改动的引用面查出来是 0。
;; @enumerator 挂在成员节点 enumerator 本身（不是容器 enumerator_list），
;; 位置即该成员的起点，覆盖度审计靠「符号起点 == 声明节点起点」判定，必须对齐。
(enumerator
  name: (identifier) @name) @enumerator

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
