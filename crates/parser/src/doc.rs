//! 签名（signature）与文档注释（doc_comment）提取 —— 7 个语言解析器共用
//!
//! 两条核心规则：
//!
//! 1. **signature 是单行、空白折叠的声明文本，不含函数体**。
//!    截断点优先取 `body` 字段的起点，其次取「不在括号内的第一个 `{`」。
//!
//! 2. **doc_comment 只认紧邻声明上方的文档注释**。
//!    判定完全基于语法树兄弟节点 + 行号相邻性：注释与声明之间**隔了空行**
//!    或**隔了别的声明**一律返回 `None`（最近邻注释 ≠ 文档注释，这是最容易写错的地方）。
//!    普通注释（`//`、`/* */`）不算文档注释，也不得冒充。

use tree_sitter::Node;

/// signature 字符数上限
const SIGNATURE_MAX_CHARS: usize = 200;
/// doc_comment 字符数上限
const DOC_MAX_CHARS: usize = 200;

/// 注释节点的 kind。
/// tree-sitter-rust / tree-sitter-java 用 `line_comment` + `block_comment`，
/// C / C++ / C# / JavaScript / TypeScript 统一叫 `comment`（实测确认）。
const COMMENT_KINDS: &[&str] = &["line_comment", "block_comment", "comment"];

/// 向上回溯时**不能跨过**的容器节点 —— 走到这一层，注释就是声明节点的兄弟。
///
/// 判定依据：容器节点通常起于更早的行（`{`、`impl Name {` 等），
/// 但 `class_body` 与 `class_declaration` 同行，因此不能只靠行号，必须显式列名。
const CONTAINER_KINDS: &[&str] = &[
    "source_file",
    "program",
    "translation_unit",
    "compilation_unit",
    "module",
    "internal_module",
    "ambient_declaration",
    "declaration_list",
    "class_body",
    "interface_body",
    "enum_body",
    "enum_variant_list",
    "enum_member_declaration_list",
    "field_declaration_list",
    "enumerator_list",
    "block",
    "statement_block",
    "compound_statement",
    "namespace_declaration",
    "modifiers",
    "attribute_list",
    "parameter_list",
    "formal_parameters",
    "arguments",
];

/// 文档注释形态 —— 各语言差异只在这里体现
#[derive(Debug, Clone, Copy)]
pub struct DocSyntax {
    /// `///` 行注释算文档注释（Rust / C / C++ / C#）
    pub triple_slash: bool,
    /// `/** ... */` 块注释算文档注释（Rust / Java / C / C++ / C# / JS / TS）
    pub javadoc_block: bool,
    /// `//!` 内部文档（仅 Rust）
    pub inner_doc: bool,
}

/// Rust：`///`、`//!`、`#[doc = "..."]`、`/** */`
pub const RUST_DOC: DocSyntax = DocSyntax {
    triple_slash: true,
    javadoc_block: true,
    inner_doc: true,
};

/// C / C++ / C#：`///` 与 `/** */`
pub const TRIPLE_SLASH_DOC: DocSyntax = DocSyntax {
    triple_slash: true,
    javadoc_block: true,
    inner_doc: false,
};

/// Java / TypeScript / JavaScript：只认 `/** */`（Javadoc / JSDoc）
pub const BLOCK_ONLY_DOC: DocSyntax = DocSyntax {
    triple_slash: false,
    javadoc_block: true,
    inner_doc: false,
};

/// 一次取出声明节点的 signature 与 doc_comment
///
/// 各解析器在产出 `Symbol` 时调用。
pub fn extract(
    node: Node,
    source: &str,
    syntax: DocSyntax,
    symbol_name: &str,
) -> (Option<String>, Option<String>) {
    let signature = signature_of(node, source);
    let doc_comment = doc_comment_before(node, source, syntax);

    // 调试日志走 tracing → stderr，不会污染 MCP 的 stdout 协议
    tracing::debug!(
        symbol = symbol_name,
        node_kind = node.kind(),
        line = node.start_position().row + 1,
        signature = signature.as_deref().unwrap_or(""),
        doc = doc_comment.as_deref().unwrap_or(""),
        "签名/文档注释提取"
    );

    (signature, doc_comment)
}

