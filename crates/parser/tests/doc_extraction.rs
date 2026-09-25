//! signature / doc_comment 提取的逐语言验证
//!
//! **每个语言一个独立 fixture 文件、一个独立测试**（7 种注释形态差异很大，
//! 不做「改完一起测」）。共同钉死的三条：
//!
//! 1. 带参数的声明能取到非空 signature，且**单行、不含函数体**（无 `{`、无换行）
//! 2. 注释与声明之间**隔空行** → `doc_comment` 必须为 `None`
//! 3. 注释与声明之间**隔别的声明** → 后一个符号的 `doc_comment` 必须为 `None`
//! 4. 只有普通注释 / 没有注释 → `None`，不得拿普通注释冒充

use std::path::{Path, PathBuf};

use codeconnect_core::types::{Symbol, SymbolKind};
use codeconnect_parser::r#trait::LanguageParser;

/// fixture 所在目录
fn fixture(name: &str) -> (PathBuf, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("doc")
        .join(name);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读取 fixture 失败 {}: {}", path.display(), e));
    (path, source)
}

/// 按名字 + 类型找符号
fn find(symbols: &[Symbol], name: &str, kind: SymbolKind) -> Symbol {
    symbols
        .iter()
        .find(|s| s.name == name && s.kind == kind)
        .unwrap_or_else(|| {
            let all: Vec<String> = symbols
                .iter()
                .map(|s| format!("{}:{:?}", s.name, s.kind))
                .collect();
            panic!("未找到 {:?} 符号 {}，实得: {:?}", kind, name, all)
        })
        .clone()
}

/// 公共断言：签名单行、不含函数体
fn assert_clean_signature(sym: &Symbol, expected: &str) {
    let sig = sym
        .signature
        .as_deref()
        .unwrap_or_else(|| panic!("{} 的 signature 不应为 None", sym.name));
    assert!(
        !sig.contains('\n') && !sig.contains('\r'),
        "{} 的签名必须单行，实得: {:?}",
        sym.name,
        sig
    );
    assert!(
        !sig.contains('{'),
        "{} 的签名不得含函数体，实得: {:?}",
        sym.name,
        sig
    );
    assert!(
        sig.chars().count() <= 201,
        "{} 的签名超长: {} 字符",
        sym.name,
        sig.chars().count()
    );
    assert_eq!(sig, expected, "{} 的签名不符", sym.name);
}

/// 公共断言：文档注释
fn assert_doc(sym: &Symbol, expected: Option<&str>) {
    assert_eq!(
        sym.doc_comment.as_deref(),
        expected,
        "{} 的 doc_comment 不符",
        sym.name
    );
}

// ============================================================================
// Rust：`///`、`//!`、`#[doc = "..."]`
// ============================================================================

#[test]
fn test_rust_doc_and_signature() {
    let (path, source) = fixture("sample.rs");
    let parser = codeconnect_parser::rust::RustParser::new();
    let tree = parser.parse(&source).expect("解析失败");
    let symbols = parser.extract_symbols(&tree, &source, &path);

    let add = find(&symbols, "add", SymbolKind::Function);
    assert_clean_signature(&add, "pub fn add(a: i32, b: i32) -> i32");
    assert_doc(&add, Some("计算两数之和。 第二行说明。 # 示例"));

    // 普通注释不得冒充文档注释
    let plain = find(&symbols, "plain", SymbolKind::Function);
    assert_clean_signature(&plain, "pub fn plain(x: i32) -> i32");
    assert_doc(&plain, None);

    // 隔空行
    let gap = find(&symbols, "gap", SymbolKind::Function);
    assert_doc(&gap, None);

    // 紧邻的拿到，隔了别的声明的拿不到
    let other = find(&symbols, "other", SymbolKind::Function);
    assert_doc(&other, Some("紧邻 other 的文档"));
    let neighbor = find(&symbols, "neighbor", SymbolKind::Function);
    assert_doc(&neighbor, None);

    // #[doc = "..."]
    let attr = find(&symbols, "attr_doc", SymbolKind::Function);
    assert_doc(&attr, Some("属性形式的文档"));

    // 结构体 / 字段 / impl 方法
    let point = find(&symbols, "Point", SymbolKind::Struct);
    assert_clean_signature(&point, "pub struct Point");
    assert_doc(&point, Some("结构体文档"));
    let field = find(&symbols, "x", SymbolKind::Field);
    assert_clean_signature(&field, "pub x: Meters");
    assert_doc(&field, Some("字段文档"));

    let alias = find(&symbols, "Meters", SymbolKind::TypeAlias);
    assert_clean_signature(&alias, "pub type Meters = f64");
    assert_doc(&alias, Some("长度单位"));
    let len = find(&symbols, "len", SymbolKind::Method);
    assert_clean_signature(&len, "pub fn len(&self) -> usize");
    assert_doc(&len, Some("方法文档"));
}

