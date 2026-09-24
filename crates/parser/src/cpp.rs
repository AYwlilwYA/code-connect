//! C++ 语言解析器
//!
//! 使用 tree-sitter-cpp grammar 解析 .cpp/.hpp/.cc/.cxx 等文件，
//! 提取类、函数、方法、结构体、枚举、宏定义、typedef、命名空间等符号定义，
//! 以及函数调用和 #include 导入语句。
//!
//! C++ grammar 与 C grammar 共享大量节点类型，因此复用 queries/c/ 下的 query 文件。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::symbol_id::StableSymbolId;
use codeconnect_core::types::{
    CallSite, CallType, Import, ImportResolution, SourceLocation, Symbol, SymbolKind,
};
use tree_sitter::{Node, Parser, Query, QueryCursor, StreamingIterator, Tree};

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

    /// 从 declarator 链解析出函数名节点，非函数返回 None
    ///
    /// 指针/引用返回类型会把 function_declarator 包在
    /// pointer_declarator / reference_declarator / parenthesized_declarator 里，
    /// 需要逐层下探；链上没有 function_declarator（数据成员、函数指针变量）则不是函数。
    fn resolve_declarator_name(node: Node) -> Option<Node> {
        let mut cur = node;
        for _ in 0..8 {
            match cur.kind() {
                "pointer_declarator" | "reference_declarator" | "parenthesized_declarator" => {
                    cur = Self::declarator_child(cur)?;
                }
                "function_declarator" => {
                    let inner = Self::declarator_child(cur)?;
                    return match inner.kind() {
                        "identifier" | "field_identifier" | "destructor_name" | "operator_name" => {
                            Some(inner)
                        }
                        // Shape::area / Box<T>::get
                        "qualified_identifier" | "template_function" => {
                            inner.child_by_field_name("name")
                        }
                        _ => None,
                    };
                }
                _ => return None,
            }
        }
        None
    }

    /// 取 declarator 链上的下一层节点
    ///
    /// 注意 grammar 差异：pointer_declarator 的子节点带 declarator 字段，
    /// reference_declarator（`int& f()`）的子节点**没有**该字段，只能按 kind 找。
    fn declarator_child(node: Node) -> Option<Node> {
        if let Some(c) = node.child_by_field_name("declarator") {
            return Some(c);
        }
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|c| Self::is_declarator_node(c.kind()))
    }

    /// 是否为 declarator 链上的节点类型
    fn is_declarator_node(kind: &str) -> bool {
        matches!(
            kind,
            "identifier"
                | "field_identifier"
                | "destructor_name"
                | "operator_name"
                | "function_declarator"
                | "qualified_identifier"
                | "template_function"
                | "pointer_declarator"
                | "reference_declarator"
                | "parenthesized_declarator"
        )
    }

    /// 判断节点是否位于类/结构体体内（用于区分 Method 与 Function）
    fn is_class_member(node: Node) -> bool {
        let mut cur = node.parent();
        for _ in 0..6 {
            let p = match cur {
                Some(p) => p,
                None => return false,
            };
            match p.kind() {
                "field_declaration_list" => return true,
                "translation_unit" | "namespace_definition" | "declaration_list"
                | "function_definition" | "compound_statement" => return false,
                _ => cur = p.parent(),
            }
        }
        false
    }

    /// 判断节点是否位于 template_declaration 内
    fn has_template_ancestor(node: Node) -> bool {
        let mut cur = node.parent();
        for _ in 0..6 {
            let p = match cur {
                Some(p) => p,
                None => return false,
            };
            match p.kind() {
                "template_declaration" => return true,
                "translation_unit" | "namespace_definition" | "declaration_list"
                | "compound_statement" => return false,
                _ => cur = p.parent(),
            }
        }
        false
    }

    /// 判定函数类符号是方法还是自由函数
    fn classify_function(name_node: Node, stmt: Node) -> SymbolKind {
        let is_method = matches!(name_node.kind(), "field_identifier" | "destructor_name")
            // 类外限定名定义：Shape::area
            || name_node
                .parent()
                .map(|p| p.kind() == "qualified_identifier")
                .unwrap_or(false)
            // 类内声明：构造函数 Shape(); 无返回类型，declarator 是裸 identifier
            || Self::is_class_member(stmt);
        if is_method {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        }
    }

    /// 组装符号对象
    fn build_symbol(
        &self,
        id: String,
        name: String,
        kind: SymbolKind,
        location: SourceLocation,
        is_template: bool,
    ) -> Symbol {
        // C++ 中函数、类、结构体、枚举、命名空间在命名空间层级都是可见的
        let is_exported = matches!(
            kind,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::Class
                | SymbolKind::Struct
                | SymbolKind::Module
                | SymbolKind::Enum
                | SymbolKind::TypeAlias
                | SymbolKind::Macro
        );

        let mut modifiers = if is_exported {
            vec!["extern".to_string()]
        } else {
            vec!["static".to_string()]
        };
        if is_template {
            modifiers.push("template".to_string());
        }

        Symbol {
            id,
            name,
            kind,
            location,
            signature: None,
            doc_comment: None,
            parent_id: None,
            modifiers,
            is_exported,
            complexity: None,
        }
    }

    /// 同一符号多次出现时的取舍权重：定义(3) > 类内声明(2) > 前置声明(1)
    fn definition_rank(node: Node) -> u8 {
        match node.kind() {
            "function_definition" => 3,
            "class_specifier" | "struct_specifier" | "union_specifier" | "enum_specifier" => {
                if node.child_by_field_name("body").is_some() {
                    3
                } else {
                    1
                }
            }
            "field_declaration" => 2,
            "declaration" => 1,
            _ => 2,
        }
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
        let mut results: Vec<Symbol> = Vec::new();

        let query = match Query::new(&self.language, &queries.symbols) {
            Ok(q) => q,
            Err(_) => return results,
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        // 符号 ID → (results 下标, 取舍权重)：声明与定义同名同类时只保留更完整的一个
        let mut seen: HashMap<String, (usize, u8)> = HashMap::new();

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
            // 函数类模式：名称与 Method/Function 需要解析 declarator 决定
            let mut declarator: Option<Node> = None;
            // 符号所在的语句节点，用于判断类成员位置与模板祖先
            let mut stmt: Option<Node> = None;

            for capture in m.captures {
                let node = capture.node;
                let capture_name = query.capture_names()[capture.index as usize];

                match capture_name {
                    "name" => {
                        name = self.node_text(node, source).to_string();
                    }
                    "declarator" => {
                        declarator = Some(node);
                    }
                    "class" => {
                        kind = SymbolKind::Class;
                        location = self.node_to_location(node, &file_path_str);
                        stmt = Some(node);
                    }
                    "struct" => {
                        kind = SymbolKind::Struct;
                        location = self.node_to_location(node, &file_path_str);
                        stmt = Some(node);
                    }
                    "union" => {
                        // SymbolKind 中没有 Union 变体，union 映射为 Struct
                        kind = SymbolKind::Struct;
                        location = self.node_to_location(node, &file_path_str);
                        stmt = Some(node);
                    }
                    "enum" => {
                        kind = SymbolKind::Enum;
                        location = self.node_to_location(node, &file_path_str);
                        stmt = Some(node);
                    }
                    "namespace" => {
                        kind = SymbolKind::Module;
                        location = self.node_to_location(node, &file_path_str);
                        stmt = Some(node);
                    }
                    "macro" => {
                        kind = SymbolKind::Macro;
                        location = self.node_to_location(node, &file_path_str);
                        stmt = Some(node);
                    }
                    "type_definition" => {
                        kind = SymbolKind::TypeAlias;
                        location = self.node_to_location(node, &file_path_str);
                        stmt = Some(node);
                    }
                    "func" | "method" | "declaration" => {
                        location = self.node_to_location(node, &file_path_str);
                        stmt = Some(node);
                    }
                    _ => {}
                }
            }

            if let Some(decl) = declarator {
                match Self::resolve_declarator_name(decl) {
                    Some(name_node) => {
                        name = self.node_text(name_node, source).to_string();
                        kind = match stmt {
                            Some(s) => Self::classify_function(name_node, s),
                            None => SymbolKind::Function,
                        };
                    }
                    // 不是函数（数据成员、函数指针变量等）
                    None => continue,
                }
            }

            if name.is_empty() {
                continue;
            }

            let kind_str = match &kind {
                SymbolKind::Function => "function",
                SymbolKind::Method => "method",
                SymbolKind::Class => "class",
                SymbolKind::Struct => "struct",
                SymbolKind::Enum => "enum",
                SymbolKind::Module => "module",
                SymbolKind::Macro => "macro",
                SymbolKind::TypeAlias => "type_alias",
                _ => "unknown",
            };

            let id = StableSymbolId::new("cpp", &file_path_str, kind_str, &name);
            let id_str = id.to_string();
            let rank = stmt.map(Self::definition_rank).unwrap_or(2);

            // 去重：同名同类符号（声明+定义、类内声明+类外定义）只产出一份
            if let Some(&(idx, old_rank)) = seen.get(&id_str) {
                if rank <= old_rank {
                    continue;
                }
                results[idx] = self.build_symbol(
                    id_str.clone(),
                    name,
                    kind,
                    location,
                    stmt.map(Self::has_template_ancestor).unwrap_or(false),
                );
                seen.insert(id_str, (idx, rank));
                continue;
            }


            let symbol = self.build_symbol(
                id_str.clone(),
                name,
                kind,
                location,
                stmt.map(Self::has_template_ancestor).unwrap_or(false),
            );
            seen.insert(id_str, (results.len(), rank));
            results.push(symbol);
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

    /// 提取符号
    fn symbols_of(source: &str) -> Vec<Symbol> {
        let (tree, parser) = parse_source(source);
        parser.extract_symbols(&tree, source, Path::new("test.cpp"))
    }

    /// 某名称 + 类型的符号个数
    fn count_of(symbols: &[Symbol], name: &str, kind: SymbolKind) -> usize {
        symbols
            .iter()
            .filter(|s| s.name == name && s.kind == kind)
            .count()
    }

    /// 查找符号
    fn find<'a>(symbols: &'a [Symbol], name: &str) -> &'a Symbol {
        symbols
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("未找到符号 {}，实得: {:?}", name, names(symbols)))
    }

    fn names(symbols: &[Symbol]) -> Vec<(String, String)> {
        symbols
            .iter()
            .map(|s| (s.name.clone(), format!("{:?}", s.kind)))
            .collect()
    }

    #[test]
    fn test_parse_class_namespace_ctor_dtor() {
        let source = "namespace geom {\nclass Shape {\npublic:\n    Shape();\n    virtual ~Shape();\n    virtual double area() const = 0;\n};\n}\n";
        let symbols = symbols_of(source);

        assert_eq!(find(&symbols, "geom").kind, SymbolKind::Module);
        assert_eq!(find(&symbols, "geom").location.line, 1);
        assert_eq!(find(&symbols, "Shape").kind, SymbolKind::Class);
        assert_eq!(find(&symbols, "Shape").location.line, 2);

        // 构造函数归类为 Method
        assert_eq!(count_of(&symbols, "Shape", SymbolKind::Method), 1);
        assert_eq!(find(&symbols, "Shape").kind, SymbolKind::Class);
        assert_eq!(count_of(&symbols, "~Shape", SymbolKind::Method), 1);
        assert_eq!(count_of(&symbols, "area", SymbolKind::Method), 1);
        assert_eq!(find(&symbols, "area").location.line, 6);
    }

    #[test]
    fn test_parse_namespace_nested_name() {
        let symbols = symbols_of("namespace outer::inner {\nvoid fn();\n}\n");
        assert_eq!(count_of(&symbols, "outer::inner", SymbolKind::Module), 1);
        assert_eq!(find(&symbols, "fn").kind, SymbolKind::Function);
    }

    #[test]
    fn test_parse_qualified_out_of_class_definition() {
        let source =
            "class Shape {\npublic:\n    double area() const;\n};\ndouble Shape::area() const { return 1.0; }\n";
        let symbols = symbols_of(source);

        // 类内声明 + 类外定义 → 只产出一份，位置取定义处
        assert_eq!(count_of(&symbols, "area", SymbolKind::Method), 1);
        assert_eq!(find(&symbols, "area").location.line, 5);
        assert_eq!(count_of(&symbols, "Shape", SymbolKind::Class), 1);
    }

    #[test]
    fn test_parse_in_class_definition_not_duplicated() {
        let source = "class Shape {\npublic:\n    double scaled(double f) const { return f; }\n};\n";
        let symbols = symbols_of(source);
        assert_eq!(count_of(&symbols, "scaled", SymbolKind::Method), 1);
        assert_eq!(find(&symbols, "scaled").location.line, 3);
        assert_eq!(symbols.len(), 2, "应只有类与类内方法: {:?}", names(&symbols));
    }

    #[test]
    fn test_parse_virtual_override_not_duplicated() {
        let source = "class Shape {\npublic:\n    virtual double area() const = 0;\n};\nclass Circle : public Shape {\npublic:\n    double area() const override;\n};\ndouble Circle::area() const { return 1.0; }\n";
        let symbols = symbols_of(source);

        // 基类纯虚 + 派生覆写 + 类外定义：同名同类只产出一份
        assert_eq!(count_of(&symbols, "area", SymbolKind::Method), 1);
        assert_eq!(find(&symbols, "area").location.line, 9);
        assert_eq!(count_of(&symbols, "Shape", SymbolKind::Class), 1);
        assert_eq!(count_of(&symbols, "Circle", SymbolKind::Class), 1);
    }

    #[test]
    fn test_parse_template_symbols_not_duplicated() {
        let source = "template <typename T>\nclass Box {\npublic:\n    T get() const;\n    T value() const { return raw_; }\nprivate:\n    T raw_;\n};\n\ntemplate <typename T>\nT Box<T>::get() const { return raw_; }\n";
        let symbols = symbols_of(source);

        assert_eq!(count_of(&symbols, "Box", SymbolKind::Class), 1);
        assert_eq!(count_of(&symbols, "get", SymbolKind::Method), 1);
        assert_eq!(count_of(&symbols, "value", SymbolKind::Method), 1);
        assert!(
            symbols.iter().all(|s| s.name != "raw_"),
            "数据成员不应产出符号: {:?}",
            names(&symbols)
        );

        // 模板类与模板方法带 template 修饰符
        assert!(find(&symbols, "Box").modifiers.contains(&"template".to_string()));
        assert!(find(&symbols, "get").modifiers.contains(&"template".to_string()));
    }

    #[test]
    fn test_parse_no_duplicate_symbol_names() {
        // 综合场景：任意 (名称, 类型) 组合都不得重复
        let source = concat!(
            "namespace geom {\n",
            "class Shape {\n",
            "public:\n",
            "    Shape();\n",
            "    virtual ~Shape();\n",
            "    virtual double area() const = 0;\n",
            "    double scaled(double f) const { return area() * f; }\n",
            "    double scaled_out(double f) const;\n",
            "};\n",
            "Shape::Shape() {}\n",
            "Shape::~Shape() {}\n",
            "double Shape::scaled_out(double f) const { return f; }\n",
            "class Circle : public Shape {\n",
            "public:\n",
            "    Circle(double r);\n",
            "    ~Circle() override;\n",
            "    double area() const override;\n",
            "};\n",
            "Circle::Circle(double r) {}\n",
            "Circle::~Circle() {}\n",
            "double Circle::area() const { return 1.0; }\n",
            "template <typename T>\n",
            "class Box {\n",
            "public:\n",
            "    T get() const;\n",
            "    T value() const { return raw_; }\n",
            "};\n",
            "template <typename T>\n",
            "T Box<T>::get() const { return raw_; }\n",
            "enum class Color { RED, GREEN };\n",
            "struct Point { int x; int y; };\n",
            "int free_fn(int x) { return x + 1; }\n",
            "}\n"
        );
        let symbols = symbols_of(source);

        let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        for s in &symbols {
            let key = (s.name.clone(), format!("{:?}", s.kind));
            assert!(seen.insert(key.clone()), "符号重复产出: {:?}", key);
        }

        // 类与命名空间
        assert_eq!(count_of(&symbols, "Shape", SymbolKind::Class), 1);
        assert_eq!(count_of(&symbols, "Circle", SymbolKind::Class), 1);
        assert_eq!(count_of(&symbols, "Box", SymbolKind::Class), 1);
        assert_eq!(count_of(&symbols, "Point", SymbolKind::Struct), 1);
        assert_eq!(count_of(&symbols, "Color", SymbolKind::Enum), 1);
        assert_eq!(count_of(&symbols, "geom", SymbolKind::Module), 1);

        // 自由函数保持 Function，类成员一律 Method
        assert_eq!(find(&symbols, "free_fn").kind, SymbolKind::Function);
        for name in ["scaled", "scaled_out", "area", "value", "get"] {
            assert_eq!(find(&symbols, name).kind, SymbolKind::Method, "{} 应为方法", name);
        }
        assert_eq!(count_of(&symbols, "Shape", SymbolKind::Method), 1);
        assert_eq!(count_of(&symbols, "~Shape", SymbolKind::Method), 1);
        assert_eq!(count_of(&symbols, "Circle", SymbolKind::Method), 1);
    }

    #[test]
    fn test_parse_pointer_reference_return_types() {
        let source = "int* ptr_ret() { return 0; }\nint& ref_ret() { static int x = 1; return x; }\nclass K {\npublic:\n    K& operator+=(const K& o);\n    int* m_ptr();\n};\nint* K::m_ptr() { return 0; }\n";
        let symbols = symbols_of(source);

        assert_eq!(find(&symbols, "ptr_ret").kind, SymbolKind::Function);
        assert_eq!(find(&symbols, "ref_ret").kind, SymbolKind::Function);
        assert_eq!(find(&symbols, "operator+=").kind, SymbolKind::Method);
        assert_eq!(count_of(&symbols, "m_ptr", SymbolKind::Method), 1);
        assert_eq!(find(&symbols, "m_ptr").location.line, 8);
    }

    #[test]
    fn test_parse_data_members_skipped() {
        let source = "class K {\npublic:\n    static int count;\n    double radius_;\n    int (*fp)(int);\n};\n";
        let symbols = symbols_of(source);
        assert_eq!(symbols.len(), 1, "数据成员与函数指针不应产出符号: {:?}", names(&symbols));
        assert_eq!(find(&symbols, "K").kind, SymbolKind::Class);
    }

    #[test]
    fn test_parse_union_mapped_to_struct() {
        let symbols = symbols_of("union Value { int i; float f; };\n");
        // SymbolKind 无 Union 变体，映射为 Struct
        assert_eq!(find(&symbols, "Value").kind, SymbolKind::Struct);
    }

    #[test]
    fn test_parse_alias_and_enum_class() {
        let source = "using Size = unsigned long;\ntypedef int Handle;\nenum class Color { RED };\n";
        let symbols = symbols_of(source);
        assert_eq!(find(&symbols, "Size").kind, SymbolKind::TypeAlias);
        assert_eq!(find(&symbols, "Handle").kind, SymbolKind::TypeAlias);
        assert_eq!(find(&symbols, "Color").kind, SymbolKind::Enum);
    }

    #[test]
    fn test_parse_function_pointer_skipped() {
        let symbols = symbols_of("int (*fp)(int);\nvoid proto(int a);\n");
        assert_eq!(count_of(&symbols, "proto", SymbolKind::Function), 1);
        assert!(symbols.iter().all(|s| s.name != "fp"), "函数指针变量不应产出符号");
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
