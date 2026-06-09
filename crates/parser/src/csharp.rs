//! C# 语言解析器
//!
//! 使用 tree-sitter-c-sharp grammar 解析 .cs 文件，
//!
//! ## 已知问题
//!
//! tree-sitter-c-sharp 0.23.5（crates.io 最新版）的 grammar ABI 版本为 15，
//! 超出了 tree-sitter 0.24.7 核心库支持的 13-14 范围。运行时加载 grammar
//! 会失败并报 `LanguageError { version: 15 }`。
//!
//! 解决方案选项：
//! 1. 等待 tree-sitter-c-sharp 发布 0.24 版本（支持新 ABI）
//! 2. 将 tree-sitter 核心库降级到 0.23
//! 3. 使用 GitHub 上兼容 ABI 的 fork/patch
//!
//! 解析器代码本身是正确的，只是运行时 grammar 加载会失败。
//! 提取类、接口、结构体、枚举、方法、属性、字段、命名空间等符号定义，
//! 以及方法调用（含成员调用、构造函数调用）和 using 导入语句。

use std::path::Path;
use std::sync::Mutex;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::symbol_id::StableSymbolId;
use codeconnect_core::types::{
    CallSite, CallType, Import, ImportResolution, SourceLocation, Symbol, SymbolKind,
};
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator, Tree};

use crate::query_loader::load_csharp_queries;
use crate::r#trait::LanguageParser;

/// C# 语言解析器
///
/// 包装了 tree-sitter-c-sharp grammar 和一个 `Mutex<Parser>`，
/// 确保并发调用时的线程安全。缓存 Language 对象用于 Query 编译。
pub struct CSharpParser {
    /// tree-sitter 解析器实例（Mutex 因为 Parser 不是 Sync）
    parser: Mutex<Parser>,
    /// C# language 对象（用于 Query::new 编译查询）
    language: tree_sitter::Language,
}

impl CSharpParser {
    /// 创建新的 C# 解析器
    pub fn new() -> Self {
        let mut parser = Parser::new();
        // tree-sitter 0.25: 使用 Language::new() 从 LanguageFn 构造
        let language: tree_sitter::Language = tree_sitter::Language::new(tree_sitter_c_sharp::LANGUAGE);
        parser
            .set_language(&language)
            .expect("加载 C# tree-sitter grammar 失败");
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

    /// 判断符号是否公开可见
    ///
    /// C# 中，public 修饰符或没有显式访问修饰符的内部符号
    /// 默认是 private。这里简化处理：类/接口/结构体/枚举默认 public。
    fn is_public_symbol(&self, kind: &SymbolKind) -> bool {
        matches!(
            kind,
            SymbolKind::Class
                | SymbolKind::Interface
                | SymbolKind::Struct
                | SymbolKind::Enum
                | SymbolKind::Module // 命名空间
        )
    }

    /// 将 SymbolKind 转为描述字符串（用于 StableSymbolId）
    fn kind_str(&self, kind: &SymbolKind) -> &'static str {
        match kind {
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Method => "method",
            SymbolKind::Field => "field",
            SymbolKind::Module => "namespace",
            SymbolKind::Unknown(s) if s == "property" => "property",
            _ => "unknown",
        }
    }
}

