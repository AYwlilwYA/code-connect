;; using System — 单个标识符作为命名空间
(using_directive
  (identifier) @import.path
) @import

;; using System.Collections.Generic — 限定名
(using_directive
  (qualified_name) @import.path
) @import