// ============================================================================
// Java：只认 `/** ... */`（Javadoc）
// ============================================================================

#[test]
fn test_java_doc_and_signature() {
    let (path, source) = fixture("Sample.java");
    let parser = codeconnect_parser::java::JavaParser::new();
    let tree = parser.parse(&source).expect("解析失败");
    let symbols = parser.extract_symbols(&tree, &source, &path);

    let cls = find(&symbols, "Calculator", SymbolKind::Class);
    assert_clean_signature(&cls, "public class Calculator");
    assert_doc(&cls, Some("计算器。 第二行。"));

    let add = find(&symbols, "add", SymbolKind::Method);
    assert_clean_signature(&add, "public int add(int a, int b)");
    assert_doc(&add, Some("两数之和。 @param a 第一个数 @param b 第二个数 @return 和"));

    let plain = find(&symbols, "plain", SymbolKind::Method);
    assert_doc(&plain, None);

    let gap = find(&symbols, "gap", SymbolKind::Method);
    assert_doc(&gap, None);

    let other = find(&symbols, "other", SymbolKind::Method);
    assert_doc(&other, Some("紧邻 other 的文档"));
    let neighbor = find(&symbols, "neighbor", SymbolKind::Method);
    assert_doc(&neighbor, None);
}

// ============================================================================
// TypeScript：`/** ... */`（JSDoc）
// ============================================================================

#[test]
fn test_typescript_doc_and_signature() {
    let (path, source) = fixture("sample.ts");
    let parser = codeconnect_parser::typescript::TypeScriptParser::new();
    let tree = parser.parse(&source).expect("解析失败");
    let symbols = parser.extract_symbols(&tree, &source, &path);

    // export class 的注释挂在 export_statement 上，需要向上爬一层
    let cls = find(&symbols, "Calculator", SymbolKind::Class);
    assert_clean_signature(&cls, "class Calculator");
    assert_doc(&cls, Some("计算器。"));

    let add = find(&symbols, "add", SymbolKind::Method);
    assert_clean_signature(&add, "add(a: number, b: number): number");
    assert_doc(&add, Some("两数之和。 @param a 第一个数"));

    let top_add = find(&symbols, "topAdd", SymbolKind::Function);
    assert_clean_signature(
        &top_add,
        "function topAdd(a: number, b: number): number",
    );
    assert_doc(&top_add, Some("顶层函数文档"));

    let plain = find(&symbols, "plain", SymbolKind::Function);
    assert_doc(&plain, None);

    let gap = find(&symbols, "gap", SymbolKind::Function);
    assert_doc(&gap, None);

    let other = find(&symbols, "other", SymbolKind::Function);
    assert_doc(&other, Some("紧邻 other 的文档"));
    let neighbor = find(&symbols, "neighbor", SymbolKind::Function);
    assert_doc(&neighbor, None);
}

// ============================================================================
// JavaScript：`/** ... */`
// ============================================================================

#[test]
fn test_javascript_doc_and_signature() {
    let (path, source) = fixture("sample.js");
    let parser = codeconnect_parser::javascript::JavaScriptParser::new();
    let tree = parser.parse(&source).expect("解析失败");
    let symbols = parser.extract_symbols(&tree, &source, &path);

    let cls = find(&symbols, "Calculator", SymbolKind::Class);
    assert_clean_signature(&cls, "class Calculator");
    assert_doc(&cls, Some("计算器。"));

    let add = find(&symbols, "add", SymbolKind::Method);
    assert_clean_signature(&add, "add(a, b)");
    assert_doc(&add, Some("两数之和。 @param a 第一个数"));

    let top_add = find(&symbols, "topAdd", SymbolKind::Function);
    assert_clean_signature(&top_add, "function topAdd(a, b)");
    assert_doc(&top_add, Some("顶层函数文档"));

    let plain = find(&symbols, "plain", SymbolKind::Function);
    assert_doc(&plain, None);

    let gap = find(&symbols, "gap", SymbolKind::Function);
    assert_doc(&gap, None);

    let other = find(&symbols, "other", SymbolKind::Function);
    assert_doc(&other, Some("紧邻 other 的文档"));
    let neighbor = find(&symbols, "neighbor", SymbolKind::Function);
    assert_doc(&neighbor, None);
}

