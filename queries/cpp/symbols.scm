;; C++ 符号查询 — 类、命名空间、函数/方法、枚举、别名、宏
;;
;; 注意：函数类的三条模式（function_definition / field_declaration / declaration）
;; 各自只匹配一种节点类型，互不重叠；同名同类符号（声明+定义）的去重在 cpp.rs 中处理。
;; 名称统一通过 declarator 字段交给 Rust 侧解析（见 cpp.rs 的 resolve_declarator_name），
;; 因为指针/引用返回类型会把 function_declarator 包在 pointer_declarator/reference_declarator 里。

;; ---- 类型 ----

;; 类（含模板特化 class Box<int>）
(class_specifier
  name: [
    (type_identifier) @name
    (template_type name: (type_identifier) @name)
  ]) @class

;; C++ 中 struct 也是类
(struct_specifier
  name: (type_identifier) @name) @struct

;; union
(union_specifier
  name: (type_identifier) @name) @union

;; 枚举（含 enum class / enum struct）
(enum_specifier
  name: (type_identifier) @name) @enum

;; ---- 命名空间 ----

;; namespace geometry / namespace outer::inner（匿名 namespace 无 name，跳过）
(namespace_definition
  name: (_) @name) @namespace

;; ---- 函数与方法 ----

;; 函数定义：自由函数、类内直接定义的方法、类外限定名定义（Shape::area）
(function_definition
  declarator: (_) @declarator) @func

;; 类内方法声明：含虚函数、纯虚函数、override
(field_declaration
  declarator: (_) @declarator) @method

;; 函数声明：类内构造函数/析构函数声明、自由函数原型（含 auto f() -> T）
(declaration
  declarator: (_) @declarator) @declaration

;; ---- 别名 ----

;; using Alias = T;（含模板别名）
(alias_declaration
  name: (type_identifier) @name) @type_definition

;; typedef T Alias;
(type_definition
  declarator: (type_identifier) @name) @type_definition

;; ---- 宏 ----

(preproc_def
  name: (identifier) @name
  value: (preproc_arg) @value) @macro

(preproc_function_def
  name: (identifier) @name
  parameters: (preproc_params) @params
  value: (preproc_arg) @value) @macro