/// 提取单行签名，不含函数体
pub fn signature_of(node: Node, source: &str) -> Option<String> {
    let start = node.start_byte();
    let mut end = node.end_byte();

    // 截断点 1：body 字段（各 grammar 对函数体/类体统一用 body 字段名）
    if let Some(body) = node.child_by_field_name("body") {
        end = end.min(body.start_byte());
    }
    // 截断点 2：不在括号内的第一个 `{`
    // （prototype、宏等没有 body 字段；括号层级用于避开 TS 参数里的对象类型 `{a: number}`）
    if let Some(brace) = first_top_level_brace(&source[start..end]) {
        end = end.min(start + brace);
    }

    let text = collapse_ws(&source[start..end]);
    // 声明末尾的 `;` / `,` 不属于签名
    let text = text.trim_end_matches([';', ',']).trim_end();
    let text = truncate_chars(text, SIGNATURE_MAX_CHARS);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// 提取紧邻声明之上的文档注释；隔空行 / 隔别的声明 / 只有普通注释 → `None`
pub fn doc_comment_before(node: Node, source: &str, syntax: DocSyntax) -> Option<String> {
    let anchor = climb_to_statement(node);
    let mut parts: Vec<String> = Vec::new();
    // 下一个（更靠上的）兄弟节点的结束行必须紧邻当前锚点行，中间不许有空行
    let mut expected_row = anchor.start_position().row;
    let mut cur = anchor;

    while let Some(prev) = cur.prev_named_sibling() {
        let text = prev.utf8_text(source.as_bytes()).unwrap_or("");
        if last_content_row(prev, text) + 1 != expected_row {
            break; // 隔着空行
        }

        if COMMENT_KINDS.contains(&prev.kind()) {
            match doc_text_of(text, syntax) {
                Some(t) => parts.push(t),
                None => break, // 普通注释：不认，且到此为止（不得拿它冒充文档注释）
            }
        } else if prev.kind() == "attribute_item" || prev.kind() == "attribute" {
            // Rust 的 `#[doc = "..."]` 算文档；其余属性（#[derive] 等）透明跳过，
            // 这样 `/// 文档` + `#[derive(Debug)]` + 声明的组合仍能取到文档
            if let Some(t) = doc_attr_text(text) {
                parts.push(t);
            }
        } else {
            break; // 隔着别的声明
        }

        expected_row = prev.start_position().row;
        cur = prev;
    }

    if parts.is_empty() {
        return None;
    }
    parts.reverse();
    let joined = truncate_chars(&collapse_ws(&parts.join(" ")), DOC_MAX_CHARS);
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// 把声明节点上爬到「注释真正的兄弟层级」
///
/// 例：TS 的 `/** 文档 */ export function f()`，注释挂在 `export_statement` 上，
/// 而查询捕获的是内部的 `function_declaration`；箭头函数的 `arrow_function`
/// 也要先经 `variable_declarator` 爬回 `lexical_declaration`。
///
/// 上爬条件：父节点不是容器，且与当前节点**起始行相同**（说明父节点没有向上扩张）。
fn climb_to_statement(mut node: Node) -> Node {
    while let Some(parent) = node.parent() {
        if CONTAINER_KINDS.contains(&parent.kind()) {
            break;
        }
        if parent.start_position().row != node.start_position().row {
            break;
        }
        node = parent;
    }
    node
}

/// 节点最后一行的**内容行号**
///
/// tree-sitter-rust 的 `line_comment` 把行尾换行符也算进节点范围
/// （`/// 文档\n` 的 end_position 落在下一行行首），直接用 `end_position().row`
/// 会把「紧邻」误判成「隔空行」。
fn last_content_row(node: Node, text: &str) -> usize {
    if text.ends_with('\n') {
        node.end_position().row.saturating_sub(1)
    } else {
        node.end_position().row
    }
}

/// 跳过括号层级后的第一个 `{` 的字节偏移；没有则 `None`
fn first_top_level_brace(text: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    for (idx, ch) in text.char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '{' if depth <= 0 => return Some(idx),
            _ => {}
        }
    }
    None
}

