//! C 语言解析器
//!
//! 使用 tree-sitter-c grammar 解析 .c/.h 文件，
//! 提取函数、结构体、联合体、枚举、宏定义、typedef 等符号定义，
//! 以及函数调用和 #include 导入语句。

use std::path::Path;
use std::sync::Mutex;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::symbol_id::StableSymbolId;
use codeconnect_core::types::{
    CallSite, CallType, Import, ImportResolution, SourceLocation, Symbol, SymbolKind,
};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::query_loader::load_c_queries;
use crate::r#trait::LanguageParser;

/// C 语言解析器
///
/// 包装了 tree-sitter-c grammar 和一个 `Mutex<Parser>`，
/// 确保并发调用时的线程安全。缓存 Language 对象用于 Query 编译。
pub struct CParser {
    /// tree-sitter 解析器实例（Mutex 因为 Parser 不是 Sync）
    parser: Mutex<Parser>,
    /// C language 对象（用于 Query::new 编译查询）
    language: tree_sitter::Language,
}

impl CParser {
    /// 创建新的 C 解析器
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter::Language::new(tree_sitter_c::LANGUAGE);
        parser
            .set_language(&language)
            .expect("加载 C tree-sitter grammar 失败");
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

impl Default for CParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for CParser {
    fn language(&self) -> &'static str {
        "c"
    }

