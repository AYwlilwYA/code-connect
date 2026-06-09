;; import 声明 — import_declaration 没有命名字段，直接用子节点
(import_declaration
  (scoped_identifier) @import.path
) @import

;; package 声明 — package_declaration 也没有命名字段
(package_declaration
  (scoped_identifier) @import.package
)
