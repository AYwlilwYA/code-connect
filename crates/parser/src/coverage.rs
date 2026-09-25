//! 声明节点覆盖度审计（spec B）与 grammar 驱动的完整性检查（spec C）
//!
//! 关键词：枚举量 enumerator 静默缺口 覆盖度 声明节点 declaration node kinds 豁免名单
//!
//! # 为什么存在这个模块
//!
//! 历史上三次事故（索引过期、限定名不建边、枚举量不索引）的共同致命点不是「漏了某类符号」，
//! 而是「**漏了却装成「没有」**」—— 工具返回 0 时没有任何警告，而 0 是一个看起来有意义的答案。
//!
//! - **B（[`audit`]）**：解析完一个文件后，把「AST 里实际存在的声明类节点种类」与
//!   「真的产出了符号的种类」做差集。差集非空就上浮到索引统计与工具响应，绝不静默。
//! - **C（本模块的 `#[cfg(test)] tests`）**：每个 grammar 的声明类节点必须**逐项表态**
//!   —— 要么被 `symbols.scm` 捕获（[`DeclStatus::Required`]），要么明示为已知缺口
//!   （[`DeclStatus::KnownGap`]），要么显式豁免并写明理由（[`DeclStatus::Exempt`]）。
//!   grammar 升级新增节点、或删掉一条捕获时，测试必须失败。
//!
//! # 范围控制
//!
//! 只覆盖**解析层可见的事实**：节点种类 vs 产出种类。
//! 不判断「某个具体的名字有没有被引用」—— 那是引用解析的事，越界会失控。

use std::collections::{BTreeMap, BTreeSet, HashMap};

use codeconnect_core::types::Symbol;
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Tree};

// ============================================================================
// 声明节点表态表
// ============================================================================

