//! C++ 语言解析器
//!
//! 使用 tree-sitter-cpp grammar 解析 .cpp/.hpp/.cc/.cxx 等文件，
//! 提取类、函数、方法、结构体、枚举、宏定义、typedef、命名空间等符号定义，
//! 以及函数调用和 #include 导入语句。
//!
//! C++ grammar 与 C grammar 共享大量节点类型，因此复用 queries/c/ 下的 query 文件。

use std::path::Path;
use std::sync::Mutex;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::symbol_id::StableSymbolId;
use codeconnect_core::types::{
    CallSite, CallType, Import, ImportResolution, SourceLocation, Symbol, SymbolKind,
};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::query_loader::load_cpp_queries;
use crate::r#trait::LanguageParser;

/// C++ 语言解析器
///
/// 包装了 tree-sitter-cpp grammar 和一个 `Mutex<Parser>`，
/// 确保并发调用时的线程安全。缓存 Language 对象用于 Query 编译。
pub struct CppParser {
    /// tree-sitter 解析器实例（Mutex 因为 Parser 不是 Sync）
    parser: Mutex<Parser>,
    /// C++ language 对象（用于 Query::new 编译查询）
    language: tree_sitter::Language,
}

impl CppParser {
    /// 创建新的 C++ 解析器
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language: tree_sitter::Language =
            tree_sitter::Language::new(tree_sitter_cpp::LANGUAGE);
        parser
            .set_language(&language)
            .expect("加载 C++ tree-sitter grammar 失败");
        Self {
            parser: Mutex::new(parser),
            language,
        }
    }

    /// 将 tree-sitter 节点转为源码位置（0-based → 1-based）
    fn node_to_location(&self, node: tree_sitter::Node, file_path: &str) -> SourceLocation {
        let start = node.start_position();
        let end = node.end_position();
        SourceLocation {
            file_path: file_path.to_string(),
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

impl Default for CppParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for CppParser {
    fn language(&self) -> &'static str {
        "cpp"
    }

    fn file_extensions(&self) -> &[&str] {
        &["cpp", "hpp", "cc", "cxx", "c++", "h++", "hh", "hxx"]
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
                message: "C++ 源码解析失败，可能包含语法错误".to_string(),
            })
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<Symbol> {
        let queries = load_cpp_queries();
        let file_path_str = file_path.to_string_lossy().to_string();
        let mut results = Vec::new();

        let query = match Query::new(&self.language, &queries.symbols) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        // 用于追踪已处理的类/结构体（避免重复）
        use std::collections::HashSet;
        let mut seen_ids: HashSet<String> = HashSet::new();

        while let Some(m) = matches.next() {
            let mut name = String::new();
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
                    "name" => {
                        name = self.node_text(node, source).to_string();
                    }
                    "func" => {
                        kind = SymbolKind::Function;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "struct" => {
                        kind = SymbolKind::Struct;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "union" => {
                        // SymbolKind 中没有 Union 变体，union 映射为 Struct
                        kind = SymbolKind::Struct;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "enum" => {
                        kind = SymbolKind::Enum;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "macro" => {
                        kind = SymbolKind::Macro;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "type_definition" => {
                        kind = SymbolKind::TypeAlias;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    _ => {}
                }
            }

            if name.is_empty() {
                continue;
            }

            let kind_str = match &kind {
                SymbolKind::Function => "function",
                SymbolKind::Struct => "struct",
                SymbolKind::Enum => "enum",
                SymbolKind::Macro => "macro",
                SymbolKind::TypeAlias => "type_alias",
                _ => "unknown",
            };

            let id = StableSymbolId::new("cpp", &file_path_str, kind_str, &name);

            // 去重：C++ 中 class/struct 可能被 query 重复捕获
            if !seen_ids.insert(id.to_string()) {
                continue;
            }

            // C++ 中函数、类、结构体、枚举默认都是全局可见的
            let is_exported = matches!(
                kind,
                SymbolKind::Function
                    | SymbolKind::Struct
                    | SymbolKind::Enum
                    | SymbolKind::TypeAlias
                    | SymbolKind::Macro
            );

            let modifiers = if is_exported {
                vec!["extern".to_string()]
            } else {
                vec!["static".to_string()]
            };

            results.push(Symbol {
                id: id.to_string(),
                name,
                kind,
                location,
                signature: None,
                doc_comment: None,
                parent_id: None,
                modifiers,
                is_exported,
                complexity: None,
            });
        }

        results
    }

    fn extract_calls(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<CallSite> {
        let queries = load_cpp_queries();
        let file_path_str = file_path.to_string_lossy().to_string();
        let mut results = Vec::new();

        let query = match Query::new(&self.language, &queries.calls) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        // 去重：(callee_name, 行号) -> (call_type, location)
        use std::collections::HashMap;
        let mut seen: HashMap<(String, u64), (CallType, SourceLocation)> = HashMap::new();

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
            let mut capture_kind = "";

            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                match capture_name {
                    "caller_name" => {
                        callee_name = self.node_text(node, source).to_string();
                        capture_kind = "call";
                    }
                    "method_name" => {
                        callee_name = self.node_text(node, source).to_string();
                        capture_kind = "method_call";
                    }
                    "call" => {
                        location = self.node_to_location(node, &file_path_str);
                        if capture_kind.is_empty() {
                            capture_kind = "call";
                        }
                    }
                    "method_call" => {
                        location = self.node_to_location(node, &file_path_str);
                        call_type = CallType::Virtual;
                        capture_kind = "method_call";
                    }
                    _ => {}
                }
            }

            if !callee_name.is_empty() && location.line > 0 {
                let key = (callee_name.clone(), location.line);
                // method_call 优先于 call（更具体的匹配）
                if let Some(_existing) = seen.get(&key) {
                    if capture_kind == "method_call" {
                        seen.insert(key, (call_type, location));
                    }
                } else {
                    seen.insert(key, (call_type, location));
                }
            }
        }

        for ((callee_name, _), (call_type, location)) in seen {
            results.push(CallSite {
                caller_id: String::new(),
                callee_name,
                location,
                call_type,
                confidence: 0.9,
            });
        }

        results
    }

    fn extract_imports(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<Import> {
        let queries = load_cpp_queries();
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
            let mut import_line: u64 = 0;

            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                match capture_name {
                    "path" => {
                        let raw = self.node_text(node, source);
                        import_path = raw
                            .trim_matches('"')
                            .trim_start_matches('<')
                            .trim_end_matches('>')
                            .to_string();
                    }
                    "include" => {
                        import_line = node.start_position().row as u64 + 1;
                    }
                    _ => {}
                }
            }

            if !import_path.is_empty() {
                if !results.iter().any(|i: &Import| i.import_path == import_path) {
                    results.push(Import {
                        file_path: file_path_str.clone(),
                        import_path,
                        alias: None,
                        line: import_line,
                        resolution: ImportResolution::Unresolved,
                    });
                }
            }
        }

        results
    }

    fn infer_package(&self, file_path: &Path, _source: &str) -> Option<String> {
        // C++ 项目：从文件路径推断模块/包名
        let mut current = file_path.parent();
        while let Some(dir) = current {
            if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                let lower = name.to_lowercase();
                if lower == "src" || lower == "include" || lower == "build" || lower == "test" {
                    current = dir.parent();
                    continue;
                }
                if !name.is_empty() && !name.starts_with('.') {
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

    fn parse_source(source: &str) -> (Tree, CppParser) {
        let parser = CppParser::new();
        let tree = parser.parse(source).expect("解析测试源码失败");
        (tree, parser)
    }

    #[test]
    fn test_parse_function() {
        let source = r#"
int main() {
    return 0;
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cpp"));
        assert!(!symbols.is_empty(), "应提取到至少一个符号");
        let main_fn = symbols.iter().find(|s| s.name == "main");
        assert!(main_fn.is_some(), "应找到 main 函数");
        assert_eq!(main_fn.unwrap().kind, SymbolKind::Function);
    }

    #[test]
    fn test_parse_struct() {
        let source = r#"
struct Point {
    int x;
    int y;
};
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cpp"));
        let point = symbols.iter().find(|s| s.name == "Point");
        assert!(point.is_some(), "应找到 Point 结构体");
        assert_eq!(point.unwrap().kind, SymbolKind::Struct);
    }

    #[test]
    fn test_parse_enum() {
        let source = r#"
enum Color { RED, GREEN, BLUE };
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cpp"));
        let color = symbols.iter().find(|s| s.name == "Color");
        assert!(color.is_some(), "应找到 Color 枚举");
        assert_eq!(color.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn test_parse_macro() {
        let source = r#"
#define MAX_SIZE 100
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cpp"));
        let max_size = symbols.iter().find(|s| s.name == "MAX_SIZE");
        assert!(max_size.is_some(), "应找到 MAX_SIZE 宏");
        assert_eq!(max_size.unwrap().kind, SymbolKind::Macro);
    }

    #[test]
    fn test_parse_function_calls() {
        let source = r#"
void foo() {
    bar();
    obj.method();
}
"#;
        let (tree, parser) = parse_source(source);
        let calls = parser.extract_calls(&tree, source, Path::new("test.cpp"));
        let bar_call = calls.iter().find(|c| c.callee_name == "bar");
        assert!(bar_call.is_some(), "应有 bar 函数调用");
        let method_call = calls.iter().find(|c| c.callee_name == "method");
        assert!(method_call.is_some(), "应有 method 方法调用");
    }

    #[test]
    fn test_parse_include_imports() {
        let source = r#"
#include <iostream>
#include "myheader.h"
"#;
        let (tree, parser) = parse_source(source);
        let imports = parser.extract_imports(&tree, source, Path::new("test.cpp"));
        assert!(!imports.is_empty(), "应有 include 语句");
        let iostream = imports.iter().find(|i| i.import_path.contains("iostream"));
        assert!(iostream.is_some(), "应有 iostream 导入");
    }

    #[test]
    fn test_language_name() {
        let parser = CppParser::new();
        assert_eq!(parser.language(), "cpp");
    }

    #[test]
    fn test_file_extensions() {
        let parser = CppParser::new();
        let exts = parser.file_extensions();
        assert!(exts.contains(&"cpp"));
        assert!(exts.contains(&"hpp"));
        assert!(exts.contains(&"cc"));
        assert!(exts.contains(&"cxx"));
    }

    #[test]
    fn test_parse_empty_file() {
        let source = "";
        let parser = CppParser::new();
        let tree = parser.parse(source).expect("解析空文件应返回空 Tree");
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cpp"));
        assert!(symbols.is_empty(), "空文件应无符号");
    }

    #[test]
    fn test_infer_package() {
        let parser = CppParser::new();
        let path = std::path::PathBuf::from("/home/user/projects/mylib/src/main.cpp");
        let pkg = parser.infer_package(&path, "");
        assert_eq!(pkg, Some("mylib".to_string()));
    }
}
