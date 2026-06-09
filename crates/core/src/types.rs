//! 核心数据类型定义
//!
//! 包含整个 CodeConnect 项目共享的符号、调用点、导入信息、
//! 图边等数据结构。所有类型均派生 Serialize/Deserialize 以支持
//! 序列化存储和 MCP 消息传递。

use serde::{Deserialize, Serialize};

// ============================================================================
// 源码位置
// ============================================================================

/// 源码中的精确位置信息
///
/// 使用行号+列号定位，支持范围表示（从起始到结束）。
/// 行号和列号均为 1-based。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceLocation {
    /// 相对于项目根目录的文件路径
    pub file_path: String,
    /// 起始行号（1-based）
    pub line: u64,
    /// 起始列号（1-based）
    pub column: u64,
    /// 结束行号（1-based）
    pub end_line: u64,
    /// 结束列号（1-based）
    pub end_column: u64,
}

// ============================================================================
// 符号相关
// ============================================================================

/// 符号类型枚举
///
/// 涵盖主流语言的符号种类，Unknown 变体用于处理未识别的符号类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    /// 函数（顶层函数或自由函数）
    Function,
    /// 方法（附加在类型上的函数）
    Method,
    /// 类（面向对象语言中的 class）
    Class,
    /// 接口（interface/trait 的统称，语言无关）
    Interface,
    /// 结构体（Rust struct, Go struct 等）
    Struct,
    /// 枚举
    Enum,
    /// 特质（Rust trait 专用）
    Trait,
    /// 类型别名
    TypeAlias,
    /// 变量
    Variable,
    /// 字段（结构体或类的成员字段）
    Field,
    /// 参数
    Parameter,
    /// 模块（module / namespace）
    Module,
    /// 宏（Rust macro, C #define 等）
    Macro,
    /// 未知类型 — 保留原始类型名用于日志/调试
    Unknown(String),
}

/// 核心符号结构
///
/// 表示代码中的单个符号（函数、类、变量等），包含位置信息、
/// 签名、文档注释、可见性等元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    /// 稳定的全局唯一符号 ID（由 StableSymbolId 生成）
    pub id: String,
    /// 符号名称（源代码中的标识符）
    pub name: String,
    /// 符号类型
    pub kind: SymbolKind,
    /// 源码位置
    pub location: SourceLocation,
    /// 函数/方法签名（可选）
    pub signature: Option<String>,
    /// 文档注释（可选）
    pub doc_comment: Option<String>,
    /// 父符号 ID（例如方法的父类是所在的类）
    pub parent_id: Option<String>,
    /// 修饰符列表（public, static, async, const 等）
    pub modifiers: Vec<String>,
    /// 是否被导出（公开 API）
    pub is_exported: bool,
    /// 圈复杂度（可选，由分析器计算）
    pub complexity: Option<u64>,
}

// ============================================================================
// 调用关系
// ============================================================================

/// 调用类型枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CallType {
    /// 直接调用
    Direct,
    /// 虚函数/动态分发调用
    Virtual,
    /// 回调函数调用
    Callback,
    /// 宏展开调用
    MacroExpansion,
    /// 无法确定的调用类型
    Unknown,
}

/// 调用点
///
/// 记录调用者符号在某个位置调用了某个被调用者名称。
/// 注意：此处的 callee_name 是解析前的名称，最终会通过
/// 解析过程转换为 CallEdge。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallSite {
    /// 调用者符号 ID
    pub caller_id: String,
    /// 被调用的符号名称（解析前）
    pub callee_name: String,
    /// 调用发生的源码位置
    pub location: SourceLocation,
    /// 调用类型
    pub call_type: CallType,
    /// 置信度 0.0-1.0
    pub confidence: f64,
}

/// 调用边
///
/// 经过解析后的调用关系，caller_id 和 callee_id 均已确定。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallEdge {
    /// 调用者符号 ID
    pub caller_id: String,
    /// 被调用者符号 ID
    pub callee_id: String,
    /// 调用位置
    pub location: SourceLocation,
    /// 调用类型
    pub call_type: CallType,
    /// 置信度 0.0-1.0
    pub confidence: f64,
}

// ============================================================================
// 引用关系
// ============================================================================

/// 引用类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RefType {
    /// 读取引用
    Read,
    /// 写入引用
    Write,
    /// 读写引用
    ReadWrite,
    /// 类型注解中的引用
    TypeAnnotation,
    /// 参数传递
    ParameterPass,
    /// 返回值
    ReturnValue,
}

/// 引用边
///
/// 表示一个符号引用了另一个符号的关系。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefEdge {
    /// 引用者符号 ID
    pub ref_id: String,
    /// 被引用符号 ID
    pub target_id: String,
    /// 引用发生的位置
    pub location: SourceLocation,
    /// 引用类型
    pub ref_type: RefType,
}

// ============================================================================
// 导入信息
// ============================================================================

/// 导入解析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImportResolution {
    /// 已成功解析到目标文件或符号 ID
    Resolved(String),
    /// 解析为外部包（如 crates.io 包、npm 包）
    External(String),
    /// 无法解析
    Unresolved,
}

/// 导入信息
///
/// 记录文件中 import/use 语句的信息及解析结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    /// 导入语句所在的文件路径
    pub file_path: String,
    /// 导入路径（如 std::collections::HashMap）
    pub import_path: String,
    /// 别名（如 use Foo as Bar 中的 Bar）
    pub alias: Option<String>,
    /// 导入语句所在行号
    pub line: u64,
    /// 导入解析结果
    pub resolution: ImportResolution,
}

// ============================================================================
// 文件元信息
// ============================================================================

/// 文件元信息
///
/// 记录已索引文件的元数据，用于增量更新和缓存管理。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    /// 相对于项目根目录的文件路径
    pub file_path: String,
    /// 编程语言
    pub language: String,
    /// 文件内容的 blake3 哈希（用于检测变更）
    pub content_hash: String,
    /// 文件中包含的符号数量
    pub symbol_count: u64,
    /// 索引时间戳（Unix 秒）
    pub indexed_at: i64,
}