/// 一个声明类节点在符号索引里的处理态度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclStatus {
    /// 必须被 `symbols.scm` 捕获。
    ///
    /// 捕获一旦消失（被注释掉、被改错名字），C 项的测试立刻失败 —— 这是防回归的锚点。
    Required,
    /// 已知缺口：这个节点声明了项目级可见的名字，但当前查询没有对应模式。
    ///
    /// 审计会**照常上报**（不静默）—— 因为「查不到」正是会被误读成「不存在」的那一类。
    /// 理由写在这里，是为了让下一个读代码的人知道「这是已知的，不是刚坏的」。
    KnownGap(&'static str),
    /// 显式豁免：故意不索引，且**无需上报**。
    ///
    /// 只在两种情况下用：(1) 该节点本身不是「有名字的声明」（关键字、类型说明符、容器）；
    /// (2) 该名字的可见性局限在局部作用域，或已由别的查询（如 `imports.scm`）单独索引。
    Exempt(&'static str),
}

/// 表态表里的一项
#[derive(Debug, Clone, Copy)]
pub struct DeclKind {
    /// grammar 里的节点类型名
    pub kind: &'static str,
    /// 处理态度
    pub status: DeclStatus,
}

/// 构造表态表条目的简写
const fn req(kind: &'static str) -> DeclKind {
    DeclKind {
        kind,
        status: DeclStatus::Required,
    }
}
/// 已知缺口的简写
const fn gap(kind: &'static str, why: &'static str) -> DeclKind {
    DeclKind {
        kind,
        status: DeclStatus::KnownGap(why),
    }
}
/// 豁免的简写
const fn exempt(kind: &'static str, why: &'static str) -> DeclKind {
    DeclKind {
        kind,
        status: DeclStatus::Exempt(why),
    }
}

/// 已实现解析器的语言清单（kotlin 解析器尚未启用，故不在此列）
pub const AUDITED_LANGUAGES: &[&str] =
    &["rust", "typescript", "javascript", "java", "csharp", "c", "cpp"];

// ---------------------------------------------------------------------------
// Rust
// ---------------------------------------------------------------------------
const RUST_DECL_KINDS: &[DeclKind] = &[
    req("function_item"),
    req("struct_item"),
    req("trait_item"),
    req("enum_item"),
    req("impl_item"),
    req("type_item"),
    req("macro_definition"),
    req("mod_item"),
    req("let_declaration"),
    req("field_declaration"),
    req("enum_variant"),
    gap("function_signature_item", "trait 里的方法签名未建索引（只有 impl 中的实现被索引）"),
    gap("const_item", "Rust const 定义未建索引"),
    gap("static_item", "Rust static 定义未建索引"),
    gap("union_item", "Rust union 定义未建索引"),
    exempt("use_declaration", "use 语句不是符号，已由 imports.scm 单独索引"),
    exempt("extern_crate_declaration", "extern crate 不是符号，已由 imports.scm 单独索引"),
    exempt("foreign_mod_item", "extern 块本身不是符号，块内声明由各自模式处理"),
    exempt("attribute_item", "属性 #[...] 是元数据，不是符号"),
    exempt("inner_attribute_item", "内部属性 #![...] 是元数据，不是符号"),
    exempt("mutable_specifier", "mut 关键字，不是声明"),
    exempt("fragment_specifier", "macro_rules! 的片段说明符 $x:expr，不是符号"),
];

// ---------------------------------------------------------------------------
// TypeScript
// ---------------------------------------------------------------------------
const TYPESCRIPT_DECL_KINDS: &[DeclKind] = &[
    req("class_declaration"),
    req("interface_declaration"),
    req("enum_declaration"),
    req("type_alias_declaration"),
    req("function_declaration"),
    req("method_definition"),
    req("variable_declarator"),
    // TS 的枚举成员没有独立声明节点，@enumerator 挂在 enum_body 的 name 字段上，
    // 因此这里把容器 enum_body 本身当作该成员的声明节点来表态
    req("enum_body"),
    gap("abstract_class_declaration", "abstract class 用独立节点类型，未捕获"),
    gap("abstract_method_signature", "抽象方法签名未建索引"),
    gap("method_signature", "接口/类型字面量的方法签名未建索引"),
    gap("property_signature", "接口/类型字面量的属性签名未建索引"),
    gap("function_signature", "重载签名与 declare function 未建索引"),
    gap("generator_function_declaration", "generator 函数（function*）用独立节点类型，未捕获"),
    gap("public_field_definition", "类字段（含 private #field）未建索引"),
    exempt("ambient_declaration", "declare 块是容器，块内声明由各自模式处理"),
    exempt("export_specifier", "export { a } 的成员表不是声明，被指向的符号另有声明处"),
    exempt("import_specifier", "import 成员表不是符号，已由 imports.scm 索引"),
    exempt("lexical_declaration", "let/const 语句是容器，其 variable_declarator 已被捕获"),
    exempt("variable_declaration", "var 语句是容器，其 variable_declarator 已被捕获"),
    exempt("call_signature", "类型字面量里的调用签名没有名字，不是符号"),
    exempt("construct_signature", "类型字面量里的构造签名没有名字，不是符号"),
    exempt("index_signature", "类型字面量里的索引签名没有名字，不是符号"),
];

// ---------------------------------------------------------------------------
// JavaScript
// ---------------------------------------------------------------------------
const JAVASCRIPT_DECL_KINDS: &[DeclKind] = &[
    req("class_declaration"),
    req("function_declaration"),
    req("method_definition"),
    req("variable_declarator"),
    gap("generator_function_declaration", "generator 函数（function*）用独立节点类型，未捕获"),
    gap("field_definition", "类字段（class field）未建索引"),
    exempt("export_specifier", "export { a } 的成员表不是声明，被指向的符号另有声明处"),
    exempt("import_specifier", "import 成员表不是符号，已由 imports.scm 索引"),
    exempt("lexical_declaration", "let/const 语句是容器，其 variable_declarator 已被捕获"),
    exempt("variable_declaration", "var 语句是容器，其 variable_declarator 已被捕获"),
];

// ---------------------------------------------------------------------------
// Java
// ---------------------------------------------------------------------------
const JAVA_DECL_KINDS: &[DeclKind] = &[
    req("class_declaration"),
    req("interface_declaration"),
    req("enum_declaration"),
    req("method_declaration"),
    req("constructor_declaration"),
    req("field_declaration"),
    req("annotation_type_declaration"),
    req("enum_constant"),
    exempt("enum_body", "枚举体是容器，其成员 enum_constant 已被捕获"),
    gap("constant_declaration", "接口常量（interface 内字段）未建索引"),
    gap("record_declaration", "record 类型（Java 16+）用独立节点类型，未捕获"),
    gap("compact_constructor_declaration", "record 的紧凑构造器未捕获"),
    gap("annotation_type_element_declaration", "注解类型元素（@interface 的方法）未建索引"),
    gap("module_declaration", "Java 9 module-info 声明未索引"),
    exempt("package_declaration", "包声明不是符号，包名由 infer_package 提供"),
    exempt("import_declaration", "import 不是符号，已由 imports.scm 索引"),
    exempt("local_variable_declaration", "方法内局部变量不建索引，作用域局限在方法内"),
    exempt("variable_declarator", "字段名节点，所属 field_declaration 已建符号，名字已可达"),
];

// ---------------------------------------------------------------------------
// C#
// ---------------------------------------------------------------------------
const CSHARP_DECL_KINDS: &[DeclKind] = &[
    req("class_declaration"),
    req("interface_declaration"),
    req("struct_declaration"),
    req("enum_declaration"),
    req("method_declaration"),
    req("property_declaration"),
    req("field_declaration"),
    req("namespace_declaration"),
    req("enum_member_declaration"),
    gap("constructor_declaration", "构造函数未捕获（C# 构造函数无返回类型，查询未覆盖）"),
    gap("destructor_declaration", "析构函数未捕获"),
    gap("delegate_declaration", "委托类型未捕获"),
    gap("event_declaration", "事件未捕获"),
    gap("event_field_declaration", "事件字段未捕获"),
    gap("indexer_declaration", "索引器 this[...] 未捕获"),
    gap("operator_declaration", "运算符重载未捕获"),
    gap("conversion_operator_declaration", "转换运算符未捕获"),
    gap("record_declaration", "record 类型用独立节点类型，未捕获"),
    gap("file_scoped_namespace_declaration", "file-scoped namespace（C# 10）未捕获"),
    exempt("variable_declaration", "局部/字段变量声明是容器，字段由 field_declaration 模式覆盖"),
    exempt("variable_declarator", "字段名节点，所属 field_declaration 已建符号，名字已可达"),
    exempt("accessor_declaration", "get/set 访问器是属性的一部分，不单独建符号"),
    exempt("catch_declaration", "catch 变量作用域局限在 catch 块内"),
    exempt("array_rank_specifier", "数组秩说明符 [,] 不是声明"),
    exempt("attribute_target_specifier", "属性目标说明符（assembly: 等）不是声明"),
    exempt("explicit_interface_specifier", "显式接口前缀不是声明"),
];

// ---------------------------------------------------------------------------
// C
// ---------------------------------------------------------------------------
const C_DECL_KINDS: &[DeclKind] = &[
    req("function_definition"),
    req("struct_specifier"),
    req("union_specifier"),
    req("enum_specifier"),
    req("type_definition"),
    req("preproc_def"),
    req("preproc_function_def"),
    req("enumerator"),
    gap("declaration", "文件作用域的变量/函数原型声明未建索引"),
    gap("field_declaration", "结构体成员未建索引"),
    exempt("parameter_declaration", "函数参数不单独建符号"),
    exempt("sized_type_specifier", "类型说明符不是声明"),
    exempt("storage_class_specifier", "static/extern 存储类说明符不是声明"),
    exempt("attribute_declaration", "GNU __attribute__ 不是符号"),
    exempt("attribute_specifier", "GNU __attribute__ 不是符号"),
    exempt("macro_type_specifier", "宏体里的类型说明符不是声明"),
];

// ---------------------------------------------------------------------------
// C++
// ---------------------------------------------------------------------------
const CPP_DECL_KINDS: &[DeclKind] = &[
    req("class_specifier"),
    req("struct_specifier"),
    req("union_specifier"),
    req("enum_specifier"),
    req("namespace_definition"),
    req("function_definition"),
    req("field_declaration"),
    req("declaration"),
    req("alias_declaration"),
    req("type_definition"),
    req("preproc_def"),
    req("preproc_function_def"),
    req("enumerator"),
    gap("concept_definition", "C++20 concept 未索引"),
    gap("namespace_alias_definition", "namespace 别名未索引"),
    exempt("template_declaration", "template 头是修饰，其内部实体各自有模式"),
    exempt("access_specifier", "public/private/protected 关键字不是声明"),
    exempt("friend_declaration", "friend 只是可见性授予，不引入新符号"),
    exempt("using_declaration", "using namespace / using std::x 是引入而非声明"),
    exempt("static_assert_declaration", "static_assert 不是符号"),
    exempt("nested_namespace_specifier", "namespace A::B 中的嵌套部分不是独立声明"),
    exempt("explicit_function_specifier", "explicit 关键字不是声明"),
    exempt("virtual_specifier", "override/final 关键字不是声明"),
    exempt("throw_specifier", "异常规格说明不是声明"),
    exempt("lambda_capture_specifier", "lambda 捕获列表不是声明"),
    exempt("parameter_declaration", "函数参数不单独建符号"),
    exempt("optional_parameter_declaration", "函数参数不单独建符号"),
    exempt("variadic_parameter_declaration", "函数参数不单独建符号"),
    exempt("type_parameter_declaration", "模板类型参数不单独建符号"),
    exempt("optional_type_parameter_declaration", "模板类型参数不单独建符号"),
    exempt("template_template_parameter_declaration", "模板模板参数不单独建符号"),
    exempt("variadic_type_parameter_declaration", "模板类型参数不单独建符号"),
    exempt("placeholder_type_specifier", "auto 占位类型说明符不是声明"),
    exempt("sized_type_specifier", "类型说明符不是声明"),
    exempt("storage_class_specifier", "static/extern 存储类说明符不是声明"),
    exempt("attribute_declaration", "属性声明不是符号"),
    exempt("attribute_specifier", "属性说明符不是符号"),
];

/// 取某语言的声明节点表态表
///
/// 返回 `None` 表示该语言没有解析器（如 kotlin），审计与完整性检查都跳过。
pub fn declaration_kinds(language: &str) -> Option<&'static [DeclKind]> {
    match language {
        "rust" => Some(RUST_DECL_KINDS),
        "typescript" => Some(TYPESCRIPT_DECL_KINDS),
        "javascript" => Some(JAVASCRIPT_DECL_KINDS),
        "java" => Some(JAVA_DECL_KINDS),
        "csharp" => Some(CSHARP_DECL_KINDS),
        "c" => Some(C_DECL_KINDS),
        "cpp" => Some(CPP_DECL_KINDS),
        _ => None,
    }
}

