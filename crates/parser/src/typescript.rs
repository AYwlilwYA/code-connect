//! TypeScript 语言解析器
//!
//! 使用 tree-sitter-typescript grammar 解析 .ts/.tsx 文件，
//! 提取函数、类、接口、方法等符号定义，
//! 以及函数调用和 import 导入语句。

use std::path::Path;
use std::sync::Mutex;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::symbol_id::StableSymbolId;
use codeconnect_core::types::{
    CallSite, CallType, Import, ImportResolution, SourceLocation, Symbol, SymbolKind,
};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::query_loader::load_typescript_queries;
use crate::r#trait::LanguageParser;

/// TypeScript 语言解析器
///
/// 包装了 tree-sitter-typescript grammar 和一个 `Mutex<Parser>`，
/// 确保并发调用时的线程安全。
pub struct TypeScriptParser {
    /// tree-sitter 解析器实例（Mutex 因为 Parser 不是 Sync）
    parser: Mutex<Parser>,
    /// TS language 对象（用于 Query::new 编译查询）
    language: tree_sitter::Language,
}

impl TypeScriptParser {
    /// 创建新的 TypeScript 解析器
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter::Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT);
        parser
            .set_language(&language)
            .expect("加载 TypeScript tree-sitter grammar 失败");
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

impl Default for TypeScriptParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for TypeScriptParser {
    fn language(&self) -> &'static str {
        "typescript"
    }

    fn file_extensions(&self) -> &[&str] {
        &["ts", "tsx"]
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
                message: "TypeScript 源码解析失败，可能包含语法错误".to_string(),
            })
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<Symbol> {
        let queries = load_typescript_queries();
        let file_path_str = file_path.to_string_lossy().to_string();
        let mut results = Vec::new();
        // 用 HashSet 去重——同一个符号可能被多个 query pattern 匹配
        let mut seen_names: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        // 先用查询收集所有被 export 声明的名称
        let mut exported_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for export_pattern in &[
            r#"(export_statement (function_declaration name: (identifier) @export.name))"#,
            r#"(export_statement (class_declaration name: (type_identifier) @export.name))"#,
        ] {
            if let Ok(export_query) = tree_sitter::Query::new(&self.language, export_pattern) {
                let mut ec = QueryCursor::new();
                let mut em = ec.matches(&export_query, tree.root_node(), source.as_bytes());
                while let Some(m) = em.next() {
                    for cap in m.captures {
                        if export_query.capture_names()[cap.index as usize] == "export.name" {
                            exported_names.insert(self.node_text(cap.node, source).to_string());
                        }
                    }
                }
            }
        }

        let query = match Query::new(&self.language, &queries.symbols) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        while let Some(m) = matches.next() {
            let mut name = String::new();
            let mut kind = SymbolKind::Unknown("unknown".to_string());
            let mut location = self.node_to_location(tree.root_node(), &file_path_str);
            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                match capture_name {
                    "symbol.name" => {
                        name = self.node_text(node, source).to_string();
                    }
                    "symbol.function" => {
                        kind = SymbolKind::Function;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "symbol.method" => {
                        kind = SymbolKind::Method;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "symbol.class" => {
                        kind = SymbolKind::Class;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "symbol.interface" => {
                        kind = SymbolKind::Interface;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "symbol.enum" => {
                        kind = SymbolKind::Enum;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "symbol.type_alias" => {
                        kind = SymbolKind::TypeAlias;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "symbol.variable" => {
                        kind = SymbolKind::Variable;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    _ => {}
                }
            }

            if name.is_empty() {
                continue;
            }

            // 去重：同一文件中同名符号优先保留更具体的类型
            // 函数优先于变量（箭头函数同时匹配 variable_declarator + arrow_function）
            let new_kind_str = kind_to_str(&kind);
            let dedup_key = (name.clone(), new_kind_str.to_string());
            if seen_names.contains(&dedup_key) {
                continue;
            }
            // 检查是否存在同名的低优先级类型，如果存在则替换
            let kind_priority = kind_priority_val(&kind);
            let to_remove: Option<(String, String)> = seen_names
                .iter()
                .find(|(n, k)| *n == name && kind_priority_val_for_str(k) < kind_priority)
                .cloned();
            if let Some(ref key) = to_remove {
                seen_names.remove(key);
                results.retain(|s: &Symbol| !(s.name == name && kind_to_str(&s.kind) == key.1));
            }
            seen_names.insert(dedup_key);

            let id = StableSymbolId::new("typescript", &file_path_str, new_kind_str, &name);

            // 检查是否在导出列表中（通过 export_query 预收集）
            let is_exported = exported_names.contains(&name);

            // TS 中 export 关键字标记的符号为公开 API
            let modifiers = if is_exported {
                vec!["export".to_string()]
            } else {
                vec![]
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
        let queries = load_typescript_queries();
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
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "call.method" => {
                        call_type = CallType::Virtual;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    "call.constructor" => {
                        call_type = CallType::Direct;
                        location = self.node_to_location(node, &file_path_str);
                    }
                    _ => {}
                }
            }

            if !callee_name.is_empty() && location.line > 0 {
                results.push(CallSite {
                    caller_id: String::new(),
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
        let queries = load_typescript_queries();
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
            let mut match_line: u64 = 0;

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
                    "import.default" => {
                        alias = Some(self.node_text(node, source).to_string());
                    }
                    _ => {
                        if match_line == 0 {
                            match_line = node.start_position().row as u64 + 1;
                        }
                    }
                }
            }

            if !import_path.is_empty() {
                results.push(Import {
                    file_path: file_path_str.clone(),
                    import_path,
                    alias,
                    line: match_line,
                    resolution: ImportResolution::Unresolved,
                });
            }
        }

        results
    }

    fn infer_package(&self, file_path: &Path, _source: &str) -> Option<String> {
        // TS 项目：从文件路径推断包名——取 src 目录的父目录名
        let mut current = file_path.parent();
        while let Some(dir) = current {
            if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                if name == "src" {
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

/// 辅助函数：将 SymbolKind 转为字符串
fn kind_to_str(kind: &SymbolKind) -> &str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Class => "class",
        SymbolKind::Interface => "interface",
        SymbolKind::Enum => "enum",
        SymbolKind::TypeAlias => "type_alias",
        SymbolKind::Variable => "variable",
        _ => "unknown",
    }
}

/// 返回 SymbolKind 的去重优先级（数值越大越优先保留）
fn kind_priority_val(kind: &SymbolKind) -> u32 {
    match kind {
        SymbolKind::Function => 10,
        SymbolKind::Method => 9,
        SymbolKind::Class => 8,
        SymbolKind::Interface => 7,
        SymbolKind::Enum => 6,
        SymbolKind::TypeAlias => 5,
        SymbolKind::Variable => 1,
        _ => 0,
    }
}

/// 从字符串获取优先级（用于 seen_names 中的类型比较）
fn kind_priority_val_for_str(kind_str: &str) -> u32 {
    match kind_str {
        "function" => 10,
        "method" => 9,
        "class" => 8,
        "interface" => 7,
        "enum" => 6,
        "type_alias" => 5,
        "variable" => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse_source(source: &str) -> (Tree, TypeScriptParser) {
        let parser = TypeScriptParser::new();
        let tree = parser.parse(source).expect("解析测试源码失败");
        (tree, parser)
    }

    #[test]
    fn test_parse_function() {
        let source = r#"
function greet(name: string): string {
    return `Hello, ${name}`;
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.ts"));
        let func = symbols.iter().find(|s| s.name == "greet");
        assert!(func.is_some(), "应找到 greet 函数");
        assert_eq!(func.unwrap().kind, SymbolKind::Function);
    }

    #[test]
    fn test_parse_class() {
        let source = r#"
class Person {
    name: string;
    constructor(name: string) {
        this.name = name;
    }
    greet(): string {
        return `Hi, I'm ${this.name}`;
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.ts"));
        let cls = symbols.iter().find(|s| s.name == "Person");
        assert!(cls.is_some(), "应找到 Person 类");
        assert_eq!(cls.unwrap().kind, SymbolKind::Class);
    }

    #[test]
    fn test_parse_interface() {
        let source = r#"
interface Config {
    port: number;
    host: string;
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.ts"));
        let iface = symbols.iter().find(|s| s.name == "Config");
        assert!(iface.is_some(), "应找到 Config 接口");
        assert_eq!(iface.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn test_parse_arrow_function() {
        let source = r#"
const add = (a: number, b: number): number => a + b;
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.ts"));
        let func = symbols.iter().find(|s| s.name == "add");
        assert!(func.is_some(), "应找到 add 箭头函数");
        assert_eq!(func.unwrap().kind, SymbolKind::Function);
    }

    #[test]
    fn test_parse_function_calls() {
        let source = r#"
function main() {
    foo();
    bar.baz();
    new Person("Alice");
}
"#;
        let (tree, parser) = parse_source(source);
        let calls = parser.extract_calls(&tree, source, Path::new("test.ts"));
        let foo_call = calls.iter().find(|c| c.callee_name == "foo");
        let baz_call = calls.iter().find(|c| c.callee_name == "baz");
        let person_call = calls.iter().find(|c| c.callee_name == "Person");
        assert!(foo_call.is_some(), "应有 foo 调用");
        assert!(baz_call.is_some(), "应有 baz 方法调用");
        assert!(person_call.is_some(), "应有 Person 构造调用");
    }

    #[test]
    fn test_parse_imports() {
        let source = r#"
import { AuthService } from "./auth";
import * as utils from "./utils";
"#;
        let (tree, parser) = parse_source(source);
        let imports = parser.extract_imports(&tree, source, Path::new("test.ts"));
        assert!(!imports.is_empty(), "应有导入语句");
        let auth_import = imports.iter().find(|i| i.import_path == "./auth");
        assert!(auth_import.is_some(), "应有 ./auth 导入");
    }

    #[test]
    fn test_language_name() {
        let parser = TypeScriptParser::new();
        assert_eq!(parser.language(), "typescript");
    }

    #[test]
    fn test_file_extensions() {
        let parser = TypeScriptParser::new();
        assert_eq!(parser.file_extensions(), &["ts", "tsx"]);
    }
}