    fn file_extensions(&self) -> &[&str] {
        &["c", "h"]
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
                message: "C 源码解析失败，可能包含语法错误".to_string(),
            })
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<Symbol> {
        let queries = load_c_queries();
        let file_path_str = file_path.to_string_lossy().to_string();
        let mut results = Vec::new();

        let query = match Query::new(&self.language, &queries.symbols) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

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
                    "typedef" => {
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

            let id = StableSymbolId::new("c", &file_path_str, kind_str, &name);

            // C 语言中函数、结构体、枚举默认都是全局可见的（头文件中声明的）
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
        let queries = load_c_queries();
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
                    "caller_name" => {
                        callee_name = self.node_text(node, source).to_string();
                    }
                    "method_name" => {
                        // 方法调用（通过结构体指针调用）
                        callee_name = self.node_text(node, source).to_string();
                    }
                    "call" => {
                        location = self.node_to_location(node, &file_path_str);
                        call_type = CallType::Direct;
                    }
                    "method_call" => {
                        location = self.node_to_location(node, &file_path_str);
                        call_type = CallType::Virtual;
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
        let queries = load_c_queries();
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
                        // #include 中的路径，去掉引号保留原始字符串
                        let raw = self.node_text(node, source);
                        import_path = raw.trim_matches('"').to_string();
                    }
                    "include" => {
                        import_line = node.start_position().row as u64 + 1;
                    }
                    _ => {}
                }
            }

            if !import_path.is_empty() {
                // 避免重复
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
        // C 项目：从文件路径推断模块/包名
        // 取文件所在目录名（相对项目根），如果没有则取父目录名
        let mut current = file_path.parent();
        while let Some(dir) = current {
            if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                let lower = name.to_lowercase();
                // 跳过常见的构建/平台目录
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

    /// 辅助函数：创建解析器并返回解析结果
    fn parse_source(source: &str) -> (Tree, CParser) {
        let parser = CParser::new();
        let tree = parser.parse(source).expect("解析测试源码失败");
        (tree, parser)
    }

    #[test]
    fn test_parse_simple_function() {
        let source = r#"
int main(void) {
    return 0;
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.c"));
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
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.c"));
        let point_struct = symbols.iter().find(|s| s.name == "Point");
        assert!(point_struct.is_some(), "应找到 Point 结构体");
        assert_eq!(point_struct.unwrap().kind, SymbolKind::Struct);
    }

    #[test]
    fn test_parse_enum() {
        let source = r#"
enum Color {
    RED,
    GREEN,
    BLUE
};
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.c"));
        let color_enum = symbols.iter().find(|s| s.name == "Color");
        assert!(color_enum.is_some(), "应找到 Color 枚举");
        assert_eq!(color_enum.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn test_parse_union() {
        let source = r#"
union Data {
    int i;
    float f;
    char str[20];
};
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.c"));
        let data_union = symbols.iter().find(|s| s.name == "Data");
        assert!(data_union.is_some(), "应找到 Data 联合体");
        // union 映射为 Struct
        assert_eq!(data_union.unwrap().kind, SymbolKind::Struct);
    }

    #[test]
    fn test_parse_macro() {
        let source = r#"
#define MAX_SIZE 100
#define SQUARE(x) ((x) * (x))
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.c"));
        let max_size = symbols.iter().find(|s| s.name == "MAX_SIZE");
        assert!(max_size.is_some(), "应找到 MAX_SIZE 宏");
        assert_eq!(max_size.unwrap().kind, SymbolKind::Macro);
        let square = symbols.iter().find(|s| s.name == "SQUARE");
        assert!(square.is_some(), "应找到 SQUARE 宏");
    }

    #[test]
    fn test_parse_typedef() {
        let source = r#"
typedef unsigned long size_t;
typedef struct { int x; int y; } Point;
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.c"));
        let size_t_sym = symbols.iter().find(|s| s.name == "size_t");
        assert!(size_t_sym.is_some(), "应找到 size_t typedef");
        assert_eq!(size_t_sym.unwrap().kind, SymbolKind::TypeAlias);
    }

    #[test]
    fn test_parse_function_calls() {
        let source = r#"
void foo() {
    bar();
    baz(1, 2);
}
"#;
        let (tree, parser) = parse_source(source);
        let calls = parser.extract_calls(&tree, source, Path::new("test.c"));
        let bar_call = calls.iter().find(|c| c.callee_name == "bar");
        assert!(bar_call.is_some(), "应有 bar 函数调用");
        let baz_call = calls.iter().find(|c| c.callee_name == "baz");
        assert!(baz_call.is_some(), "应有 baz 函数调用");
    }

    #[test]
    fn test_parse_include_imports() {
        let source = r#"
#include <stdio.h>
#include "myheader.h"
"#;
        let (tree, parser) = parse_source(source);
        let imports = parser.extract_imports(&tree, source, Path::new("test.c"));
        assert!(!imports.is_empty(), "应有 include 语句");
        let stdio = imports.iter().find(|i| i.import_path.contains("stdio.h"));
        assert!(stdio.is_some(), "应有 stdio.h 导入");
        let myheader = imports.iter().find(|i| i.import_path.contains("myheader.h"));
        assert!(myheader.is_some(), "应有 myheader.h 导入");
    }

    #[test]
    fn test_language_name() {
        let parser = CParser::new();
        assert_eq!(parser.language(), "c");
    }

    #[test]
    fn test_file_extensions() {
        let parser = CParser::new();
        assert_eq!(parser.file_extensions(), &["c", "h"]);
    }

    #[test]
    fn test_parse_empty_file() {
        let source = "";
        let parser = CParser::new();
        let tree = parser.parse(source).expect("解析空文件应返回空 Tree");
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.c"));
        assert!(symbols.is_empty(), "空文件应无符号");
    }

    #[test]
    fn test_infer_package() {
        let parser = CParser::new();
        let path = std::path::PathBuf::from("/home/user/projects/mylib/src/main.c");
        let pkg = parser.infer_package(&path, "");
        assert_eq!(pkg, Some("mylib".to_string()));
    }

    #[test]
    fn test_infer_package_include() {
        let parser = CParser::new();
        let path = std::path::PathBuf::from("/home/user/projects/mylib/include/mylib/utils.h");
        let pkg = parser.infer_package(&path, "");
        assert_eq!(pkg, Some("mylib".to_string()));
    }
}