/// 取某语言的 `symbols.scm` 文本（编译期嵌入，与解析器实际使用的一致）
pub fn symbols_scm(language: &str) -> Option<&'static str> {
    match language {
        "rust" => Some(crate::query_loader::load_rust_queries().symbols),
        "typescript" => Some(crate::query_loader::load_typescript_queries().symbols),
        "javascript" => Some(crate::query_loader::load_javascript_queries().symbols),
        "java" => Some(crate::query_loader::load_java_queries().symbols),
        "csharp" => Some(crate::query_loader::load_csharp_queries().symbols),
        "c" => Some(crate::query_loader::load_c_queries().symbols),
        "cpp" => Some(crate::query_loader::load_cpp_queries().symbols),
        _ => None,
    }
}

// ============================================================================
// .scm 文本 → 节点名集合（C 项的「怎么判断被捕获」）
// ============================================================================

/// 从 `symbols.scm` 文本里提取所有**作为模式出现**的节点类型名
///
/// 选择「解析 .scm 文本」而不是「从 Query 的 capture 列表反推」，理由：
/// 1. **零依赖**：不需要构造 tree-sitter `Language` / 编译 `Query`，任何语言在任何
///    上下文都能查（包括 .scm 因别的原因编译失败时）。
/// 2. **正是我们要断言的事实**：「这份词汇表里到底写了哪些节点类型」——
///    capture 列表反推只能得到 capture 名，得不到节点名。
///
/// 扫描规则（tree-sitter query 语法的一个子集，足够覆盖本项目全部 8 份 .scm）：
/// - `;` 之后到行尾是注释，跳过；
/// - `"..."` 是字符串字面量（`#match?` 等谓词的参数），跳过；
/// - `(` 之后的第一个标识符是节点类型名；若紧跟 `:` 说明那是字段名（如 `name:`），跳过；
/// - `_` 是通配符，跳过；`.` 是锚点，跳过。
pub fn query_node_names(scm: &str) -> BTreeSet<String> {
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'!' || b == b'?'
    }

    let bytes = scm.as_bytes();
    let mut names = BTreeSet::new();
    let mut i = 0usize;

    while i < bytes.len() {
        match bytes[i] {
            b';' => {
                // 行注释
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'"' => {
                // 字符串字面量，处理 \" 转义
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'(' => {
                i += 1;
                // 跳过空白与匿名节点锚点 "."
                loop {
                    while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == b'.' {
                        i += 1;
                        continue;
                    }
                    break;
                }
                let start = i;
                while i < bytes.len() && is_ident_byte(bytes[i]) {
                    i += 1;
                }
                if i == start {
                    // `(` 后面不是标识符（如 `(#eq? ...)`）
                    continue;
                }
                let ident = &scm[start..i];
                // 跳过标识符后的空白，判断是否为字段名 `name:`
                let mut j = i;
                while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
                    j += 1;
                }
                let is_field_name = j < bytes.len() && bytes[j] == b':';
                if !is_field_name && ident != "_" && ident != "." {
                    names.insert(ident.to_string());
                }
            }
            _ => i += 1,
        }
    }

    names
}