// ============================================================================
// C：`/** ... */` 与 `///`
// ============================================================================

#[test]
fn test_c_doc_and_signature() {
    let (path, source) = fixture("sample.c");
    let parser = codeconnect_parser::c::CParser::new();
    let tree = parser.parse(&source).expect("解析失败");
    let symbols = parser.extract_symbols(&tree, &source, &path);

    let add = find(&symbols, "add", SymbolKind::Function);
    assert_clean_signature(&add, "int add(int a, int b)");
    assert_doc(&add, Some("两数之和。 @param a 第一个数 @param b 第二个数"));

    // `///` 行文档
    let triple = find(&symbols, "triple", SymbolKind::Function);
    assert_clean_signature(&triple, "void triple(int x)");
    assert_doc(&triple, Some("行文档函数"));

    let plain = find(&symbols, "plain", SymbolKind::Function);
    assert_doc(&plain, None);

    let gap = find(&symbols, "gap", SymbolKind::Function);
    assert_doc(&gap, None);

    let other = find(&symbols, "other", SymbolKind::Function);
    assert_doc(&other, Some("紧邻 other 的文档"));
    let neighbor = find(&symbols, "neighbor", SymbolKind::Function);
    assert_doc(&neighbor, None);
}

// ============================================================================
// C++：`/** ... */` 与 `///`
// ============================================================================

#[test]
fn test_cpp_doc_and_signature() {
    let (path, source) = fixture("sample.cpp");
    let parser = codeconnect_parser::cpp::CppParser::new();
    let tree = parser.parse(&source).expect("解析失败");
    let symbols = parser.extract_symbols(&tree, &source, &path);

    let cls = find(&symbols, "Shape", SymbolKind::Class);
    assert_clean_signature(&cls, "class Shape");
    assert_doc(&cls, Some("图形基类。"));

    // 类内纯虚函数：签名截到 `= 0`，不含函数体
    let area = find(&symbols, "area", SymbolKind::Method);
    assert_clean_signature(&area, "virtual double area() const = 0");
    assert_doc(&area, Some("面积。"));

    // `///` 行文档
    let scaled = find(&symbols, "scaled", SymbolKind::Method);
    assert_clean_signature(&scaled, "double scaled(double f) const");
    assert_doc(&scaled, Some("缩放。"));

    let plain = find(&symbols, "plain", SymbolKind::Method);
    assert_doc(&plain, None);

    let gap = find(&symbols, "gap", SymbolKind::Method);
    assert_doc(&gap, None);

    let add = find(&symbols, "add", SymbolKind::Function);
    assert_clean_signature(&add, "int add(int a, int b)");
    assert_doc(&add, Some("自由函数文档"));

    let other = find(&symbols, "other", SymbolKind::Function);
    assert_doc(&other, Some("紧邻 other 的文档"));
    let neighbor = find(&symbols, "neighbor", SymbolKind::Function);
    assert_doc(&neighbor, None);
}

// ============================================================================
// C#：`///` 与 `/** ... */`
// ============================================================================

#[test]
fn test_csharp_doc_and_signature() {
    let (path, source) = fixture("Sample.cs");
    let parser = codeconnect_parser::csharp::CSharpParser::new();
    let tree = parser.parse(&source).expect("解析失败");
    let symbols = parser.extract_symbols(&tree, &source, &path);

    let cls = find(&symbols, "Calculator", SymbolKind::Class);
    assert_clean_signature(&cls, "public class Calculator");
    assert_doc(&cls, Some("计算器。 第二行。"));

    // `///` XML 文档
    let add = find(&symbols, "Add", SymbolKind::Method);
    assert_clean_signature(&add, "public int Add(int a, int b)");
    assert_doc(&add, Some("<summary> 两数之和。 </summary>"));

    // `/** ... */` 块文档
    let block = find(&symbols, "BlockDoc", SymbolKind::Method);
    assert_clean_signature(&block, "public int BlockDoc(int x)");
    assert_doc(&block, Some("块文档方法。"));

    let plain = find(&symbols, "Plain", SymbolKind::Method);
    assert_doc(&plain, None);

    let gap = find(&symbols, "Gap", SymbolKind::Method);
    assert_doc(&gap, None);

    let other = find(&symbols, "Other", SymbolKind::Method);
    assert_doc(&other, Some("紧邻 Other 的文档"));
    let neighbor = find(&symbols, "Neighbor", SymbolKind::Method);
    assert_doc(&neighbor, None);
}
