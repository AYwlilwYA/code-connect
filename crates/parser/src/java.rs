//! Java 语言解析器
//!
//! 使用 tree-sitter-java grammar 解析 .java 文件，
//! 提取类、接口、枚举、方法、构造函数、字段、注解等符号定义，
//! 以及方法调用、对象创建和 import 导入语句。

use std::path::Path;
use std::sync::Mutex;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::symbol_id::StableSymbolId;
use codeconnect_core::types::{
    CallSite, CallType, Import, ImportResolution, SourceLocation, Symbol, SymbolKind,
};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::query_loader::load_java_queries;
use crate::r#trait::LanguageParser;

/// Java 语言解析器
///
/// 包装了 tree-sitter-java grammar 和一个 `Mutex<Parser>`，
/// 确保并发调用时的线程安全。缓存 Language 对象用于 Query 编译。
pub struct JavaParser {
    /// tree-sitter 解析器实例（Mutex 因为 Parser 不是 Sync）
    parser: Mutex<Parser>,
    /// Java language 对象（用于 Query::new 编译查询）
    language: tree_sitter::Language,
}

impl JavaParser {
    /// 创建新的 Java 解析器
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter::Language::new(tree_sitter_java::LANGUAGE);
        parser
            .set_language(&language)
            .expect("加载 Java tree-sitter grammar 失败");
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

impl Default for JavaParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for JavaParser {
    fn language(&self) -> &'static str {
        "java"
    }

