//! tree-sitter Query 加载器
//!
//! 使用 `include_str!` 在编译期嵌入 queries/ 目录下的 .scm 查询文件，
//! 避免运行时文件系统依赖，使二进制文件自包含。

/// 预编译的查询集合
pub struct LanguageQueries {
    /// 符号提取查询内容
    pub symbols: &'static str,
    /// 调用点提取查询内容
    pub calls: &'static str,
    /// 导入提取查询内容
    pub imports: &'static str,
}

impl LanguageQueries {
    pub fn new(symbols: &'static str, calls: &'static str, imports: &'static str) -> Self {
        Self {
            symbols,
            calls,
            imports,
        }
    }
}

/// 为 Rust 加载编译期查询文件
pub fn load_rust_queries() -> LanguageQueries {
    LanguageQueries::new(
        include_str!("../../../queries/rust/symbols.scm"),
        include_str!("../../../queries/rust/calls.scm"),
        include_str!("../../../queries/rust/imports.scm"),
    )
}

/// 为 TypeScript 加载编译期查询文件
pub fn load_typescript_queries() -> LanguageQueries {
    LanguageQueries::new(
        include_str!("../../../queries/typescript/symbols.scm"),
        include_str!("../../../queries/typescript/calls.scm"),
        include_str!("../../../queries/typescript/imports.scm"),
    )
}

/// 为 JavaScript 加载编译期查询文件
pub fn load_javascript_queries() -> LanguageQueries {
    LanguageQueries::new(
        include_str!("../../../queries/javascript/symbols.scm"),
        include_str!("../../../queries/javascript/calls.scm"),
        include_str!("../../../queries/javascript/imports.scm"),
    )
}

/// 为 Java 加载编译期查询文件
pub fn load_java_queries() -> LanguageQueries {
    LanguageQueries::new(
        include_str!("../../../queries/java/symbols.scm"),
        include_str!("../../../queries/java/calls.scm"),
        include_str!("../../../queries/java/imports.scm"),
    )
}

/// 为 C# 加载编译期查询文件
pub fn load_csharp_queries() -> LanguageQueries {
    LanguageQueries::new(
        include_str!("../../../queries/csharp/symbols.scm"),
        include_str!("../../../queries/csharp/calls.scm"),
        include_str!("../../../queries/csharp/imports.scm"),
    )
}

/// 为 C 加载编译期查询文件
pub fn load_c_queries() -> LanguageQueries {
    LanguageQueries::new(
        include_str!("../../../queries/c/symbols.scm"),
        include_str!("../../../queries/c/calls.scm"),
        include_str!("../../../queries/c/imports.scm"),
    )
}

/// 为 C++ 加载编译期查询文件
pub fn load_cpp_queries() -> LanguageQueries {
    LanguageQueries::new(
        include_str!("../../../queries/cpp/symbols.scm"),
        include_str!("../../../queries/cpp/calls.scm"),
        include_str!("../../../queries/cpp/imports.scm"),
    )
}

/// 为 Kotlin 加载编译期查询文件
pub fn load_kotlin_queries() -> LanguageQueries {
    LanguageQueries::new(
        include_str!("../../../queries/kotlin/symbols.scm"),
        include_str!("../../../queries/kotlin/calls.scm"),
        include_str!("../../../queries/kotlin/imports.scm"),
    )
}
