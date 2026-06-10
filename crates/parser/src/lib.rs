//! CodeConnect 多语言解析器模块
//!
//! 使用 tree-sitter 对 Rust、TypeScript/JavaScript、Java、Kotlin、C# 进行 AST 解析，
//! 提取符号定义、调用关系、导入关系。
//!
//! 核心 trait:
//! - [`LanguageParser`] — 语言解析器统一接口
//! - [`ImportResolver`] — 跨文件导入解析策略

pub mod factory;
pub mod import_resolver;
pub mod query_loader;
pub mod r#trait;

// 语言解析器
pub mod c;
pub mod cpp;
pub mod csharp;
pub mod java;
pub mod javascript;
pub mod rust;
pub mod typescript;

// pub mod kotlin;  // 待 tree-sitter-kotlin grammar 就绪后启用
