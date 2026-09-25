//! 语言解析器统一接口
//!
//! 所有语言解析器须实现此 trait，确保索引引擎能以多态方式处理不同语言。
//! 每个解析器负责：解析源码为 AST → 提取符号/调用/导入 → 输出结构化数据。

use std::path::Path;
use codeconnect_core::error::CodeConnectError;
use codeconnect_core::types::{CallSite, Import, Symbol};
use tree_sitter::Tree;

/// 语言解析器统一接口
pub trait LanguageParser: Send + Sync {
    /// 返回此解析器支持的语言名称（如 "rust", "java"）
    fn language(&self) -> &'static str;

    /// 返回此解析器支持的文件扩展名列表（如 &["rs"]）
    fn file_extensions(&self) -> &[&str];

    /// 解析源代码为 tree-sitter 语法树
    fn parse(&self, source: &str) -> Result<Tree, CodeConnectError>;

    /// 从语法树中提取所有符号定义（函数、类、结构体、变量等）
    fn extract_symbols(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &Path,
    ) -> Vec<Symbol>;

    /// 从语法树中提取所有函数/方法调用点
    fn extract_calls(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &Path,
    ) -> Vec<CallSite>;

    /// 从语法树中提取所有 import/use/include 语句
    fn extract_imports(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &Path,
    ) -> Vec<Import>;

    /// 从文件路径和源码推断模块/包名
    fn infer_package(&self, file_path: &Path, source: &str) -> Option<String>;

    /// 审计本文件的声明节点覆盖度（spec B）
    ///
    /// 把「AST 里存在、属于声明类节点」的种类与「真的产出了符号」的种类做差集，
    /// 差集非空即上浮 —— 避免「没索引」被调用方读成「不存在」。
    ///
    /// 默认实现对所有语言通用（清单与判定都在 [`crate::coverage`]），
    /// 各语言解析器无需覆写。
    fn audit_coverage(&self, tree: &Tree, symbols: &[Symbol]) -> crate::coverage::FileCoverage {
        crate::coverage::audit(self.language(), tree, symbols)
    }
}
