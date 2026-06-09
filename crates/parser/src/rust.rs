//! Rust 语言解析器
//!
//! 使用 tree-sitter-rust grammar 解析 .rs 文件，
//! 提取函数、结构体、trait、枚举、方法等符号定义，
//! 以及函数调用和 use 导入语句。

use std::path::Path;
use std::sync::Mutex;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::symbol_id::StableSymbolId;
use codeconnect_core::types::{
    CallSite, CallType, Import, ImportResolution, SourceLocation, Symbol, SymbolKind,
};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::query_loader::load_rust_queries;
use crate::r#trait::LanguageParser;

/// Rust 语言解析器
///
/// 包装了 tree-sitter-rust grammar 和一个 `Mutex<Parser>`，
/// 确保并发调用时的线程安全。缓存 Language 对象用于 Query 编译。
pub struct RustParser {
    /// tree-sitter 解析器实例（Mutex 因为 Parser 不是 Sync）
    parser: Mutex<Parser>,
    /// Rust language 对象（用于 Query::new 编译查询）
    language: tree_sitter::Language,
}

impl RustParser {
    /// 创建新的 Rust 解析器
    pub fn new() -> Self {
        let mut parser = Parser::new();
        // tree-sitter-rust 0.23: LANGUAGE 是 LanguageFn 常量，通过 .into() 转为 Language
        let language: tree_sitter::Language = tree_sitter::Language::new(tree_sitter_rust::LANGUAGE);
        parser
            .set_language(&language)
            .expect("加载 Rust tree-sitter grammar 失败");
        Self {
            parser: Mutex::new(parser),
            language,
        }
    }

    /// 将 tree-sitter 节点转为源码位置（0-based → 1-based）
    fn node_to_location(&self, node: tree_sitter::Node) -> SourceLocation {
        let start = node.start_position();
        let end = node.end_position();
        SourceLocation {
            file_path: String::new(), // 由外部调用者填充
            line: start.row as u64 + 1,
            column: start.column as u64 + 1,
            end_line: end.row as u64 + 1,
            end_column: end.column as u64 + 1,
        }
    }

    /// 获取节点的源码文本（UTF-8 安全）
    fn node_text<'a>(&self, node: tree_sitter::Node, source: &'a str) -> &'a str {
        node.utf8_text(source.as_bytes()).unwrap_or("")
    }
}

impl Default for RustParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for RustParser {
    fn language(&self) -> &'static str {
        "rust"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn parse(&self, source: &str) -> Result<Tree, CodeConnectError> {
        let mut parser = self.parser.lock().map_err(|e| CodeConnectError::Parse {
            file: Path::new("").to_path_buf(),
            message: format!("获取解析器锁失败: {}", e),
        })?;
        parser
            .parse(source, None)
            .ok_or_else(|| CodeConnectError::Parse {
                file: Path::new("").to_path_buf(),
                message: "Rust 源码解析失败，可能包含语法错误".to_string(),
            })
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<Symbol> {
        let queries = load_rust_queries();
        let file_path_str = file_path.to_string_lossy().to_string();
        let mut results = Vec::new();

        let query = match Query::new(&self.language, &queries.symbols) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        // tree-sitter 0.24: matches() 返回 StreamingIterator，不是标准 Iterator
        // 需要导入 streaming_iterator::StreamingIterator 使用 next()/advance()+get()
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        while let Some(m) = matches.next() {
            let mut name = String::new();
            let mut parent = String::new();
            let mut kind = SymbolKind::Unknown("unknown".to_string());
            let mut location = SourceLocation {
                file_path: file_path_str.clone(),
                line: 0,
                column: 0,
                end_line: 0,
                end_column: 0,
            };

            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                match capture_name {
                    "symbol.name" => {
                        name = self.node_text(node, source).to_string();
                    }
                    "symbol.function" => {
                        kind = SymbolKind::Function;
                        location = self.node_to_location(node);
                    }
                    "symbol.method" => {
                        kind = SymbolKind::Method;
                        location = self.node_to_location(node);
                    }
                    "symbol.struct" => {
                        kind = SymbolKind::Struct;
                        location = self.node_to_location(node);
                    }
                    "symbol.trait" => {
                        kind = SymbolKind::Trait;
                        location = self.node_to_location(node);
                    }
                    "symbol.enum" => {
                        kind = SymbolKind::Enum;
                        location = self.node_to_location(node);
                    }
                    "symbol.type_alias" => {
                        kind = SymbolKind::TypeAlias;
                        location = self.node_to_location(node);
                    }
                    "symbol.macro" => {
                        kind = SymbolKind::Macro;
                        location = self.node_to_location(node);
                    }
                    "symbol.module" => {
                        kind = SymbolKind::Module;
                        location = self.node_to_location(node);
                    }
                    "symbol.variable" => {
                        kind = SymbolKind::Variable;
                        location = self.node_to_location(node);
                    }
                    "symbol.field" => {
                        kind = SymbolKind::Field;
                        location = self.node_to_location(node);
                    }
                    "symbol.parent" => {
                        parent = self.node_text(node, source).to_string();
                    }
                    _ => {}
                }
            }

            if name.is_empty() {
                continue;
            }

            let kind_str = match &kind {
                SymbolKind::Function => "function",
                SymbolKind::Method => "method",
                SymbolKind::Struct => "struct",
                SymbolKind::Trait => "trait",
                SymbolKind::Enum => "enum",
                SymbolKind::TypeAlias => "type_alias",
                SymbolKind::Macro => "macro",
                SymbolKind::Module => "module",
                SymbolKind::Variable => "variable",
                SymbolKind::Field => "field",
                _ => "unknown",
            };

            let id = StableSymbolId::new("rust", &file_path_str, kind_str, &name);

            // Rust 顶层符号默认是私有的，以大写字母开头的符号大概率是公开 API
            let is_exported = name.chars().next().map_or(false, |c| c.is_uppercase())
                || matches!(
                    kind,
                    SymbolKind::Function
                        | SymbolKind::Struct
                        | SymbolKind::Trait
                        | SymbolKind::Enum
                );

            let parent_id = if parent.is_empty() {
                None
            } else {
                Some(StableSymbolId::new("rust", &file_path_str, "struct", &parent).to_string())
            };

            let modifiers = if is_exported {
                vec!["pub".to_string()]
            } else {
                vec!["private".to_string()]
            };

            results.push(Symbol {
                id: id.to_string(),
                name,
                kind,
                location,
                signature: None,
                doc_comment: None,
                parent_id,
                modifiers,
                is_exported,
                complexity: None,
            });
        }

        results
    }