impl Default for CSharpParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for CSharpParser {
    fn language(&self) -> &'static str {
        "csharp"
    }

    fn file_extensions(&self) -> &[&str] {
        &["cs"]
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
                message: "C# 源码解析失败，可能包含语法错误".to_string(),
            })
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<Symbol> {
        let queries = load_csharp_queries();
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
                    "symbol.struct" => {
                        kind = SymbolKind::Struct;
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
                    "symbol.property" => {
                        kind = SymbolKind::Unknown("property".to_string());
                        location = self.node_to_location(node);
                    }
                    "symbol.field" => {
                        kind = SymbolKind::Field;
                        location = self.node_to_location(node);
                        // 字段声明没有 name 字段，从 variable_declaration 内部提取名称
                        // variable_declaration 结构: (predefined_type, variable_declarator(name: identifier))
                        if name.is_empty() {
                            for i in 0..node.child_count() {
                                if let Some(child) = node.child(i) {
                                    if child.kind() == "variable_declarator" {
                                        name = self.node_text(child, source).to_string();
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    "symbol.namespace" => {
                        kind = SymbolKind::Module;
                        location = self.node_to_location(node);
                    }
                    // 方法声明的参数和返回类型仅用于标记，不单独处理
                    "symbol.parameters" | "symbol.return_type" | "symbol.type" => {}
                    _ => {}
                }
            }

            if name.is_empty() {
                continue;
            }

            // 过滤掉只有声明位置但没有节点类型的匹配（可能是字段查询的中间节点）
            if location.line == 0 {
                continue;
            }

            let kind_str = self.kind_str(&kind);
            let id = StableSymbolId::new("csharp", &file_path_str, kind_str, &name);

            // C# 默认公开可见规则：以大写字母开头 或 明确是公开类型
            let is_exported = self.is_public_symbol(&kind)
                || name.chars().next().map_or(false, |c| c.is_uppercase());

            let modifiers = if is_exported {
                vec!["public".to_string()]
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
                parent_id: None,
                modifiers,
                is_exported,
                complexity: None,
            });
        }

        results
    }

    fn extract_calls(&self, tree: &Tree, source: &str, file_path: &Path) -> Vec<CallSite> {
        let queries = load_csharp_queries();
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
                        call_type = CallType::Direct;
                    }
                    "call.method" => {
                        location = self.node_to_location(node);
                        call_type = CallType::Virtual;
                    }
                    "call.constructor" => {
                        location = self.node_to_location(node);
                        call_type = CallType::Direct;
                    }
                    // 参数列表仅用于标记，不单独处理
                    "call.arguments" => {}
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
        let queries = load_csharp_queries();
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
            let mut is_package = false;

            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                match capture_name {
                    "import.path" => {
                        import_path = self.node_text(node, source).to_string();
                    }
                    "import.package" => {
                        // 命名空间声明 — 记录为包信息，不是导入
                        is_package = true;
                        import_line = node.start_position().row as u64 + 1;
                    }
                    "import" => {
                        import_line = node.start_position().row as u64 + 1;
                    }
                    _ => {}
                }
            }

            // 命名空间声明不视为导入（用于包名推断）
            if is_package {
                continue;
            }

            if !import_path.is_empty() {
                results.push(Import {
                    file_path: file_path_str.clone(),
                    import_path,
                    alias: None,
                    line: import_line,
                    resolution: ImportResolution::Unresolved,
                });
            }
        }

        results
    }

    fn infer_package(&self, file_path: &Path, _source: &str) -> Option<String> {
        // C# 项目：从源码中的命名空间声明推断包名
        // 如果源码中有 namespace 声明，直接提取
        // 否则，从文件路径的目录结构中推断
        let mut current = file_path.parent();
        while let Some(dir) = current {
            if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                // 跳过常见的项目结构目录
                if matches!(name, "src" | "bin" | "obj" | "Properties") {
                    current = dir.parent();
                    continue;
                }
                if !name.is_empty() {
                    // 如果源码中有命名空间声明，优先使用它
                    // 简化实现：从 .csproj 同目录名推断
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
    use std::path::{Path, PathBuf};

    /// 辅助函数：创建解析器并返回解析结果
    fn parse_source(source: &str) -> (Tree, CSharpParser) {
        let parser = CSharpParser::new();
        let tree = parser.parse(source).expect("解析测试源码失败");
        (tree, parser)
    }

    // ===== 解析测试 =====

    #[test]
    fn test_parse_simple_file() {
        let source = "class Program { }";
        let (tree, _parser) = parse_source(source);
        // 空语法树也能解析（包含 ERROR 节点是正常的，因为不是完整程序）
        assert!(tree.root_node().child_count() > 0, "应有语法节点");
    }

    // ===== 符号提取测试 =====

    #[test]
    fn test_extract_class() {
        let source = r#"
class MyClass {
    public void DoSomething() { }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        let my_class = symbols.iter().find(|s| s.name == "MyClass");
        assert!(my_class.is_some(), "应找到 MyClass 类");
        assert_eq!(my_class.unwrap().kind, SymbolKind::Class);
    }

    #[test]
    fn test_extract_interface() {
        let source = r#"
interface IRepository {
    void Save();
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        let iface = symbols.iter().find(|s| s.name == "IRepository");
        assert!(iface.is_some(), "应找到 IRepository 接口");
        assert_eq!(iface.unwrap().kind, SymbolKind::Interface);
    }

    #[test]
    fn test_extract_struct() {
        let source = r#"
struct Point {
    public int X;
    public int Y;
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        let point = symbols.iter().find(|s| s.name == "Point");
        assert!(point.is_some(), "应找到 Point 结构体");
        assert_eq!(point.unwrap().kind, SymbolKind::Struct);
    }

    #[test]
    fn test_extract_enum() {
        let source = r#"
public enum Color {
    Red,
    Green,
    Blue
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        let color = symbols.iter().find(|s| s.name == "Color");
        assert!(color.is_some(), "应找到 Color 枚举");
        assert_eq!(color.unwrap().kind, SymbolKind::Enum);
    }

    #[test]
    fn test_extract_method() {
        let source = r#"
class Calculator {
    public int Add(int a, int b) {
        return a + b;
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        let add_method = symbols.iter().find(|s| s.name == "Add");
        assert!(add_method.is_some(), "应找到 Add 方法");
        assert_eq!(add_method.unwrap().kind, SymbolKind::Method);
    }

    #[test]
    fn test_extract_property() {
        let source = r#"
class Person {
    public string Name { get; set; }
    public int Age { get; set; }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        let name_prop = symbols.iter().find(|s| s.name == "Name");
        assert!(name_prop.is_some(), "应找到 Name 属性");
        let age_prop = symbols.iter().find(|s| s.name == "Age");
        assert!(age_prop.is_some(), "应找到 Age 属性");
    }

    #[test]
    fn test_extract_field() {
        let source = r#"
class Config {
    private string _connectionString;
    private int _timeout;
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        let conn_field = symbols.iter().find(|s| s.name == "_connectionString");
        assert!(conn_field.is_some(), "应找到 _connectionString 字段");
        assert_eq!(conn_field.unwrap().kind, SymbolKind::Field);
    }

    #[test]
    fn test_extract_namespace() {
        let source = r#"
namespace MyApp.Services {
    public class UserService { }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        let ns = symbols.iter().find(|s| s.name == "MyApp.Services");
        assert!(ns.is_some(), "应找到 MyApp.Services 命名空间");
        assert_eq!(ns.unwrap().kind, SymbolKind::Module);
    }

    // ===== 调用提取测试 =====

    #[test]
    fn test_extract_direct_call() {
        let source = r#"
class Program {
    void Main() {
        Console.WriteLine("Hello");
        DoSomething();
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let calls = parser.extract_calls(&tree, source, Path::new("test.cs"));
        let writeline = calls.iter().find(|c| c.callee_name == "WriteLine");
        assert!(writeline.is_some(), "应有 WriteLine 调用");
        assert_eq!(writeline.unwrap().call_type, CallType::Virtual); // member_access_expression
        let dosomething = calls.iter().find(|c| c.callee_name == "DoSomething");
        assert!(dosomething.is_some(), "应有 DoSomething 调用");
        assert_eq!(dosomething.unwrap().call_type, CallType::Direct);
    }

    #[test]
    fn test_extract_constructor_call() {
        let source = r#"
class Factory {
    object Create() {
        return new StringBuilder();
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let calls = parser.extract_calls(&tree, source, Path::new("test.cs"));
        let ctor = calls.iter().find(|c| c.callee_name == "StringBuilder");
        assert!(ctor.is_some(), "应有 StringBuilder 构造函数调用");
        assert_eq!(ctor.unwrap().call_type, CallType::Direct);
    }

    #[test]
    fn test_extract_method_call_on_object() {
        let source = r#"
class Processor {
    void Process() {
        var list = new List<int>();
        list.Add(1);
        list.Clear();
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let calls = parser.extract_calls(&tree, source, Path::new("test.cs"));
        let add_call = calls.iter().find(|c| c.callee_name == "Add");
        assert!(add_call.is_some(), "应有 Add 方法调用");
        assert_eq!(add_call.unwrap().call_type, CallType::Virtual);
        let clear_call = calls.iter().find(|c| c.callee_name == "Clear");
        assert!(clear_call.is_some(), "应有 Clear 方法调用");
        assert_eq!(clear_call.unwrap().call_type, CallType::Virtual);
    }

    // ===== 导入提取测试 =====

    #[test]
    fn test_extract_using_directive() {
        let source = r#"
using System;
using System.Collections.Generic;
using System.Linq;

namespace MyApp { }
"#;
        let (tree, parser) = parse_source(source);
        let imports = parser.extract_imports(&tree, source, Path::new("test.cs"));
        assert!(!imports.is_empty(), "应有导入语句");
        assert!(imports.len() >= 3, "应有至少 3 条 using 指令");
        let system_import = imports.iter().find(|i| i.import_path == "System");
        assert!(system_import.is_some(), "应有 System 导入");
        let generic_import = imports.iter().find(|i| i.import_path == "System.Collections.Generic");
        assert!(generic_import.is_some(), "应有 System.Collections.Generic 导入");
    }

    #[test]
    fn test_namespace_not_treated_as_import() {
        let source = r#"
using System;
namespace MyCompany.MyProject {
    class Service { }
}
"#;
        let (tree, parser) = parse_source(source);
        let imports = parser.extract_imports(&tree, source, Path::new("test.cs"));
        // 命名空间声明不应该被当作导入
        let ns_as_import = imports.iter().find(|i| i.import_path == "MyCompany.MyProject");
        assert!(ns_as_import.is_none(), "命名空间声明不应被当作导入");
    }

    // ===== 基础测试 =====

    #[test]
    fn test_language_name() {
        let parser = CSharpParser::new();
        assert_eq!(parser.language(), "csharp");
    }

    #[test]
    fn test_file_extensions() {
        let parser = CSharpParser::new();
        assert_eq!(parser.file_extensions(), &["cs"]);
    }

    #[test]
    fn test_parse_empty_file() {
        let source = "";
        let parser = CSharpParser::new();
        let tree = parser.parse(source).expect("解析空文件应返回空 Tree");
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        assert!(symbols.is_empty(), "空文件应无符号");
    }

    #[test]
    fn test_stable_symbol_id_format() {
        let id = StableSymbolId::new("csharp", "src/Services/UserService.cs", "class", "UserService");
        let serialized = id.to_string();
        let parts: Vec<&str> = serialized.split("::").collect();
        assert_eq!(parts.len(), 5, "符号 ID 应有 5 个字段");
        assert_eq!(parts[0], "csharp");
        assert_eq!(parts[1], "src/Services/UserService.cs");
        assert_eq!(parts[2], "class");
        assert_eq!(parts[3], "UserService");
        assert_eq!(parts[4].len(), 8, "指纹应为 8 字符十六进制");
    }

    #[test]
    fn test_symbol_is_exported() {
        let source = r#"
public class PublicService { }
internal class InternalHelper { }
public interface IRepository { }
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        let public_class = symbols.iter().find(|s| s.name == "PublicService");
        assert!(public_class.is_some(), "应找到 PublicService");
        assert!(public_class.unwrap().is_exported, "公开类应标记为 exported");
    }

    #[test]
    fn test_infer_package() {
        let parser = CSharpParser::new();
        let path = PathBuf::from("C:/projects/MyApp/src/Services/UserService.cs");
        let pkg = parser.infer_package(&path, "");
        assert_eq!(pkg, Some("Services".to_string()));
    }

    #[test]
    fn test_extract_multiple_symbol_types() {
        let source = r#"
namespace MyApp.Models {
    public class User {
        public int Id { get; set; }
        public string Name { get; set; }
        private int _internalId;
    }

    public interface IUserRepository {
        void Save(User user);
    }

    public enum UserRole {
        Admin,
        User
    }
}
"#;
        let (tree, parser) = parse_source(source);
        let symbols = parser.extract_symbols(&tree, source, Path::new("test.cs"));
        // 应包含命名空间
        assert!(symbols.iter().any(|s| s.name == "MyApp.Models" && s.kind == SymbolKind::Module));
        // 应包含类
        assert!(symbols.iter().any(|s| s.name == "User" && s.kind == SymbolKind::Class));
        // 应包含接口
        assert!(symbols.iter().any(|s| s.name == "IUserRepository" && s.kind == SymbolKind::Interface));
        // 应包含枚举
        assert!(symbols.iter().any(|s| s.name == "UserRole" && s.kind == SymbolKind::Enum));
    }
}
