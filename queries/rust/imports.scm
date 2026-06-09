;; use 声明
(use_declaration
  argument: (scoped_identifier) @import.path
) @import

;; use 别名
(use_declaration
  argument: (scoped_use_list
    path: (scoped_identifier)? @import.path
    list: (use_list
      (identifier) @import.name
    )
  )
)

;; extern crate
(extern_crate_declaration
  name: (identifier) @import.name
) @import.extern