    fn extract_calls(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<CallSite> {
        let queries = load_rust_queries();
        let file_path_str = file_path.to_string_lossy().to_string();
        let mut results = Vec::new();

        let query = match Query::new(&self.language, &queries.calls) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        while let Some(m) = matches.next() {
            let mut callee_name = String::new();
            let mut call_type = CallType::Direct;
            let mut location = SourceLocation {
                file_path: file_path_str.clone(),
                line: 0,
                column: 0,
                end_line: 0,
                end_column: 0,
            };

            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                match capture_name {
                    "call.name" => {
                        callee_name = self.node_text(node, source).to_string();
                    }
                    "call" => {
                        location = self.node_to_location(node);
                    }
                    "call.method" => {
                        call_type = CallType::Virtual; // Rust 方法调用可能通过 trait 动态分发
                        location = self.node_to_location(node);
                    }
                    "call.macro" => {
                        call_type = CallType::MacroExpansion;
                        location = self.node_to_location(node);
                    }
                    _ => {}
                }
            }

            if !callee_name.is_empty() && location.line > 0 {
                results.push(CallSite {
                    caller_id: String::new(), // 由外部调用者上下文补充
                    callee_name,
                    location,
                    call_type,
                    confidence: 0.9,
                });
            }
        }

        results
    }

    fn extract_imports(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<Import> {
        let queries = load_rust_queries();
        let file_path_str = file_path.to_string_lossy().to_string();
        let mut results = Vec::new();

        let query = match Query::new(&self.language, &queries.imports) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        while let Some(m) = matches.next() {
            let mut import_path = String::new();
            let mut alias = None;
            let mut import_line: u64 = 0;

            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                match capture_name {
                    "import.path" => {
                        import_path = self.node_text(node, source).to_string();
                    }
                    "import.name" => {
                        alias = Some(self.node_text(node, source).to_string());
                    }
                    "import" | "import.extern" => {
                        import_line = node.start_position().row as u64 + 1;
                    }
                    _ => {}
                }
            }

            // 对于 extern crate（仅 alias 无 import_path），使用 alias 作为路径
            let path = if import_path.is_empty() {
                alias.clone().unwrap_or_default()
            } else {
                import_path
            };

            if !path.is_empty() {
                results.push(Import {
                    file_path: file_path_str.clone(),
                    import_path: path,
                    alias,
                    line: import_line,
                    resolution: ImportResolution::Unresolved,
                });
            }
        }

        results
    }