    fn file_extensions(&self) -> &[&str] {
        &["java"]
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
                message: "Java 源码解析失败，可能包含语法错误".to_string(),
            })
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<Symbol> {
        let queries = load_java_queries();
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
                    "symbol.name" => {
                        name = self.node_text(node, source).to_string();
                    }
                    "symbol.class" => {
                        kind = SymbolKind::Class;
                        location = self.node_to_location(node);
                    }
                    "symbol.interface" => {
                        kind = SymbolKind::Interface;
                        location = self.node_to_location(node);
                    }
                    "symbol.enum" => {
                        kind = SymbolKind::Enum;
                        location = self.node_to_location(node);
                    }
                    "symbol.method" => {
                        kind = SymbolKind::Method;
                        location = self.node_to_location(node);
                    }
                    "symbol.constructor" => {
                        kind = SymbolKind::Method; // 构造函数归类为 Method，通过 name 区分
                        location = self.node_to_location(node);
                    }
                    "symbol.field" => {
                        kind = SymbolKind::Field;
                        location = self.node_to_location(node);
                    }
                    "symbol.annotation" => {
                        kind = SymbolKind::Interface; // 注解本质是特殊的接口
                        location = self.node_to_location(node);
                    }
                    _ => {}
                }
            }

            if name.is_empty() {
                continue;
            }

            let kind_str = match &kind {
                SymbolKind::Class => "class",
                SymbolKind::Interface => "interface",
                SymbolKind::Enum => "enum",
                SymbolKind::Method => "method",
                SymbolKind::Field => "field",
                _ => "unknown",
            };

            let id = StableSymbolId::new("java", &file_path_str, kind_str, &name);

            // Java 类、接口、枚举和方法默认通过 public 修饰符控制可见性
            // 此处简化处理：标记为导出，实际可见性由 scope 查询另行确定
            let is_exported = matches!(
                kind,
                SymbolKind::Class | SymbolKind::Interface | SymbolKind::Enum
            );

            let modifiers = if is_exported {
                vec!["public".to_string()]
            } else {
                vec!["default".to_string()]
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
        let queries = load_java_queries();
        let file_path_str = file_path.to_string_lossy().to_string();
        let mut results = Vec::new();

        let query = match Query::new(&self.language, &queries.calls) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        // 临时存储：(callee_name, 行号) -> (call_type, location)
        // 用于去重：同名同行的调用，call.method 优先于 call
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
                    "call.name" => {
                        callee_name = self.node_text(node, source).to_string();
                    }
                    "call" => {
                        location = self.node_to_location(node);
                        capture_kind = "call";
                    }
                    "call.method" => {
                        call_type = CallType::Virtual;
                        location = self.node_to_location(node);
                        capture_kind = "call.method";
                    }
                    "call.constructor" => {
                        call_type = CallType::Direct;
                        location = self.node_to_location(node);
                        capture_kind = "call.constructor";
                    }
                    _ => {}
                }
            }

            if !callee_name.is_empty() && location.line > 0 {
                let key = (callee_name.clone(), location.line);
                // call.method/constructor 优先于 call（更具体的匹配）
                if let Some(_existing) = seen.get(&key) {
                    if capture_kind != "call" {
                        seen.insert(key, (call_type, location));
                    }
                } else {
                    seen.insert(key, (call_type, location));
                }
            }
        }

        // 转为结果 Vec
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
        let queries = load_java_queries();
        let file_path_str = file_path.to_string_lossy().to_string();
        let mut results = Vec::new();

        let query = match Query::new(&self.language, &queries.imports) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        // 遍历所有匹配：import 路径与 package 声明
        while let Some(m) = matches.next() {
            let mut import_path = String::new();
            let mut import_line: u64 = 0;

            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                match capture_name {
                    "import.path" => {
                        import_path = self.node_text(node, source).to_string();
                    }
                    "import" => {
                        import_line = node.start_position().row as u64 + 1;
                    }
                    "import.package" => {
                        import_path = self.node_text(node, source).to_string();
                        import_line = node.start_position().row as u64 + 1;
                    }
                    _ => {}
                }
            }

            if !import_path.is_empty() {
                // 检查是否已存在相同路径的记录（避免重复）
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
        // Java 项目：从目录结构推断包名
        // 逐级向上查找父目录，直到遇到非 Java 包名的目录
        // 典型结构：src/main/java/com/example/MyClass.java → com.example
        let mut components: Vec<&str> = Vec::new();

        // 收集文件名上方的所有目录名，直到遇到特殊目录（src, java, test 等）
        let mut current = file_path.parent();
        let stop_dirs = ["src", "java", "test", "main", "resources"];

        while let Some(dir) = current {
            if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                let lower = name.to_lowercase();
                if stop_dirs.contains(&lower.as_str()) {
                    break;
                }
                if !name.is_empty() && !name.starts_with('.') {
                    components.push(name);
                }
            }
            current = dir.parent();
        }

        if components.is_empty() {
            None
        } else {
            components.reverse();
            Some(components.join("."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 辅助函数：创建解析器并返回解析结果
    fn parse_source(source: &str) -> (Tree, JavaParser) {
        let parser = JavaParser::new();
        let tree = parser.parse(source).expect("解析测试源码失败");
        (tree, parser)
    }

    #[test]
    fn test_parse_simple_class() {
        let source = r#"
public class HelloWorld {
    public static void main(String[] args) {
        System.out.println("Hello, World!");
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.java"));
        let class_sym = symbols.iter().find(|s| s.name == "HelloWorld");
        assert!(class_sym.is_some(), "应找到 HelloWorld 类");
        assert_eq!(class_sym.unwrap().kind, SymbolKind::Class);
    }

    #[test]
    fn test_parse_interface() {
        let source = r#"
public interface Printable {
    void print();
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.java"));
        let interface_sym = symbols.iter().find(|s| s.name == "Printable");
        assert!(interface_sym.is_some(), "应找到 Printable 接口");
        assert_eq!(interface_sym.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn test_parse_enum() {
        let source = r#"
public enum Color {
    RED, GREEN, BLUE;
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.java"));
        let enum_sym = symbols.iter().find(|s| s.name == "Color");
        assert!(enum_sym.is_some(), "应找到 Color 枚举");
        assert_eq!(enum_sym.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn test_parse_method() {
        let source = r#"
public class Calculator {
    public int add(int a, int b) {
        return a + b;
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.java"));
        let method_sym = symbols.iter().find(|s| s.name == "add");
        assert!(method_sym.is_some(), "应找到 add 方法");
        assert_eq!(method_sym.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn test_parse_constructor() {
        let source = r#"
public class Point {
    private int x, y;

    public Point(int x, int y) {
        this.x = x;
        this.y = y;
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.java"));
        // Point 既作为类名也作为构造函数名出现，找到所有匹配
        let point_symbols: Vec<_> = symbols.iter().filter(|s| s.name == "Point").collect();
        assert!(!point_symbols.is_empty(), "应找到 Point 符号");
        // 至少有一个是 Class，有一个是 Method（构造函数）
        let has_class = point_symbols.iter().any(|s| s.kind == SymbolKind::Class);
        let has_ctor = point_symbols.iter().any(|s| s.kind == SymbolKind::Method);
        assert!(has_class, "Point 应有类声明");
        assert!(has_ctor, "Point 应有构造函数（归类为 Method）");
    }

    #[test]
    fn test_parse_field() {
        let source = r#"
public class Person {
    private String name;
    private int age;
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.java"));
        let name_field = symbols.iter().find(|s| s.name == "name");
        let age_field = symbols.iter().find(|s| s.name == "age");
        assert!(name_field.is_some(), "应找到 name 字段");
        assert_eq!(name_field.unwrap().kind, SymbolKind::Field);
        assert!(age_field.is_some(), "应找到 age 字段");
    }

    #[test]
    fn test_parse_annotation() {
        let source = r#"
public @interface MyAnnotation {
    String value() default "";
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.java"));
        let anno_sym = symbols.iter().find(|s| s.name == "MyAnnotation");
        assert!(anno_sym.is_some(), "应找到 MyAnnotation 注解");
    }

    #[test]
    fn test_parse_method_calls() {
        let source = r#"
public class Main {
    public static void main(String[] args) {
        foo();
        bar.baz();
        new Thread().start();
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let calls = parser.extract_calls(&tree, source, Path::new("test.java"));
        let foo_call = calls.iter().find(|c| c.callee_name == "foo");
        let baz_call = calls.iter().find(|c| c.callee_name == "baz");
        let thread_call = calls.iter().find(|c| c.callee_name == "Thread");
        let start_call = calls.iter().find(|c| c.callee_name == "start");
        assert!(foo_call.is_some(), "应有 foo 方法调用");
        assert!(baz_call.is_some(), "应有 baz 对象方法调用");
        assert!(thread_call.is_some(), "应有 Thread 构造函数调用");
        assert!(start_call.is_some(), "应有 start 方法调用");
    }

    #[test]
    fn test_parse_constructor_call() {
        let source = r#"
public class Main {
    void test() {
        Object obj = new Object();
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let calls = parser.extract_calls(&tree, source, Path::new("test.java"));
        let obj_call = calls.iter().find(|c| c.callee_name == "Object");
        assert!(obj_call.is_some(), "应有 Object 构造函数调用");
    }

    #[test]
    fn test_parse_imports() {
        let source = r#"
import java.util.List;
import java.util.Map;
import java.io.*;

public class ImportsTest {
}
"#;
        let (tree, parser) = parse_source(source);
        let imports = parser.extract_imports(&tree, source, Path::new("test.java"));
        assert!(!imports.is_empty(), "应有导入语句");
        let list_import = imports.iter().find(|i| i.import_path.contains("List"));
        assert!(list_import.is_some(), "应有 java.util.List 导入");
    }

    #[test]
    fn test_parse_package() {
        let source = r#"
package com.example.myapp;

public class App {
}
"#;
        let (tree, parser) = parse_source(source);
        let imports = parser.extract_imports(&tree, source, Path::new("test.java"));
        let pkg = imports.iter().find(|i| i.import_path == "com.example.myapp");
        assert!(pkg.is_some(), "应有 package 声明作为特殊导入记录");
    }

    #[test]
    fn test_stable_symbol_id_format() {
        let id = StableSymbolId::new("java", "src/main/java/com/example/App.java", "class", "App");
        let serialized = id.to_string();
        let parts: Vec<&str> = serialized.split("::").collect();
        assert_eq!(parts.len(), 5, "符号 ID 应有 5 个字段");
        assert_eq!(parts[0], "java");
        assert_eq!(parts[1], "src/main/java/com/example/App.java");
        assert_eq!(parts[2], "class");
        assert_eq!(parts[3], "App");
        assert_eq!(parts[4].len(), 8, "指纹应为 8 字符十六进制");
    }

    #[test]
    fn test_infer_package_from_path() {
        let parser = JavaParser::new();
        let path = PathBuf::from("/home/user/projects/myapp/src/main/java/com/example/App.java");
        let pkg = parser.infer_package(&path, "");
        assert_eq!(pkg, Some("com.example".to_string()));
    }

    #[test]
    fn test_infer_package_simple() {
        let parser = JavaParser::new();
        let path = PathBuf::from("/home/user/projects/myapp/src/main/java/myapp/App.java");
        let pkg = parser.infer_package(&path, "");
        assert_eq!(pkg, Some("myapp".to_string()));
    }

    #[test]
    fn test_language_name() {
        let parser = JavaParser::new();
        assert_eq!(parser.language(), "java");
    }

    #[test]
    fn test_file_extensions() {
        let parser = JavaParser::new();
        assert_eq!(parser.file_extensions(), &["java"]);
    }

    #[test]
    fn test_parse_empty_file() {
        let source = "";
        let parser = JavaParser::new();
        let tree = parser.parse(source).expect("解析空文件应返回空 Tree");
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.java"));
        assert!(symbols.is_empty(), "空文件应无符号");
    }

    /// 诊断测试：验证 Java 解析器能正确处理构造函数和调用表达式
    #[test]
    fn test_dump_constructor_and_calls_ast() {
        let source = r#"class Main { void run() { new Thread(); foo(); } }"#;
        let parser = JavaParser::new();
        let tree = parser.parse(source).expect("解析失败");

        // 验证 Tree 解析非空
        let root_kind = tree.root_node().kind();
        assert_eq!(root_kind, "program");

        // 验证 calls 提取包含 Thread 的构造函数调用
        let calls = parser.extract_calls(&tree, source, Path::new("test.java"));
        let has_thread = calls.iter().any(|c| c.callee_name == "Thread");
        assert!(has_thread, "应找到 Thread 构造函数调用，实际调用: {:?}", calls.iter().map(|c| &c.callee_name).collect::<Vec<_>>());
    }
}