// ============================================================================
// B：覆盖度审计
// ============================================================================

/// 单个节点种类的覆盖情况
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindCoverage {
    /// grammar 节点类型名
    pub kind: String,
    /// 本文件中该种类的节点个数
    pub nodes: u64,
    /// 由该种类节点产出的符号个数（审计时恒为 0，保留字段供调用方对齐语义）
    pub symbols: u64,
    /// `symbols.scm` 里是否有该节点类型的模式
    pub captured_by_query: bool,
    /// 表态：`not_captured`（查询漏了）/ `captured_but_no_symbol`（捕获了但没产出）/ `known_gap`
    pub status: String,
    /// 表态理由（仅 `known_gap` 有）
    pub reason: Option<String>,
}

/// 单个文件的覆盖度审计结果
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileCoverage {
    /// AST 里出现过的声明类节点种类（升序）
    pub declaration_kinds_seen: Vec<String>,
    /// 其中有节点、却一个符号都没产出的种类（升序）—— 这些查询会返回「假 0」
    pub kinds_without_symbols: Vec<String>,
    /// 逐种类的明细
    pub details: Vec<KindCoverage>,
    /// 说人话的告警（给 AI 调用方看，直接写在它会误读的地方）
    pub note: Option<String>,
}

impl FileCoverage {
    /// 是否存在静默缺口
    pub fn has_gaps(&self) -> bool {
        !self.kinds_without_symbols.is_empty()
    }
}

/// note 文案里最多点名的种类数（明细本身不截断，保证索引级汇总不丢数）
const MAX_NOTE_KINDS: usize = 6;