    fn infer_package(&self, file_path: &Path, _source: &str) -> Option<String> {
        // Rust 项目：从文件路径的目录结构中推断 crate 名
        // 逐级向上查找父目录，跳过 "src" 目录，取第一个非空目录名
        let mut current = file_path.parent();
        while let Some(dir) = current {
            if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                if name == "src" {
                    // 跳过 src 目录，继续向上查找
                    current = dir.parent();
                    continue;
                }
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
            current = dir.parent();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 辅助函数：创建解析器并返回解析结果
    fn parse_source(source: &str) -> (Tree, RustParser) {
        let parser = RustParser::new();
        let tree = parser.parse(source).expect("解析测试源码失败");
        (tree, parser)
    }

    #[test]
    fn test_parse_simple_function() {
        let source = r#"
fn main() {
    println!("Hello, world!");
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.rs"));
        assert!(!symbols.is_empty(), "应提取到至少一个符号");
        let main_fn = symbols.iter().find(|s| s.name == "main");
        assert!(main_fn.is_some(), "应找到 main 函数");
        assert_eq!(main_fn.unwrap().kind, SymbolKind::Function);
    }

    #[test]
    fn test_parse_struct() {
        let source = r#"
pub struct Point {
    pub x: f64,
    pub y: f64,
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.rs"));
        let point_struct = symbols.iter().find(|s| s.name == "Point");
        assert!(point_struct.is_some(), "应找到 Point 结构体");
        assert_eq!(point_struct.unwrap().kind, SymbolKind::Struct);
    }

    #[test]
    fn test_parse_trait() {
        let source = r#"
pub trait Display {
    fn fmt(&self) -> String;
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.rs"));
        let display_trait = symbols.iter().find(|s| s.name == "Display");
        assert!(display_trait.is_some(), "应找到 Display trait");
        assert_eq!(display_trait.unwrap().kind, SymbolKind::Trait);
    }

    #[test]
    fn test_parse_enum() {
        let source = r#"
pub enum Color {
    Red,
    Green,
    Blue,
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.rs"));
        let color_enum = symbols.iter().find(|s| s.name == "Color");
        assert!(color_enum.is_some(), "应找到 Color 枚举");
        assert_eq!(color_enum.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn test_parse_impl_method() {
        let source = r#"
impl Point {
    fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.rs"));
        let new_method = symbols.iter().find(|s| s.name == "new");
        assert!(new_method.is_some(), "应找到 new 方法");
        assert_eq!(new_method.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn test_parse_function_calls() {
        let source = r#"
fn main() {
    foo();
    bar.baz();
}
"#;
        let (tree, parser) = parse_source(source);
        let calls = parser.extract_calls(&tree, source, Path::new("test.rs"));
        let foo_call = calls.iter().find(|c| c.callee_name == "foo");
        let baz_call = calls.iter().find(|c| c.callee_name == "baz");
        assert!(foo_call.is_some(), "应有 foo 函数调用");
        assert!(baz_call.is_some(), "应有 baz 方法调用");
        assert_eq!(baz_call.unwrap().call_type, CallType::Virtual);
    }

    #[test]
    fn test_parse_use_imports() {
        let source = r#"
use std::collections::HashMap;
use std::io::{self, Write};
"#;
        let (tree, parser) = parse_source(source);
        let imports = parser.extract_imports(&tree, source, Path::new("test.rs"));
        assert!(!imports.is_empty(), "应有导入语句");
        let hashmap_import = imports.iter().find(|i| i.import_path.contains("HashMap"));
        assert!(hashmap_import.is_some(), "应有 HashMap 导入");
    }

    #[test]
    fn test_parse_macro() {
        let source = r#"
macro_rules! my_vec {
    ($($x:expr),*) => {
        vec![$($x),*]
    };
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.rs"));
        let my_macro = symbols.iter().find(|s| s.name == "my_vec");
        assert!(my_macro.is_some(), "应找到 my_vec 宏");
        assert_eq!(my_macro.unwrap().kind, SymbolKind::Macro);
    }

    #[test]
    fn test_stable_symbol_id_format() {
        let id = StableSymbolId::new("rust", "src/main.rs", "function", "main");
        let serialized = id.to_string();
        // 格式: language::relative_path::kind::name::fingerprint
        let parts: Vec<&str> = serialized.split("::").collect();
        assert_eq!(parts.len(), 5, "符号 ID 应有 5 个字段");
        assert_eq!(parts[0], "rust");
        assert_eq!(parts[1], "src/main.rs");
        assert_eq!(parts[2], "function");
        assert_eq!(parts[3], "main");
        assert_eq!(parts[4].len(), 8, "指纹应为 8 字符十六进制");
    }

    #[test]
    fn test_infer_package() {
        let parser = RustParser::new();
        let path = PathBuf::from("/home/user/projects/my_crate/src/main.rs");
        let pkg = parser.infer_package(&path, "");
        assert_eq!(pkg, Some("my_crate".to_string()));
    }

    #[test]
    fn test_infer_package_not_src() {
        let parser = RustParser::new();
        // 非 src 目录的情况
        let path = PathBuf::from("/home/user/projects/my_lib/lib.rs");
        let pkg = parser.infer_package(&path, "");
        assert_eq!(pkg, Some("my_lib".to_string()));
    }

    #[test]
    fn test_language_name() {
        let parser = RustParser::new();
        assert_eq!(parser.language(), "rust");
    }

    #[test]
    fn test_file_extensions() {
        let parser = RustParser::new();
        assert_eq!(parser.file_extensions(), &["rs"]);
    }

    #[test]
    fn test_parse_empty_file() {
        let source = "";
        let parser = RustParser::new();
        let tree = parser.parse(source).expect("解析空文件应返回空 Tree");
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.rs"));
        assert!(symbols.is_empty(), "空文件应无符号");
    }
}