/// 注释文本 → 文档注释正文；不是文档注释返回 `None`
fn doc_text_of(text: &str, syntax: DocSyntax) -> Option<String> {
    let t = text.trim();

    // 块注释 `/** ... */`
    if t.starts_with("/*") {
        if !syntax.javadoc_block || !t.starts_with("/**") || t.starts_with("/**/") {
            return None; // `/* */` 与空注释 `/**/` 不是文档注释
        }
        let inner = t.strip_prefix("/**").unwrap_or(t);
        let inner = inner.strip_suffix("*/").unwrap_or(inner);
        // 去掉每行行首的 `*` 装饰
        let lines: Vec<&str> = inner
            .lines()
            .map(|l| l.trim().trim_start_matches('*').trim())
            .filter(|l| !l.is_empty())
            .collect();
        return Some(lines.join(" "));
    }

    // 行注释 `/// ...`
    if let Some(rest) = t.strip_prefix("///") {
        if !syntax.triple_slash {
            return None;
        }
        // Rust 里 `////` 是普通注释，不是文档注释
        if rest.starts_with('/') {
            return None;
        }
        return Some(rest.trim().to_string());
    }

    // 行注释 `//! ...`
    if let Some(rest) = t.strip_prefix("//!") {
        if !syntax.inner_doc {
            return None;
        }
        return Some(rest.trim().to_string());
    }

    None
}

/// `#[doc = "..."]` / `#[doc = r"..."]` → 字符串内容；其它属性返回 `None`
fn doc_attr_text(text: &str) -> Option<String> {
    let t = text.trim();
    if !t.starts_with("#[doc") {
        return None;
    }
    let start = t.find('"')?;
    let rest = &t[start + 1..];
    let end = rest.find('"')?;
    let s = rest[..end].trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 把连续空白折叠成单个空格并去掉首尾空白
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 按字符数截断（避免切断 UTF-8）
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_ws() {
        assert_eq!(collapse_ws("  a \n\t b  "), "a b");
    }

    #[test]
    fn test_first_top_level_brace_skips_parens() {
        // 参数里的对象类型 `{a: number}` 不算函数体
        let text = "function f(o: {a: number}) {";
        let idx = first_top_level_brace(text).unwrap();
        assert_eq!(&text[idx..], "{");
        assert!(idx > text.find("number").unwrap());
    }

    #[test]
    fn test_truncate_chars_is_utf8_safe() {
        let s = "中".repeat(300);
        let out = truncate_chars(&s, 200);
        assert_eq!(out.chars().count(), 201); // 200 + 省略号
    }

    #[test]
    fn test_doc_text_plain_comments_rejected() {
        assert!(doc_text_of("// 普通注释", RUST_DOC).is_none());
        assert!(doc_text_of("/* 普通块注释 */", RUST_DOC).is_none());
        assert!(doc_text_of("/**/", RUST_DOC).is_none());
        // Rust 的 `////` 是普通注释
        assert!(doc_text_of("//// 四个斜杠", RUST_DOC).is_none());
    }

    #[test]
    fn test_doc_text_java_block_only_rejects_triple_slash() {
        assert!(doc_text_of("/// 不是 Java 文档", BLOCK_ONLY_DOC).is_none());
        assert_eq!(
            doc_text_of("/** Javadoc */", BLOCK_ONLY_DOC).as_deref(),
            Some("Javadoc")
        );
    }

    #[test]
    fn test_doc_attr_text() {
        assert_eq!(
            doc_attr_text("#[doc = \"属性文档\"]").as_deref(),
            Some("属性文档")
        );
        assert!(doc_attr_text("#[derive(Debug)]").is_none());
    }
}