/// 一个声明节点的位置范围：(节点种类, 起点(行,列), 终点(行,列))，行列均 1-based
type DeclRange = (&'static str, (u64, u64), (u64, u64));

/// 审计单个文件：AST 里有的声明节点种类 vs 真的产出了符号的种类
///
/// # 判定方法
///
/// 1. 走一遍 AST，收集所有属于表态表的声明类节点，记下各自的**起始行列**与计数；
///    `Exempt` 的条目不参与（它们本就不该有符号）。
/// 2. 对每个符号，取它的起始行列，凡是**起始位置与之相同的声明节点**都算「产出了符号」；
///    若某个种类没有任何符号精确落在它的起始位置，再退一步看**有没有符号落在它的范围内**
///    （应对 TS 这类「枚举成员没有独立声明节点」的 grammar，见下方注释）。
/// 3. 有节点、却两种对法都没对上符号的种类 → 记入 `kinds_without_symbols` 并上浮。
///
/// 用位置对齐而非「哪条 capture 触发的」是因为：不同语言取名字的节点深浅不同
/// （C# 字段的位置落在内层 `variable_declaration` 上，TS 枚举成员落在 property_identifier 上），
/// 位置对得上即可视作同一处声明。
pub fn audit(language: &str, tree: &Tree, symbols: &[Symbol]) -> FileCoverage {
    let table = match declaration_kinds(language) {
        Some(t) => t,
        None => return FileCoverage::default(),
    };

    // 表态表 → 查询用的哈希表（跳过豁免项）
    let mut audited: HashMap<&'static str, &'static DeclKind> = HashMap::new();
    for entry in table {
        if !matches!(entry.status, DeclStatus::Exempt(_)) {
            audited.insert(entry.kind, entry);
        }
    }
    if audited.is_empty() {
        return FileCoverage::default();
    }

    // 第一步：走 AST，记下每个声明类节点的起始位置与范围
    // 位置统一转成 1-based，与 Symbol.location 同一坐标系
    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut by_start_pos: HashMap<(usize, usize), Vec<&'static str>> = HashMap::new();
    // (节点种类, 起点(行,列), 终点(行,列))，均为 1-based
    let mut ranges: Vec<DeclRange> = Vec::new();

    let mut stack: Vec<Node> = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        // 匿名节点（标点等）不可能是声明，直接跳过
        let entry = audited.get(node.kind()).filter(|_| node.is_named());
        if let Some(entry) = entry {
            *counts.entry(entry.kind).or_insert(0) += 1;
            let start = node.start_position();
            let end = node.end_position();
            by_start_pos
                .entry((start.row, start.column))
                .or_default()
                .push(entry.kind);
            ranges.push((
                entry.kind,
                (start.row as u64 + 1, start.column as u64 + 1),
                (end.row as u64 + 1, end.column as u64 + 1),
            ));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }

    if counts.is_empty() {
        return FileCoverage::default();
    }

    // 第二步之一：按符号起始位置精确对齐
    let mut producing: BTreeSet<&'static str> = BTreeSet::new();
    let mut symbol_positions: Vec<(u64, u64)> = Vec::with_capacity(symbols.len());
    for symbol in symbols {
        if symbol.location.line == 0 {
            continue;
        }
        symbol_positions.push((symbol.location.line, symbol.location.column));
        let key = (
            symbol.location.line.saturating_sub(1) as usize,
            symbol.location.column.saturating_sub(1) as usize,
        );
        if let Some(kinds) = by_start_pos.get(&key) {
            producing.extend(kinds.iter().copied());
        }
    }

    // 第二步之二：精确对不上的种类，再看有没有符号落在它的范围内
    //
    // 为什么需要这一级：tree-sitter-typescript 的枚举成员没有自己的声明节点，
    // 成员名字直接挂在 enum_body 的 name 字段上 —— 符号位置在成员名上，
    // 而「声明节点」enum_body 从 `{` 开始，精确对齐永远对不上。
    // 容器节点「内部有符号」即视为该种类确实产出了符号。
    symbol_positions.sort_unstable();
    for (kind, start, end) in &ranges {
        if producing.contains(kind) {
            continue;
        }
        let idx = symbol_positions.partition_point(|p| p < start);
        if idx < symbol_positions.len() && symbol_positions[idx] <= *end {
            producing.insert(kind);
        }
    }

    // 第三步：做差集
    let scm_names = symbols_scm(language).map(query_node_names).unwrap_or_default();
    let mut details: Vec<KindCoverage> = Vec::new();

    for (kind, nodes) in &counts {
        if producing.contains(kind) {
            continue;
        }
        let captured = scm_names.contains(*kind);
        let (status, reason) = match audited.get(kind).map(|e| &e.status) {
            Some(DeclStatus::KnownGap(why)) => ("known_gap".to_string(), Some((*why).to_string())),
            _ if captured => ("captured_but_no_symbol".to_string(), None),
            _ => ("not_captured".to_string(), None),
        };
        details.push(KindCoverage {
            kind: (*kind).to_string(),
            nodes: *nodes,
            symbols: 0,
            captured_by_query: captured,
            status,
            reason,
        });
    }

    let declaration_kinds_seen: Vec<String> = counts.keys().map(|k| (*k).to_string()).collect();
    let kinds_without_symbols: Vec<String> =
        details.iter().map(|d| d.kind.clone()).collect::<Vec<_>>();

    let note = build_note(&details);

    FileCoverage {
        declaration_kinds_seen,
        kinds_without_symbols,
        details,
        note,
    }
}

/// 生成给人（AI 调用方）看的告警文案 —— 必须在它会误读 0 的地方直接写明
fn build_note(details: &[KindCoverage]) -> Option<String> {
    if details.is_empty() {
        return None;
    }

    let parts: Vec<String> = details
        .iter()
        .take(MAX_NOTE_KINDS)
        .map(|d| {
            let tail = match d.status.as_str() {
                "known_gap" => format!(
                    "（已知缺口：{}）",
                    d.reason.as_deref().unwrap_or("未说明")
                ),
                "captured_but_no_symbol" => {
                    "（查询里有该节点的模式，但本文件没产出符号，多半是解析器只认其中一部分写法）".to_string()
                }
                _ => "（symbols.scm 里没有该节点类型的模式）".to_string(),
            };
            format!("{} 个 {} 节点、0 个符号{}", d.nodes, d.kind, tail)
        })
        .collect();

    let more = if details.len() > MAX_NOTE_KINDS {
        format!("，另有 {} 种未列出（见 details）", details.len() - MAX_NOTE_KINDS)
    } else {
        String::new()
    };

    Some(format!(
        "本文件存在「有声明节点、但没有产出符号」的种类：{}{}。\
         对这类名字的查询会返回 0，那是「没建索引」而不是「不存在」——不要据此判断可以安全改删。",
        parts.join("；"),
        more
    ))
}

/// 把单文件的覆盖度结果并入全局统计
///
/// 由索引器调用，避免 full_indexer 里重复实现合并逻辑。
pub fn merge_into(
    acc: &mut BTreeMap<String, KindGapSummary>,
    file_path: &str,
    coverage: &FileCoverage,
) {
    for detail in &coverage.details {
        let entry = acc
            .entry(detail.kind.clone())
            .or_insert_with(|| KindGapSummary {
                kind: detail.kind.clone(),
                node_count: 0,
                files: 0,
                captured_by_query: detail.captured_by_query,
                status: detail.status.clone(),
                reason: detail.reason.clone(),
                sample_files: Vec::new(),
            });
        entry.node_count += detail.nodes;
        entry.files += 1;
        if entry.sample_files.len() < MAX_SAMPLE_FILES {
            entry.sample_files.push(file_path.to_string());
        }
    }
}

/// 每个缺口种类最多保留的样例文件数
pub const MAX_SAMPLE_FILES: usize = 5;

/// 全项目范围内「某种声明节点没产出符号」的汇总
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindGapSummary {
    /// grammar 节点类型名
    pub kind: String,
    /// 涉及节点总数
    pub node_count: u64,
    /// 涉及文件数
    pub files: u64,
    /// `symbols.scm` 里是否有该节点类型的模式
    pub captured_by_query: bool,
    /// `not_captured` / `captured_but_no_symbol` / `known_gap`
    pub status: String,
    /// 表态理由（仅 `known_gap` 有）
    pub reason: Option<String>,
    /// 样例文件（最多 [`MAX_SAMPLE_FILES`] 个）
    pub sample_files: Vec<String>,
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Language;

    /// 判断一个节点类型名「看起来像能产生声明的节点」
    ///
    /// 这是 C 项里「grammar 升级新增节点必须表态」的抓手：grammar 加了
    /// `foo_declaration` 之类的新节点而没人往表态表里写一行，测试就会失败。
    ///
    /// 注意这是**必要而非充分**条件：它按命名规律圈出候选集，命名风格完全
    /// 例外的节点（如 Java 的 `enum_constant`）由 [`EXTRA_CANDIDATES`] 显式补上。
    /// 反过来，圈进来的未必真是声明（如 `type_specifier`），这些必须以
    /// [`DeclStatus::Exempt`] 表态 —— 这正是「不许沉默」的用意。
    fn looks_like_declaration_candidate(kind: &str) -> bool {
        const SUFFIXES: &[&str] = &[
            "_declaration",
            "_definition",
            "_specifier",
            "_item",
            "_signature",
            "_variant",
            "_constant",
            "_entry",
        ];
        SUFFIXES.iter().any(|s| kind.ends_with(s)) || EXTRA_CANDIDATES.contains(&kind)
    }

    /// 命名规律覆盖不到、但确实是声明的节点名，手工补录
    const EXTRA_CANDIDATES: &[&str] = &[
        "enumerator",        // C/C++ 枚举量
        "enum_body",         // TS/Java 枚举体（枚举量挂在它下面）
        "method_definition", // JS/TS 方法定义
        "declaration",       // C/C++ 通用声明
        "preproc_def",       // C/C++ 宏定义
        "preproc_function_def",
        "variable_declarator", // TS/JS 变量声明符
    ];

    /// 取各语言的 tree-sitter Language（与解析器实际使用的一致）
    fn grammar_of(language: &str) -> Language {
        match language {
            "rust" => Language::new(tree_sitter_rust::LANGUAGE),
            "typescript" => Language::new(tree_sitter_typescript::LANGUAGE_TYPESCRIPT),
            "javascript" => Language::new(tree_sitter_javascript::LANGUAGE),
            "java" => Language::new(tree_sitter_java::LANGUAGE),
            "csharp" => Language::new(tree_sitter_c_sharp::LANGUAGE),
            "c" => Language::new(tree_sitter_c::LANGUAGE),
            "cpp" => Language::new(tree_sitter_cpp::LANGUAGE),
            other => panic!("未登记的语法: {}", other),
        }
    }

    /// 枚举 grammar 里所有「具名且非超类型」的节点类型名
    fn grammar_named_kinds(language: &str) -> BTreeSet<String> {
        let lang = grammar_of(language);
        let mut out = BTreeSet::new();
        for id in 0..lang.node_kind_count() as u16 {
            // 超类型（grammar 里的抽象父类）不会作为实际节点出现，排除掉
            let name = lang.node_kind_for_id(id).filter(|name| {
                lang.node_kind_is_named(id) && !lang.node_kind_is_supertype(id) && !name.is_empty()
            });
            if let Some(name) = name {
                out.insert(name.to_string());
            }
        }
        out
    }

    /// C-1：表态表里的每一项都必须自洽
    ///
    /// - `Required` 必须真的被 `symbols.scm` 捕获 —— **删掉一条捕获，这里就红**
    /// - `KnownGap` 必须真的**没**被捕获 —— 一旦补上了捕获，这里提醒改成 `Required`
    /// - 所有条目都必须是该 grammar 里真实存在的节点类型（防拼写错误 / 防 grammar 改名）
    #[test]
    fn test_declaration_kinds_match_symbols_scm() {
        let mut failures: Vec<String> = Vec::new();

        for &language in AUDITED_LANGUAGES {
            let scm = symbols_scm(language).expect("语言应有 symbols.scm");
            let captured = query_node_names(scm);
            let grammar = grammar_named_kinds(language);

            for entry in declaration_kinds(language).expect("语言应有表态表") {
                if !grammar.contains(entry.kind) {
                    failures.push(format!(
                        "[{}] 表态表里的 `{}` 在该 grammar 里不存在（拼写错误或 grammar 已改名）",
                        language, entry.kind
                    ));
                    continue;
                }
                let is_captured = captured.contains(entry.kind);
                match entry.status {
                    DeclStatus::Required if !is_captured => failures.push(format!(
                        "[{}] `{}` 标为 Required，但 symbols.scm 里已经没有捕获它的模式了",
                        language, entry.kind
                    )),
                    DeclStatus::KnownGap(_) if is_captured => failures.push(format!(
                        "[{}] `{}` 标为 KnownGap，但 symbols.scm 现在捕获它了 —— 请改成 Required",
                        language, entry.kind
                    )),
                    _ => {}
                }
            }
        }

        assert!(
            failures.is_empty(),
            "声明节点表态表与 symbols.scm 不一致：\n{}",
            failures.join("\n")
        );
    }

    /// C-2：grammar 里所有「像声明」的节点都必须表态
    ///
    /// grammar 升级新增 `foo_declaration` 之类节点而没人往表里写一行 → 测试失败。
    #[test]
    fn test_every_declaration_candidate_kind_is_classified() {
        let mut unclassified: Vec<String> = Vec::new();

        for &language in AUDITED_LANGUAGES {
            let table = declaration_kinds(language).expect("语言应有表态表");
            let classified: BTreeSet<&str> = table.iter().map(|e| e.kind).collect();

            for kind in grammar_named_kinds(language) {
                if looks_like_declaration_candidate(&kind) && !classified.contains(kind.as_str()) {
                    unclassified.push(format!("[{}] {}", language, kind));
                }
            }
        }

        assert!(
            unclassified.is_empty(),
            "以下 grammar 节点像声明但没人表态（新增节点或漏写清单）：\n{}",
            unclassified.join("\n")
        );
    }

    /// `.scm` 文本扫描器的正反用例
    #[test]
    fn test_query_node_names_extraction() {
        let scm = r#"
;; 注释里的 (function_item) 不该被算进去
(enumerator_list
  (enumerator
    name: (identifier) @name) @enumerator)

(function_definition
  declarator: (_) @declarator) @func

(class_specifier name: [(type_identifier) @name (template_type name: (type_identifier) @name)]) @class
"#;
        let names = query_node_names(scm);
        for expect in [
            "enumerator_list",
            "enumerator",
            "identifier",
            "function_definition",
            "class_specifier",
            "type_identifier",
            "template_type",
        ] {
            assert!(names.contains(expect), "应提取到节点名 {}：{:?}", expect, names);
        }
        assert!(!names.contains("name"), "字段名 name 不该被当成节点名");
        assert!(!names.contains("function_item"), "注释里的节点名不该被算进去");
        assert!(!names.contains("_"), "通配符 _ 不该被当成节点名");
    }

    // ------------------------------------------------------------------
    // B：审计机制的正反用例
    // ------------------------------------------------------------------

    /// 造一个 C++ fixture，解析出符号后审计
    fn cpp_audit(source: &str) -> (Vec<Symbol>, FileCoverage) {
        use crate::cpp::CppParser;
        use crate::r#trait::LanguageParser;
        let parser = CppParser::new();
        let tree = parser.parse(source).expect("解析失败");
        let symbols = parser.extract_symbols(&tree, source, std::path::Path::new("t.cpp"));
        let coverage = audit("cpp", &tree, &symbols);
        (symbols, coverage)
    }

    /// ★ B 的核心不变量：枚举量要么**产出了符号**，要么**被上报为缺口** —— 不许两者都没有
    ///
    /// 这条断言在 @enumerator 捕获落地前后都成立：
    /// - 落地前：`Up` 查不到，但 `enumerator` 出现在 `kinds_without_symbols` 里（安全）
    /// - 落地后：`Up` 是一个符号，审计自然不再报它
    #[test]
    fn test_enum_members_are_never_silently_missing() {
        let source = "enum class Dir { Up, Down };\nenum Color { RED, GREEN };\n";
        let (symbols, coverage) = cpp_audit(source);

        let has_member_symbol = symbols.iter().any(|s| s.name == "Up");
        let reported = coverage.kinds_without_symbols.iter().any(|k| k == "enumerator");

        assert!(
            has_member_symbol || reported,
            "枚举量既没产出符号、也没被覆盖度审计上报 —— 这正是要防的静默缺口。\n\
             符号: {:?}\n覆盖度: {:?}",
            symbols.iter().map(|s| &s.name).collect::<Vec<_>>(),
            coverage
        );
    }

    /// 审计会说人话：note 里必须出现「不是不存在」这类反误读的提示
    #[test]
    fn test_audit_note_explains_the_zero() {
        // 构造一个「有 field_declaration 节点但一个符号都没产出」的场景：
        // C++ 类里的数据成员目前不建索引，审计必须把它挑出来
        let source = "class S {\n  int x;\n  int y;\n};\n";
        let (_, coverage) = cpp_audit(source);

        assert!(
            coverage.has_gaps(),
            "只有数据成员的类应被审计出缺口: {:?}",
            coverage
        );
        let note = coverage.note.as_deref().unwrap_or("");
        assert!(
            note.contains("没建索引") && note.contains("不存在"),
            "note 必须直接写明「这个 0 是没建索引、不是不存在」: {}",
            note
        );
        assert!(
            coverage.details.iter().any(|d| d.kind == "field_declaration"),
            "应定位到 field_declaration: {:?}",
            coverage.details
        );
    }

    /// 审计不能误报：真正的函数定义/类定义产出符号后，不该出现在缺口里
    #[test]
    fn test_audit_does_not_flag_covered_kinds() {
        let source = "class S { public: void f(); };\nvoid g() {}\n";
        let (_, coverage) = cpp_audit(source);

        for kind in &coverage.kinds_without_symbols {
            assert_ne!(kind, "class_specifier", "类已产出符号，不该报缺口");
        }
        assert!(
            coverage.declaration_kinds_seen.contains(&"class_specifier".to_string()),
            "声明种类清单应包含 class_specifier: {:?}",
            coverage.declaration_kinds_seen
        );
    }

    /// ★ 8 个语言的枚举量端到端：解析出 `SymbolKind::Constant`，且行号与源码一致，
    /// 并且审计不再把它报成缺口
    ///
    /// JavaScript 不在表里：标准 JS 没有 enum 语法（tree-sitter-javascript 的
    /// node-types.json 里 "enum" 出现 0 次），属于「本语言显式不适用」。
    #[test]
    fn test_enum_members_across_languages() {
        use crate::r#trait::LanguageParser;

        /// (语言, 源码, 期望命中的枚举量名, 期望行号)
        const CASES: &[(&str, &str, &str, u64)] = &[
            ("rust", "enum Color {\n    Red,\n    Green,\n}\n", "Green", 3),
            ("typescript", "enum Color {\n  Red,\n  Green,\n}\n", "Green", 3),
            ("java", "enum Color {\n  RED,\n  GREEN\n}\n", "GREEN", 3),
            ("csharp", "enum Color {\n  Red,\n  Green,\n}\n", "Green", 3),
            ("c", "enum Color { RED,\n  GREEN };\n", "GREEN", 2),
            ("cpp", "enum class Dir { Up,\n  Down };\n", "Down", 2),
        ];

        for &(language, source, member, line) in CASES {
            let parser: std::sync::Arc<dyn LanguageParser> = match language {
                "rust" => std::sync::Arc::new(crate::rust::RustParser::new()),
                "typescript" => std::sync::Arc::new(crate::typescript::TypeScriptParser::new()),
                "java" => std::sync::Arc::new(crate::java::JavaParser::new()),
                "csharp" => std::sync::Arc::new(crate::csharp::CSharpParser::new()),
                "c" => std::sync::Arc::new(crate::c::CParser::new()),
                "cpp" => std::sync::Arc::new(crate::cpp::CppParser::new()),
                other => panic!("未登记的语言: {}", other),
            };

            let tree = parser.parse(source).expect("解析失败");
            let symbols = parser.extract_symbols(&tree, source, std::path::Path::new("t"));
            let coverage = parser.audit_coverage(&tree, &symbols);

            let found = symbols.iter().find(|s| s.name == member).unwrap_or_else(|| {
                panic!(
                    "[{}] 枚举量 `{}` 没被索引 —— 符号: {:?}",
                    language,
                    member,
                    symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
                )
            });
            assert_eq!(
                found.kind,
                codeconnect_core::types::SymbolKind::Constant,
                "[{}] 枚举量 `{}` 的 kind 应为 Constant",
                language,
                member
            );
            assert_eq!(
                found.location.line, line,
                "[{}] 枚举量 `{}` 的行号应为 {}，实际 {}",
                language, member, line, found.location.line
            );
            assert!(
                !coverage.kinds_without_symbols.iter().any(|k| k.starts_with("enum")),
                "[{}] 枚举量已产出符号，审计不该再把它算作缺口: {:?}",
                language,
                coverage.kinds_without_symbols
            );
        }
    }

    /// 带注解/属性的枚举量：符号位置必须落在**成员节点**起点，而不是名字标识符起点
    ///
    /// Java 的 `@Deprecated OLD`、C# 的 `[Obsolete] Legacy` 里，`enum_constant` /
    /// `enum_member_declaration` 节点从 `@` / `[` 开始，identifier 在它们之后。
    /// 若查询把 `@enumerator` 挂在 identifier 上，符号位置就会与成员节点起点错开，
    /// 审计会把自己的产品误报成「该种类 0 个符号」。
    #[test]
    fn test_enum_members_with_modifiers_stay_aligned() {
        use crate::r#trait::LanguageParser;

        /// (语言, 源码, 成员名, 期望行号, 成员节点种类)
        const CASES: &[(&str, &str, &str, u64, &str)] = &[
            (
                "java",
                "enum Annotated {\n  @Deprecated\n  OLD,\n  FRESH\n}\n",
                "OLD",
                3,
                "enum_constant",
            ),
            (
                "csharp",
                "enum Tagged {\n  [Obsolete]\n  Legacy,\n  Current\n}\n",
                "Legacy",
                3,
                "enum_member_declaration",
            ),
            ("cpp", "enum class E {\n  A = 1,\n  B\n};\n", "B", 3, "enumerator"),
        ];

        for &(language, source, member, line, decl_kind) in CASES {
            let parser: std::sync::Arc<dyn LanguageParser> = match language {
                "java" => std::sync::Arc::new(crate::java::JavaParser::new()),
                "csharp" => std::sync::Arc::new(crate::csharp::CSharpParser::new()),
                "cpp" => std::sync::Arc::new(crate::cpp::CppParser::new()),
                other => panic!("未登记的语言: {}", other),
            };

            let tree = parser.parse(source).expect("解析失败");
            let symbols = parser.extract_symbols(&tree, source, std::path::Path::new("t"));
            let coverage = parser.audit_coverage(&tree, &symbols);

            let found = symbols
                .iter()
                .find(|s| s.name == member)
                .unwrap_or_else(|| {
                    panic!(
                        "[{}] 枚举量 `{}` 没被索引 —— 符号: {:?}",
                        language,
                        member,
                        symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
                    )
                });
            assert_eq!(
                found.location.line, line,
                "[{}] 带修饰符的枚举量 `{}` 行号应为 {}，实际 {}",
                language, member, line, found.location.line
            );
            assert!(
                !coverage
                    .kinds_without_symbols
                    .iter()
                    .any(|k| k == decl_kind),
                "[{}] `{}` 已产出符号，审计不该把它算作缺口（说明 @enumerator 的位置与成员节点错开了）: {:?}",
                language,
                decl_kind,
                coverage.details
            );
        }
    }

    /// 豁免项不参与审计（Exempt 的节点即使一个符号都没有也不出声）
    #[test]
    fn test_exempt_kinds_are_silent() {
        // `#include <a.h>` 的 preproc_include 不在表态表里；这里用 C 的
        // storage_class_specifier（Exempt）验证豁免项确实不参与
        use crate::c::CParser;
        use crate::r#trait::LanguageParser;
        let source = "static int helper(void) { return 1; }\n";
        let parser = CParser::new();
        let tree = parser.parse(source).expect("解析失败");
        let symbols = parser.extract_symbols(&tree, source, std::path::Path::new("t.c"));
        let coverage = audit("c", &tree, &symbols);

        assert!(
            !coverage
                .kinds_without_symbols
                .iter()
                .any(|k| k == "storage_class_specifier"),
            "Exempt 的节点不该出现在缺口里: {:?}",
            coverage.kinds_without_symbols
        );
    }

    /// 合并汇总：多个文件的缺口按节点种类归并
    #[test]
    fn test_merge_into_aggregates_by_kind() {
        let mut acc: BTreeMap<String, KindGapSummary> = BTreeMap::new();
        let mut c1 = FileCoverage::default();
        c1.details.push(KindCoverage {
            kind: "enumerator".into(),
            nodes: 2,
            symbols: 0,
            captured_by_query: false,
            status: "not_captured".into(),
            reason: None,
        });
        let mut c2 = FileCoverage::default();
        c2.details.push(KindCoverage {
            kind: "enumerator".into(),
            nodes: 3,
            symbols: 0,
            captured_by_query: false,
            status: "not_captured".into(),
            reason: None,
        });

        merge_into(&mut acc, "a.cpp", &c1);
        merge_into(&mut acc, "b.cpp", &c2);

        let entry = acc.get("enumerator").expect("应汇总出 enumerator");
        assert_eq!(entry.node_count, 5);
        assert_eq!(entry.files, 2);
        assert_eq!(entry.sample_files, vec!["a.cpp", "b.cpp"]);
    }
}
